use crate::net::{is_blocked_url, resolve_url};
use scraper::{ElementRef, Html, Node, Selector};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineSpan {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    Link { text: String, href: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomNode {
    Heading(u8, String),
    Paragraph(Vec<InlineSpan>),
    List(Vec<Vec<InlineSpan>>),
    CodeBlock(String),
    Quote(String),
    HorizontalRule,
}

#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub title: String,
    pub nodes: Vec<DomNode>,
}

/// Parses raw HTML into a clean flat-block sequence of semantic `DomNode` items.
pub fn parse_html(raw_html: &str, base_url_str: &str, reader_mode: bool) -> ParsedDocument {
    let document = Html::parse_document(raw_html);
    let base_url = Url::parse(base_url_str).ok();

    let title = extract_title(&document).unwrap_or_else(|| base_url_str.to_string());
    let mut nodes = Vec::new();

    if reader_mode {
        if let Some(roots) = find_reader_roots(&document) {
            for root in roots {
                walk_block_container(root, base_url.as_ref(), reader_mode, &mut nodes);
            }
        }
    }

    // Fallback to <body> (or document root) if not in reader mode or if <article>/<main> yielded nothing.
    if nodes.is_empty() {
        if let Ok(body_sel) = Selector::parse("body") {
            if let Some(body) = document.select(&body_sel).next() {
                walk_block_container(body, base_url.as_ref(), reader_mode, &mut nodes);
            } else {
                walk_block_container(
                    document.root_element(),
                    base_url.as_ref(),
                    reader_mode,
                    &mut nodes,
                );
            }
        }
    }

    ParsedDocument { title, nodes }
}

fn extract_title(document: &Html) -> Option<String> {
    let sel = Selector::parse("title").ok()?;
    let el = document.select(&sel).next()?;
    let raw: String = el.text().collect();
    let cleaned = collapse_whitespace(&raw);
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn find_reader_roots(document: &Html) -> Option<Vec<ElementRef<'_>>> {
    for selector_str in ["article", "main", "[role=\"main\"]"] {
        if let Ok(sel) = Selector::parse(selector_str) {
            let matches: Vec<ElementRef<'_>> = document.select(&sel).collect();
            if !matches.is_empty() {
                return Some(matches);
            }
        }
    }
    None
}

fn should_strip_element(tag: &str, reader_mode: bool) -> bool {
    match tag {
        "script" | "style" | "svg" | "noscript" | "iframe" | "head" | "meta" | "link"
        | "canvas" | "audio" | "video" | "picture" | "source" | "object" | "embed" => true,
        "nav" | "footer" | "aside" => reader_mode,
        _ => false,
    }
}

fn is_block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "h1" | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "p"
            | "ul"
            | "ol"
            | "li"
            | "pre"
            | "blockquote"
            | "hr"
            | "div"
            | "section"
            | "article"
            | "main"
            | "header"
            | "footer"
            | "nav"
            | "aside"
            | "table"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
            | "dl"
            | "dt"
            | "dd"
            | "figure"
            | "figcaption"
            | "details"
            | "summary"
            | "form"
            | "fieldset"
            | "address"
            | "center"
    )
}

fn has_nested_block_elements(el: ElementRef<'_>, reader_mode: bool) -> bool {
    for child in el.children() {
        if let Node::Element(child_el) = child.value() {
            let tag = child_el.name();
            if should_strip_element(tag, reader_mode) {
                continue;
            }
            if is_block_tag(tag) {
                return true;
            }
            if let Some(child_ref) = ElementRef::wrap(child) {
                if has_nested_block_elements(child_ref, reader_mode) {
                    return true;
                }
            }
        }
    }
    false
}

