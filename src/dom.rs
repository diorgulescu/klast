use crate::adblock_filter::AdblockFilter;
use crate::net::resolve_url;
use scraper::{ElementRef, Html, Node, Selector};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineSpan {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    Link { text: String, href: String },
    ImageBadge { alt: String, url: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormInput {
    pub name: String,
    pub initial_value: String,
    pub placeholder: String,
    pub is_hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormNode {
    pub action: String,
    pub method: String,
    pub inputs: Vec<FormInput>,
    pub submit_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomNode {
    Heading(u8, String),
    Paragraph(Vec<InlineSpan>),
    List(Vec<Vec<InlineSpan>>),
    CodeBlock(String),
    Quote(String),
    Form(FormNode),
    Image { url: String, alt: String },
    HorizontalRule,
}

#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub title: String,
    pub nodes: Vec<DomNode>,
    pub blocked_count: usize,
}

struct ParseContext<'a> {
    base_url: Option<Url>,
    source_url: &'a str,
    reader_mode: bool,
    images_enabled: bool,
    adblock: &'a AdblockFilter,
    blocked_count: usize,
}

/// Parses raw HTML into a clean flat-block sequence of semantic `DomNode` items,
/// applying adblock URL & cosmetic rules, form extraction, and lazy image handling.
pub fn parse_html(
    raw_html: &str,
    base_url_str: &str,
    reader_mode: bool,
    images_enabled: bool,
    adblock: &AdblockFilter,
) -> ParsedDocument {
    let document = Html::parse_document(raw_html);
    let base_url = Url::parse(base_url_str).ok();

    let title = extract_title(&document).unwrap_or_else(|| base_url_str.to_string());
    let mut nodes = Vec::new();
    let mut ctx = ParseContext {
        base_url,
        source_url: base_url_str,
        reader_mode,
        images_enabled,
        adblock,
        blocked_count: 0,
    };

    if reader_mode {
        if let Some(roots) = find_reader_roots(&document) {
            for root in roots {
                walk_block_container(root, &mut ctx, &mut nodes);
            }
        }
    }

    // Fallback to <body> (or document root) if not in reader mode or if <article>/<main> yielded nothing.
    if nodes.is_empty() {
        if let Ok(body_sel) = Selector::parse("body") {
            if let Some(body) = document.select(&body_sel).next() {
                walk_block_container(body, &mut ctx, &mut nodes);
            } else {
                walk_block_container(document.root_element(), &mut ctx, &mut nodes);
            }
        }
    }

    ParsedDocument {
        title,
        nodes,
        blocked_count: ctx.blocked_count,
    }
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

fn should_strip_tag(tag: &str, reader_mode: bool) -> bool {
    match tag {
        "script" | "style" | "svg" | "noscript" | "iframe" | "head" | "meta" | "link"
        | "canvas" | "audio" | "video" | "picture" | "source" | "object" | "embed" => true,
        "nav" | "footer" | "aside" => reader_mode,
        _ => false,
    }
}

fn check_and_skip_element(el: ElementRef<'_>, ctx: &mut ParseContext<'_>) -> bool {
    let tag = el.value().name();
    if should_strip_tag(tag, ctx.reader_mode) {
        return true;
    }

    let id = el.value().attr("id");
    let class = el.value().attr("class");
    if ctx.adblock.should_block_element(tag, id, class) {
        ctx.blocked_count += 1;
        return true;
    }

    false
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
            if should_strip_tag(tag, reader_mode) {
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
    ctx: &mut ParseContext<'_>,
    out: &mut Vec<DomNode>,
) {
    let mut inline_buf: Vec<InlineSpan> = Vec::new();

    for child in container.children() {
        match child.value() {
            Node::Text(text) => {
                push_clean_text(&mut inline_buf, &text.text);
            }
            Node::Element(el_data) => {
                let Some(child_el) = ElementRef::wrap(child) else {
                    continue;
                };
                if check_and_skip_element(child_el, ctx) {
                    continue;
                }

                let tag = el_data.name();
                if tag == "br" {
                    flush_paragraph(&mut inline_buf, out);
                    continue;
                }

                if tag == "img" && ctx.images_enabled {
                    flush_paragraph(&mut inline_buf, out);
                    if let Some(img_node) = parse_image_node(child_el, ctx) {
                        out.push(img_node);
                    }
                    continue;
                }

                if is_block_tag(tag) {
                    flush_paragraph(&mut inline_buf, out);
                    process_block_element(child_el, ctx, out);
                } else {
                    collect_inline_spans(child_el, ctx, &mut inline_buf, out);
                }
            }
            _ => {}
        }
    }

    flush_paragraph(&mut inline_buf, out);
}

fn process_block_element(
    el: ElementRef<'_>,
    ctx: &mut ParseContext<'_>,
    out: &mut Vec<DomNode>,
) {
    if check_and_skip_element(el, ctx) {
        return;
    }

    let tag = el.value().name();
    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = tag[1..].parse::<u8>().unwrap_or(1).clamp(1, 6);
            let text = extract_visible_text(el, ctx.reader_mode);
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
            let quote_text = extract_visible_text(el, ctx.reader_mode);
            if !quote_text.is_empty() {
                out.push(DomNode::Quote(quote_text));
            }
        }
        "form" => {
            if let Some(form_node) = extract_form_node(el, ctx) {
                out.push(DomNode::Form(form_node));
            }
            // If the form also wraps rich content tables/lists (e.g. legacy forums),
            // walk any non-input content blocks inside it.
            if form_contains_content_blocks(el) {
                walk_block_container(el, ctx, out);
            }
        }
        "ul" | "ol" => {
            let mut items: Vec<Vec<InlineSpan>> = Vec::new();
            for child in el.children() {
                if let Some(child_el) = ElementRef::wrap(child) {
                    if check_and_skip_element(child_el, ctx) {
                        continue;
                    }
                    if child_el.value().name() == "li" {
                        let mut spans = Vec::new();
                        collect_inline_spans(child_el, ctx, &mut spans, out);
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
            collect_inline_spans(el, ctx, &mut spans, out);
            flush_paragraph(&mut spans, out);
        }
        "tr" => {
            if !row_has_block_containers(el, ctx.reader_mode) {
                let mut row_spans: Vec<InlineSpan> = Vec::new();
                for cell in el.children().filter_map(ElementRef::wrap) {
                    if check_and_skip_element(cell, ctx) {
                        continue;
                    }
                    let cell_tag = cell.value().name();
                    if cell_tag == "td" || cell_tag == "th" {
                        let mut cell_spans = Vec::new();
                        collect_inline_spans(cell, ctx, &mut cell_spans, out);
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
                walk_block_container(el, ctx, out);
            }
        }
        _ => {
            walk_block_container(el, ctx, out);
        }
    }
}

fn extract_form_node(form_el: ElementRef<'_>, ctx: &ParseContext<'_>) -> Option<FormNode> {
    let raw_action = form_el.value().attr("action").unwrap_or("");
    let action = if raw_action.trim().is_empty() {
        ctx.source_url.to_string()
    } else {
        resolve_url(ctx.base_url.as_ref(), raw_action)
            .unwrap_or_else(|| ctx.source_url.to_string())
    };

    let method = form_el
        .value()
        .attr("method")
        .unwrap_or("GET")
        .trim()
        .to_ascii_uppercase();

    let mut inputs: Vec<FormInput> = Vec::new();
    let mut submit_label: Option<String> = None;
    let mut has_visible_input = false;

    if let Ok(sel) = Selector::parse("input, button") {
        for control in form_el.select(&sel) {
            let tag = control.value().name();
            if tag == "input" {
                let input_type = control
                    .value()
                    .attr("type")
                    .unwrap_or("text")
                    .trim()
                    .to_ascii_lowercase();

                match input_type.as_str() {
                    "text" | "search" | "url" | "email" | "tel" | "number" => {
                        let name = control
                            .value()
                            .attr("name")
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .unwrap_or("q")
                            .to_string();
                        let initial_value =
                            control.value().attr("value").unwrap_or("").to_string();
                        let placeholder = control
                            .value()
                            .attr("placeholder")
                            .or_else(|| control.value().attr("title"))
                            .or_else(|| control.value().attr("aria-label"))
                            .unwrap_or(&name)
                            .to_string();

                        inputs.push(FormInput {
                            name,
                            initial_value,
                            placeholder: collapse_whitespace(&placeholder),
                            is_hidden: false,
                        });
                        has_visible_input = true;
                    }
                    "hidden" => {
                        if let Some(name) = control
                            .value()
                            .attr("name")
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            let initial_value =
                                control.value().attr("value").unwrap_or("").to_string();
                            inputs.push(FormInput {
                                name: name.to_string(),
                                initial_value,
                                placeholder: String::new(),
                                is_hidden: true,
                            });
                        }
                    }
                    "submit" => {
                        if submit_label.is_none() {
                            let val = control
                                .value()
                                .attr("value")
                                .map(collapse_whitespace)
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| "Submit".to_string());
                            submit_label = Some(val);
                        }
                    }
                    _ => {}
                }
            } else if tag == "button" {
                let btn_type = control
                    .value()
                    .attr("type")
                    .unwrap_or("submit")
                    .trim()
                    .to_ascii_lowercase();
                if btn_type == "submit" && submit_label.is_none() {
                    let text = extract_visible_text(control, ctx.reader_mode);
                    let fallback = control
                        .value()
                        .attr("value")
                        .or_else(|| control.value().attr("aria-label"))
                        .map(collapse_whitespace)
                        .unwrap_or_default();
                    let chosen = if !text.is_empty() {
                        text
                    } else if !fallback.is_empty() {
                        fallback
                    } else {
                        "Submit".to_string()
                    };
                    submit_label = Some(chosen);
                }
            }
        }
    }

    if !has_visible_input {
        return None;
    }

    Some(FormNode {
        action,
        method,
        inputs,
        submit_label: submit_label.unwrap_or_else(|| "Search / Submit".to_string()),
    })
}

fn form_contains_content_blocks(form_el: ElementRef<'_>) -> bool {
    if let Ok(sel) = Selector::parse("table, article, ul, ol, blockquote, pre") {
        return form_el.select(&sel).next().is_some();
    }
    false
}

fn parse_image_node(img_el: ElementRef<'_>, ctx: &mut ParseContext<'_>) -> Option<DomNode> {
    let raw_src = img_el
        .value()
        .attr("src")
        .or_else(|| img_el.value().attr("data-src"))?;
    let resolved = resolve_url(ctx.base_url.as_ref(), raw_src)?;

    if ctx.adblock.should_block_url(&resolved, ctx.source_url, "image") {
        ctx.blocked_count += 1;
        return None;
    }

    let raw_alt = img_el
        .value()
        .attr("alt")
        .or_else(|| img_el.value().attr("title"))
        .unwrap_or("image");
    let alt = {
        let cleaned = collapse_whitespace(raw_alt);
        if cleaned.is_empty() {
            "image".to_string()
        } else {
            cleaned
        }
    };

    if ctx.images_enabled {
        Some(DomNode::Image { url: resolved, alt })
    } else {
        Some(DomNode::Paragraph(vec![InlineSpan::ImageBadge {
            alt,
            url: resolved,
        }]))
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
    ctx: &mut ParseContext<'_>,
    spans: &mut Vec<InlineSpan>,
    out: &mut Vec<DomNode>,
) {
    if check_and_skip_element(el, ctx) {
        return;
    }

    let tag = el.value().name();
    match tag {
        "a" => {
            let link_text = extract_visible_text(el, ctx.reader_mode);
            if link_text.is_empty() {
                // Check if the anchor wraps an image
                if let Ok(img_sel) = Selector::parse("img") {
                    if let Some(img_el) = el.select(&img_sel).next() {
                        if let Some(node) = parse_image_node(img_el, ctx) {
                            match node {
                                DomNode::Image { url, alt } => {
                                    out.push(DomNode::Image { url, alt });
                                }
                                DomNode::Paragraph(mut badge_spans) => {
                                    ensure_space_before_inline(spans);
                                    spans.append(&mut badge_spans);
                                }
                                _ => {}
                            }
                        }
                    }
                }
                return;
            }
            if let Some(raw_href) = el.value().attr("href") {
                if let Some(resolved) = resolve_url(ctx.base_url.as_ref(), raw_href) {
                    let final_href = unwrap_search_redirect(&resolved);
                    if ctx
                        .adblock
                        .should_block_url(&final_href, ctx.source_url, "sub_frame")
                    {
                        ctx.blocked_count += 1;
                        return;
                    }
                    ensure_space_before_inline(spans);
                    spans.push(InlineSpan::Link {
                        text: link_text,
                        href: final_href,
                    });
                    return;
                }
            }
            push_clean_text(spans, &link_text);
        }
        "img" => {
            if let Some(node) = parse_image_node(el, ctx) {
                match node {
                    DomNode::Image { url, alt } => {
                        out.push(DomNode::Image { url, alt });
                    }
                    DomNode::Paragraph(mut badge_spans) => {
                        ensure_space_before_inline(spans);
                        spans.append(&mut badge_spans);
                    }
                    _ => {}
                }
            }
        }
        "b" | "strong" => {
            let text = extract_visible_text(el, ctx.reader_mode);
            if !text.is_empty() {
                ensure_space_before_inline(spans);
                spans.push(InlineSpan::Bold(text));
            }
        }
        "i" | "em" => {
            let text = extract_visible_text(el, ctx.reader_mode);
            if !text.is_empty() {
                ensure_space_before_inline(spans);
                spans.push(InlineSpan::Italic(text));
            }
        }
        "code" | "kbd" | "samp" => {
            let text = extract_visible_text(el, ctx.reader_mode);
            if !text.is_empty() {
                ensure_space_before_inline(spans);
                spans.push(InlineSpan::Code(text));
            }
        }
        "br" => {
            spans.push(InlineSpan::Text("\n".to_string()));
        }
        "input" | "button" => {
            // Inputs and buttons inside <form> are already rendered as native widgets via DomNode::Form.
        }
        _ => {
            for child in el.children() {
                match child.value() {
                    Node::Text(t) => {
                        push_clean_text(spans, &t.text);
                    }
                    Node::Element(_) => {
                        if let Some(child_el) = ElementRef::wrap(child) {
                            collect_inline_spans(child_el, ctx, spans, out);
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
                if should_strip_tag(child_data.name(), reader_mode) {
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

/// Parses a plain-text Markdown document into a native `Vec<DomNode>` AST.
/// Supports `#` headings, `- ` / `* ` lists, `> ` quotes, fenced code blocks,
/// horizontal rules (`---`), and inline `[Title](url)`, `` `code` ``, `**bold**`, and `*italic*`.
pub fn parse_markdown_to_dom(md: &str) -> Vec<DomNode> {
    let mut nodes: Vec<DomNode> = Vec::new();
    let mut in_code_block = false;
    let mut code_buf = String::new();
    let mut list_buf: Vec<Vec<InlineSpan>> = Vec::new();
    let mut quote_buf: Vec<String> = Vec::new();
    let mut para_lines: Vec<String> = Vec::new();

    let flush_all = |list_buf: &mut Vec<Vec<InlineSpan>>,
                     quote_buf: &mut Vec<String>,
                     para_lines: &mut Vec<String>,
                     nodes: &mut Vec<DomNode>| {
        if !para_lines.is_empty() {
            let joined = para_lines.join(" ");
            para_lines.clear();
            let spans = parse_markdown_inline(&joined);
            if !spans.is_empty() {
                nodes.push(DomNode::Paragraph(spans));
            }
        }
        if !list_buf.is_empty() {
            nodes.push(DomNode::List(std::mem::take(list_buf)));
        }
        if !quote_buf.is_empty() {
            let joined = quote_buf.join(" ");
            quote_buf.clear();
            if !joined.is_empty() {
                nodes.push(DomNode::Quote(joined));
            }
        }
    };

    for raw_line in md.lines() {
        let trimmed_end = raw_line.trim_end();
        let trimmed = trimmed_end.trim_start();

        if trimmed.starts_with("```") {
            flush_all(&mut list_buf, &mut quote_buf, &mut para_lines, &mut nodes);
            if in_code_block {
                let cleaned = code_buf.trim_end_matches('\n').to_string();
                nodes.push(DomNode::CodeBlock(cleaned));
                code_buf.clear();
                in_code_block = false;
            } else {
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            code_buf.push_str(raw_line);
            code_buf.push('\n');
            continue;
        }

        if trimmed.is_empty() {
            flush_all(&mut list_buf, &mut quote_buf, &mut para_lines, &mut nodes);
            continue;
        }

        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            flush_all(&mut list_buf, &mut quote_buf, &mut para_lines, &mut nodes);
            nodes.push(DomNode::HorizontalRule);
            continue;
        }

        if trimmed.starts_with('#') {
            let hash_count = trimmed.chars().take_while(|&c| c == '#').count();
            if (1..=6).contains(&hash_count) {
                let rest = trimmed[hash_count..].trim();
                if !rest.is_empty() {
                    flush_all(&mut list_buf, &mut quote_buf, &mut para_lines, &mut nodes);
                    nodes.push(DomNode::Heading(hash_count as u8, rest.to_string()));
                    continue;
                }
            }
        }

        if let Some(item_str) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            if !para_lines.is_empty() || !quote_buf.is_empty() {
                flush_all(&mut list_buf, &mut quote_buf, &mut para_lines, &mut nodes);
            }
            let item_spans = parse_markdown_inline(item_str.trim());
            if !item_spans.is_empty() {
                list_buf.push(item_spans);
            }
            continue;
        }

        if let Some(quote_line) = trimmed
            .strip_prefix("> ")
            .or_else(|| trimmed.strip_prefix('>'))
        {
            if !para_lines.is_empty() || !list_buf.is_empty() {
                flush_all(&mut list_buf, &mut quote_buf, &mut para_lines, &mut nodes);
            }
            let clean_quote = quote_line.trim();
            if !clean_quote.is_empty() {
                quote_buf.push(clean_quote.to_string());
            }
            continue;
        }

        if !list_buf.is_empty() || !quote_buf.is_empty() {
            flush_all(&mut list_buf, &mut quote_buf, &mut para_lines, &mut nodes);
        }
        para_lines.push(trimmed.to_string());
    }

    if in_code_block && !code_buf.is_empty() {
        nodes.push(DomNode::CodeBlock(code_buf.trim_end_matches('\n').to_string()));
    }
    flush_all(&mut list_buf, &mut quote_buf, &mut para_lines, &mut nodes);

    nodes
}

fn parse_markdown_inline(input: &str) -> Vec<InlineSpan> {
    let mut spans: Vec<InlineSpan> = Vec::new();
    let mut text_buf = String::new();
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    let flush_text = |text_buf: &mut String, spans: &mut Vec<InlineSpan>| {
        if !text_buf.is_empty() {
            spans.push(InlineSpan::Text(std::mem::take(text_buf)));
        }
    };

    while i < len {
        // Check for ![alt](url) image syntax
        if chars[i] == '!' && i + 1 < len && chars[i + 1] == '[' {
            if let Some((alt, url, next_idx)) = try_parse_link(&chars, i + 1) {
                flush_text(&mut text_buf, &mut spans);
                spans.push(InlineSpan::ImageBadge { alt, url });
                i = next_idx;
                continue;
            }
        }

        // Check for [text](url) link syntax
        if chars[i] == '[' {
            if let Some((label, href, next_idx)) = try_parse_link(&chars, i) {
                flush_text(&mut text_buf, &mut spans);
                if let Some(alt) = label.strip_prefix("IMG: ") {
                    spans.push(InlineSpan::ImageBadge {
                        alt: alt.to_string(),
                        url: href,
                    });
                } else {
                    spans.push(InlineSpan::Link { text: label, href });
                }
                i = next_idx;
                continue;
            }
        }

        // Check for `code`
        if chars[i] == '`' {
            if let Some((code_str, next_idx)) = try_parse_delimited(&chars, i + 1, '`') {
                flush_text(&mut text_buf, &mut spans);
                spans.push(InlineSpan::Code(code_str));
                i = next_idx;
                continue;
            }
        }

        // Check for **bold**
        if chars[i] == '*' && i + 1 < len && chars[i + 1] == '*' {
            if let Some((bold_str, next_idx)) = try_parse_double_asterisk(&chars, i + 2) {
                flush_text(&mut text_buf, &mut spans);
                spans.push(InlineSpan::Bold(bold_str));
                i = next_idx;
                continue;
            }
        }

        // Check for *italic*
        if chars[i] == '*' {
            if let Some((italic_str, next_idx)) = try_parse_delimited(&chars, i + 1, '*') {
                flush_text(&mut text_buf, &mut spans);
                spans.push(InlineSpan::Italic(italic_str));
                i = next_idx;
                continue;
            }
        }

        text_buf.push(chars[i]);
        i += 1;
    }

    flush_text(&mut text_buf, &mut spans);
    spans
}

fn try_parse_link(chars: &[char], start_bracket: usize) -> Option<(String, String, usize)> {
    let len = chars.len();
    let mut close_bracket = None;
    for (idx, &ch) in chars.iter().enumerate().skip(start_bracket + 1) {
        if ch == ']' {
            close_bracket = Some(idx);
            break;
        }
    }
    let close_b = close_bracket?;
    if close_b + 1 >= len || chars[close_b + 1] != '(' {
        return None;
    }

    let mut close_paren = None;
    let mut paren_depth = 0usize;
    for (idx, &ch) in chars.iter().enumerate().skip(close_b + 2) {
        if ch == '(' {
            paren_depth += 1;
        } else if ch == ')' {
            if paren_depth == 0 {
                close_paren = Some(idx);
                break;
            }
            paren_depth -= 1;
        }
    }
    let close_p = close_paren?;

    let label: String = chars[start_bracket + 1..close_b].iter().collect();
    let url: String = chars[close_b + 2..close_p].iter().collect();
    if label.is_empty() || url.trim().is_empty() {
        return None;
    }
    Some((label, url.trim().to_string(), close_p + 1))
}

fn try_parse_delimited(chars: &[char], start: usize, delimiter: char) -> Option<(String, usize)> {
    for (idx, &ch) in chars.iter().enumerate().skip(start) {
        if ch == delimiter {
            let content: String = chars[start..idx].iter().collect();
            if !content.is_empty() {
                return Some((content, idx + 1));
            }
            return None;
        }
    }
    None
}

fn try_parse_double_asterisk(chars: &[char], start: usize) -> Option<(String, usize)> {
    let len = chars.len();
    let mut idx = start;
    while idx + 1 < len {
        if chars[idx] == '*' && chars[idx + 1] == '*' {
            let content: String = chars[start..idx].iter().collect();
            if !content.is_empty() {
                return Some((content, idx + 2));
            }
            return None;
        }
        idx += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reader_mode_and_cosmetic_blocking() {
        let html = r#"
        <html>
            <body>
                <div id="cookie-consent-popup">Accept all cookies</div>
                <nav><a href="/home">Nav Link</a></nav>
                <article>
                    <h1>Article Title</h1>
                    <div class="newsletter-promo">Subscribe to our newsletter!</div>
                    <p>Hello <b>world</b> and <a href="/docs">Docs</a>.</p>
                </article>
                <footer>Footer text</footer>
            </body>
        </html>
        "#;

        let adblock = AdblockFilter::new();
        let reader_doc = parse_html(html, "https://example.com/blog/post", true, false, &adblock);
        assert_eq!(reader_doc.blocked_count, 1); // newsletter-promo inside article blocked
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

        let full_doc = parse_html(html, "https://example.com/blog/post", false, false, &adblock);
        assert_eq!(full_doc.blocked_count, 2); // cookie-consent-popup + newsletter-promo blocked
    }

    #[test]
    fn test_form_and_lazy_image_parsing() {
        let html = r#"
        <html>
            <body>
                <form action="/search" method="get">
                    <input type="search" name="q" value="rust" placeholder="Search..." />
                    <button type="submit">Go</button>
                </form>
                <p>Diagram: <img src="/img/arch.png" alt="Architecture Diagram" /></p>
            </body>
        </html>
        "#;

        let adblock = AdblockFilter::new();
        let doc_images_off = parse_html(html, "https://example.com", false, false, &adblock);
        assert!(doc_images_off.nodes.iter().any(|n| matches!(
            n,
            DomNode::Form(f) if f.action == "https://example.com/search" && f.inputs.len() == 1 && f.inputs[0].name == "q"
        )));
        assert!(doc_images_off.nodes.iter().any(|n| matches!(
            n,
            DomNode::Paragraph(spans) if spans.contains(&InlineSpan::ImageBadge {
                alt: "Architecture Diagram".to_string(),
                url: "https://example.com/img/arch.png".to_string(),
            })
        )));

        let doc_images_on = parse_html(html, "https://example.com", false, true, &adblock);
        assert!(doc_images_on.nodes.iter().any(|n| matches!(
            n,
            DomNode::Image { url, alt } if url == "https://example.com/img/arch.png" && alt == "Architecture Diagram"
        )));
    }
}
