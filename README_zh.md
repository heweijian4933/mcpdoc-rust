# mcpdoc-rust

> 高性能 Rust MCP 服务器,提供 `llms.txt` 文档检索与 BM25 全文搜索

[![CI](https://github.com/heweijian4933/mcpdoc-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/heweijian4933/mcpdoc-rust/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75+-orange.svg)](https://www.rust-lang.org)
[![MCP](https://img.shields.io/badge/MCP-2025--03--26-blue.svg)](https://modelcontextprotocol.io)

[English](README.md) | [中文](README_zh.md)

**mcpdoc-rust** 是 [mcpdoc](https://github.com/langchain-ai/mcpdoc) 的 Rust 重写版 —— 一个 MCP(模型上下文协议)服务器,为 Claude Code、Cursor、Windsurf 等 AI 助手提供可审计的 `llms.txt` 文档访问能力。

## 为什么选择 mcpdoc-rust?

原版 [mcpdoc](https://github.com/langchain-ai/mcpdoc)(Python)启发了本项目。mcpdoc-rust 解决了以下痛点:

| 痛点 | mcpdoc (Python) | mcpdoc-rust |
|------|-----------------|-------------|
| **启动速度** | ~2-5秒(uvx + Python 解释器) | ~50毫秒(单一原生二进制) |
| **内存占用** | ~50-100MB(Python 运行时) | ~10-20MB |
| **全文搜索** | ❌ 不支持 | ✅ BM25 + 标题加权 |
| **多进程** | 每个实例重复抓取 | 持久化索引缓存,多进程共享 |
| **部署** | 需要 Python + uvx | 单一自包含 `.exe` |

## 功能特性

### 核心 MCP 工具(3 个)

- **`list_doc_sources`** — 列出所有已配置的文档源(名称 + URL/路径 + 描述)
- **`fetch_docs`** — 获取并解析文档,支持 URL 和本地文件,HTML→Markdown 转换
- **`search_docs`** — 跨所有文档源的 BM25 全文搜索,标题字段加权 2 倍

### 亮点

- 🚀 **单一二进制** — 无运行时依赖,一个 `.exe` 挂载无限个 `llms.txt` 源
- 🔍 **BM25 搜索** — 内置 [tantivy](https://github.com/quickwit-oss/tantivy) 引擎,标题加权排序
- 💾 **持久化索引** — 索引缓存到磁盘,多进程共享只读加载(懒加载)
- 📁 **本地文档** — 支持本地文件和相对路径解析(相对于 `llms.txt` 所在目录)
- 🔒 **域名白名单** — 自动加入 `llms.txt` 所在域名,支持 `*` 通配符
- 🌐 **双传输模式** — stdio(适配 Cursor/Claude Code/Desktop)和 SSE(适配远程/Web 客户端)
- 🌐 **跨平台** — Windows、Linux、macOS
- 🎯 **工具调用命中率优化** — 服务器指令与工具描述内置上下文触发条件,覆盖 how-to 查询、概念查找、教程请求、助手自我验证等场景;推荐 `search_docs` 作为检索入口

## 安装

### 方式一:下载预编译二进制(推荐)

从 [Releases 页面](https://github.com/heweijian4933/mcpdoc-rust/releases)下载对应平台的压缩包:

| 平台 | 文件 |
|------|------|
| Windows x64 | `mcpdoc-rust-windows-x86_64.zip` |
| Linux x64 | `mcpdoc-rust-linux-x86_64.tar.gz` |
| macOS x64 (Intel) | `mcpdoc-rust-macos-x86_64.tar.gz` |
| macOS ARM64 (Apple Silicon) | `mcpdoc-rust-macos-aarch64.tar.gz` |

**校验下载**(每个版本都附带 SHA256 校验和):

```bash
# Linux/macOS
sha256sum mcpdoc-rust-linux-x86_64.tar.gz
# 与发布页面的 checksums.txt 对比

# Windows
certutil -hashfile mcpdoc-rust-windows-x86_64.zip SHA256
```

**安装步骤:**

```bash
# Windows (PowerShell)
Expand-Archive mcpdoc-rust-windows-x86_64.zip -DestinationPath C:\Tools\mcpdoc-rust
# 添加到 PATH:设置 → 环境变量 → Path → 新建 C:\Tools\mcpdoc-rust

# Linux / macOS
tar xzf mcpdoc-rust-linux-x86_64.tar.gz
sudo mv mcpdoc-rust /usr/local/bin/
chmod +x /usr/local/bin/mcpdoc-rust

# 验证
mcpdoc-rust --version
```

### 方式二:从源码构建

```bash
git clone https://github.com/heweijian4933/mcpdoc-rust.git
cd mcpdoc-rust
cargo build --release
# 产物:target/release/mcpdoc-rust(Windows 下为 .exe)
```

## 快速开始

### 1. 准备 llms.txt 文件

收集你需要服务的 `llms.txt` 文件。格式参见 [llmstxt.org](https://llmstxt.org/)。

### 2. 创建配置文件(推荐多源场景)

复制示例配置并编辑:

```bash
# 复制模板
cp examples/config.example.yaml config.yaml

# 编辑 config.yaml — 添加你的 llms.txt 文档源
```

配置文件格式(YAML):

```yaml
# 每个条目:
#   name:        显示名称(在搜索结果和提示词中展示)
#   llms_txt:    URL(https://...)或本地文件路径
#   description: 简短描述(可选,但推荐填写)

- name: ClaudeCode
  llms_txt: /path/to/claude-code-local-llms.txt
  description: Claude Code 文档
- name: uv
  llms_txt: /path/to/uv-docs-local-llms.txt
  description: uv Python 包管理器文档
- name: LangGraph
  llms_txt: https://langchain-ai.github.io/langgraph/llms.txt
  description: LangGraph Python 文档
```

也支持 JSON 格式(`--json config.json`),见 `examples/config.example.json`。

> **路径格式:** Windows 下正斜杠(`/`)和反斜杠(`\`)均可。推荐使用绝对路径。`llms_txt` 条目中的相对路径会相对于 `llms.txt` 文件所在目录解析。

### 3. 连接到 Claude Code

```bash
claude mcp add-json mcpdoc-rust '{
  "type": "stdio",
  "command": "mcpdoc-rust",
  "args": ["--yaml", "/path/to/config.yaml", "--allowed-domains", "*"]
}' -s user
```

将 `/path/to/config.yaml` 替换为你的配置文件实际路径,例如:
- **Windows:** `C:/Users/yourname/config.yaml` 或 `C:\\Users\\yourname\\config.yaml`
- **Linux/macOS:** `/home/yourname/config.yaml` 或 `~/.config/mcpdoc-rust/config.yaml`

> **提示:** 如果 `mcpdoc-rust` 不在 PATH 中,请在 `"command"` 中使用完整路径。

### 4. 连接到 Cursor / Windsurf / Claude Desktop

添加到 MCP 配置文件(`~/.cursor/mcp.json`、`~/.codeium/windsurf/mcp_config.json` 或等效位置):

```json
{
  "mcpServers": {
    "mcpdoc-rust": {
      "command": "mcpdoc-rust",
      "args": ["--yaml", "/path/to/config.yaml", "--allowed-domains", "*"]
    }
  }
}
```

### 5. 验证

重启 MCP 客户端,确认 `mcpdoc-rust` 服务器已连接,包含 3 个工具:`list_doc_sources`、`fetch_docs`、`search_docs`。

## 客户端配置指南

### Claude Code (CLI)

```bash
# 添加为用户级 MCP(所有项目可用)
claude mcp add-json mcpdoc-rust '{
  "type": "stdio",
  "command": "mcpdoc-rust",
  "args": ["--yaml", "/path/to/config.yaml", "--allowed-domains", "*"]
}' -s user

# 或添加为项目级 MCP(仅当前项目)
claude mcp add-json mcpdoc-rust '{
  "type": "stdio",
  "command": "mcpdoc-rust",
  "args": ["--yaml", "/path/to/config.yaml", "--allowed-domains", "*"]
}' -s local

# 验证
claude mcp list
```

### Codex CLI

添加到 `~/.codex/config.toml`(或项目级 `codex.toml`):

```toml
[mcp_servers.mcpdoc-rust]
command = "mcpdoc-rust"
args = ["--yaml", "/path/to/config.yaml", "--allowed-domains", "*"]
```

### Cursor

编辑 `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "mcpdoc-rust": {
      "command": "mcpdoc-rust",
      "args": ["--yaml", "/path/to/config.yaml", "--allowed-domains", "*"]
    }
  }
}
```

或通过界面:`Settings` → `MCP` → `Add MCP Server`。

### Windsurf

编辑 `~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "mcpdoc-rust": {
      "command": "mcpdoc-rust",
      "args": ["--yaml", "/path/to/config.yaml", "--allowed-domains", "*"]
    }
  }
}
```

### Claude Desktop

编辑配置文件:
- **macOS:** `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Windows:** `%APPDATA%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "mcpdoc-rust": {
      "command": "mcpdoc-rust",
      "args": ["--yaml", "/path/to/config.yaml", "--allowed-domains", "*"]
    }
  }
}
```

### SSE 模式(远程 / Web 客户端)

启动服务器:

```bash
mcpdoc-rust --yaml config.yaml --transport sse --host 0.0.0.0 --port 9000
```

从任何支持 SSE 的 MCP 客户端连接:

```
SSE 端点:  http://<host>:9000/sse
消息端点:  http://<host>:9000/message
```

> **注意:** 如果 `mcpdoc-rust` 不在 PATH 中,请在 `"command"` 中使用完整路径(如 Windows 下 `"C:\\Tools\\mcpdoc-rust\\mcpdoc-rust.exe"`)。

## CLI 参数

```
mcpdoc-rust [OPTIONS]

Options:
  -y, --yaml <PATH>              YAML 配置文件
  -j, --json <PATH>              JSON 配置文件
  -u, --urls <URLS>...           llms.txt URL/路径列表(格式:name:url 或 url)
      --follow-redirects         跟随 HTTP 重定向
      --allowed-domains <DOMAINS>...  允许的域名(* 表示全部)
      --timeout <SECONDS>        HTTP 超时秒数 [默认: 10]
      --transport <TYPE>         传输协议:stdio 或 sse [默认: stdio]
      --log-level <LEVEL>        日志级别:DEBUG/INFO/WARNING/ERROR [默认: INFO]
      --index <BOOL>             是否启用 BM25 索引 [默认: true]
      --cache-dir <PATH>         索引缓存目录 [默认: <exe目录>/cache]
  -h, --help                     显示帮助
  -V, --version                  显示版本
```

## 配置文件格式

### YAML (`config.yaml`)

```yaml
- name: LangGraph
  llms_txt: https://langchain-ai.github.io/langgraph/llms.txt
  description: LangGraph Python 文档
```

### JSON (`config.json`)

```json
[
  {
    "name": "LangGraph",
    "llms_txt": "https://langchain-ai.github.io/langgraph/llms.txt",
    "description": "LangGraph Python 文档"
  }
]
```

## 工作原理

### MCP 工具工作流

```
用户提问:"Claude Code 的 hooks 怎么用?"
                    │
                    ▼
         ┌─────────────────────┐
         │  search_docs("hooks")│  ← 跨所有源 BM25 搜索
         └─────────────────────┘
                    │
                    ▼
         ┌─────────────────────┐
         │  fetch_docs(url)     │  ← 读取完整页面内容
         └─────────────────────┘
                    │
                    ▼
         基于文档的回答
```

### 持久化索引缓存

多个 Claude Code 会话共享同一索引缓存:

```
Claude Code #1 ──┐
Claude Code #2 ──┼──► cache/index-<hash>/  (共享,只读)
Claude Code #3 ──┘
```

首个进程构建索引(~3秒),后续进程秒级加载(~50毫秒)。

### 资源占用:多会话对比

同时运行多个 MCP 客户端时(如 3 个 Claude Code 窗口):

| 指标 | mcpdoc (Python/uvx) | mcpdoc-rust |
|------|---------------------|-------------|
| **进程数** | 3 × (uvx + Python) | 3 × (原生二进制) |
| **总内存** | ~150-300MB | ~30-60MB |
| **索引构建** | 3 × (重复抓取 + 重复索引) | 1 × 构建 + 2 × 缓存加载 |
| **启动时间(第2个起)** | ~2-5秒/个 | ~50毫秒/个 |
| **网络请求** | 3 × 重复抓取 | 1 × 抓取(磁盘缓存) |

## 技术栈

| 组件 | Crate | 用途 |
|------|-------|------|
| MCP 协议 | [rmcp](https://github.com/modelcontextprotocol/rust-sdk) 0.2 | MCP 服务器实现 |
| 全文搜索 | [tantivy](https://github.com/quickwit-oss/tantivy) 0.22 | BM25 排序,倒排索引 |
| HTTP 客户端 | [reqwest](https://github.com/seanmonstar/reqwest) 0.12 | 获取远程文档 |
| HTML 解析 | [scraper](https://github.com/causal-agent/scraper) 0.20 | HTML→Markdown 转换 |
| CLI | [clap](https://github.com/clap-rs/clap) 4.5 | 命令行参数解析 |
| 异步运行时 | [tokio](https://github.com/tokio-rs/tokio) | 异步 I/O |

## 项目结构

```
src/
├── lib.rs           # 库入口
├── main.rs          # 二进制入口(CLI + 传输分发)
├── cli.rs           # clap CLI 定义
├── config.rs        # DocSource + 配置加载
├── server.rs        # MCP ServerHandler + 工具
├── fetch.rs         # 文档获取(HTTP/本地)
├── html_to_md.rs    # HTML→Markdown 转换
├── domain.rs        # URL 域名 + 白名单 + 路径解析
├── error.rs         # 错误类型
└── index/
    ├── mod.rs       # tantivy BM25 索引(懒加载,持久化)
    └── schema.rs    # 索引 schema 定义
```

## 致谢

本项目是 [**mcpdoc**](https://github.com/langchain-ai/mcpdoc)(由 [LangChain](https://github.com/langchain-ai) 开发)的 Rust 重写版。感谢原作者的概念和设计。

与原版 mcpdoc 的主要差异:
- 用 Rust 重写,提升性能,支持单一二进制部署
- 新增 BM25 全文搜索(`search_docs` 工具)
- 新增持久化索引缓存,多进程共享
- 新增本地 `llms.txt` 相对路径解析
- 优化工具调用提示词,提升 MCP 命中率

## 许可证

MIT — 详见 [LICENSE](LICENSE)。

## 贡献

欢迎贡献 — 详见 [CONTRIBUTING.md](CONTRIBUTING.md)。更新历史详见 [CHANGELOG.md](CHANGELOG.md)。