fn walk_block_container(
    container: ElementRef<'_>,
    base_url: Option<&Url>,
    reader_mode: bool,
    out: &mut Vec<DomNode>,
) {
    let mut inline_buf: Vec<InlineSpan> = Vec::new();

    for child in container.children() {
        match child.value() {
            Node::Text(text) => {
                push_clean_text(&mut inline_buf, &text.text);
            }
            Node::Element(el_data) => {
                let tag = el_data.name();
                if should_strip_element(tag, reader_mode) {
                    continue;
                }

                let Some(child_el) = ElementRef::wrap(child) else {
                    continue;
                };

                if tag == "br" {
                    flush_paragraph(&mut inline_buf, out);
                    continue;
                }

                if is_block_tag(tag) {
                    flush_paragraph(&mut inline_buf, out);
                    process_block_element(child_el, base_url, reader_mode, out);
                } else {
                    collect_inline_spans(child_el, base_url, reader_mode, &mut inline_buf);
                }
            }
            _ => {}
        }
    }

    flush_paragraph(&mut inline_buf, out);
}

fn process_block_element(
    el: ElementRef<'_>,
    base_url: Option<&Url>,
    reader_mode: bool,
    out: &mut Vec<DomNode>,
) {
    let tag = el.value().name();
    if should_strip_element(tag, reader_mode) {
        return;
    }

    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = tag[1..].parse::<u8>().unwrap_or(1).clamp(1, 6);
            let text = extract_visible_text(el, reader_mode);
            if !text.is_empty() {
                out.push(DomNode::Heading(level, text));
            }
        }
        "hr" => {
            if !matches!(out.last(), Some(DomNode::HorizontalRule)) {
                out.push(DomNode::HorizontalRule);
            }
        }
        "pre" => {
            let raw = extract_preformatted_text(el);
            let trimmed = raw.trim_matches('\n').to_string();
            if !trimmed.trim().is_empty() {
                out.push(DomNode::CodeBlock(trimmed));
            }
        }
        "blockquote" => {
            let quote_text = extract_visible_text(el, reader_mode);
            if !quote_text.is_empty() {
                out.push(DomNode::Quote(quote_text));
            }
        }
        "ul" | "ol" => {
            let mut items: Vec<Vec<InlineSpan>> = Vec::new();
            for child in el.children() {
                if let Some(child_el) = ElementRef::wrap(child) {
                    if child_el.value().name() == "li" {
                        let mut spans = Vec::new();
                        collect_inline_spans(child_el, base_url, reader_mode, &mut spans);
                        normalize_spans(&mut spans);
                        if !spans.is_empty() {
                            items.push(spans);
                        }
                    }
                }
            }
            if !items.is_empty() {
                out.push(DomNode::List(items));
            }
        }
        "p" | "dt" | "dd" | "figcaption" | "summary" => {
            let mut spans = Vec::new();
            collect_inline_spans(el, base_url, reader_mode, &mut spans);
            flush_paragraph(&mut spans, out);
        }
        "tr" => {
            // If this table row contains only inline content inside its cells (like Hacker News rows),
            // render the cells together as a single paragraph line.
            if !row_has_block_containers(el, reader_mode) {
                let mut row_spans: Vec<InlineSpan> = Vec::new();
                for cell in el.children().filter_map(ElementRef::wrap) {
                    let cell_tag = cell.value().name();
                    if cell_tag == "td" || cell_tag == "th" {
                        let mut cell_spans = Vec::new();
                        collect_inline_spans(cell, base_url, reader_mode, &mut cell_spans);
                        normalize_spans(&mut cell_spans);
                        if !cell_spans.is_empty() {
                            if !row_spans.is_empty() {
                                row_spans.push(InlineSpan::Text(" ".to_string()));
                            }
                            row_spans.extend(cell_spans);
                        }
                    }
                }
                flush_paragraph(&mut row_spans, out);
            } else {
                walk_block_container(el, base_url, reader_mode, out);
            }
        }
        _ => {
            walk_block_container(el, base_url, reader_mode, out);
        }
    }
}

fn row_has_block_containers(tr_el: ElementRef<'_>, reader_mode: bool) -> bool {
    for cell in tr_el.children().filter_map(ElementRef::wrap) {
        let cell_tag = cell.value().name();
        if cell_tag == "td" || cell_tag == "th" {
            if has_nested_block_elements(cell, reader_mode) {
                return true;
            }
        }
    }
    false
}

