# Design: stdio↔SSE Bridge for Shared Single-Process Memory

**Date:** 2026-09-02
**Status:** Approved (brainstorming complete)

## Problem

每个 Claude Code / Codex 会话以 stdio 模式启动一个 mcpdoc-rust 进程，各自加载 doc_sources + 索引。虽然每进程私有内存只有 ~3.4MB，但多会话时仍重复加载索引和配置。需要一个共享方案让所有会话复用同一个 SSE server 进程，同时不改变 Claude/Codex 的 stdio 配置。

## Solution

在 stdio 模式启动时自动检测并 spawn 一个常驻 SSE server 子进程，然后桥接进程变成 stdio↔SSE 转发器。用户无感知，不需要改任何 Claude/Codex 配置。

```
Claude Code ──stdio──→ [桥接进程] ──SSE──→ [SSE server 子进程]
  (会话)                  (极小,~2MB)        (常驻,加载索引)
```

## Design Decisions

### 1. 自动 spawn + 常驻

- 桥接进程启动时检测端口 9000
- 没在跑 → spawn `mcpdoc-rust.exe --transport sse --port 9000 ...`（detached 子进程）
- 已在跑 → 直接连接
- SSE server 只 spawn 一次，常驻不退出
- SSE server 崩溃 → 下一个桥接进程检测到端口空 → 自动重新 spawn

### 2. 桥接进程不加载 McpDocServer

桥接进程只做 JSON-RPC 字节级转发，不解析 doc_sources、不建索引、不加载配置。真正的业务逻辑只在 SSE server 进程里。

### 3. transport 层透传

不用 rmcp 的 Peer/Service 层——直接在 transport 层做字节转发（JSON-RPC message 透传），避免 rmcp Service 层的初始化握手开销和类型转换。

### 4. Fallback 到 stdio 直接模式

如果 spawn 或连接 SSE server 失败，回退到原来的 stdio 直接模式（加载 McpDocServer 本地跑），保证可用性。

## Architecture

### 进程模型

| 进程 | 启动 | 退出 | 内存 |
|------|------|------|------|
| SSE server 子进程 | 第一个会话 spawn | 常驻（不主动退出） | ~14MB（加载索引+配置） |
| 桥接进程 ×N | 每个会话启动（Claude Code spawn） | 会话关闭时退出（stdin EOF） | ~2MB（只转发） |

### 启动流程

```
main.rs stdio 模式:
1. 解析 CLI 参数（拿到 --urls, --port 等）
2. 检测端口 9000（HTTP GET /sse，超时 1 秒）
3. 端口没占用 →
   a. spawn 子进程: Command::new(current_exe)
      .args(["--transport", "sse", "--port", "9000", ...原始 args...])
      .stdin(Null).stdout(Null).stderr(Null)  // detached
      .spawn()
   b. 重试连接 SSE server（每隔 200ms，最多 5 秒）
4. 连接 SSE server: SseClientTransport::start("http://127.0.0.1:9000/sse")
5. 启动桥接：stdin → SSE client, SSE client → stdout
6. stdin EOF → 退出
```

### 桥接实现

用 rmcp 0.2.1 的 `SseClientTransport::start(url)` 连 SSE server，stdio 端用 stdin/stdout。

转发逻辑：
- stdio 收到 JSON-RPC message → 发给 SSE client transport
- SSE client transport 收到 response/notification → 写到 stdout

## Code Changes

| 文件 | 改动 |
|------|------|
| `src/main.rs` | stdio 模式改为：检测端口→spawn SSE server→启动桥接。SSE 模式不变。新增 fallback 到 stdio 直接模式。 |
| `src/bridge.rs`（新增） | `async fn run_bridge(sse_url: &str)` — stdio↔SSE 转发逻辑 |
| Claude/Codex 配置 | 不变（stdio 启动 exe） |

### main.rs 改动

`run_stdio` 函数改为：
1. 尝试 spawn 或连接 SSE server
2. 成功 → 调用 `bridge::run_bridge(sse_url)`
3. 失败 → 回退原来的 `run_stdio(server)` 直接模式

### bridge.rs 新增

- `pub async fn run_bridge(sse_url: &str) -> anyhow::Result<()>`
  - 连接 SSE server（SseClientTransport）
  - stdio ↔ SSE 转发（JSON-RPC message 透传）
  - stdin EOF → 退出

- `fn check_sse_server(url: &str) -> bool`
  - HTTP GET /sse，检测 server 是否在跑

- `fn spawn_sse_server(args: &[String]) -> anyhow::Result<()>`
  - spawn `current_exe --transport sse --port 9000 ...`
  - detached（stdin/stdout/stderr = Null）

- `async fn wait_for_sse_server(url: &str, timeout: Duration) -> bool`
  - 重试连接，最多等 5 秒

## Error Handling

| 场景 | 行为 |
|------|------|
| 端口检测超时（1 秒） | 认为 server 没在跑，尝试 spawn |
| spawn SSE server 失败 | 回退到 stdio 直接模式 |
| SSE server 5 秒内没就绪 | 回退到 stdio 直接模式 |
| 桥接转发中 SSE 连接断开 | 向 stdout 写错误，退出进程 |
| SSE server 崩溃 | 下个桥接进程自动重建 |

## Out of Scope

- 开机自启（后续考虑）
- Streamable HTTP 新协议（当前用旧版 SSE，rmcp 0.2.1 兼容）
- 引用计数清理 SSE server（常驻不退出，简单可靠）
- 端口冲突检测（9000 被其他程序占用时 fallback 到 stdio）
