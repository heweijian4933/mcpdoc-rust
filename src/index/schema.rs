//! Tantivy schema 定义:BM25 全文索引字段

use tantivy::schema::{Field, NumericOptions, Schema, SchemaBuilder, STORED, TEXT};

/// 索引字段句柄
pub struct IndexSchema {
    pub url: Field,
    pub title: Field,
    pub content: Field,
    pub source_name: Field,
    pub fetched_at: Field,
}

/// 构建 tantivy schema
pub fn build_schema() -> (Schema, IndexSchema) {
    let mut builder = SchemaBuilder::new();

    let url = builder.add_text_field("url", TEXT | STORED);
    let title = builder.add_text_field("title", TEXT | STORED);
    let content = builder.add_text_field("content", TEXT);
    let source_name = builder.add_text_field("source_name", TEXT | STORED);
    let fetched_at = builder.add_i64_field(
        "fetched_at",
        NumericOptions::default().set_stored().set_indexed(),
    );

    let schema = builder.build();
    (
        schema,
        IndexSchema {
            url,
            title,
            content,
            source_name,
            fetched_at,
        },
    )
}
