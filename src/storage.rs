// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Klast Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

use crate::dom::{DomNode, InlineSpan};
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use url::Url;

const STARTER_BOOKMARKS_MD: &str = r#"# ★ Bookmarks

- [Hacker News](https://news.ycombinator.com) — Tech & startup community links
- [Lobsters](https://lobste.rs) — Computing-focused link aggregation
- [Wikipedia (Rust)](https://en.wikipedia.org/wiki/Rust_(programming_language)) — Free encyclopedia
- [DuckDuckGo Lite](https://html.duckduckgo.com/html/) — Lightweight HTML search engine
- [Klast Welcome Guide](about:welcome) — Built-in browser overview & controls

---

## Offline Reading List

"#;

#[derive(Debug, Clone)]
pub struct StorageManager {
    archive_dir: PathBuf,
    bookmarks_file: PathBuf,
}

impl Default for StorageManager {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageManager {
    /// Initializes `~/.config/klast/`, `~/.config/klast/archive/`, and `~/.config/klast/bookmarks.md`.
    pub fn new() -> Self {
        let home_dir = env::var_os("HOME")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));

        let base_dir = home_dir.join(".config").join("klast");
        Self::with_base_dir(base_dir)
    }

    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        let archive_dir = base_dir.join("archive");
        let bookmarks_file = base_dir.join("bookmarks.md");

        let _ = fs::create_dir_all(&archive_dir);
        if !bookmarks_file.exists() {
            let _ = fs::write(&bookmarks_file, STARTER_BOOKMARKS_MD);
        }

        Self {
            archive_dir,
            bookmarks_file,
        }
    }

    /// Reads the raw Markdown contents of `bookmarks.md`.
    pub fn read_bookmarks_markdown(&self) -> io::Result<String> {
        if !self.bookmarks_file.exists() {
            fs::create_dir_all(&self.archive_dir)?;
            fs::write(&self.bookmarks_file, STARTER_BOOKMARKS_MD)?;
        }
        fs::read_to_string(&self.bookmarks_file)
    }

    /// Appends `- [Page Title](current_url)` to `bookmarks.md` under `# ★ Bookmarks`,
    /// preventing duplicate URLs. Returns `Ok(true)` if added, `Ok(false)` if already present.
    pub fn add_bookmark(&self, title: &str, url: &str) -> io::Result<bool> {
        let mut content = self.read_bookmarks_markdown()?;
        let clean_title = sanitize_markdown_label(title, url);
        let target_needle = format!("]({url})");

        if content.contains(&target_needle) {
            return Ok(false);
        }

        let entry_line = format!("- [{clean_title}]({url})\n");

        // Insert before `## Offline Reading List` (or `---` preceding it) if present
        if let Some(offline_idx) = content.find("## Offline Reading List") {
            let insert_pos = content[..offline_idx].rfind("---").unwrap_or(offline_idx);
            let mut updated = String::with_capacity(content.len() + entry_line.len() + 2);
            updated.push_str(content[..insert_pos].trim_end());
            updated.push('\n');
            updated.push_str(&entry_line);
            updated.push('\n');
            updated.push_str(&content[insert_pos..]);
            content = updated;
        } else {
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&entry_line);
        }

        fs::write(&self.bookmarks_file, content)?;
        Ok(true)
    }

    /// Serializes `nodes` to `~/.config/klast/archive/<sanitized_title>.md` and appends
    /// a `file://` link under `## Offline Reading List` in `bookmarks.md`.
    /// Returns the saved file path and size in bytes.
    pub fn save_offline_article(
        &self,
        title: &str,
        url: &str,
        nodes: &[DomNode],
    ) -> io::Result<(PathBuf, usize)> {
        fs::create_dir_all(&self.archive_dir)?;

        let stem = sanitize_filename(if title.trim().is_empty() { url } else { title });
        let file_path = self.archive_dir.join(format!("{stem}.md"));

        let markdown = dom_to_markdown(title, url, nodes);
        let bytes_written = markdown.len();
        fs::write(&file_path, &markdown)?;

        let file_url = Url::from_file_path(&file_path)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| format!("file://{}", file_path.display()));

        let mut bookmarks_md = self.read_bookmarks_markdown()?;
        let link_needle = format!("]({file_url})");
        if !bookmarks_md.contains(&link_needle) {
            let clean_title = sanitize_markdown_label(title, url);
            let kb = (bytes_written as f32 / 1024.0).max(0.1);
            let offline_entry =
                format!("- [{clean_title}]({file_url}) — Saved offline ({kb:.1} KB)\n");

            if !bookmarks_md.contains("## Offline Reading List") {
                if !bookmarks_md.ends_with('\n') {
                    bookmarks_md.push('\n');
                }
                bookmarks_md.push_str("\n---\n\n## Offline Reading List\n\n");
            } else if !bookmarks_md.ends_with('\n') {
                bookmarks_md.push('\n');
            }

            bookmarks_md.push_str(&offline_entry);
            fs::write(&self.bookmarks_file, bookmarks_md)?;
        }

        Ok((file_path, bytes_written))
    }
}

