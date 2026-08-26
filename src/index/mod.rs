//! BM25 全文搜索索引模块:使用 tantivy 实现。
//!
//! 特性:
//! - 懒加载:首次 search_docs 调用时构建
//! - header 加权:title 字段 BM25 分数 ×2
//! - 持久化缓存:索引保存到磁盘,多进程共享只读加载
//! - 内容增强:llms.txt 中链接的描述文本加入 content 字段

mod schema;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::QueryParser;
use tantivy::schema::Value;
use tantivy::{Index, IndexReader, TantivyDocument};
use tracing::{info, warn};

use crate::config::DocSource;
use crate::domain::{is_http_or_https, normalize_path, parse_llms_txt};
use crate::fetch::{fetch_remote, read_local_file};

pub use schema::IndexSchema;

/// 缓存 manifest 文件名
const MANIFEST_FILE: &str = "manifest.json";

/// 搜索结果
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub source_name: String,
    pub score: f32,
}

/// 搜索索引(懒加载,持久化)
pub struct SearchIndex {
    index: Index,
    reader: IndexReader,
    schema: Arc<IndexSchema>,
}

/// 计算文档源指纹(用于缓存键,FNV-1a)
fn compute_fingerprint(doc_sources: &[DocSource]) -> String {
    let mut input = String::new();
    for s in doc_sources {
        input.push_str(&s.llms_txt);
        input.push('|');
        if let Some(n) = &s.name {
            input.push_str(n);
        }
        input.push('\n');
    }
    let mut hash: u64 = 14695981039346656037;
    for byte in input.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("index-{hash:016x}")
}

/// 缓存 manifest:记录每个 llms.txt 的版本信息,用于判断缓存是否过期。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    /// 文档源指纹(与缓存目录名对应)
    fingerprint: String,
    /// 构建时间(Unix 秒)
    built_at: i64,
    /// 每个文档源的版本信息
    sources: Vec<SourceVersion>,
}

/// 单个文档源的版本信息
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceVersion {
    /// llms.txt 的 URL 或路径
    llms_txt: String,
    /// 本地文件的 mtime(Unix 秒),远程文件为 None
    mtime: Option<i64>,
    /// 远程文件的 ETag(若有)
    etag: Option<String>,
    /// 远程文件的 Last-Modified(若有)
    last_modified: Option<String>,
    /// 文档条目数(用于快速校验)
    entry_count: usize,
}

/// 获取本地文件的版本信息(mtime)
fn get_local_version(path: &Path) -> Option<SourceVersion> {
    let metadata = std::fs::metadata(path).ok()?;
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    Some(SourceVersion {
        llms_txt: path.display().to_string(),
        mtime,
        etag: None,
        last_modified: None,
        entry_count: 0,
    })
}

/// 获取远程文件的版本信息(HEAD 请求,取 ETag/Last-Modified)
async fn get_remote_version(
    url: &str,
    http_client: &Client,
    timeout: Duration,
) -> Option<SourceVersion> {
    // 先尝试 HEAD 请求(轻量),失败则用 GET 请求只取 header
    let response = http_client.head(url).timeout(timeout).send().await;

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            // HEAD 可能被某些服务器拒绝,回退到 GET
            warn!(
                "HEAD request failed for {}, falling back to GET: {}",
                url, e
            );
            http_client.get(url).timeout(timeout).send().await.ok()?
        }
    };

    let etag = response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let last_modified = response
        .headers()
        .get("last-modified")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    if etag.is_none() && last_modified.is_none() {
        warn!("no etag or last-modified header for {}", url);
    }

    Some(SourceVersion {
        llms_txt: url.to_string(),
        mtime: None,
        etag,
        last_modified,
        entry_count: 0,
    })
}

/// 收集所有文档源的当前版本信息
async fn collect_versions(
    doc_sources: &[DocSource],
    http_client: &Client,
    timeout: Duration,
) -> Vec<SourceVersion> {
    let mut versions = Vec::new();
    for source in doc_sources {
        let version = if is_http_or_https(&source.llms_txt) {
            get_remote_version(&source.llms_txt, http_client, timeout).await
        } else {
            let path = normalize_path(&source.llms_txt);
            get_local_version(&path)
        };
        if let Some(mut v) = version {
            v.llms_txt = source.llms_txt.clone();
            versions.push(v);
        }
    }
    versions
}

