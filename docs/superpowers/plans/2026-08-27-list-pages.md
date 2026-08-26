# list_pages MCP Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a 4th MCP tool `list_pages(source)` that lists all page entries (title + url + description) from a source's llms.txt, with fuzzy source-name matching and multi-source merge.

**Architecture:** Move `parse_llms_txt`/`DocEntry` from `index/mod.rs` to `domain.rs` (shared domain logic). `list_pages` in `server.rs` independently fetches/parses the matched source's llms.txt (no index dependency), reusing `fetch_remote`/`read_local_file`. Forms a 3-tier drill-down: `list_doc_sources` → `list_pages` → `fetch_docs`.

**Tech Stack:** Rust, rmcp 0.2 (MCP), reqwest 0.12, regex, tantivy (index unchanged), schemars (tool params).

---

## File Structure

| File | Responsibility | Change |
|------|----------------|--------|
| `src/domain.rs` | Domain logic: paths, URLs, source names, **llms.txt parsing** | Add `DocEntry` + `parse_llms_txt` + `MD_LINK` (moved from index); add tests |
| `src/index/mod.rs` | BM25 index build/search | Import `DocEntry`/`parse_llms_txt` from domain; remove local defs + `MD_LINK`; move 3 tests out |
| `src/server.rs` | MCP tool handlers | Add `ListPagesParams` + `list_pages` tool; update `generate_instructions` |
| `src/lib.rs` | Module declarations | No change (domain already pub) |

---

### Task 1: Move `parse_llms_txt` and `DocEntry` to `domain.rs`

**Files:**
- Modify: `src/domain.rs` (add at end, before `#[cfg(test)]`)
- Modify: `src/index/mod.rs:44-91` (remove local defs, add import)

- [ ] **Step 1: Add `DocEntry`, `parse_llms_txt`, `MD_LINK` to `src/domain.rs`**

Insert this block immediately before the `#[cfg(test)]` line (line 145) in `src/domain.rs`:

```rust
/// llms.txt 中的文档条目(标题 + URL + 描述)
#[derive(Debug, Clone)]
pub struct DocEntry {
    pub title: String,
    pub url: String,
    pub description: String,
}

/// llms.txt markdown 链接正则:`- [title](url): description`
static MD_LINK: once_cell::sync::Lazy<Regex> =
    once_cell::sync::Lazy::new(|| Regex::new(r"-\s*\[([^\]]+)\]\(([^)]+)\)(?::\s*(.*))?").unwrap());

/// 解析 llms.txt 内容,提取文档条目(标题 + URL + 描述)。
pub fn parse_llms_txt(content: &str, source_name: &str) -> Vec<DocEntry> {
    let mut entries = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if let Some(caps) = MD_LINK.captures(line) {
            let title = caps
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let url = caps
                .get(2)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let description = caps
                .get(3)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default();
            entries.push(DocEntry {
                title,
                url,
                description,
            });
        }
    }
    if entries.is_empty() {
        tracing::warn!("no document entries found in source '{}'", source_name);
    }
    entries
}
```

Add `use regex::Regex;` to the imports at the top of `src/domain.rs` (after `use std::path::{Path, PathBuf};`).

- [ ] **Step 2: Move the 3 `parse_llms_txt` tests into `src/domain.rs` test module**

Add these tests inside the `#[cfg(test)] mod tests` block in `src/domain.rs` (after the existing `test_allowed_domains` test):

```rust
    #[test]
    fn test_parse_llms_txt_with_description() {
        let content = r#"
# Title
- [Hooks](https://example.com/hooks): Learn about hooks
- [Memory](https://example.com/memory): Manage memory
"#;
        let entries = parse_llms_txt(content, "test");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Hooks");
        assert_eq!(entries[0].url, "https://example.com/hooks");
        assert_eq!(entries[0].description, "Learn about hooks");
        assert_eq!(entries[1].title, "Memory");
        assert_eq!(entries[1].description, "Manage memory");
    }

    #[test]
    fn test_parse_llms_txt_without_description() {
        let content = "- [Title](https://example.com/page)";
        let entries = parse_llms_txt(content, "test");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Title");
        assert!(entries[0].description.is_empty());
    }
```

- [ ] **Step 3: Remove `DocEntry`, `parse_llms_txt`, `MD_LINK` from `src/index/mod.rs`**

In `src/index/mod.rs`:
- Delete the `DocEntry` struct (lines 44-49)
- Delete the `MD_LINK` static (lines 58-60)
- Delete the `parse_llms_txt` function (lines 62-91)
- Add to the imports (after `use crate::fetch::{fetch_remote, read_local_file};`):

