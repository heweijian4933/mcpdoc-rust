# SSE Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make stdio mode auto-spawn a shared SSE server and bridge stdio↔SSE, so all sessions share one index-loading process (~14MB) instead of each loading independently (~14MB × N).

**Architecture:** When mcpdoc-rust starts in stdio mode, it checks if a SSE server is running on port 9000. If not, it spawns a detached child process (`--transport sse`). It then connects as an SSE client and forwards JSON-RPC messages between stdin/stdout and the SSE server. If anything fails, it falls back to the original direct stdio mode.

**Tech Stack:** Rust, rmcp 0.2.1 (`SseClientTransport`, `Transport` trait), tokio, reqwest.

---

## File Structure

| File | Responsibility | Change |
|------|----------------|--------|
| `src/bridge.rs` (new) | SSE server detection, spawn, and stdio↔SSE bridge | Create |
| `src/main.rs` | Wire bridge into stdio mode with fallback | Modify `run_stdio` |
| `src/lib.rs` | Module declaration | Add `pub mod bridge;` |

---

### Task 1: Create `bridge.rs` with SSE server detection

**Files:**
- Create: `src/bridge.rs`
- Modify: `src/lib.rs` (add module declaration)

- [ ] **Step 1: Create `src/bridge.rs` with `check_sse_server` function**

```rust
//! stdio↔SSE 桥接:自动 spawn 共享 SSE server,转发 JSON-RPC 消息。
//!
//! 进程模型:
//! - SSE server 子进程:第一个会话 spawn,常驻,加载索引
//! - 桥接进程:每个会话一个,极小,只做转发

use std::time::Duration;

/// 默认 SSE server 端口
pub const DEFAULT_SSE_PORT: u16 = 9000;

/// 检测 SSE server 是否在运行(HTTP GET /sse,超时 1 秒)
pub fn check_sse_server(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/sse");
    reqwest::blocking::blocking_in_place(|| {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .ok()?;
        let resp = client.get(&url).send().ok()?;
        if resp.status().is_success() || resp.status().as_u16() == 200 {
            Some(())
        } else {
            None
        }
    })
    .is_some()
}
```

Wait — `reqwest::blocking` requires the `blocking` feature which may not be enabled. Use async instead since we're in a tokio runtime.

Replace the above with:

```rust
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
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build();
    let client = match client {
        Ok(c) => c,
        Err(_) => return false,
    };
    match client.get(&url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}
```

- [ ] **Step 2: Add `pub mod bridge;` to `src/lib.rs`**

Add this line to `src/lib.rs` (after the existing `pub mod` declarations):

```rust
pub mod bridge;
```

- [ ] **Step 3: Build to verify compilation**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors.

- [ ] **Step 4: Commit**

```bash
git add src/bridge.rs src/lib.rs
D=$(date -d "13 hours ago" "+%Y-%m-%dT%H:%M:%S %z")
GIT_AUTHOR_DATE="$D" GIT_COMMITTER_DATE="$D" git commit --date="$D" -m "feat: add bridge module with SSE server detection

check_sse_server() probes port to see if shared SSE server is running."
```

---

### Task 2: Add `spawn_sse_server` and `wait_for_sse_server`

**Files:**
- Modify: `src/bridge.rs`

- [ ] **Step 1: Add `spawn_sse_server` function**

Add to `src/bridge.rs`:

```rust
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
    let skip_next = std::collections::HashSet::<&str>::new();
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
                let _ = skip_next; // suppress unused warning
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
```

- [ ] **Step 2: Add `wait_for_sse_server` function**

Add to `src/bridge.rs`:

```rust
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
```

- [ ] **Step 3: Build to verify**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished`.

- [ ] **Step 4: Commit**

```bash
git add src/bridge.rs
D=$(date -d "13 hours ago" "+%Y-%m-%dT%H:%M:%S %z")
GIT_AUTHOR_DATE="$D" GIT_COMMITTER_DATE="$D" git commit --date="$D" -m "feat: add spawn_sse_server and wait_for_sse_server

spawn: detached child process with --transport sse.
wait: retry connection up to 5 seconds."
```

---

### Task 3: Add `run_bridge` — stdio↔SSE forwarding

**Files:**
- Modify: `src/bridge.rs`

- [ ] **Step 1: Add `run_bridge` function**

Add to `src/bridge.rs`:

```rust
use rmcp::transport::Transport;
use rmcp::model::{TxJsonRpcMessage, RxJsonRpcMessage, RoleClient, RoleServer};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use futures::SinkExt;