/// 对比当前版本与缓存 manifest 中的版本,判断缓存是否有效。
/// - 本地文件:mtime 一致则有效
/// - 远程文件:ETag 或 Last-Modified 一致则有效
/// - 无法获取版本信息时,保守认为有效(避免不必要的重建)
fn is_cache_valid(current: &[SourceVersion], cached: &[SourceVersion]) -> bool {
    if current.len() != cached.len() {
        return false;
    }
    for (curr, cache) in current.iter().zip(cached.iter()) {
        if curr.llms_txt != cache.llms_txt {
            return false;
        }
        // 本地文件:对比 mtime
        if let (Some(curr_mtime), Some(cached_mtime)) = (curr.mtime, cache.mtime) {
            if curr_mtime != cached_mtime {
                info!(
                    "cache invalid: {} mtime changed ({} -> {})",
                    curr.llms_txt, cached_mtime, curr_mtime
                );
                return false;
            }
        }
        // 远程文件:对比 ETag
        if let (Some(ref curr_etag), Some(ref cached_etag)) = (&curr.etag, &cache.etag) {
            if curr_etag != cached_etag {
                info!(
                    "cache invalid: {} etag changed ({} -> {})",
                    curr.llms_txt, cached_etag, curr_etag
                );
                return false;
            }
        } else if let (Some(ref curr_lm), Some(ref cached_lm)) =
            (&curr.last_modified, &cache.last_modified)
        {
            // 无 ETag 时对比 Last-Modified
            if curr_lm != cached_lm {
                info!(
                    "cache invalid: {} last-modified changed ({} -> {})",
                    curr.llms_txt, cached_lm, curr_lm
                );
                return false;
            }
        }
        // 无法获取版本信息,保守认为有效
    }
    true
}

/// 读取缓存目录中的 manifest
fn load_manifest(cache_path: &Path) -> Option<Manifest> {
    let manifest_path = cache_path.join(MANIFEST_FILE);
    let content = std::fs::read_to_string(&manifest_path).ok()?;
    serde_json::from_str(&content).ok()
}

/// 保存 manifest 到缓存目录
fn save_manifest(cache_path: &Path, manifest: &Manifest) -> Result<(), String> {
    let manifest_path = cache_path.join(MANIFEST_FILE);
    let content = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("failed to serialize manifest: {e}"))?;
    std::fs::write(&manifest_path, content).map_err(|e| format!("failed to write manifest: {e}"))
}

impl SearchIndex {
    /// 构建或加载索引。
    ///
    /// 缓存策略:
    /// 1. 计算文档源指纹(基于配置),确定缓存目录
    /// 2. 收集每个 llms.txt 的当前版本信息(本地 mtime / 远程 ETag)
    /// 3. 对比缓存 manifest,若全部有效则直接加载(秒级)
    /// 4. 否则重建索引并保存新 manifest
    pub async fn build(
        doc_sources: &[DocSource],
        http_client: &Client,
        timeout: Duration,
        cache_dir: Option<&Path>,
    ) -> Result<Self, String> {
        let (schema, index_schema) = schema::build_schema();
        let index_schema = Arc::new(index_schema);
        let fingerprint = compute_fingerprint(doc_sources);

        // 收集当前版本信息(用于缓存校验)
        let current_versions = if cache_dir.is_some() {
            info!("checking llms.txt versions for cache validation");
            collect_versions(doc_sources, http_client, timeout).await
        } else {
            Vec::new()
        };

        // 尝试从缓存加载:检查 manifest 版本一致性
        if let Some(dir) = cache_dir {
            let cache_path = dir.join(&fingerprint);
            if cache_path.exists() {
                // 检查 manifest
                let cache_valid = if let Some(manifest) = load_manifest(&cache_path) {
                    if is_cache_valid(&current_versions, &manifest.sources) {
                        info!("cache is up-to-date (all llms.txt unchanged)");
                        true
                    } else {
                        info!("cache is stale, rebuilding");
                        false
                    }
                } else {
                    info!("no manifest found, rebuilding");
                    false
                };

                if cache_valid {
                    info!("loading index from cache: {}", cache_path.display());
                    match Self::load_from_dir(&cache_path, index_schema.clone()) {
                        Ok(idx) => {
                            info!("index loaded from cache");
                            return Ok(idx);
                        }
                        Err(e) => {
                            warn!("failed to load cache, rebuilding: {e}");
                            let _ = std::fs::remove_dir_all(&cache_path);
                        }
                    }
                } else {
                    // 缓存过期,删除旧缓存
                    let _ = std::fs::remove_dir_all(&cache_path);
                }
            }
        }

        // 构建新索引:直接写入磁盘目录(若提供 cache_dir),否则用内存
        let cache_path = cache_dir.map(|d| d.join(&fingerprint));
        let index = if let Some(ref cache_path) = cache_path {
            std::fs::create_dir_all(cache_path)
                .map_err(|e| format!("failed to create cache dir: {e}"))?;
            info!("building index to: {}", cache_path.display());
            Index::create_in_dir(cache_path, schema.clone())
                .map_err(|e| format!("failed to create index in dir: {e}"))?
        } else {
            Index::create_in_ram(schema.clone())
        };

        let mut writer = index
            .writer(50_000_000)
            .map_err(|e| format!("failed to create index writer: {e}"))?;

        let now = jiff::Timestamp::now().as_second();
        let mut total_entries = 0usize;

        for source in doc_sources {
            let source_name =
                crate::domain::get_source_name(&source.llms_txt, source.name.as_deref());
            info!("indexing source: {}", source_name);

            let content = if is_http_or_https(&source.llms_txt) {
                fetch_remote(&source.llms_txt, http_client, timeout).await
            } else {
                let path = normalize_path(&source.llms_txt);
                read_local_file(&path).await
            };

            let content = match content {
                Ok(c) => c,
                Err(e) => {
                    warn!("failed to fetch llms.txt for '{}': {}", source_name, e);
                    continue;
                }
            };

            let entries = parse_llms_txt(&content, &source_name);
            info!("source '{}' has {} entries", source_name, entries.len());
            total_entries += entries.len();

            for entry in entries {
                let mut doc = TantivyDocument::new();
                doc.add_text(index_schema.url, &entry.url);
                doc.add_text(index_schema.title, &entry.title);
                // content = title + description,提升 BM25 搜索质量
                let content_text = if entry.description.is_empty() {
                    entry.title.clone()
                } else {
                    format!("{} {}", entry.title, entry.description)
                };
                doc.add_text(index_schema.content, &content_text);
                doc.add_text(index_schema.source_name, &source_name);
                doc.add_i64(index_schema.fetched_at, now);

                if let Err(e) = writer.add_document(doc) {
                    warn!("failed to add document '{}': {}", entry.title, e);
                }
            }
        }

        writer
            .commit()
            .map_err(|e| format!("failed to commit index: {e}"))?;

        let reader = index
            .reader_builder()
            .reload_policy(tantivy::ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| format!("failed to create reader: {e}"))?;

        info!(
            "index built successfully with {} sources, {} entries",
            doc_sources.len(),
            total_entries
        );

        // 保存 manifest(记录版本信息,供下次启动校验)
        if let Some(ref cache_path) = cache_path {
            let manifest = Manifest {
                fingerprint: fingerprint.clone(),
                built_at: now,
                sources: current_versions,
            };
            if let Err(e) = save_manifest(cache_path, &manifest) {
                warn!("failed to save manifest: {e}");
            } else {
                info!(
                    "manifest saved to: {}",
                    cache_path.join(MANIFEST_FILE).display()
                );
            }
        }

        Ok(Self {
            index,
            reader,
            schema: index_schema,
        })
    }