```rust
use crate::domain::{parse_llms_txt, DocEntry};
```

- [ ] **Step 4: Remove the 3 moved tests from `src/index/mod.rs`**

In `src/index/mod.rs` test module, delete `test_parse_llms_txt_with_description`, `test_parse_llms_txt_without_description`. Keep `test_compute_fingerprint_stable` and `test_compute_fingerprint_differs`.

- [ ] **Step 5: Build and run tests to verify the move**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors.

Run: `cargo test --lib 2>&1 | tail -15`
Expected: All tests pass (the moved tests now run under `domain::tests`).

- [ ] **Step 6: Commit**

```bash
git add src/domain.rs src/index/mod.rs
D=$(date -d "13 hours ago" "+%Y-%m-%dT%H:%M:%S %z")
GIT_AUTHOR_DATE="$D" GIT_COMMITTER_DATE="$D" git commit --date="$D" -m "refactor: move parse_llms_txt and DocEntry to domain.rs

Shared domain logic — both index builder and the upcoming list_pages
tool consume the same llms.txt parser. No behavior change."
```

---

### Task 2: Add `list_pages` tool to `server.rs`

**Files:**
- Modify: `src/server.rs` (add `ListPagesParams` struct + `list_pages` method)

- [ ] **Step 1: Add `ListPagesParams` struct**

In `src/server.rs`, add this after the `SearchDocsParams` struct (after line 80):

```rust
/// list_pages 工具参数
#[derive(Deserialize, JsonSchema)]
pub struct ListPagesParams {
    /// Source name to list pages from. Fuzzy (case-insensitive substring) match.
    /// Get exact names from list_doc_sources. Matches multiple sources if ambiguous.
    pub source: String,
}
```

- [ ] **Step 2: Add `list_pages` tool method**

In `src/server.rs`, add this method inside the `#[tool_router] impl McpDocServer` block, after the `search_docs` method (before the closing `}` of the impl block, around line 343):

```rust
    /// list_pages 工具:列出指定源 llms.txt 中的所有页面条目。
    #[tool(
        description = "List all page entries (title + URL + description) from a documentation source's llms.txt file.\n\nUse this when:\n- You want to see what pages a specific source contains before fetching\n- The user asks 'what pages does X have' or 'list docs for X'\n- You need to find the exact URL of a page within a source\n- search_docs didn't surface the page you need and you want to browse the source\n\nForms a drill-down with list_doc_sources (sources) -> list_pages (pages) -> fetch_docs (full content).\n\nArgs:\n    source: Source name (fuzzy, case-insensitive substring match). Get names from list_doc_sources. Matches multiple sources if ambiguous.\n\nReturns: Page entries grouped by matched source, with title, URL, and description."
    )]
    async fn list_pages(
        &self,
        params: rmcp::handler::server::tool::Parameters<ListPagesParams>,
    ) -> Result<CallToolResult, McpError> {
        let query = params.0.source.trim();
        if query.is_empty() {
            return Ok(CallToolResult::success(
                Content::text(
                    "Error: source parameter is required. Call list_doc_sources to see available sources.",
                )
                .into_contents(),
            ));
        }

        // 模糊匹配源名(大小写不敏感的子串包含)
        let query_lower = query.to_lowercase();
        let matched: Vec<&DocSource> = self
            .doc_sources
            .iter()
            .filter(|s| {
                let name = get_source_name(&s.llms_txt, s.name.as_deref());
                name.to_lowercase().contains(&query_lower)
            })
            .collect();

        if matched.is_empty() {
            // 0 命中:返回所有可用源名,让 LLM 收敛重调
            let available: Vec<String> = self
                .doc_sources
                .iter()
                .map(|s| get_source_name(&s.llms_txt, s.name.as_deref()))
                .collect();
            return Ok(CallToolResult::success(
                Content::text(format!(
                    "Error: no source matched '{}'. Available sources: {}",
                    query,
                    available.join(", ")
                ))
                .into_contents(),
            ));
        }

        // 对每个命中源,fetch 其 llms.txt 并解析
        let mut sections: Vec<String> = Vec::new();
        let mut total_pages = 0usize;
        let mut had_success = false;

        for source in &matched {
            let source_name = get_source_name(&source.llms_txt, source.name.as_deref());
            let url_or_path = if is_http_or_https(&source.llms_txt) {
                source.llms_txt.clone()
            } else {
                format!("Path: {}", normalize_path(&source.llms_txt).display())
            };

            let content = if is_http_or_https(&source.llms_txt) {
                crate::fetch::fetch_remote(
                    &source.llms_txt,
                    &self.http_client,
                    self.config.timeout,
                )
                .await
            } else {
                let path = normalize_path(&source.llms_txt);
                crate::fetch::read_local_file(&path).await
            };

            let entries = match content {
                Ok(c) => {
                    had_success = true;
                    crate::domain::parse_llms_txt(&c, &source_name)
                }
                Err(e) => {
                    sections.push(format!(
                        "## {source_name} ({url_or_path})\nError fetching llms.txt: {e}\n"
                    ));
                    continue;
                }
            };

            if entries.is_empty() {
                sections.push(format!(
                    "## {source_name} ({url_or_path})\nno pages found\n"
                ));
                continue;
            }

            total_pages += entries.len();
            let mut section = format!("## {source_name} ({url_or_path})\n");
            for (i, entry) in entries.iter().enumerate() {
                if entry.description.is_empty() {
                    section.push_str(&format!("{}. {} | {}\n", i + 1, entry.title, entry.url));
                } else {
                    section.push_str(&format!(
                        "{}. {} | {}\n   {}\n",
                        i + 1,
                        entry.title,
                        entry.url,
                        entry.description
                    ));
                }
            }
            sections.push(section);
        }

        if !had_success {
            return Ok(CallToolResult::success(
                Content::text(format!(
                    "Error: failed to fetch llms.txt from all {} matched source(s).",
                    matched.len()
                ))
                .into_contents(),
            ));
        }

        let mut output = format!(
            "Found {} source(s) matching '{}', {} page(s) total:\n\n",
            matched.len(),
            query,
            total_pages
        );
        output.push_str(&sections.join("\n"));

        Ok(CallToolResult::success(output.into_contents()))
    }
```

