//! stdio↔SSE 桥接:自动 spawn 共享 SSE server,转发 JSON-RPC 消息。
//!
//! 进程模型:
//! - SSE server 子进程:第一个会话 spawn,常驻,加载索引
//! - 桥接进程:每个会话一个,极小,只做转发
//!
//! 端口选择:
//! - 默认从 9123 开始,如果被占用就递增到 9124、9125...
//! - 选定端口后写入 exe 同目录的 mcpdoc-rust.port 文件
//! - 后续桥接进程读文件确定端口,复用同一个 SSE server
//! - 端口文件会过期(如果 server 崩溃,文件里的端口对应的 server 不在了)

use std::time::Duration;

use rmcp::service::TxJsonRpcMessage;
use rmcp::transport::Transport;
use rmcp::RoleClient;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// 默认起始端口(冷门端口,避免与常见服务冲突)
pub const PORT_RANGE_START: u16 = 9123;
pub const PORT_RANGE_END: u16 = 9199;

/// 端口文件名(写在 exe 同目录)
const PORT_FILE: &str = "mcpdoc-rust.port";

/// 获取端口文件路径(exe 同目录)
fn port_file_path() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join(PORT_FILE)))
}

/// 读取端口文件中记录的端口
fn read_port_file() -> Option<u16> {
    let path = port_file_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    content.trim().parse::<u16>().ok()
}

/// 写入端口到文件
fn write_port_file(port: u16) {
    if let Some(path) = port_file_path() {
        let _ = std::fs::write(&path, port.to_string());
    }
}

/// 检测端口上是否有我们的 SSE server(TCP 连接 + HTTP /sse 验证)。
///
/// TCP 连接成功只说明端口有东西在监听,还需要验证它是不是我们的 SSE server。
/// 用 HTTP GET /sse 检查:如果返回 text/event-stream 则是我们的 server。
/// 注意:用 reqwest 的 timeout + 只读响应头,不读 body(SSE 流不关闭)。
async fn is_our_sse_server(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/sse");
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    match client.get(&url).send().await {
        Ok(resp) => {
            // 检查 content-type 是否为 text/event-stream
            resp.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.contains("text/event-stream"))
                .unwrap_or(false)
        }
        Err(_) => false,
    }
}

/// 简单 TCP 端口占用检测(不验证是否为 SSE server)
fn is_port_in_use(port: u16) -> bool {
    std::net::TcpListener::bind(format!("127.0.0.1:{port}")).is_err()
}

/// 查找可用的空闲端口(从 start 到 end 递增)
fn find_free_port() -> Option<u16> {
    (PORT_RANGE_START..=PORT_RANGE_END).find(|&port| !is_port_in_use(port))
}

/// Spawn 一个 detached SSE server 子进程。
///
/// 子进程用相同的 exe + `--transport sse --port <port>` 启动,
/// 继承原始的 --urls 参数。stdin/stdout/stderr 设为 detached。
pub fn spawn_sse_server(port: u16, original_urls: &[String]) -> anyhow::Result<()> {
    let current_exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(&current_exe);
    cmd.arg("--transport")
        .arg("sse")
        .arg("--port")
        .arg(port.to_string());

    // original_urls 是 --urls 后面的值列表(clap 解析后的 Vec<String>)
    // 子进程需要 --urls url1 url2 ...
    if !original_urls.is_empty() {
        cmd.arg("--urls");
        for url in original_urls {
            cmd.arg(url);
        }
    }

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit());

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

/// 重试验证 SSE server 已就绪,最多等待 timeout。
async fn wait_for_sse_server(port: u16, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if is_our_sse_server(port).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// 尝试连接或 spawn SSE server,返回可用的端口。
///
/// 逻辑:
/// 1. 读端口文件 → 有端口 → 验证是否为我们的 SSE server → 是则复用
/// 2. 端口文件没有或验证失败 → 找空闲端口 → spawn → 等待就绪 → 写端口文件
/// 3. spawn 失败 → 返回 None(调用方 fallback 到直接 stdio)
pub async fn ensure_sse_server(urls: &[String]) -> Option<u16> {
    // 1. 尝试从端口文件读
    if let Some(port) = read_port_file() {
        if is_our_sse_server(port).await {
            tracing::info!("bridge: SSE server already running on port {port} (from port file)");
            return Some(port);
        }
        // 端口文件过期(server 不在了或端口被别的程序占了)
        tracing::info!("bridge: port file points to {port} but no SSE server there, respawning");
    }

    // 2. 找空闲端口并 spawn
    let port = find_free_port()?;
    tracing::info!("bridge: spawning SSE server on port {port}");

    if let Err(e) = spawn_sse_server(port, urls) {
        tracing::warn!("bridge: failed to spawn SSE server: {e}");
        return None;
    }

    // 3. 等待就绪
    if !wait_for_sse_server(port, Duration::from_secs(10)).await {
        tracing::warn!("bridge: SSE server did not become ready on port {port}");
        return None;
    }

    // 4. 写端口文件
    write_port_file(port);
    tracing::info!("bridge: SSE server is ready on port {port}");
    Some(port)
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