    /// 从磁盘目录加载只读索引(多进程共享)
    fn load_from_dir(cache_path: &Path, index_schema: Arc<IndexSchema>) -> Result<Self, String> {
        let mmap_dir = MmapDirectory::open(cache_path)
            .map_err(|e| format!("failed to open cache directory: {e}"))?;
        let index = Index::open(mmap_dir).map_err(|e| format!("failed to open index: {e}"))?;

        let reader = index
            .reader_builder()
            .reload_policy(tantivy::ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| format!("failed to create reader: {e}"))?;

        Ok(Self {
            index,
            reader,
            schema: index_schema,
        })
    }

    /// BM25 搜索,title 字段加权 2.0
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        let searcher = self.reader.searcher();

        let mut parser =
            QueryParser::for_index(&self.index, vec![self.schema.title, self.schema.content]);
        parser.set_field_boost(self.schema.title, 2.0);

        let (query, errors) = parser.parse_query_lenient(query);
        if !errors.is_empty() {
            warn!("query parse errors: {:?}", errors);
        }

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit))
            .map_err(|e| format!("search failed: {e}"))?;

        let mut results = Vec::new();
        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher
                .doc(doc_address)
                .map_err(|e| format!("failed to retrieve doc: {e}"))?;

            let url = doc
                .get_first(self.schema.url)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = doc
                .get_first(self.schema.title)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let source_name = doc
                .get_first(self.schema.source_name)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // snippet 从 content 字段取前 200 字符
            let snippet = doc
                .get_first(self.schema.content)
                .and_then(|v| v.as_str())
                .map(|s| s.chars().take(200).collect())
                .unwrap_or_else(|| title.clone());

            results.push(SearchResult {
                url,
                title,
                snippet,
                source_name,
                score,
            });
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_fingerprint_stable() {
        let sources = vec![DocSource {
            name: Some("test".to_string()),
            llms_txt: "https://example.com/llms.txt".to_string(),
            description: None,
        }];
        let fp1 = compute_fingerprint(&sources);
        let fp2 = compute_fingerprint(&sources);
        assert_eq!(fp1, fp2);
        assert!(fp1.starts_with("index-"));
    }

    #[test]
    fn test_compute_fingerprint_differs() {
        let s1 = vec![DocSource {
            name: None,
            llms_txt: "https://a.com/llms.txt".to_string(),
            description: None,
        }];
        let s2 = vec![DocSource {
            name: None,
            llms_txt: "https://b.com/llms.txt".to_string(),
            description: None,
        }];
        assert_ne!(compute_fingerprint(&s1), compute_fingerprint(&s2));
    }
}