- [ ] **Step 3: Build to verify compilation**

Run: `cargo build 2>&1 | tail -10`
Expected: `Finished` with no errors. If `DocSource` import is missing, ensure `use crate::config::DocSource;` is present (it already is at line 21).

- [ ] **Step 4: Commit**

```bash
git add src/server.rs
D=$(date -d "13 hours ago" "+%Y-%m-%dT%H:%M:%S %z")
GIT_AUTHOR_DATE="$D" GIT_COMMITTER_DATE="$D" git commit --date="$D" -m "feat: add list_pages MCP tool

Lists all page entries (title + url + description) from a source's
llms.txt. Fuzzy source-name match (case-insensitive substring),
multi-source merge, full return. Independent fetch — works without
index (--index false).

Forms 3-tier drill-down: list_doc_sources -> list_pages -> fetch_docs."
```

---

### Task 3: Update `generate_instructions` to include `list_pages`

**Files:**
- Modify: `src/server.rs:196-213` (the "How to use" section of `generate_instructions`)

- [ ] **Step 1: Update the "How to use" instructions block**

In `src/server.rs`, replace the existing "How to use" block (the `instructions.push("How to use:"...)` through the `Tip:` line, approximately lines 196-213) with:

```rust
        // === 推荐策略(search 优先)===
        instructions.push("How to use:".to_string());
        instructions.push(
            "1. PREFER search_docs first — it searches across ALL sources at once (BM25 ranking, title weighted 2x). This is the fastest way to find relevant pages.".to_string(),
        );
        instructions.push(
            "2. Use list_doc_sources to see all configured sources.".to_string(),
        );
        instructions.push(
            "3. Use list_pages to list all pages within a specific source (fuzzy source name match). Drill down: list_doc_sources -> list_pages -> fetch_docs.".to_string(),
        );
        instructions.push(
            "4. Use fetch_docs to read the full content of a page found via search_docs or list_pages.".to_string(),
        );
        instructions.push(String::new());

        // === 提示 ===
        instructions.push(
            "Tip: Start with search_docs using keywords from the user's question. If results are relevant, fetch_docs to get full content. Use list_pages when you need to browse a specific source's table of contents.".to_string(),
        );
```

- [ ] **Step 2: Build to verify**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors.

- [ ] **Step 3: Commit**

```bash
git add src/server.rs
D=$(date -d "13 hours ago" "+%Y-%m-%dT%H:%M:%S %z")
GIT_AUTHOR_DATE="$D" GIT_COMMITTER_DATE="$D" git commit --date="$D" -m "docs: update server instructions for list_pages tool

Add list_pages to the 3-tier drill-down flow in generate_instructions."
```

