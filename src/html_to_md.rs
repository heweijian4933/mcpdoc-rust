//! HTML 转 Markdown 转换,使用 scraper 解析 DOM。
//! 对齐 Python `markdownify` 的功能。

use scraper::{ElementRef, Html, Node, Selector};

/// 将 HTML 字符串转换为 Markdown。
pub fn html_to_markdown(html: &str) -> String {
    let document = Html::parse_document(html);
    let mut out = String::with_capacity(html.len());
    // 找到 body 元素,如果没有就用 root
    let body_selector = Selector::parse("body").unwrap();
    let root: ElementRef = document
        .select(&body_selector)
        .next()
        .unwrap_or_else(|| ElementRef::wrap(*document.root_element()).unwrap());

    render_element(root, &mut out, &mut RenderState::default());
    // 压缩多余空行
    let mut result = String::with_capacity(out.len());
    let mut blank = false;
    for line in out.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            if !blank {
                result.push('\n');
                blank = true;
            }
        } else {
            result.push_str(trimmed);
            result.push('\n');
            blank = false;
        }
    }
    result.trim().to_string()
}

#[derive(Default)]
struct RenderState {
    list_depth: usize,
    ordered: Vec<bool>,
    item_index: Vec<usize>,
    in_pre: bool,
}

fn render_element(el: ElementRef, out: &mut String, state: &mut RenderState) {
    for child in el.children() {
        match child.value() {
            Node::Text(t) => {
                if state.in_pre {
                    out.push_str(&t.text);
                } else {
                    // 压缩连续空白为单个空格,保留词间空格
                    let text = t.text.chars().fold(String::new(), |mut acc, c| {
                        if c.is_whitespace() {
                            if !acc.ends_with(' ') {
                                acc.push(' ');
                            }
                        } else {
                            acc.push(c);
                        }
                        acc
                    });
                    out.push_str(&text);
                }
            }
            Node::Element(e) => {
                if let Some(child_el) = ElementRef::wrap(child) {
                    render_tag(e.name(), child_el, out, state);
                }
            }
            _ => {}
        }
    }
}

