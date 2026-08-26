//! 文档获取:支持远程 HTTP 和本地文件,带域名/路径白名单。
//! 对齐 Python `mcpdoc/main.py` 的 `fetch_docs` 工具逻辑。

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use regex::Regex;
use reqwest::Client;

use crate::domain::{
    is_http_or_https, is_path_under_dirs, normalize_path, resolve_local_path, AllowedDomains,
};
use crate::html_to_md::html_to_markdown;

/// meta refresh 标签正则,对齐 Python 的 `<meta http-equiv="refresh" ...>` 检测
static META_REFRESH: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+http-equiv="refresh"\s+content="[^;]+;\s*url=([^"]+)""#).unwrap()
});

/// 获取文档内容并转换为 Markdown。
///
/// 镜像 Python `fetch_docs` 逻辑:
/// 1. 非 HTTP/HTTPS:视为本地文件,检查白名单后读取
/// 2. HTTP/HTTPS:检查域名白名单,GET 请求,可选 meta refresh 重定向处理
/// 3. HTML 内容转换为 Markdown
pub async fn fetch_document(
    url: &str,
    http_client: &Client,
    allowed_domains: &AllowedDomains,
    allowed_local_files: &HashSet<std::path::PathBuf>,
    allowed_local_dirs: &[std::path::PathBuf],
    follow_redirects: bool,
    timeout: Duration,
) -> Result<String, String> {
    let url = url.trim();

    if !is_http_or_https(url) {
        // 本地文件:支持相对路径解析(相对于 llms.txt 所在目录)
        let abs_path = if Path::new(url).is_absolute() || url.starts_with("file://") {
            normalize_path(url)
        } else {
            // 相对路径:相对于 llms.txt 所在目录解析
            resolve_local_path(url, allowed_local_dirs)
        };

        // 安全检查:路径必须在某个 llms.txt 所在目录下
        if !allowed_local_files.contains(&abs_path)
            && !is_path_under_dirs(&abs_path, allowed_local_dirs)
        {
            let allowed: Vec<String> = allowed_local_files
                .iter()
                .map(|p| p.display().to_string())
                .collect();
            return Err(format!(
                "Local file not allowed: {}. Allowed files: {}",
                abs_path.display(),
                allowed.join(", ")
            ));
        }

        let content = tokio::fs::read_to_string(&abs_path)
            .await
            .map_err(|e| format!("Error reading local file: {e}"))?;
        // .md 文件已是 Markdown,直接返回;其他文件(如 .html)走 HTML→Markdown 转换
        if abs_path.extension().map(|e| e == "md").unwrap_or(false) {
            return Ok(content);
        }
        return Ok(html_to_markdown(&content));
    }

    // HTTP/HTTPS
    if !allowed_domains.is_url_allowed(url) {
        return Err(format!(
            "Error: URL not allowed. Must start with one of the following domains: {}",
            allowed_domains.display()
        ));
    }

    let response = http_client
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| format!("Encountered an HTTP error: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("Encountered an HTTP error: status {status}"));
    }

    let mut content = response
        .text()
        .await
        .map_err(|e| format!("Encountered an HTTP error: {e}"))?;

    // meta refresh 重定向处理(对齐 Python follow_redirects 逻辑)
    if follow_redirects {
        if let Some(caps) = META_REFRESH.captures(&content) {
            let redirect_url = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if !redirect_url.is_empty() {
                let new_url = url_join(url, redirect_url);
                if !allowed_domains.is_url_allowed(&new_url) {
                    return Err(format!(
                        "Error: Redirect URL not allowed. Must start with one of the following domains: {}",
                        allowed_domains.display()
                    ));
                }
                let resp = http_client
                    .get(&new_url)
                    .timeout(timeout)
                    .send()
                    .await
                    .map_err(|e| format!("Encountered an HTTP error: {e}"))?;
                content = resp
                    .text()
                    .await
                    .map_err(|e| format!("Encountered an HTTP error: {e}"))?;
            }
        }
    }

    Ok(html_to_markdown(&content))
}

/// 简单的 URL 拼接(对齐 Python `urljoin`)。
fn url_join(base: &str, relative: &str) -> String {
    if relative.starts_with("http://") || relative.starts_with("https://") {
        return relative.to_string();
    }
    if let Ok(base_url) = url::Url::parse(base) {
        if let Ok(joined) = base_url.join(relative) {
            return joined.to_string();
        }
    }
    relative.to_string()
}

/// 读取本地 llms.txt 文件内容(不做白名单检查,用于索引构建)。
pub async fn read_local_file(path: &Path) -> Result<String, String> {
    // 用同步 std::fs 读取,避免 tokio::fs::asyncify 在 Windows 长路径前缀上
    // 返回 "background task failed"(spawn_blocking 的 JoinError)。索引构建是
    // 一次性操作,短暂阻塞 runtime 可接受。
    std::fs::read_to_string(path)
        .map_err(|e| format!("Error reading local file {}: {e}", path.display()))
}

/// 获取远程 llms.txt 内容(不做白名单检查,用于索引构建)。
pub async fn fetch_remote(
    url: &str,
    http_client: &Client,
    timeout: Duration,
) -> Result<String, String> {
    let response = http_client
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| format!("HTTP error fetching {url}: {e}"))?;
    response
        .text()
        .await
        .map_err(|e| format!("HTTP error reading body from {url}: {e}"))
}

/// 获取 llms.txt 内容(远程或本地),用于索引构建和 list_pages 工具。
/// 不做白名单检查——只读取已配置的 doc_sources(启动时已校验)。
pub async fn fetch_llms_txt_content(
    source: &crate::config::DocSource,
    http_client: &Client,
    timeout: Duration,
) -> Result<String, String> {
    if crate::domain::is_http_or_https(&source.llms_txt) {
        fetch_remote(&source.llms_txt, http_client, timeout).await
    } else {
        let path = crate::domain::normalize_path(&source.llms_txt);
        read_local_file(&path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_join() {
        assert_eq!(
            url_join("https://example.com/a/b", "c"),
            "https://example.com/a/c"
        );
        assert_eq!(
            url_join("https://example.com/a/b", "https://other.com/x"),
            "https://other.com/x"
        );
        assert_eq!(
            url_join("https://example.com/a/b", "/x"),
            "https://example.com/x"
        );
    }
}