fn collect_inline_spans(
    el: ElementRef<'_>,
    base_url: Option<&Url>,
    reader_mode: bool,
    spans: &mut Vec<InlineSpan>,
) {
    let tag = el.value().name();
    if should_strip_element(tag, reader_mode) {
        return;
    }

    match tag {
        "a" => {
            let link_text = extract_visible_text(el, reader_mode);
            if link_text.is_empty() {
                return;
            }
            if let Some(raw_href) = el.value().attr("href") {
                if let Some(resolved) = resolve_url(base_url, raw_href) {
                    let final_href = unwrap_search_redirect(&resolved);
                    if !is_blocked_url(&final_href) {
                        ensure_space_before_inline(spans);
                        spans.push(InlineSpan::Link {
                            text: link_text,
                            href: final_href,
                        });
                        return;
                    }
                }
            }
            push_clean_text(spans, &link_text);
        }
        "b" | "strong" => {
            let text = extract_visible_text(el, reader_mode);
            if !text.is_empty() {
                ensure_space_before_inline(spans);
                spans.push(InlineSpan::Bold(text));
            }
        }
        "i" | "em" => {
            let text = extract_visible_text(el, reader_mode);
            if !text.is_empty() {
                ensure_space_before_inline(spans);
                spans.push(InlineSpan::Italic(text));
            }
        }
        "code" | "kbd" | "samp" => {
            let text = extract_visible_text(el, reader_mode);
            if !text.is_empty() {
                ensure_space_before_inline(spans);
                spans.push(InlineSpan::Code(text));
            }
        }
        "br" => {
            spans.push(InlineSpan::Text("\n".to_string()));
        }
        "input" => {
            let input_type = el.value().attr("type").unwrap_or("text");
            if input_type != "hidden" {
                let label = el
                    .value()
                    .attr("value")
                    .or_else(|| el.value().attr("placeholder"))
                    .unwrap_or(input_type);
                let clean = collapse_whitespace(label);
                if !clean.is_empty() {
                    ensure_space_before_inline(spans);
                    spans.push(InlineSpan::Code(format!("[{clean}]")));
                }
            }
        }
        _ => {
            for child in el.children() {
                match child.value() {
                    Node::Text(t) => {
                        push_clean_text(spans, &t.text);
                    }
                    Node::Element(child_data) => {
                        if should_strip_element(child_data.name(), reader_mode) {
                            continue;
                        }
                        if let Some(child_el) = ElementRef::wrap(child) {
                            collect_inline_spans(child_el, base_url, reader_mode, spans);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Unwraps DuckDuckGo HTML redirect links (`/l/?uddg=...`) directly to the destination URL.
fn unwrap_search_redirect(resolved_url: &str) -> String {
    if let Ok(parsed) = Url::parse(resolved_url) {
        if parsed.path().starts_with("/l/") {
            for (k, v) in parsed.query_pairs() {
                if k == "uddg" && (v.starts_with("http://") || v.starts_with("https://")) {
                    return v.into_owned();
                }
            }
        }
    }
    resolved_url.to_string()
}

fn extract_visible_text(el: ElementRef<'_>, reader_mode: bool) -> String {
    let mut buf = String::new();
    collect_raw_visible_text(el, reader_mode, &mut buf);
    collapse_whitespace(&buf)
}

fn collect_raw_visible_text(el: ElementRef<'_>, reader_mode: bool, buf: &mut String) {
    for child in el.children() {
        match child.value() {
            Node::Text(t) => {
                buf.push_str(&t.text);
                buf.push(' ');
            }
            Node::Element(child_data) => {
                if should_strip_element(child_data.name(), reader_mode) {
                    continue;
                }
                if let Some(child_el) = ElementRef::wrap(child) {
                    collect_raw_visible_text(child_el, reader_mode, buf);
                }
            }
            _ => {}
        }
    }
}

fn extract_preformatted_text(el: ElementRef<'_>) -> String {
    let mut buf = String::new();
    for text in el.text() {
        buf.push_str(text);
    }
    buf
}

fn collapse_whitespace(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for word in input.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

fn push_clean_text(spans: &mut Vec<InlineSpan>, raw: &str) {
    if raw.is_empty() {
        return;
    }
    let has_leading_space = raw.chars().next().is_some_and(char::is_whitespace);
    let has_trailing_space = raw.chars().last().is_some_and(char::is_whitespace);
    let collapsed = collapse_whitespace(raw);

    if collapsed.is_empty() {
        if has_leading_space && !spans.is_empty() && !ends_with_whitespace(spans) {
            if let Some(InlineSpan::Text(last)) = spans.last_mut() {
                last.push(' ');
            } else {
                spans.push(InlineSpan::Text(" ".to_string()));
            }
        }
        return;
    }

    let mut piece = String::new();
    if has_leading_space && !spans.is_empty() && !ends_with_whitespace(spans) {
        piece.push(' ');
    }
    piece.push_str(&collapsed);
    if has_trailing_space {
        piece.push(' ');
    }

    if let Some(InlineSpan::Text(last)) = spans.last_mut() {
        last.push_str(&piece);
    } else {
        spans.push(InlineSpan::Text(piece));
    }
}

fn ensure_space_before_inline(spans: &mut Vec<InlineSpan>) {
    if spans.is_empty() || ends_with_whitespace(spans) {
        return;
    }
    if let Some(InlineSpan::Text(last)) = spans.last() {
        if last.ends_with('(') || last.ends_with('[') || last.ends_with('"') || last.ends_with('\'')
        {
            return;
        }
    }
    spans.push(InlineSpan::Text(" ".to_string()));
}

fn ends_with_whitespace(spans: &[InlineSpan]) -> bool {
    match spans.last() {
        Some(InlineSpan::Text(t)) => t.chars().last().is_some_and(char::is_whitespace),
        _ => false,
    }
}

fn normalize_spans(spans: &mut Vec<InlineSpan>) {
    if let Some(InlineSpan::Text(first)) = spans.first_mut() {
        let trimmed = first.trim_start().to_string();
        *first = trimmed;
    }
    if matches!(spans.first(), Some(InlineSpan::Text(t)) if t.is_empty()) {
        spans.remove(0);
    }

    if let Some(InlineSpan::Text(last)) = spans.last_mut() {
        let trimmed = last.trim_end().to_string();
        *last = trimmed;
    }
    if matches!(spans.last(), Some(InlineSpan::Text(t)) if t.is_empty()) {
        spans.pop();
    }
}

fn flush_paragraph(spans: &mut Vec<InlineSpan>, out: &mut Vec<DomNode>) {
    normalize_spans(spans);
    if !spans.is_empty() {
        out.push(DomNode::Paragraph(std::mem::take(spans)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reader_mode_strips_nav_and_keeps_article() {
        let html = r#"
        <html>
            <body>
                <nav><a href="/home">Nav Link</a></nav>
                <article>
                    <h1>Article Title</h1>
                    <p>Hello <b>world</b> and <a href="/docs">Docs</a>.</p>
                </article>
                <footer>Footer text</footer>
            </body>
        </html>
        "#;

        let reader_doc = parse_html(html, "https://example.com/blog/post", true);
        assert_eq!(
            reader_doc.nodes,
            vec![
                DomNode::Heading(1, "Article Title".to_string()),
                DomNode::Paragraph(vec![
                    InlineSpan::Text("Hello ".to_string()),
                    InlineSpan::Bold("world".to_string()),
                    InlineSpan::Text(" and ".to_string()),
                    InlineSpan::Link {
                        text: "Docs".to_string(),
                        href: "https://example.com/docs".to_string(),
                    },
                    InlineSpan::Text(".".to_string()),
                ]),
            ]
        );

        let full_doc = parse_html(html, "https://example.com/blog/post", false);
        assert_eq!(full_doc.nodes.len(), 4);
    }
}