fn render_tag(tag: &str, el: ElementRef, out: &mut String, state: &mut RenderState) {
    match tag {
        "script" | "style" | "head" | "meta" | "link" | "title" => (),
        "h1" => header(out, el, 1, state),
        "h2" => header(out, el, 2, state),
        "h3" => header(out, el, 3, state),
        "h4" => header(out, el, 4, state),
        "h5" => header(out, el, 5, state),
        "h6" => header(out, el, 6, state),
        "p" => {
            ensure_block(out);
            render_element(el, out, state);
            ensure_block(out);
        }
        "br" => out.push('\n'),
        "hr" => {
            ensure_block(out);
            out.push_str("---\n\n");
        }
        "a" => {
            let href = el.value().attr("href").unwrap_or("");
            let mut text = String::new();
            let mut inner_state = RenderState::default();
            render_element(el, &mut text, &mut inner_state);
            let text = text.trim();
            if text.is_empty() {
                out.push_str(href);
            } else {
                out.push('[');
                out.push_str(text);
                out.push_str("](");
                out.push_str(href);
                out.push(')');
            }
        }
        "strong" | "b" => {
            out.push_str("**");
            render_element(el, out, state);
            out.push_str("**");
        }
        "em" | "i" => {
            out.push('*');
            render_element(el, out, state);
            out.push('*');
        }
        "code" => {
            if state.in_pre {
                render_element(el, out, state);
            } else {
                out.push('`');
                render_element(el, out, state);
                out.push('`');
            }
        }
        "pre" => {
            ensure_block(out);
            out.push_str("```\n");
            state.in_pre = true;
            render_element(el, out, state);
            state.in_pre = false;
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("```\n\n");
        }
        "blockquote" => {
            ensure_block(out);
            let mut inner = String::new();
            let mut inner_state = RenderState::default();
            render_element(el, &mut inner, &mut inner_state);
            for line in inner.lines() {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
            out.push('\n');
        }
        "ul" => {
            ensure_block(out);
            state.ordered.push(false);
            state.item_index.push(0);
            render_element(el, out, state);
            state.ordered.pop();
            state.item_index.pop();
            ensure_block(out);
        }
        "ol" => {
            ensure_block(out);
            state.ordered.push(true);
            state.item_index.push(0);
            render_element(el, out, state);
            state.ordered.pop();
            state.item_index.pop();
            ensure_block(out);
        }
        "li" => {
            let indent = "  ".repeat(state.list_depth);
            out.push('\n');
            out.push_str(&indent);
            if state.ordered.last() == Some(&true) {
                let idx = state.item_index.last_mut().unwrap();
                *idx += 1;
                out.push_str(&format!("{}. ", *idx));
            } else {
                out.push_str("- ");
            }
            state.list_depth += 1;
            render_element(el, out, state);
            state.list_depth -= 1;
        }
        "img" => {
            let alt = el.value().attr("alt").unwrap_or("");
            let src = el.value().attr("src").unwrap_or("");
            out.push_str(&format!("![{}]({})", alt, src));
        }
        "table" => render_table(el, out, state),
        "div" | "section" | "article" | "main" | "span" => {
            render_element(el, out, state);
        }
        _ => {
            render_element(el, out, state);
        }
    }
}

fn header(out: &mut String, el: ElementRef, level: usize, _state: &mut RenderState) {
    ensure_block(out);
    out.push_str(&"#".repeat(level));
    out.push(' ');
    let mut text = String::new();
    let mut inner_state = RenderState::default();
    render_element(el, &mut text, &mut inner_state);
    out.push_str(text.trim());
    out.push_str("\n\n");
}

/// 渲染 HTML 表格为 Markdown 表格语法
fn render_table(el: ElementRef, out: &mut String, _state: &mut RenderState) {
    use scraper::Selector;
    let row_selector = match Selector::parse("tr") {
        Ok(s) => s,
        Err(_) => return,
    };
    let cell_selector = match Selector::parse("th, td") {
        Ok(s) => s,
        Err(_) => return,
    };

    let rows: Vec<Vec<String>> = el
        .select(&row_selector)
        .map(|tr| {
            tr.select(&cell_selector)
                .map(|cell| {
                    let mut text = String::new();
                    let mut inner_state = RenderState::default();
                    render_element(cell, &mut text, &mut inner_state);
                    text.trim().replace('|', "\\|").replace('\n', " ")
                })
                .collect()
        })
        .collect();

    if rows.is_empty() {
        return;
    }

    ensure_block(out);

    // 第一行作为表头
    let header_row = &rows[0];
    out.push('|');
    for cell in header_row {
        out.push(' ');
        out.push_str(cell);
        out.push_str(" |");
    }
    out.push('\n');

    // 分隔行
    out.push('|');
    for _ in header_row {
        out.push_str(" --- |");
    }
    out.push('\n');

    // 数据行
    for row in rows.iter().skip(1) {
        out.push('|');
        for cell in row {
            out.push(' ');
            out.push_str(cell);
            out.push_str(" |");
        }
        out.push('\n');
    }
    out.push('\n');
}

fn ensure_block(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.ends_with("\n\n") {
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_html_to_md() {
        let html = "<h1>Title</h1><p>Hello <strong>world</strong></p>";
        let md = html_to_markdown(html);
        assert!(md.contains("# Title"));
        assert!(md.contains("Hello **world**"));
    }

    #[test]
    fn test_links_and_lists() {
        let html = r#"<ul><li><a href="https://example.com">Link</a></li><li>Item 2</li></ul>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("[Link](https://example.com)"));
        assert!(md.contains("- "));
        assert!(md.contains("Item 2"));
    }

    #[test]
    fn test_code_block() {
        let html = "<pre><code>fn main() {}</code></pre>";
        let md = html_to_markdown(html);
        assert!(md.contains("```"));
        assert!(md.contains("fn main() {}"));
    }

    #[test]
    fn test_table() {
        let html = r#"<table><thead><tr><th>Name</th><th>Value</th></tr></thead><tbody><tr><td>foo</td><td>bar</td></tr><tr><td>a|b</td><td>c</td></tr></tbody></table>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("| Name | Value |"), "header row: {md}");
        assert!(md.contains("| --- | --- |"), "separator: {md}");
        assert!(md.contains("| foo | bar |"), "data row: {md}");
        assert!(md.contains(r"\|"), "pipe escaped: {md}");
    }
}
