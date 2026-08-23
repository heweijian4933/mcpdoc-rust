//! URL 域名提取、白名单管理与本地路径标准化,对齐 Python mcpdoc/main.py。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 允许的域名集合。`All` 表示允许全部(对应 `--allowed-domains '*'`)。
#[derive(Debug, Clone)]
pub enum AllowedDomains {
    All,
    Set(HashSet<String>),
}

impl AllowedDomains {
    /// 创建空集合
    pub fn empty() -> Self {
        Self::Set(HashSet::new())
    }

    /// 从迭代器创建集合
    pub fn from_set<I: IntoIterator<Item = String>>(iter: I) -> Self {
        Self::Set(iter.into_iter().collect())
    }

    /// 是否允许给定 URL
    pub fn is_url_allowed(&self, url: &str) -> bool {
        match self {
            Self::All => true,
            Self::Set(domains) => domains.iter().any(|d| url.starts_with(d)),
        }
    }

    /// 是否允许全部
    pub fn is_all(&self) -> bool {
        matches!(self, Self::All)
    }

    /// 格式化为可读字符串(用于错误消息)
    pub fn display(&self) -> String {
        match self {
            Self::All => "*".to_string(),
            Self::Set(domains) => domains.iter().cloned().collect::<Vec<_>>().join(", "),
        }
    }
}

/// 从 URL 提取带协议和尾斜杠的域名(含端口),如 `https://example.com/` 或 `http://127.0.0.1:18099/`。
/// 对齐 Python `extract_domain`(使用 netloc,含端口)。
pub fn extract_domain(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    match parsed.port() {
        Some(p) => Some(format!("{}://{}:{}/", parsed.scheme(), host, p)),
        None => Some(format!("{}://{}/", parsed.scheme(), host)),
    }
}

/// 检查 URL 是否为 HTTP 或 HTTPS 协议。对齐 Python `_is_http_or_https`。
pub fn is_http_or_https(url: &str) -> bool {
    url.starts_with("http:") || url.starts_with("https:")
}

/// 将 `file:///` 前缀或相对路径转换为绝对路径。对齐 Python `_normalize_path`。
pub fn normalize_path(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("file://") {
        PathBuf::from(stripped)
    } else {
        PathBuf::from(path)
    }
    .canonicalize()
    .map(|c| {
        // Windows canonicalize 会加 `\\?\` 长路径前缀,tokio::fs / std::fs 在某些
        // 路径上读取失败("background task failed"),去掉前缀恢复为普通绝对路径。
        let s = c.to_string_lossy().into_owned();
        PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s))
    })
    .unwrap_or_else(|_| PathBuf::from(path))
}

/// 解析本地文件路径,支持相对路径(相对于给定基础目录)。
///
/// - 绝对路径或 file:/// 路径:直接 normalize
/// - 相对路径:相对于 `base_dir` 解析后 normalize
pub fn resolve_local_path(path: &str, base_dirs: &[PathBuf]) -> PathBuf {
    // file:/// 前缀
    if let Some(stripped) = path.strip_prefix("file://") {
        return PathBuf::from(stripped)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(stripped));
    }
    // 绝对路径
    if Path::new(path).is_absolute() {
        return PathBuf::from(path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(path));
    }
    // 相对路径:尝试每个 base_dir
    for base in base_dirs {
        let resolved = base.join(path);
        if let Ok(canon) = resolved.canonicalize() {
            return canon;
        }
    }
    // 无法解析,返回原始路径
    PathBuf::from(path)
}

/// 检查路径是否在某个基础目录下(安全限制)。
pub fn is_path_under_dirs(path: &Path, dirs: &[PathBuf]) -> bool {
    let path = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    for dir in dirs {
        if let Ok(dir_canon) = dir.canonicalize() {
            if path.starts_with(&dir_canon) {
                return true;
            }
        }
    }
    false
}

/// 获取文档源的显示名称:优先 name,其次域名(HTTP),最后文件名(本地)。
pub fn get_source_name(llms_txt: &str, name: Option<&str>) -> String {
    if let Some(n) = name {
        return n.to_string();
    }
    if is_http_or_https(llms_txt) {
        if let Some(domain) = extract_domain(llms_txt) {
            return domain
                .trim_end_matches('/')
                .split("//")
                .nth(1)
                .unwrap_or(&domain)
                .to_string();
        }
    }
    Path::new(llms_txt)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(llms_txt)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("https://langchain-ai.github.io/langgraph/llms.txt"),
            Some("https://langchain-ai.github.io/".to_string())
        );
        assert_eq!(
            extract_domain("http://example.com:8080/path"),
            Some("http://example.com:8080/".to_string())
        );
        assert_eq!(
            extract_domain("https://example.com/path"),
            Some("https://example.com/".to_string())
        );
    }

    #[test]
    fn test_is_http_or_https() {
        assert!(is_http_or_https("http://example.com"));
        assert!(is_http_or_https("https://example.com"));
        assert!(!is_http_or_https("file:///tmp/test"));
        assert!(!is_http_or_https("/tmp/test"));
        assert!(!is_http_or_https("ftp://example.com"));
    }

    #[test]
    fn test_allowed_domains() {
        let all = AllowedDomains::All;
        assert!(all.is_url_allowed("https://anything.com/"));
        assert!(all.is_all());

        let set = AllowedDomains::from_set(vec![
            "https://langchain-ai.github.io/".to_string(),
            "https://python.langchain.com/".to_string(),
        ]);
        assert!(set.is_url_allowed("https://langchain-ai.github.io/langgraph/llms.txt"));
        assert!(!set.is_url_allowed("https://evil.com/"));
        assert!(!set.is_all());
    }
}
