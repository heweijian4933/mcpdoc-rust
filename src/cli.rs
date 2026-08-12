//! 命令行接口,对齐 Python mcpdoc/cli.py 的参数与行为。

use std::path::PathBuf;

use clap::Parser;
use serde::Deserialize;

use crate::config::DocSource;

/// MCP LLMS-TXT Documentation Server (Rust 版)
#[derive(Parser, Debug, Clone)]
#[command(
    name = "mcpdoc-rust",
    version,
    about = "MCP LLMS-TXT Documentation Server",
    long_about = None,
    after_help = EXAMPLES
)]
pub struct Cli {
    /// YAML 配置文件路径
    #[arg(short = 'y', long)]
    pub yaml: Option<PathBuf>,

    /// JSON 配置文件路径
    #[arg(short = 'j', long)]
    pub json: Option<PathBuf>,

    /// llms.txt URL 或文件路径列表,格式 `url_or_path` 或 `name:url_or_path`
    #[arg(short = 'u', long, num_args = 1..)]
    pub urls: Vec<String>,

    /// 是否跟随 HTTP 重定向
    #[arg(long)]
    pub follow_redirects: bool,

    /// 额外允许的域名列表。使用 '*' 允许全部域名
    #[arg(long, num_args = 0..)]
    pub allowed_domains: Vec<String>,

    /// HTTP 请求超时(秒)
    #[arg(long, default_value_t = 10.0)]
    pub timeout: f64,

    /// MCP 传输协议
    #[arg(long, default_value = "stdio")]
    pub transport: TransportType,

    /// SSE 绑定主机(仅 --transport sse 使用)
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// SSE 绑定端口(仅 --transport sse 使用)
    #[arg(long, default_value_t = 8000)]
    pub port: u16,

    /// 日志级别:DEBUG, INFO, WARNING, ERROR
    #[arg(long, default_value = "INFO")]
    pub log_level: String,

    /// 是否启用 BM25 索引(search_docs 工具)
    #[arg(long, default_value_t = true)]
    pub index: bool,

    /// 索引缓存目录(持久化,多进程共享)。默认 ~/.cache/mcpdoc-rust
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,
}

/// 传输协议类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportType {
    Stdio,
    Sse,
}

impl std::str::FromStr for TransportType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "stdio" => Ok(Self::Stdio),
            "sse" => Ok(Self::Sse),
            other => Err(format!(
                "unsupported transport: {other} (expected stdio or sse)"
            )),
        }
    }
}

impl std::fmt::Display for TransportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stdio => write!(f, "stdio"),
            Self::Sse => write!(f, "sse"),
        }
    }
}

impl Cli {
    /// 加载并合并所有来源的文档源配置
    pub fn load_doc_sources(&self) -> anyhow::Result<Vec<DocSource>> {
        crate::config::merge_doc_sources(self.yaml.as_deref(), self.json.as_deref(), &self.urls)
    }

    /// 是否允许全部域名
    pub fn allow_all_domains(&self) -> bool {
        self.allowed_domains.iter().any(|d| d == "*")
    }
}

const EXAMPLES: &str = "\
Examples:
  # 直接指定 llms.txt URL(带可选名称)
  mcpdoc-rust --urls LangGraph:https://langchain-ai.github.io/langgraph/llms.txt

  # 使用本地文件
  mcpdoc-rust --urls LocalDocs:/path/to/llms.txt --allowed-domains '*'

  # 使用 YAML 配置
  mcpdoc-rust --yaml sample_config.yaml

  # 组合多个文档源
  mcpdoc-rust --yaml sample_config.yaml --urls LangGraph:https://langchain-ai.github.io/langgraph/llms.txt

  # SSE 传输(自定义主机端口)
  mcpdoc-rust --yaml sample_config.yaml --transport sse --host 0.0.0.0 --port 9000

  # 允许全部域名
  mcpdoc-rust --yaml sample_config.yaml --allowed-domains '*'
";