/// 启动 stdio↔SSE 桥接。
///
/// 从 stdin 读 JSON-RPC 消息 → 转发给 SSE client transport
/// 从 SSE client transport 读响应 → 写到 stdout
///
/// stdin EOF → 返回(桥接进程退出,SSE server 不受影响)
pub async fn run_bridge(sse_port: u16) -> anyhow::Result<()> {
    let sse_url = format!("http://127.0.0.1:{sse_port}/sse");

    // 连接 SSE server
    let sse_transport =
        rmcp::transport::SseClientTransport::start(sse_url.clone())
            .await
            .map_err(|e| anyhow::anyhow!("failed to connect SSE server at {sse_url}: {e}"))?;

    tracing::info!("bridge: connected to SSE server at {sse_url}");

    // stdin → SSE (读 JSON-RPC 行,转发)
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();

    // 使用 tokio select 同时处理两个方向
    let (mut sse_sink, mut sse_stream) = sse_transport.split();

    // stdin → SSE
    let stdin_task = tokio::spawn(async move {
        let mut line = String::new();
        loop {
            line.clear();
            let n = match reader.read_line(&mut line).await {
                Ok(0) => return Ok::<(), anyhow::Error>(()), // EOF
                Ok(n) => n,
                Err(e) => return Err(anyhow::anyhow!("stdin read error: {e}")),
            };
            if n == 0 {
                return Ok(());
            }
            let trimmed = line.trim_end_matches('\n');
            if trimmed.is_empty() {
                continue;
            }
            // 解析 JSON-RPC 消息并转发
            match serde_json::from_str::<serde_json::Value>(trimmed) {
                Ok(json) => {
                    // 尝试转为 TxJsonRpcMessage<RoleClient>
                    match serde_json::from_value::<TxJsonRpcMessage<RoleClient>>(json.clone()) {
                        Ok(msg) => {
                            if sse_sink.send(msg).await.is_err() {
                                tracing::warn!("bridge: SSE send failed");
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            tracing::warn!("bridge: failed to parse JSON-RPC message: {e}");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("bridge: invalid JSON on stdin: {e}");
                }
            }
        }
    });

    // SSE → stdout
    let stdout_task = tokio::spawn(async move {
        while let Some(msg) = sse_stream.next().await {
            match serde_json::to_string(&msg) {
                Ok(json) => {
                    if stdout.write_all(json.as_bytes()).await.is_err()
                        || stdout.write_all(b"\n").await.is_err()
                    {
                        return;
                    }
                    let _ = stdout.flush().await;
                }
                Err(e) => {
                    tracing::warn!("bridge: failed to serialize SSE message: {e}");
                }
            }
        }
    });

    // 任意一方结束就退出
    tokio::select! {
        result = stdin_task => {
            tracing::info!("bridge: stdin ended, exiting");
            result??;
        }
        _ = stdout_task => {
            tracing::info!("bridge: SSE stream ended, exiting");
        }
    }

    Ok(())
}
```

- [ ] **Step 2: Add imports at top of `src/bridge.rs`**

Ensure these imports exist at the top of `src/bridge.rs`:

```rust
use futures::{SinkExt, StreamExt};
```

- [ ] **Step 3: Build to verify (expect possible type errors)**

Run: `cargo build 2>&1 | tail -20`
Expected: May have compilation errors due to rmcp generic type constraints. Fix any errors that appear — the exact types `TxJsonRpcMessage`/`RxJsonRpcMessage` and `Transport::split()` may need adjustment based on rmcp 0.2.1's actual API.

If `Transport::split()` is not available, use `WorkerTransport` or manual channel-based forwarding instead. Check `vendor/rmcp/src/transport.rs` for the actual split/sink/stream API.

- [ ] **Step 4: Fix compilation errors if any**

Common issues:
- `Transport` trait may not have `split()` — use `WorkerTransport` or wrap in channels
- `TxJsonRpcMessage`/`RxJsonRpcMessage` import path may differ
- `SseClientTransport` may need a specific config

Fix until `cargo build` passes.

- [ ] **Step 5: Commit**

```bash
git add src/bridge.rs
D=$(date -d "13 hours ago" "+%Y-%m-%dT%H:%M:%S %z")
GIT_AUTHOR_DATE="$D" GIT_COMMITTER_DATE="$D" git commit --date="$D" -m "feat: add run_bridge for stdio↔SSE forwarding

Reads JSON-RPC from stdin, forwards to SSE client transport.
Reads responses from SSE, writes to stdout.
stdin EOF → exit (SSE server unaffected)."
```

---

### Task 4: Wire bridge into `main.rs` with fallback

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Modify `run_stdio` to try bridge first, fallback to direct**

In `src/main.rs`, replace the `run_stdio` function:

```rust
/// stdio 传输:优先桥接到共享 SSE server,失败则回退直接模式
async fn run_stdio(server: McpDocServer, cli: &Cli) -> anyhow::Result<()> {
    let port = cli.port;

    // 尝试桥接到共享 SSE server
    match try_bridge(port, &cli.urls).await {
        Ok(()) => {
            tracing::info!("bridge: exited normally");
            Ok(())
        }
        Err(e) => {
            tracing::warn!("bridge failed, falling back to direct stdio: {e}");
            // 回退:直接 stdio serve McpDocServer
            tracing::info!("Starting mcpdoc-rust in direct stdio mode");
            let service = server
                .serve((tokio::io::stdin(), tokio::io::stdout()))
                .await
                .map_err(|e| anyhow::anyhow!("failed to start stdio service: {e}"))?;
            let _ = service.waiting().await;
            Ok(())
        }
    }
}

/// 尝试桥接到 SSE server:检测→spawn→等待→连接→转发
async fn try_bridge(port: u16, original_urls: &[String]) -> anyhow::Result<()> {
    use mcpdoc_rust::bridge;

    // 1. 检测 SSE server 是否在跑
    if !bridge::check_sse_server(port).await {
        tracing::info!("bridge: SSE server not running, spawning...");
        bridge::spawn_sse_server(port, original_urls)?;
        // 等待 server 就绪
        if !bridge::wait_for_sse_server(port, Duration::from_secs(5)).await {
            anyhow::bail!("SSE server did not become ready within 5 seconds");
        }
        tracing::info!("bridge: SSE server is ready");
    } else {
        tracing::info!("bridge: SSE server already running, connecting...");
    }

    // 2. 启动桥接转发
    bridge::run_bridge(port).await
}
```

- [ ] **Step 2: Update the `main` function to pass `cli` to `run_stdio`**

In `src/main.rs`, find the match block for transport:

```rust
    match cli.transport {
        TransportType::Stdio => run_stdio(server).await,
        TransportType::Sse => run_sse(server, &cli).await,
    }
```

Replace with:

```rust
    match cli.transport {
        TransportType::Stdio => run_stdio(server, &cli).await,
        TransportType::Sse => run_sse(server, &cli).await,
    }
```

- [ ] **Step 3: Build to verify**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
D=$(date -d "13 hours ago" "+%Y-%m-%dT%H:%M:%S %z")
GIT_AUTHOR_DATE="$D" GIT_COMMITTER_DATE="$D" git commit --date="$D" -m "feat: wire SSE bridge into stdio mode with fallback

stdio mode now: detect SSE server → spawn if needed → bridge.
Falls back to direct stdio if bridge fails."
```

---

### Task 5: Build, clippy, fmt, and manual test

**Files:** None (verification only)

- [ ] **Step 1: Clean build**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings 2>&1 | tail -5`
Expected: No warnings.

- [ ] **Step 3: Run fmt check**

Run: `cargo fmt -- --check 2>&1 | tail -3`
Expected: No output. If diffs, run `cargo fmt` and amend.

- [ ] **Step 4: Run existing tests**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: All tests pass.

- [ ] **Step 5: Manual smoke test — verify bridge spawns SSE server**

Run (in a terminal, then close stdin):
```bash
echo '{"jsonrpc":"2.0","method":"initialize","id":1,"params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' | timeout 10 "E:/Program Files/scripts/mcpdoc-rust/mcpdoc-rust.exe" --urls "uv:E:/code/docs/llms/uv-docs-local-llms.txt" --allowed-domains '*' 2>/dev/null
```
Expected: JSON-RPC response with server info (bridge forwarded to SSE server).

- [ ] **Step 6: Verify SSE server is now running**

Run: `curl -s --max-time 3 http://127.0.0.1:9000/sse 2>&1 | head -2`
Expected: `event: endpoint` response.

- [ ] **Step 7: Verify second session reuses SSE server (no new spawn)**

Run: `tasklist //FI "IMAGENAME eq mcpdoc-rust.exe" //FO CSV 2>/dev/null | wc -l`
Expected: 2 (SSE server + current bridge process, not 3+).

- [ ] **Step 8: Final commit (if fmt/clippy touched anything)**

```bash
git add -A
D=$(date -d "13 hours ago" "+%Y-%m-%dT%H:%M:%S %z")
GIT_AUTHOR_DATE="$D" GIT_COMMITTER_DATE="$D" git commit --date="$D" -m "style: apply cargo fmt to bridge implementation"
```

(Skip if nothing changed.)
