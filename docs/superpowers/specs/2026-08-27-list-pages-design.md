# Design: `list_pages` MCP Tool

**Date:** 2026-08-27
**Status:** Approved (brainstorming complete)

## Problem

当前 mcpdoc-rust 有三个 MCP 工具：

| 工具 | 粒度 |
|------|------|
| `list_doc_sources` | 源级别（每个 llms.txt 一条） |
| `search_docs` | 条目级别（BM25 跨源搜索） |
| `fetch_docs` | 页面级别（取全文） |

缺少"列出某个源里有哪些页面"的能力。LLM 想在特定源内精确匹配页面时，只能 `fetch_docs` 整个 llms.txt（可能很大）或靠 `search_docs` 的 BM25 排序（只索引 title+description 元数据，非正文全文，精准度有限）。

## Solution

新增第 4 个 MCP 工具 `list_pages`，列出指定源 llms.txt 里的所有页面条目（title + url + description）。形成三级递进流程：

1. `list_doc_sources` — 看有哪些源
2. `list_pages` — 看某源里有哪些页面
3. `fetch_docs` — 取具体页面全文

`search_docs` 仍作为跨源 BM25 搜索的首选，`list_pages` 补充源内精确浏览场景。

## Design Decisions

### 1. 数据来源：独立抓取解析

每次调用 `list_pages` 时现场抓取/解析该源的 llms.txt，返回最新页面列表。

- 不依赖索引是否构建，`--index false` 时也可用
- 与 `list_doc_sources`（源级）→ `list_pages`（页面级）→ `fetch_docs`（全文）的三级流程自然契合
- 代价是每次调用有一次网络/磁盘 IO（llms.txt 通常几 KB，很快）
- 复用 `fetch.rs` 的 `fetch_remote` / `read_local_file`，以及 `self.http_client` / `self.config.timeout` / `self.allowed_local_files` / `self.allowed_local_dirs`（本地文件走与 `fetch_docs` 相同的白名单安全边界）

### 2. 参数与匹配：source 必填，模糊匹配

```rust
#[derive(Deserialize, JsonSchema)]
pub struct ListPagesParams {
    /// Source name to list pages from. Fuzzy (case-insensitive substring) match.
    /// Get exact names from list_doc_sources. Matches multiple sources if ambiguous.
    pub source: String,
}
```

- `source` 必填
- 模糊匹配：大小写不敏感的子串包含，即 `source_name.to_lowercase().contains(&query.to_lowercase())`
- 源名取 `get_source_name(&llms_txt, name)` 的结果，与 `list_doc_sources` 输出一致

### 3. 多源命中：合并返回

模糊匹配命中多个源时（如 `"react"` 同时匹配 React 和 ReactFlow），合并所有命中源的页面一起返回，按源分组输出。

### 4. 输出规模：全量返回

无 `limit` 参数，返回该源全部页面条目。llms.txt 通常几十到几百条，可接受；LLM 能看到完整页面清单做精确匹配。

## Output Format

按源分组，每个条目列出 title + url + description：

```
Found 2 sources matching 'uv', 15 pages total:

## uv (https://docs.astral.sh/llms.txt)
1. Getting Started | https://docs.astral.sh/uv/getting-started/
   Install and set up uv in minutes.
2. Guide | https://docs.astral.sh/uv/guides/
   Comprehensive guides on using uv.
...

## uv-docs (Path: E:\code\docs\llms\uv-docs-llms.txt)
1. Index | https://example.com/uv/
   ...
```

- 源头：`## {source_name} ({url_or_path})` 标题行，其中 `url_or_path` 对远程源显示 URL，对本地源显示 `Path: {normalize_path 后的绝对路径}`（与 `list_doc_sources` 一致）
- 条目：`{序号}. {title} | {url}\n   {description}`（description 为空则省略第二行）
- 顶部汇总：命中源数 + 总页面数
- 单源命中也用同样格式（保持一致）

## Error Handling

| 场景 | 行为 |
|------|------|
| `source` 为空字符串 | 返回错误："source parameter is required. Call list_doc_sources to see available sources." |
| 模糊匹配 0 个源 | 返回错误 + 所有可用源名列表（让 LLM 收敛重调） |
| 某命中源的 llms.txt fetch 失败 | 跳过该源，继续处理其他命中源；在输出里标注该源失败原因；全部失败则返回错误 |
| llms.txt 解析出 0 个条目 | 该源仍出现在输出里，标注 "no pages found" |
| 本地文件不在白名单 | 不会发生——`list_pages` 只读 `doc_sources` 里已配置的源，启动时已校验过白名单 |

错误返回方式与现有工具一致：`CallToolResult::success(Content::text(...))`（非 MCP error，让 LLM 看到错误文本）。

## Code Changes

### 1. `src/domain.rs` — 移入 llms.txt 解析逻辑

从 `index/mod.rs` 移入：
- `DocEntry` struct，改 `pub`
- `parse_llms_txt` 函数，改 `pub`
- `MD_LINK` 正则（`Lazy<Regex>`）

`domain.rs` 本就是放领域解析的地方（路径、URL、源名都在这），llms.txt 解析属于领域逻辑。

### 2. `src/index/mod.rs` — 改 import

- `parse_llms_txt` / `DocEntry` 改为 `use crate::domain::{DocEntry, parse_llms_txt}`
- 删除本地的定义和 `MD_LINK` 正则
- 行为完全不变（只是换了定义位置）

### 3. `src/server.rs` — 新增 `list_pages` 工具

- 新增 `ListPagesParams` struct
- 新增 `list_pages` async 方法，挂在 `#[tool_router] impl` 上
- 复用 `fetch_remote` / `read_local_file` / `parse_llms_txt` / `get_source_name`
- 输出格式按上文 Output Format

### 4. `src/server.rs` — 更新 `generate_instructions`

工具列表里加入 `list_pages`，更新 "How to use" 流程为三级：
1. `list_doc_sources` — 看有哪些源
2. `list_pages` — 看某源里有哪些页面（模糊匹配源名）
3. `fetch_docs` — 取具体页面全文
4. `search_docs` 仍作为跨源搜索的首选

### 5. 测试

- `parse_llms_txt` 的现有测试从 `index/mod.rs` 移到 `domain.rs`（3 个 test）
- 新增 `list_pages` 模糊匹配逻辑的测试：多源合并、0 命中、单源

## Out of Scope

- 不索引页面正文全文（那是单独的改进方向）
- 不给 `search_docs` 加源过滤参数（那是另一个改进方向）
- 不改 `search_docs` 的现有行为