/// Converts a slice of `DomNode` into a clean, human-readable Markdown document.
pub fn dom_to_markdown(title: &str, url: &str, nodes: &[DomNode]) -> String {
    let mut out = String::new();
    let display_title = if title.trim().is_empty() {
        url
    } else {
        title.trim()
    };

    out.push_str(&format!("# {display_title}\n\n"));
    out.push_str(&format!("> Source: [{url}]({url})\n\n---\n\n"));

    for node in nodes {
        match node {
            DomNode::Heading(level, text) => {
                let hashes = "#".repeat((*level as usize).clamp(1, 6));
                out.push_str(&format!("{hashes} {text}\n\n"));
            }
            DomNode::Paragraph(spans) => {
                let line = spans_to_markdown(spans);
                if !line.trim().is_empty() {
                    out.push_str(&line);
                    out.push_str("\n\n");
                }
            }
            DomNode::List(items) => {
                for item in items {
                    let line = spans_to_markdown(item);
                    if !line.trim().is_empty() {
                        out.push_str(&format!("- {}\n", line.trim()));
                    }
                }
                out.push('\n');
            }
            DomNode::OrderedList(items) => {
                for (num, item) in items {
                    let line = spans_to_markdown(item);
                    if !line.trim().is_empty() {
                        out.push_str(&format!("{num}. {}\n", line.trim()));
                    }
                }
                out.push('\n');
            }
            DomNode::Table { headers, rows } => {
                if !headers.is_empty() {
                    let header_cells: Vec<String> = headers
                        .iter()
                        .map(|h| spans_to_markdown(h).trim().to_string())
                        .collect();
                    let sep_cells: Vec<&str> = headers.iter().map(|_| "---").collect();
                    out.push_str(&format!("| {} |\n", header_cells.join(" | ")));
                    out.push_str(&format!("| {} |\n", sep_cells.join(" | ")));
                    for row in rows {
                        let row_cells: Vec<String> = row
                            .iter()
                            .map(|c| spans_to_markdown(c).trim().to_string())
                            .collect();
                        out.push_str(&format!("| {} |\n", row_cells.join(" | ")));
                    }
                    out.push('\n');
                }
            }
            DomNode::CodeBlock(code) => {
                out.push_str("```\n");
                out.push_str(code);
                if !code.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("```\n\n");
            }
            DomNode::Quote(quote) => {
                for line in quote.lines() {
                    out.push_str(&format!("> {line}\n"));
                }
                out.push('\n');
            }
            DomNode::Form(form) => {
                out.push_str(&format!(
                    "> [Form: {} -> {}]\n\n",
                    form.submit_label, form.action
                ));
            }
            DomNode::Image { url, alt } => {
                out.push_str(&format!("![{alt}]({url})\n\n"));
            }
            DomNode::HorizontalRule => {
                out.push_str("---\n\n");
            }
        }
    }

    out
}

