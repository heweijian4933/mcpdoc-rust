//! stdio↔SSE 桥接:自动 spawn 共享 SSE server,转发 JSON-RPC 消息。
//!
//! 进程模型:
//! - SSE server 子进程:第一个会话 spawn,常驻,加载索引
//! - 桥接进程:每个会话一个,极小,只做转发

use std::time::Duration;

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
