//! DocSource 类型定义与配置文件加载,对齐 Python mcpdoc/cli.py 与 mcpdoc/main.py。

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

/// 文档源定义,对齐 Python `DocSource` TypedDict。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocSource {
    /// 文档源名称(可选)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// llms.txt 的 URL 或本地文件路径
    pub llms_txt: String,
    /// 描述(可选)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// 从 YAML 配置文件加载文档源列表。
pub fn load_yaml_config(path: &Path) -> Result<Vec<DocSource>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read yaml config: {}", path.display()))?;
    let sources: Vec<DocSource> = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse yaml config: {}", path.display()))?;
    Ok(sources)
}

/// 从 JSON 配置文件加载文档源列表。
pub fn load_json_config(path: &Path) -> Result<Vec<DocSource>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read json config: {}", path.display()))?;
    let sources: Vec<DocSource> = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse json config: {}", path.display()))?;
    Ok(sources)
}

/// 从 URL/路径列表解析文档源,对齐 Python `create_doc_sources_from_urls`。
///
/// 支持格式:
/// - `name:url_or_path`(name 不以 http 开头且非 Windows 盘符)
/// - `url_or_path`(纯 URL 或路径,含 Windows 盘符如 `E:\code\...` 或 `E:/code/...`)
///
/// Windows 盘符路径(如 `E:\...`、`E:/...`)不会被误判为 name 前缀。
pub fn parse_urls(urls: &[String]) -> Vec<DocSource> {
    let mut sources = Vec::new();
    for entry in urls {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // 判断是否为 "name:path" 格式:
        // 1. 找到第一个 ':'
        // 2. 前缀不以 "http" 开头
        // 3. 前缀不是单字母 Windows 盘符(如 "E"、"C")
        // 4. 前缀后不是 '/' 或 '\'(排除 "E:/" 和 "E:\" 这种盘符路径)
        if let Some(colon_pos) = entry.find(':') {
            let prefix = &entry[..colon_pos];
            let after_colon = &entry[colon_pos + 1..];
            let is_windows_drive = prefix.len() == 1
                && prefix
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_alphabetic())
                    .unwrap_or(false)
                && (after_colon.starts_with('/') || after_colon.starts_with('\\'));
            if !prefix.starts_with("http") && !prefix.is_empty() && !is_windows_drive {
                let name = prefix.to_string();
                let llms_txt = after_colon.to_string();
                sources.push(DocSource {
                    name: Some(name),
                    llms_txt,
                    description: None,
                });
                continue;
            }
        }
        sources.push(DocSource {
            name: None,
            llms_txt: entry.to_string(),
            description: None,
        });
    }
    sources
}

/// 合并多个来源的文档源配置。
pub fn merge_doc_sources(
    yaml: Option<&Path>,
    json: Option<&Path>,
    urls: &[String],
) -> Result<Vec<DocSource>> {
    let mut all: Vec<DocSource> = Vec::new();
    if let Some(p) = yaml {
        all.extend(load_yaml_config(p)?);
    }
    if let Some(p) = json {
        all.extend(load_json_config(p)?);
    }
    all.extend(parse_urls(urls));
    if all.is_empty() {
        return Err(anyhow!(
            "at least one source option (--yaml, --json, or --urls) is required"
        ));
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_urls_name_prefix() {
        let urls = vec!["LangGraph:https://example.com/llms.txt".to_string()];
        let sources = parse_urls(&urls);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name.as_deref(), Some("LangGraph"));
        assert_eq!(sources[0].llms_txt, "https://example.com/llms.txt");
    }

    #[test]
    fn test_parse_urls_plain_url() {
        let urls = vec!["https://example.com/llms.txt".to_string()];
        let sources = parse_urls(&urls);
        assert_eq!(sources.len(), 1);
        assert!(sources[0].name.is_none());
        assert_eq!(sources[0].llms_txt, "https://example.com/llms.txt");
    }

    #[test]
    fn test_parse_urls_windows_drive_forward_slash() {
        // E:/code/... 不应被误判为 name="E"
        let urls = vec!["E:/code/docs/llms.txt".to_string()];
        let sources = parse_urls(&urls);
        assert_eq!(sources.len(), 1);
        assert!(sources[0].name.is_none(), "should not parse drive as name");
        assert_eq!(sources[0].llms_txt, "E:/code/docs/llms.txt");
    }

    #[test]
    fn test_parse_urls_windows_drive_backslash() {
        // E:\code\... 不应被误判为 name="E"
        let urls = vec!["E:\\code\\docs\\llms.txt".to_string()];
        let sources = parse_urls(&urls);
        assert_eq!(sources.len(), 1);
        assert!(sources[0].name.is_none(), "should not parse drive as name");
        assert_eq!(sources[0].llms_txt, "E:\\code\\docs\\llms.txt");
    }

    #[test]
    fn test_parse_urls_name_with_windows_path() {
        // name:E:/path 应正确解析 name 和路径
        let urls = vec!["MyDocs:E:/code/docs/llms.txt".to_string()];
        let sources = parse_urls(&urls);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name.as_deref(), Some("MyDocs"));
        assert_eq!(sources[0].llms_txt, "E:/code/docs/llms.txt");
    }

    #[test]
    fn test_parse_urls_name_with_windows_backslash_path() {
        // name:E:\path 应正确解析 name 和路径
        let urls = vec!["MyDocs:E:\\code\\docs\\llms.txt".to_string()];
        let sources = parse_urls(&urls);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name.as_deref(), Some("MyDocs"));
        assert_eq!(sources[0].llms_txt, "E:\\code\\docs\\llms.txt");
    }

    #[test]
    fn test_parse_urls_empty() {
        let urls = vec!["".to_string(), "  ".to_string()];
        let sources = parse_urls(&urls);
        assert!(sources.is_empty());
    }
}