fn spans_to_markdown(spans: &[InlineSpan]) -> String {
    let mut buf = String::new();
    for span in spans {
        match span {
            InlineSpan::Text(t) => buf.push_str(t),
            InlineSpan::Bold(b) => buf.push_str(&format!("**{b}**")),
            InlineSpan::Italic(i) => buf.push_str(&format!("*{i}*")),
            InlineSpan::BoldItalic(bi) => buf.push_str(&format!("***{bi}***")),
            InlineSpan::Strikethrough(s) => buf.push_str(&format!("~~{s}~~")),
            InlineSpan::Code(c) => buf.push_str(&format!("`{c}`")),
            InlineSpan::Link { text, href } => buf.push_str(&format!("[{text}]({href})")),
            InlineSpan::ImageBadge { alt, url } => buf.push_str(&format!("[IMG: {alt}]({url})")),
        }
    }
    buf
}

fn sanitize_markdown_label(title: &str, fallback_url: &str) -> String {
    let raw = if title.trim().is_empty() {
        fallback_url
    } else {
        title.trim()
    };
    raw.replace(['[', ']', '\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn sanitize_filename(input: &str) -> String {
    let mut slug = String::new();
    let mut prev_underscore = false;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore && !slug.is_empty() {
            slug.push('_');
            prev_underscore = true;
        }
        if slug.len() >= 64 {
            break;
        }
    }

    let trimmed = slug.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "saved_article".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::parse_markdown_to_dom;

    #[test]
    fn test_dom_to_markdown_roundtrip() {
        let nodes = vec![
            DomNode::Heading(2, "Section Header".to_string()),
            DomNode::Paragraph(vec![
                InlineSpan::Text("Read ".to_string()),
                InlineSpan::Bold("important".to_string()),
                InlineSpan::Text(" docs at ".to_string()),
                InlineSpan::Link {
                    text: "Rust Lang".to_string(),
                    href: "https://www.rust-lang.org".to_string(),
                },
            ]),
            DomNode::List(vec![vec![
                InlineSpan::Link {
                    text: "Hacker News".to_string(),
                    href: "https://news.ycombinator.com".to_string(),
                },
                InlineSpan::Text(" — Daily links".to_string()),
            ]]),
            DomNode::Quote("Less is more.".to_string()),
            DomNode::CodeBlock("fn main() {}".to_string()),
        ];

        let md = dom_to_markdown("Test Page", "https://example.com", &nodes);
        let parsed = parse_markdown_to_dom(&md);

        assert!(parsed.contains(&DomNode::Heading(1, "Test Page".to_string())));
        assert!(parsed.contains(&DomNode::Heading(2, "Section Header".to_string())));
        assert!(parsed.contains(&DomNode::Quote("Less is more.".to_string())));
        assert!(parsed.contains(&DomNode::CodeBlock("fn main() {}".to_string())));
    }

    #[test]
    fn test_storage_manager_bookmarks_and_offline_archive() {
        let temp_dir = std::env::temp_dir().join(format!("klast_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);

        let manager = StorageManager::with_base_dir(temp_dir.clone());
        let initial_md = manager.read_bookmarks_markdown().unwrap();
        assert!(initial_md.contains("# ★ Bookmarks"));
        assert!(initial_md.contains("## Offline Reading List"));

        let added = manager
            .add_bookmark("FreeBSD Project", "https://www.freebsd.org")
            .unwrap();
        assert!(added);

        let duplicate = manager
            .add_bookmark("FreeBSD Duplicate", "https://www.freebsd.org")
            .unwrap();
        assert!(!duplicate);

        let nodes = vec![
            DomNode::Heading(2, "Offline Content".to_string()),
            DomNode::Paragraph(vec![InlineSpan::Text(
                "Saved for offline reading.".to_string(),
            )]),
        ];
        let (saved_path, bytes) = manager
            .save_offline_article("FreeBSD Handbook", "https://docs.freebsd.org", &nodes)
            .unwrap();
        assert!(saved_path.exists());
        assert!(bytes > 0);

        let updated_md = manager.read_bookmarks_markdown().unwrap();
        assert!(updated_md.contains("FreeBSD Project"));
        assert!(updated_md.contains("FreeBSD Handbook"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