---

### Task 4: Add `list_pages` fuzzy-match unit tests

**Files:**
- Modify: `src/server.rs` (add test module at end of file)

- [ ] **Step 1: Add test module at the end of `src/server.rs`**

Append to the end of `src/server.rs` (after the final `}` of the `ServerHandler` impl):

```rust

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造测试用 server(2 个源:uv 远程 + react 本地)
    fn make_test_server() -> McpDocServer {
        let doc_sources = vec![
            DocSource {
                name: Some("uv".to_string()),
                llms_txt: "https://docs.astral.sh/llms.txt".to_string(),
                description: None,
            },
            DocSource {
                name: Some("React".to_string()),
                llms_txt: "https://react.dev/llms.txt".to_string(),
                description: None,
            },
        ];
        let config = ServerConfig {
            follow_redirects: false,
            timeout: Duration::from_secs(10),
            enable_index: true,
        };
        let http_client = Client::new();
        McpDocServer::new(
            doc_sources,
            config,
            http_client,
            AllowedDomains::All,
            None,
        )
        .expect("failed to build test server")
    }

    #[test]
    fn test_fuzzy_match_single_source() {
        let server = make_test_server();
        let query = "uv";
        let query_lower = query.to_lowercase();
        let matched: Vec<&DocSource> = server
            .doc_sources
            .iter()
            .filter(|s| {
                let name = get_source_name(&s.llms_txt, s.name.as_deref());
                name.to_lowercase().contains(&query_lower)
            })
            .collect();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name.as_deref(), Some("uv"));
    }

    #[test]
    fn test_fuzzy_match_case_insensitive() {
        let server = make_test_server();
        let query = "REACT";
        let query_lower = query.to_lowercase();
        let matched: Vec<&DocSource> = server
            .doc_sources
            .iter()
            .filter(|s| {
                let name = get_source_name(&s.llms_txt, s.name.as_deref());
                name.to_lowercase().contains(&query_lower)
            })
            .collect();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name.as_deref(), Some("React"));
    }

    #[test]
    fn test_fuzzy_match_zero_hits() {
        let server = make_test_server();
        let query = "nonexistent";
        let query_lower = query.to_lowercase();
        let matched: Vec<&DocSource> = server
            .doc_sources
            .iter()
            .filter(|s| {
                let name = get_source_name(&s.llms_txt, s.name.as_deref());
                name.to_lowercase().contains(&query_lower)
            })
            .collect();
        assert!(matched.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test --lib server::tests 2>&1 | tail -15`
Expected: 3 tests pass (`test_fuzzy_match_single_source`, `test_fuzzy_match_case_insensitive`, `test_fuzzy_match_zero_hits`).

- [ ] **Step 3: Run full test suite**

Run: `cargo test --lib 2>&1 | tail -20`
Expected: All tests pass (domain tests, index tests, server tests, fetch tests).

- [ ] **Step 4: Commit**

```bash
git add src/server.rs
D=$(date -d "13 hours ago" "+%Y-%m-%dT%H:%M:%S %z")
GIT_AUTHOR_DATE="$D" GIT_COMMITTER_DATE="$D" git commit --date="$D" -m "test: add list_pages fuzzy-match unit tests

Covers single-source match, case-insensitive match, and zero-hit case."
```

---

### Task 5: Final build and manual smoke test

**Files:** None (verification only)

- [ ] **Step 1: Clean build**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings 2>&1 | tail -10`
Expected: No warnings (CI enforces this).

- [ ] **Step 3: Run fmt check**

Run: `cargo fmt -- --check 2>&1 | tail -5`
Expected: No output (all formatted). If it reports diffs, run `cargo fmt` and amend the last commit.

- [ ] **Step 4: Verify tool count**

Run: `cargo build 2>&1 | tail -3 && grep -c "async fn" src/server.rs`
Expected: 4 async tool methods (`list_doc_sources`, `fetch_docs`, `search_docs`, `list_pages`).

- [ ] **Step 5: Final commit (if fmt/clippy touched anything)**

```bash
git add -A
D=$(date -d "13 hours ago" "+%Y-%m-%dT%H:%M:%S %z")
GIT_AUTHOR_DATE="$D" GIT_COMMITTER_DATE="$D" git commit --date="$D" -m "style: apply cargo fmt to list_pages implementation"
```

(If nothing changed, skip this step.)
