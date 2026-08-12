# mcpdoc-rust

> A high-performance Rust MCP server for `llms.txt` documentation retrieval with built-in BM25 full-text search.

[![CI](https://github.com/heweijian4933/mcpdoc-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/heweijian4933/mcpdoc-rust/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75+-orange.svg)](https://www.rust-lang.org)
[![MCP](https://img.shields.io/badge/MCP-2025--03--26-blue.svg)](https://modelcontextprotocol.io)

[English](README.md) | [中文](README_zh.md)

**mcpdoc-rust** is a Rust rewrite of [mcpdoc](https://github.com/langchain-ai/mcpdoc) — an MCP (Model Context Protocol) server that provides AI assistants like Claude Code, Cursor, and Windsurf with auditable access to `llms.txt` documentation files.

## Why mcpdoc-rust?

The original [mcpdoc](https://github.com/langchain-ai/mcpdoc) (Python) inspired this project. mcpdoc-rust solves several pain points:

| Problem | mcpdoc (Python) | mcpdoc-rust |
|---------|-----------------|-------------|
| **Startup speed** | ~2-5s (uvx + Python interpreter) | ~50ms (single native binary) |
| **Memory usage** | ~50-100MB (Python runtime) | ~10-20MB |
| **Full-text search** | ❌ Not available | ✅ BM25 with title boosting |
| **Multi-process** | Each instance re-fetches | Persistent index cache shared across processes |
| **Deployment** | Requires Python + uvx | Single self-contained `.exe` |

## Features

### Core Tools (3 MCP tools)

- **`list_doc_sources`** — List all configured documentation sources with names, URLs/paths, and descriptions
- **`fetch_docs`** — Fetch and parse documentation from URLs or local files, with HTML→Markdown conversion
- **`search_docs`** — BM25 full-text search across ALL sources, with title fields weighted 2× for relevance

### Highlights

- 🚀 **Single binary** — No runtime dependencies, one `.exe` serves unlimited `llms.txt` sources
- 🔍 **BM25 search** — Built-in [tantivy](https://github.com/quickwit-oss/tantivy) engine, title-weighted ranking
- 💾 **Persistent index** — Index cached to disk, shared across multiple processes (lazy-loaded)
- 📁 **Local docs** — Supports local files and relative path resolution (relative to `llms.txt` directory)
- 🔒 **Domain whitelist** — Automatic allowlist for `llms.txt` domains, with `*` wildcard support
- 🌐 **Dual transport** — stdio (for Cursor/Claude Code/Desktop) and SSE (for remote/web clients)
- 🌐 **Cross-platform** — Windows, Linux, macOS
- 🎯 **Optimized tool invocation** — Contextual triggers in server instructions and tool descriptions increase MCP hit rate: covers how-to queries, concept lookups, tutorial requests, and assistant self-verification scenarios; `search_docs` recommended as the entry point

## Installation

### Option 1: Download Pre-built Binary (Recommended)

Download the latest release for your platform from the [Releases page](https://github.com/heweijian4933/mcpdoc-rust/releases):

| Platform | File |
|----------|------|
| Windows x64 | `mcpdoc-rust-windows-x86_64.zip` |
| Linux x64 | `mcpdoc-rust-linux-x86_64.tar.gz` |
| macOS x64 (Intel) | `mcpdoc-rust-macos-x86_64.tar.gz` |
| macOS ARM64 (Apple Silicon) | `mcpdoc-rust-macos-aarch64.tar.gz` |

**Verify the download** (SHA256 checksums are published with each release):

```bash
# Linux/macOS
sha256sum mcpdoc-rust-linux-x86_64.tar.gz
# Compare with checksums.txt from the release page

# Windows
certutil -hashfile mcpdoc-rust-windows-x86_64.zip SHA256
```

**Install:**

```bash
# Windows (PowerShell)
Expand-Archive mcpdoc-rust-windows-x86_64.zip -DestinationPath C:\Tools\mcpdoc-rust
# Add to PATH: Settings → Environment Variables → Path → Add C:\Tools\mcpdoc-rust

# Linux / macOS
tar xzf mcpdoc-rust-linux-x86_64.tar.gz
sudo mv mcpdoc-rust /usr/local/bin/
chmod +x /usr/local/bin/mcpdoc-rust

# Verify
mcpdoc-rust --version
```

### Option 2: Build from Source

```bash
git clone https://github.com/heweijian4933/mcpdoc-rust.git
cd mcpdoc-rust
cargo build --release
# Binary: target/release/mcpdoc-rust (or .exe on Windows)
```

## Quick Start

### 1. Prepare Your llms.txt Files

Collect `llms.txt` files for the documentation you want to serve. See [llmstxt.org](https://llmstxt.org/) for the format.

### 2. Create a Config File (Recommended for Multiple Sources)

Copy the example config and edit it:

```bash
# Copy the template
cp examples/config.example.yaml config.yaml

# Edit config.yaml — add your llms.txt sources
```

Config file format (YAML):

```yaml
# Each entry:
#   name:        Display name (shown in search results and instructions)
#   llms_txt:    URL (https://...) or local file path to the llms.txt file
#   description: Short description (optional but recommended)

- name: ClaudeCode
  llms_txt: /path/to/claude-code-local-llms.txt
  description: Claude Code documentation
- name: uv
  llms_txt: /path/to/uv-docs-local-llms.txt
  description: uv Python package manager docs
- name: LangGraph
  llms_txt: https://langchain-ai.github.io/langgraph/llms.txt
  description: LangGraph Python docs
```

JSON format is also supported (`--json config.json`), see `examples/config.example.json`.

> **Path format:** Both forward slashes (`/`) and backslashes (`\`) work on Windows. Absolute paths are recommended. Relative paths in `llms_txt` entries are resolved relative to the `llms.txt` file's directory.

### 3. Connect to Claude Code

```bash
claude mcp add-json mcpdoc-rust '{
  "type": "stdio",
  "command": "mcpdoc-rust",
  "args": ["--yaml", "/path/to/config.yaml", "--allowed-domains", "*"]
}' -s user
```

Replace `/path/to/config.yaml` with the actual path to your config file, e.g.:
- **Windows:** `C:/Users/yourname/config.yaml` or `C:\\Users\\yourname\\config.yaml`
- **Linux/macOS:** `/home/yourname/config.yaml` or `~/.config/mcpdoc-rust/config.yaml`

> **Tip:** If `mcpdoc-rust` is not on PATH, use the full path in `"command"`.

### 4. Connect to Cursor / Windsurf / Claude Desktop

Add to your MCP config (`~/.cursor/mcp.json`, `~/.codeium/windsurf/mcp_config.json`, or equivalent):

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

### 5. Verify

Restart your MCP client and check that the `mcpdoc-rust` server is connected with 3 tools: `list_doc_sources`, `fetch_docs`, `search_docs`.

## Client Configuration Guide

### Claude Code (CLI)

```bash
# Add as user-level MCP (available in all projects)
claude mcp add-json mcpdoc-rust '{
  "type": "stdio",
  "command": "mcpdoc-rust",
  "args": ["--yaml", "/path/to/config.yaml", "--allowed-domains", "*"]
}' -s user

# Or add as project-level MCP (current project only)
claude mcp add-json mcpdoc-rust '{
  "type": "stdio",
  "command": "mcpdoc-rust",
  "args": ["--yaml", "/path/to/config.yaml", "--allowed-domains", "*"]
}' -s local

# Verify
claude mcp list
```

### Codex CLI

Add to `~/.codex/config.toml` (or project-level `codex.toml`):

```toml
[mcp_servers.mcpdoc-rust]
command = "mcpdoc-rust"
args = ["--yaml", "/path/to/config.yaml", "--allowed-domains", "*"]
```

### Cursor

Edit `~/.cursor/mcp.json`:

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

Or via UI: `Settings` → `MCP` → `Add MCP Server`.

### Windsurf

Edit `~/.codeium/windsurf/mcp_config.json`:

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

Edit the config file:
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

### SSE Mode (Remote / Web Clients)

Start the server:

```bash
mcpdoc-rust --yaml config.yaml --transport sse --host 0.0.0.0 --port 9000
```

Connect from any MCP client that supports SSE:

```
SSE endpoint:  http://<host>:9000/sse
Message endpoint: http://<host>:9000/message
```

> **Note:** Replace `"mcpdoc-rust"` in `"command"` with the full path if the binary is not on PATH (e.g., `"C:\\Tools\\mcpdoc-rust\\mcpdoc-rust.exe"` on Windows).

## CLI Reference

```
mcpdoc-rust [OPTIONS]

Options:
  -y, --yaml <PATH>              YAML config file with doc sources
  -j, --json <PATH>              JSON config file with doc sources
  -u, --urls <URLS>...           llms.txt URLs/paths (format: name:url or url)
      --follow-redirects         Follow HTTP redirects
      --allowed-domains <DOMAINS>...  Allowed domains (* for all)
      --timeout <SECONDS>        HTTP timeout [default: 10]
      --transport <TYPE>         Transport: stdio or sse [default: stdio]
      --log-level <LEVEL>        Log level: DEBUG/INFO/WARNING/ERROR [default: INFO]
      --index <BOOL>             Enable BM25 index [default: true]
      --cache-dir <PATH>         Index cache directory [default: <exe_dir>/cache]
  -h, --help                     Print help
  -V, --version                  Print version
```

## Configuration Format

### YAML (`config.yaml`)

```yaml
- name: LangGraph
  llms_txt: https://langchain-ai.github.io/langgraph/llms.txt
  description: LangGraph Python documentation
```

### JSON (`config.json`)

```json
[
  {
    "name": "LangGraph",
    "llms_txt": "https://langchain-ai.github.io/langgraph/llms.txt",
    "description": "LangGraph Python documentation"
  }
]
```

## How It Works

### MCP Tools Workflow

```
User asks: "How do Claude Code hooks work?"
                    │
                    ▼
         ┌─────────────────────┐
         │  search_docs("hooks")│  ← BM25 search across ALL sources
         └─────────────────────┘
                    │
                    ▼
         ┌─────────────────────┐
         │  fetch_docs(url)     │  ← Read full page content
         └─────────────────────┘
                    │
                    ▼
         Answer with doc-backed facts
```

### Persistent Index Cache

Multiple Claude Code sessions share the same index cache:

```
Claude Code #1 ──┐
Claude Code #2 ──┼──► cache/index-<hash>/  (shared, read-only)
Claude Code #3 ──┘
```

First process builds the index (~3s), subsequent processes load it instantly (~50ms).

### Resource Usage: Multi-Session Comparison

When running multiple MCP clients simultaneously (e.g., 3 Claude Code windows):

| Metric | mcpdoc (Python/uvx) | mcpdoc-rust |
|--------|---------------------|-------------|
| **Processes** | 3 × (uvx + Python) | 3 × (native binary) |
| **Total memory** | ~150-300MB | ~30-60MB |
| **Index builds** | 3 × (re-fetch + re-index) | 1 × build + 2 × cache load |
| **Startup (2nd+)** | ~2-5s each | ~50ms each |
| **Network requests** | 3 × duplicate fetches | 1 × fetch (cached on disk) |

## Tech Stack

| Component | Crate | Purpose |
|-----------|-------|---------|
| MCP protocol | [rmcp](https://github.com/modelcontextprotocol/rust-sdk) 0.2 | MCP server implementation |
| Full-text search | [tantivy](https://github.com/quickwit-oss/tantivy) 0.22 | BM25 ranking, inverted index |
| HTTP client | [reqwest](https://github.com/seanmonstar/reqwest) 0.12 | Fetch remote docs |
| HTML parsing | [scraper](https://github.com/causal-agent/scraper) 0.20 | HTML→Markdown conversion |
| CLI | [clap](https://github.com/clap-rs/clap) 4.5 | Argument parsing |
| Async runtime | [tokio](https://github.com/tokio-rs/tokio) | Async I/O |

## Project Structure

```
src/
├── lib.rs           # Library entry
├── main.rs          # Binary entry (CLI + transport dispatch)
├── cli.rs           # clap CLI definition
├── config.rs        # DocSource + config loading
├── server.rs        # MCP ServerHandler + tools
├── fetch.rs         # Document fetching (HTTP/local)
├── html_to_md.rs    # HTML→Markdown conversion
├── domain.rs        # URL domain + whitelist + path resolution
├── error.rs         # Error types
└── index/
    ├── mod.rs       # tantivy BM25 index (lazy-loaded, persistent)
    └── schema.rs    # Index schema definition
```

## Acknowledgments

This project is a Rust rewrite of [**mcpdoc**](https://github.com/langchain-ai/mcpdoc) by [LangChain](https://github.com/langchain-ai). We thank the original authors for the concept and design.

Key differences from the original mcpdoc:
- Rewritten in Rust for performance and single-binary deployment
- Added BM25 full-text search (`search_docs` tool)
- Added persistent index cache for multi-process sharing
- Added relative path resolution for local `llms.txt` files
- Optimized tool invocation prompts for higher MCP hit rate

## License

MIT — see [LICENSE](LICENSE).

## Contributing

Contributions welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). See [CHANGELOG.md](CHANGELOG.md) for release history.
