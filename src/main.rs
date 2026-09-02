//! mcpdoc-rust 入口:CLI 解析 + 传输分发

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use mcpdoc_rust::cli::{Cli, TransportType};
use mcpdoc_rust::config::DocSource;
use mcpdoc_rust::domain::{extract_domain, is_http_or_https, AllowedDomains};
use mcpdoc_rust::server::{McpDocServer, ServerConfig};
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // 初始化日志(输出到 stderr,避免干扰 stdio MCP 协议)
    let filter = EnvFilter::try_new(&cli.log_level).unwrap_or_else(|_| EnvFilter::new("INFO"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let doc_sources = cli.load_doc_sources()?;

    let http_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    let allowed_domains = build_allowed_domains(&doc_sources, &cli);

    let server_config = ServerConfig {
        follow_redirects: cli.follow_redirects,
        timeout: Duration::from_secs_f64(cli.timeout),
        enable_index: cli.index,
    };

    // 索引缓存目录:默认 <exe目录>/cache/mcpdoc-rust,便携部署
    let cache_dir = cli.cache_dir.clone().or_else(dirs_cache_dir);

    let server = McpDocServer::new(
        doc_sources,
        server_config,
        http_client,
        allowed_domains,
        cache_dir,
    )?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        match cli.transport {
            TransportType::Stdio => run_stdio(server, &cli).await,
            TransportType::Sse => run_sse(server, &cli).await,
        }
    })
}

/// 构建 allowed_domains,对齐 Python create_server:
/// 1. 远程源的域名自动加入
/// 2. --allowed-domains '*' → All
/// 3. --allowed-domains a b → 加入指定域名
fn build_allowed_domains(doc_sources: &[DocSource], cli: &Cli) -> AllowedDomains {
    if cli.allow_all_domains() {
        return AllowedDomains::All;
    }

    let mut domains: HashSet<String> = HashSet::new();
    for entry in doc_sources {
        if is_http_or_https(&entry.llms_txt) {
            if let Some(d) = extract_domain(&entry.llms_txt) {
                domains.insert(d);
            }
        }
    }
    for d in &cli.allowed_domains {
        if d != "*" {
            domains.insert(d.clone());
        }
    }
    AllowedDomains::from_set(domains)
}

/// 获取缓存目录:基于 exe 所在目录的 `./cache` 子目录
fn dirs_cache_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join("cache")))
}

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
        if !bridge::wait_for_sse_server(port, Duration::from_secs(10)).await {
            anyhow::bail!("SSE server did not become ready within 10 seconds");
        }
        tracing::info!("bridge: SSE server is ready");
    } else {
        tracing::info!("bridge: SSE server already running, connecting...");
    }

    // 2. 启动桥接转发
    bridge::run_bridge(port).await
}

/// SSE 传输:用 rmcp 的 SseServer 启动 HTTP 服务器
async fn run_sse(server: McpDocServer, cli: &Cli) -> anyhow::Result<()> {
    use rmcp::transport::sse_server::SseServer;

    let addr: std::net::SocketAddr = format!("{}:{}", cli.host, cli.port)
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid address: {e}"))?;

    tracing::info!("Starting mcpdoc-rust in SSE mode on http://{}", addr);

    let sse_server = SseServer::serve(addr)
        .await
        .map_err(|e| anyhow::anyhow!("failed to start SSE server: {e}"))?;

    let ct = sse_server.with_service(move || server.clone());

    // 打印启动信息到 stderr
    eprintln!();
    eprintln!("  mcpdoc-rust SSE server running at http://{}", addr);
    eprintln!("  SSE endpoint: http://{}/sse", addr);
    eprintln!("  Message endpoint: http://{}/message", addr);
    eprintln!();
    eprintln!("  {} doc sources loaded", cli.urls.len());
    eprintln!();

    ct.cancelled().await;
    Ok(())
}
