//! MCP 服务器核心:实现 ServerHandler trait,注册 list_doc_sources / fetch_docs / search_docs 工具。
//! 对齐 Python `mcpdoc/main.py` 的 `create_server`。

use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use once_cell::sync::OnceCell;
use reqwest::Client;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::model::{
    CallToolResult, Content, Implementation, IntoContents, ProtocolVersion, ServerCapabilities,
    ServerInfo,
};
use rmcp::{tool, tool_handler, tool_router, Error as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::config::DocSource;
use crate::domain::{
    extract_domain, get_source_name, is_http_or_https, normalize_path, AllowedDomains,
};
use crate::fetch::fetch_document;

/// 从 llms.txt 内容提取首行 `# 标题` 或 `> 描述` 作为文档源描述。
fn extract_llms_txt_title(content: &str) -> Option<String> {
    for line in content.lines().take(5) {
        let line = line.trim();
        if let Some(title) = line.strip_prefix("# ") {
            return Some(title.trim().to_string());
        }
        if let Some(desc) = line.strip_prefix("> ") {
            return Some(desc.trim().to_string());
        }
    }
    None
}

/// 服务器配置
#[derive(Clone)]
pub struct ServerConfig {
    pub follow_redirects: bool,
    pub timeout: Duration,
    pub enable_index: bool,
}

/// MCP 文档服务器
#[derive(Clone)]
pub struct McpDocServer {
    pub doc_sources: Arc<Vec<DocSource>>,
    pub config: ServerConfig,
    pub http_client: Client,
    pub allowed_domains: AllowedDomains,
    pub allowed_local_files: Arc<HashSet<PathBuf>>,
    /// 本地 llms.txt 所在目录(用于相对路径解析)
    pub allowed_local_dirs: Arc<Vec<PathBuf>>,
    /// 索引缓存目录(持久化,多进程共享)
    pub cache_dir: Arc<Option<PathBuf>>,
    pub tool_router: ToolRouter<Self>,
    pub index: Arc<OnceCell<crate::index::SearchIndex>>,
}

/// fetch_docs 工具参数
#[derive(Deserialize, JsonSchema)]
pub struct FetchDocsParams {
    /// The URL or file path to fetch documentation from.
    pub url: String,
}

/// search_docs 工具参数
#[derive(Deserialize, JsonSchema)]
pub struct SearchDocsParams {
    /// The search query.
    pub query: String,
    /// Maximum number of results to return. Defaults to 10.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// list_pages 工具参数
#[derive(Deserialize, JsonSchema)]
pub struct ListPagesParams {
    /// Source name to list pages from. Fuzzy (case-insensitive substring) match.
    /// Get exact names from list_doc_sources. Matches multiple sources if ambiguous.
    pub source: String,
}

impl McpDocServer {
    /// 创建服务器实例,对齐 Python `create_server`。
    pub fn new(
        doc_sources: Vec<DocSource>,
        config: ServerConfig,
        http_client: Client,
        allowed_domains: AllowedDomains,
        cache_dir: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        // 验证本地文件存在,对齐 Python 的本地源检查
        let mut allowed_local_files: HashSet<PathBuf> = HashSet::new();
        let mut allowed_local_dirs: Vec<PathBuf> = Vec::new();
        for entry in &doc_sources {
            if !is_http_or_https(&entry.llms_txt) {
                let abs_path = normalize_path(&entry.llms_txt);
                if !abs_path.exists() {
                    anyhow::bail!("Local file not found: {}", abs_path.display());
                }
                allowed_local_files.insert(abs_path.clone());
                // 记录 llms.txt 所在目录,用于相对路径解析
                if let Some(parent) = abs_path.parent() {
                    allowed_local_dirs.push(parent.to_path_buf());
                }
            }
        }

        // 为缺少 description 的本地文档源自动提取首行标题作为描述
        let doc_sources: Vec<DocSource> = doc_sources
            .into_iter()
            .map(|mut s| {
                if s.description.is_none() && !is_http_or_https(&s.llms_txt) {
                    let path = normalize_path(&s.llms_txt);
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        s.description = extract_llms_txt_title(&content);
                    }
                }
                s
            })
            .collect();

        Ok(Self {
            doc_sources: Arc::new(doc_sources),
            config,
            http_client,
            allowed_domains,
            allowed_local_files: Arc::new(allowed_local_files),
            allowed_local_dirs: Arc::new(allowed_local_dirs),
            cache_dir: Arc::new(cache_dir),
            tool_router: Self::tool_router(),
            index: Arc::new(OnceCell::new()),
        })
    }

    /// 生成服务器 instructions。
    ///
    /// 优化命中率:
    /// - 明确 "when to use" 触发条件,让 LLM 主动调用
    /// - 推荐 search_docs 优先(快、跨源)
    /// - 列出文档源名称 + 描述,帮助 LLM 判断相关性
    /// - 通用规则,不针对特定源
    fn generate_instructions(&self) -> String {
        let sources_info: Vec<(String, Option<String>)> = self
            .doc_sources
            .iter()
            .map(|e| {
                let name = get_source_name(&e.llms_txt, e.name.as_deref());
                let desc = e.description.clone();
                (name, desc)
            })
            .collect();

        let mut instructions = vec![
            "Use this server when the user asks about topics covered by the documentation sources listed below. This includes:".to_string(),
            "- How to use, configure, or troubleshoot a tool/library/framework".to_string(),
            "- API references, command-line flags, or configuration options".to_string(),
            "- Concepts, best practices, or workflows for the documented projects".to_string(),
            "- \"What is...\" or \"How do I...\" questions about these technologies".to_string(),
            "- Tutorials, getting started guides, or step-by-step examples".to_string(),
            "- When you (the assistant) are unsure about a concept, API, or behavior of a documented tool — verify against the docs instead of guessing".to_string(),
            "- When repeated mistakes occur in code/config and the root cause might be a misunderstanding of how a documented tool works".to_string(),
            String::new(),
            "Documentation sources available:".to_string(),
        ];
        const MAX_SOURCES_IN_INSTRUCTIONS: usize = 15;
        if !sources_info.is_empty() {
            if sources_info.len() <= MAX_SOURCES_IN_INSTRUCTIONS {
                for (name, desc) in &sources_info {
                    if let Some(d) = desc {
                        instructions.push(format!("- {name}: {d}"));
                    } else {
                        instructions.push(format!("- {name}"));
                    }
                }
            } else {
                let preview: Vec<String> = sources_info
                    .iter()
                    .take(8)
                    .map(|(n, d)| {
                        if let Some(desc) = d {
                            format!("- {n}: {desc}")
                        } else {
                            format!("- {n}")
                        }
                    })
                    .collect();
                instructions.push(format!(
                    "{} sources available. Some of them:\n{}\n... call list_doc_sources to see all.",
                    sources_info.len(),
                    preview.join("\n")
                ));
            }
        }
        instructions.push(String::new());

        // === 推荐策略(search 优先)===
        instructions.push("How to use:".to_string());
        instructions.push(
            "1. PREFER search_docs first — it searches across ALL sources at once (BM25 ranking, title weighted 2x). This is the fastest way to find relevant pages.".to_string(),
        );
        instructions.push(
            "2. Use list_doc_sources only if you need to see all sources or find a specific llms.txt file.".to_string(),
        );
        instructions.push(
            "3. Use fetch_docs to read the full content of a page found via search_docs or listed in an llms.txt file.".to_string(),
        );
        instructions.push(String::new());

        // === 提示 ===
        instructions.push(
            "Tip: Start with search_docs using keywords from the user's question. If results are relevant, fetch_docs to get full content. Avoid fetching entire llms.txt files unless the user asks for an overview.".to_string(),
        );

        instructions.join("\n")
    }
}

#[tool_router]
impl McpDocServer {
    /// list_doc_sources 工具:列出所有文档源(含描述)。对齐 Python `list_doc_sources`。
    #[tool(
        description = "List all available documentation sources with names, URLs/paths, and descriptions.\n\nUse this when:\n- You need to see what documentation is available\n- The user asks 'what docs do we have' or 'which sources are configured'\n- You want to find the llms.txt file for a specific source\n\nReturns a formatted list: Name + URL/Path + Description for each source."
    )]
    async fn list_doc_sources(&self) -> Result<CallToolResult, McpError> {
        let mut content = String::new();
        for entry in self.doc_sources.iter() {
            let url_or_path = &entry.llms_txt;
            if is_http_or_https(url_or_path) {
                let name = entry
                    .name
                    .clone()
                    .unwrap_or_else(|| extract_domain(url_or_path).unwrap_or_default());
                content.push_str(&format!("{name}\nURL: {url_or_path}"));
            } else {
                let path = normalize_path(url_or_path);
                let name = entry
                    .name
                    .clone()
                    .unwrap_or_else(|| path.display().to_string());
                content.push_str(&format!("{name}\nPath: {}", path.display()));
            }
            if let Some(desc) = &entry.description {
                content.push_str(&format!("\nDescription: {desc}"));
            }
            content.push_str("\n\n");
        }
        Ok(CallToolResult::success(content.into_contents()))
    }

    /// fetch_docs 工具:获取文档内容。对齐 Python `fetch_docs`。
    #[tool(
        description = "Fetch and parse documentation content from a URL or local file path.\n\nUse this when:\n- You have a specific doc URL/path (from search_docs results or an llms.txt file)\n- The user asks to read a specific documentation page\n- You need the full content of a page (search_docs only returns snippets)\n\nThe content is converted from HTML to Markdown if needed. Supports remote URLs (allowed domains) and local files (allowed paths, including relative paths from llms.txt directory).\n\nArgs:\n    url: URL or file path. Supports http(s)://, file:///, absolute paths, and relative paths.\n\nReturns: Document content as Markdown, or an error message."
    )]
    async fn fetch_docs(
        &self,
        params: rmcp::handler::server::tool::Parameters<FetchDocsParams>,
    ) -> Result<CallToolResult, McpError> {
        let url = params.0.url;
        let result = fetch_document(
            &url,
            &self.http_client,
            &self.allowed_domains,
            &self.allowed_local_files,
            &self.allowed_local_dirs,
            self.config.follow_redirects,
            self.config.timeout,
        )
        .await;

        match result {
            Ok(content) => Ok(CallToolResult::success(content.into_contents())),
            Err(e) => {
                // 对齐 Python:错误作为文本内容返回(非 MCP error),让 LLM 看到错误
                Ok(CallToolResult::success(Content::text(e).into_contents()))
            }
        }
    }

    /// search_docs 工具:BM25 全文搜索(新功能)。
    #[tool(
        description = "Search across ALL documentation sources using BM25 full-text search.\n\nUse this as the FIRST step when:\n- The user asks a 'how to', 'what is', or 'how do I' question about a documented tool\n- The user asks for tutorials, examples, or getting started guides\n- You need to find which doc page covers a topic\n- You're not sure which source contains the answer\n- You (the assistant) are unsure about a concept/API/behavior — verify against docs instead of guessing\n- Repeated code/config mistakes occur and the root cause might be a misunderstanding of a documented tool\n- The user mentions keywords related to any configured documentation\n\nThe index builds lazily on first call (a few seconds). Title fields weighted 2x for relevance.\n\nArgs:\n    query: Search keywords (e.g., 'hooks', 'memory management', 'uv add')\n    limit: Max results (default 10)\n\nReturns: List of matches with score, URL, title, source name, and snippet. Higher score = more relevant.\n\nTip: After finding relevant results, use fetch_docs to read the full page content."
    )]
    async fn search_docs(
        &self,
        params: rmcp::handler::server::tool::Parameters<SearchDocsParams>,
    ) -> Result<CallToolResult, McpError> {
        if !self.config.enable_index {
            return Ok(CallToolResult::success(
                Content::text("Error: BM25 index is disabled. Start the server with --index true to enable search_docs.").into_contents(),
            ));
        }

        let limit = params.0.limit.unwrap_or(10);
        let query = &params.0.query;
        let index = match self.index.get() {
            Some(idx) => idx,
            None => {
                // 懒加载构建索引(带持久化缓存)
                let cache_dir_path = self.cache_dir.as_ref().as_ref().map(|p| p.as_path());
                let new_index = crate::index::SearchIndex::build(
                    &self.doc_sources,
                    &self.http_client,
                    self.config.timeout,
                    cache_dir_path,
                )
                .await
                .map_err(|e| {
                    McpError::internal_error(format!("failed to build index: {e}"), None)
                })?;

                // OnceCell::get_or_init 不支持 async,用 set + get
                let _ = self.index.set(new_index);
                self.index.get().expect("index was just set")
            }
        };

        match index.search(query, limit) {
            Ok(results) => {
                if results.is_empty() {
                    return Ok(CallToolResult::success(
                        Content::text(format!("No results found for query: '{}'", query))
                            .into_contents(),
                    ));
                }
                let mut content = format!("Found {} results for '{}':\n\n", results.len(), query);
                for (i, r) in results.iter().enumerate() {
                    content.push_str(&format!(
                        "{}. [score: {:.4}] {}\n   Title: {}\n   Source: {}\n   Snippet: {}\n\n",
                        i + 1,
                        r.score,
                        r.url,
                        r.title,
                        r.source_name,
                        r.snippet.chars().take(200).collect::<String>()
                    ));
                }
                Ok(CallToolResult::success(content.into_contents()))
            }
            Err(e) => Ok(CallToolResult::success(
                Content::text(format!("Error searching: {e}")).into_contents(),
            )),
        }
    }

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

            let content = crate::fetch::fetch_llms_txt_content(
                source,
                &self.http_client,
                self.config.timeout,
            )
            .await;

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
}

#[tool_handler]
impl ServerHandler for McpDocServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::LATEST,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "mcpdoc".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(self.generate_instructions()),
        }
    }
}
