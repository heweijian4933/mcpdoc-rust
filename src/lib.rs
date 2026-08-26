//! mcpdoc-rust:基于 MCP 协议的 llms.txt 文档服务器(Rust 版)
//!
//! 提供 MCP 工具(list_doc_sources / fetch_docs / search_docs / list_pages)供 MCP 主机应用
//! (Cursor、Claude Code/Desktop 等)检索文档。

pub mod cli;
pub mod config;
pub mod domain;
pub mod error;
pub mod fetch;
pub mod html_to_md;
pub mod index;
pub mod server;
