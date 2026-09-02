//! stdio↔SSE 桥接:自动 spawn 共享 SSE server,转发 JSON-RPC 消息。
//!
//! 进程模型:
//! - SSE server 子进程:第一个会话 spawn,常驻,加载索引
//! - 桥接进程:每个会话一个,极小,只做转发

use std::time::Duration;

use rmcp::service::TxJsonRpcMessage;
use rmcp::transport::Transport;
use rmcp::RoleClient;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// 默认 SSE server 端口
pub const DEFAULT_SSE_PORT: u16 = 9000;

/// 检测 SSE server 是否在运行(HTTP GET /sse,超时 1 秒)
pub async fn check_sse_server(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/sse");
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    match client.get(&url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// Spawn 一个 detached SSE server 子进程。
///
/// 子进程用相同的 exe + `--transport sse --port <port>` 启动,
/// 继承原始的 --urls 参数。stdin/stdout/stderr 设为 Null(detached)。
pub fn spawn_sse_server(port: u16, original_args: &[String]) -> anyhow::Result<()> {
    let current_exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(&current_exe);
    cmd.arg("--transport")
        .arg("sse")
        .arg("--port")
        .arg(port.to_string());

    // 传递原始参数(过滤掉 --transport 和 --port,避免重复)
    let mut skip = false;
    for arg in original_args {
        if skip {
            skip = false;
            continue;
        }
        match arg.as_str() {
            "--transport" | "--port" => {
                skip = true;
                continue;
            }
            _ => {
                cmd.arg(arg);
            }
        }
    }

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    // Windows: CREATE_NO_WINDOW + DETACHED_PROCESS
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }

    cmd.spawn()?;
    Ok(())
}

/// 重试连接 SSE server,最多等待 timeout。
pub async fn wait_for_sse_server(port: u16, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if check_sse_server(port).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// 启动 stdio↔SSE 桥接。
///
/// 从 stdin 读 JSON-RPC 消息 → 转发给 SSE client transport
/// 从 SSE client transport 读响应 → 写到 stdout
///
/// stdin EOF → 返回(桥接进程退出,SSE server 不受影响)
pub async fn run_bridge(sse_port: u16) -> anyhow::Result<()> {
    let sse_url = format!("http://127.0.0.1:{sse_port}/sse");

    // 连接 SSE server
    let sse_transport = rmcp::transport::SseClientTransport::start(sse_url.clone())
        .await
        .map_err(|e| anyhow::anyhow!("failed to connect SSE server at {sse_url}: {e}"))?;

    tracing::info!("bridge: connected to SSE server at {sse_url}");

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut transport = sse_transport;

    let mut line = String::new();

    loop {
        tokio::select! {
            // stdin → SSE
            read_result = reader.read_line(&mut line) => {
                match read_result {
                    Ok(0) => {
                        // EOF
                        tracing::info!("bridge: stdin EOF, exiting");
                        break;
                    }
                    Ok(_) => {
                        let trimmed = line.trim_end_matches('\n');
                        if trimmed.is_empty() {
                            line.clear();
                            continue;
                        }
                        // 解析 JSON-RPC 并转发
                        match serde_json::from_str::<serde_json::Value>(trimmed) {
                            Ok(json) => {
                                match serde_json::from_value::<TxJsonRpcMessage<RoleClient>>(json) {
                                    Ok(msg) => {
                                        if let Err(e) = transport.send(msg).await {
                                            tracing::warn!("bridge: SSE send failed: {e}");
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("bridge: failed to parse JSON-RPC: {e}");
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("bridge: invalid JSON on stdin: {e}");
                            }
                        }
                        line.clear();
                    }
                    Err(e) => {
                        tracing::warn!("bridge: stdin read error: {e}");
                        break;
                    }
                }
            }
            // SSE → stdout
            msg = transport.receive() => {
                match msg {
                    Some(rx_msg) => {
                        match serde_json::to_string(&rx_msg) {
                            Ok(json) => {
                                if let Err(e) = stdout.write_all(json.as_bytes()).await {
                                    tracing::warn!("bridge: stdout write failed: {e}");
                                    break;
                                }
                                if let Err(e) = stdout.write_all(b"\n").await {
                                    tracing::warn!("bridge: stdout write newline failed: {e}");
                                    break;
                                }
                                let _ = stdout.flush().await;
                            }
                            Err(e) => {
                                tracing::warn!("bridge: failed to serialize SSE message: {e}");
                            }
                        }
                    }
                    None => {
                        tracing::info!("bridge: SSE stream closed");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}
