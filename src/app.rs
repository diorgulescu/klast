// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Klast Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

use crate::adblock_filter::AdblockFilter;
use crate::dom::{parse_html, parse_markdown_to_dom, DomNode, FormNode, InlineSpan};
use crate::net::{
    build_get_form_url, normalize_input_to_url, rewrite_url, FetchResult, NetworkClient,
};
use crate::storage::StorageManager;
use eframe::egui::{
    self, Color32, ColorImage, FontFamily, FontId, Frame, Key, Margin, RichText, Rounding,
    ScrollArea, Sense, Stroke, TextEdit, TextureHandle, TextureOptions, Vec2, Visuals,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use url::Url;

const DEFAULT_HOME_URL: &str = "https://news.ycombinator.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserTheme {
    HighContrastLight,
    Dark,
    Sepia,
}

impl BrowserTheme {
    pub fn next(self) -> Self {
        match self {
            Self::HighContrastLight => Self::Dark,
            Self::Dark => Self::Sepia,
            Self::Sepia => Self::HighContrastLight,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::HighContrastLight => "Light",
            Self::Dark => "Dark",
            Self::Sepia => "Sepia",
        }
    }

    pub fn palette(self) -> ThemePalette {
        match self {
            Self::HighContrastLight => ThemePalette {
                bg: Color32::from_rgb(250, 250, 250),
                panel_bg: Color32::from_rgb(232, 232, 232),
                code_bg: Color32::from_rgb(238, 238, 238),
                fg: Color32::from_rgb(15, 15, 15),
                muted_fg: Color32::from_rgb(85, 85, 85),
                link: Color32::from_rgb(0, 72, 186),
                border: Color32::from_rgb(180, 180, 180),
                accent: Color32::from_rgb(30, 90, 190),
                is_dark: false,
            },
            Self::Dark => ThemePalette {
                bg: Color32::from_rgb(20, 22, 25),
                panel_bg: Color32::from_rgb(30, 33, 38),
                code_bg: Color32::from_rgb(34, 38, 44),
                fg: Color32::from_rgb(226, 230, 235),
                muted_fg: Color32::from_rgb(150, 158, 168),
                link: Color32::from_rgb(95, 175, 255),
                border: Color32::from_rgb(62, 68, 76),
                accent: Color32::from_rgb(80, 150, 230),
                is_dark: true,
            },
            Self::Sepia => ThemePalette {
                bg: Color32::from_rgb(244, 236, 216),
                panel_bg: Color32::from_rgb(230, 220, 195),
                code_bg: Color32::from_rgb(235, 225, 202),
                fg: Color32::from_rgb(58, 44, 28),
                muted_fg: Color32::from_rgb(112, 92, 68),
                link: Color32::from_rgb(148, 52, 20),
                border: Color32::from_rgb(196, 180, 152),
                accent: Color32::from_rgb(148, 52, 20),
                is_dark: false,
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ThemePalette {
    pub bg: Color32,
    pub panel_bg: Color32,
    pub code_bg: Color32,
    pub fg: Color32,
    pub muted_fg: Color32,
    pub link: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub is_dark: bool,
}

enum ImageState {
    Loading,
    Loaded(TextureHandle, Vec2),
    Failed(String),
}

pub struct KlastApp {
    address_input: String,
    current_url: String,
    page_title: String,
    raw_html: String,
    is_markdown_doc: bool,
    dom_nodes: Vec<DomNode>,
    bytes_received: usize,

    // Form state keyed by input `name`
    form_inputs: HashMap<String, String>,

    // Navigation history stack
    history: Vec<String>,
    history_index: Option<usize>,

    // Browser mode toggles
    reader_mode: bool,
    smart_redirects: bool,
    images_enabled: bool,
    theme: BrowserTheme,

    // Adblock engine and live counter
    adblock: AdblockFilter,
    blocked_count: usize,

    // Plain-text Markdown bookmarks & offline storage manager
    storage: StorageManager,

    // Lazy image cache and on-demand preview set
    image_cache: HashMap<String, ImageState>,
    previewed_images: HashSet<String>,

    is_loading: bool,
    status_message: String,
    hovered_link: Option<String>,

    net_client: NetworkClient,
}

impl KlastApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = BrowserTheme::Dark;
        Self::apply_flat_visuals(&cc.egui_ctx, theme);

        let storage = StorageManager::new();

        let mut app = Self {
            address_input: DEFAULT_HOME_URL.to_string(),
            current_url: DEFAULT_HOME_URL.to_string(),
            page_title: "Klast Browser".to_string(),
            raw_html: String::new(),
            is_markdown_doc: false,
            dom_nodes: Vec::new(),
            bytes_received: 0,
            form_inputs: HashMap::new(),
            history: Vec::new(),
            history_index: None,
            reader_mode: false,
            smart_redirects: true,
            images_enabled: false,
            theme,
            adblock: AdblockFilter::new(),
            blocked_count: 0,
            storage,
            image_cache: HashMap::new(),
            previewed_images: HashSet::new(),
            is_loading: false,
            status_message: "Ready".to_string(),
            hovered_link: None,
            net_client: NetworkClient::new(),
        };

        // Render internal welcome guide immediately while initial page fetches in background
        app.raw_html = crate::net::build_internal_page("about:welcome");
        app.reparse_current_html();
        app.navigate_to(DEFAULT_HOME_URL.to_string(), true);
        app
    }

    fn apply_flat_visuals(ctx: &egui::Context, theme: BrowserTheme) {
        let p = theme.palette();
        let mut visuals = if p.is_dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };

        // Disable visual bloat: flat borders, zero rounding, no shadows, zero animation time
        visuals.window_rounding = Rounding::ZERO;
        visuals.menu_rounding = Rounding::ZERO;
        visuals.panel_fill = p.bg;
        visuals.window_fill = p.bg;
        visuals.extreme_bg_color = p.bg;
        visuals.faint_bg_color = p.code_bg;
        visuals.code_bg_color = p.code_bg;
        visuals.hyperlink_color = p.link;
        visuals.override_text_color = Some(p.fg);

        visuals.widgets.noninteractive.rounding = Rounding::ZERO;
        visuals.widgets.noninteractive.bg_fill = p.panel_bg;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.fg);

        visuals.widgets.inactive.rounding = Rounding::ZERO;
        visuals.widgets.inactive.bg_fill = p.bg;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.fg);

        visuals.widgets.hovered.rounding = Rounding::ZERO;
        visuals.widgets.hovered.bg_fill = p.code_bg;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.fg);

        visuals.widgets.active.rounding = Rounding::ZERO;
        visuals.widgets.active.bg_fill = p.code_bg;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, p.fg);

        let mut style = (*ctx.style()).clone();
        style.animation_time = 0.0;
        style.visuals = visuals;
        ctx.set_style(style);
    }

    fn record_history_entry(&mut self, url: &str) {
        if let Some(idx) = self.history_index {
            self.history.truncate(idx + 1);
        }
        if self.history.last().map(String::as_str) != Some(url) {
            self.history.push(url.to_string());
            self.history_index = Some(self.history.len() - 1);
        }
    }

    fn navigate_to(&mut self, raw_input: String, record_history: bool) {
        self.form_inputs.clear();
        self.previewed_images.clear();

        let normalized = normalize_input_to_url(&raw_input);

        // Handle internal `about:bookmarks` scheme
        if normalized == "about:bookmarks" {
            self.current_url = normalized.clone();
            self.address_input = normalized.clone();
            self.is_loading = false;
            self.is_markdown_doc = true;
            self.blocked_count = 0;

            match self.storage.read_bookmarks_markdown() {
                Ok(md) => {
                    self.bytes_received = md.len();
                    self.raw_html = md.clone();
                    self.dom_nodes = parse_markdown_to_dom(&md);
                    self.page_title = "Bookmarks & Offline Reading List".to_string();
                    self.status_message = "Ready (about:bookmarks)".to_string();
                }
                Err(err) => {
                    self.bytes_received = 0;
                    self.dom_nodes = vec![
                        DomNode::Heading(1, "Bookmarks Error".to_string()),
                        DomNode::Paragraph(vec![InlineSpan::Text(format!(
                            "Could not read bookmarks.md: {err}"
                        ))]),
                    ];
                    self.status_message = "Failed to load bookmarks.md".to_string();
                }
            }

            if record_history {
                self.record_history_entry(&normalized);
            }
            return;
        }

        // Handle local `file://` scheme for offline Markdown or HTML articles
        if normalized.starts_with("file://") {
            self.current_url = normalized.clone();
            self.address_input = normalized.clone();
            self.is_loading = false;
            self.blocked_count = 0;

            let file_path = Url::parse(&normalized)
                .ok()
                .and_then(|u| u.to_file_path().ok())
                .unwrap_or_else(|| PathBuf::from(normalized.trim_start_matches("file://")));

            match fs::read_to_string(&file_path) {
                Ok(contents) => {
                    self.bytes_received = contents.len();
                    self.raw_html = contents.clone();
                    let is_md = file_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|ext| {
                            ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
                        });
                    self.is_markdown_doc = is_md;

                    if is_md {
                        self.dom_nodes = parse_markdown_to_dom(&contents);
                        self.page_title = self
                            .dom_nodes
                            .iter()
                            .find_map(|n| match n {
                                DomNode::Heading(1, t) => Some(t.clone()),
                                _ => None,
                            })
                            .unwrap_or_else(|| file_path.display().to_string());
                    } else {
                        self.reparse_current_html();
                    }
                    self.status_message =
                        format!("Offline article loaded ({})", file_path.display());
                }
                Err(err) => {
                    self.bytes_received = 0;
                    self.is_markdown_doc = true;
                    self.dom_nodes = vec![
                        DomNode::Heading(1, "Offline File Not Found".to_string()),
                        DomNode::Paragraph(vec![
                            InlineSpan::Bold("Path: ".to_string()),
                            InlineSpan::Code(file_path.display().to_string()),
                        ]),
                        DomNode::Paragraph(vec![InlineSpan::Text(format!("Error: {err}"))]),
                    ];
                    self.status_message = "Error reading local file".to_string();
                }
            }

            if record_history {
                self.record_history_entry(&normalized);
            }
            return;
        }

        // Standard HTTP/HTTPS or internal `about:welcome` navigation
        self.is_markdown_doc = false;
        let resolved_url = self.net_client.fetch(&raw_input, self.smart_redirects);
        self.current_url = resolved_url.clone();
        self.address_input = resolved_url.clone();
        self.is_loading = true;
        self.status_message = format!("Loading {resolved_url}...");

        if record_history {
            self.record_history_entry(&resolved_url);
        }
    }

    fn can_go_back(&self) -> bool {
        matches!(self.history_index, Some(idx) if idx > 0)
    }

    fn can_go_forward(&self) -> bool {
        match self.history_index {
            Some(idx) => idx + 1 < self.history.len(),
            None => false,
        }
    }

    fn go_back(&mut self) {
        if let Some(idx) = self.history_index {
            if idx > 0 {
                let target_idx = idx - 1;
                self.history_index = Some(target_idx);
                let target_url = self.history[target_idx].clone();
                self.navigate_to(target_url, false);
            }
        }
    }

    fn go_forward(&mut self) {
        if let Some(idx) = self.history_index {
            if idx + 1 < self.history.len() {
                let target_idx = idx + 1;
                self.history_index = Some(target_idx);
                let target_url = self.history[target_idx].clone();
                self.navigate_to(target_url, false);
            }
        }
    }

    fn reload(&mut self) {
        let url = self.current_url.clone();
        self.navigate_to(url, false);
    }

    fn bookmark_current_page(&mut self) {
        let title = if self.page_title.trim().is_empty() {
            &self.current_url
        } else {
            &self.page_title
        };
        match self.storage.add_bookmark(title, &self.current_url) {
            Ok(true) => {
                self.status_message = "Bookmarked!".to_string();
                if self.current_url == "about:bookmarks" {
                    self.reload();
                    self.status_message = "Bookmarked!".to_string();
                }
            }
            Ok(false) => {
                self.status_message = "Bookmarked! (Already in bookmarks.md)".to_string();
            }
            Err(err) => {
                self.status_message = format!("Bookmark error: {err}");
            }
        }
    }

    fn save_current_page_offline(&mut self) {
        let title = if self.page_title.trim().is_empty() {
            &self.current_url
        } else {
            &self.page_title
        };
        match self
            .storage
            .save_offline_article(title, &self.current_url, &self.dom_nodes)
        {
            Ok((_path, bytes)) => {
                let kb = (bytes as f32 / 1024.0).max(0.1);
                self.status_message = format!("Saved offline ({kb:.1} KB)");
            }
            Err(err) => {
                self.status_message = format!("Failed saving offline: {err}");
            }
        }
    }

    fn spawn_webkit_fallback(&mut self) {
        match std::env::current_exe() {
            Ok(exe_path) => {
                match std::process::Command::new(exe_path)
                    .arg("--webview")
                    .arg(&self.current_url)
                    .spawn()
                {
                    Ok(_) => {
                        self.status_message =
                            "Opened in WebKit Fallback process (RAM isolated)".to_string();
                    }
                    Err(err) => {
                        self.status_message =
                            format!("Failed to spawn WebKit Fallback process: {err}");
                    }
                }
            }
            Err(err) => {
                self.status_message = format!("Failed to locate Klast executable: {err}");
            }
        }
    }

    fn reparse_current_html(&mut self) {
        if self.raw_html.is_empty() {
            return;
        }
        if self.is_markdown_doc {
            self.dom_nodes = parse_markdown_to_dom(&self.raw_html);
            return;
        }

        let parsed = parse_html(
            &self.raw_html,
            &self.current_url,
            self.reader_mode,
            self.images_enabled,
            &self.adblock,
        );
        self.page_title = parsed.title;
        self.blocked_count = parsed.blocked_count;
        self.dom_nodes = parsed.nodes;

        // Seed form_inputs map with initial values for any form fields not yet edited
        for node in &self.dom_nodes {
            if let DomNode::Form(form) = node {
                for input in &form.inputs {
                    self.form_inputs
                        .entry(input.name.clone())
                        .or_insert_with(|| input.initial_value.clone());
                }
            }
        }
    }

    fn submit_form(&mut self, form: &FormNode) {
        let field_pairs: Vec<(String, String)> = form
            .inputs
            .iter()
            .map(|inp| (inp.name.clone(), inp.name.clone()))
            .collect();

        // Construct GET URL with query parameters using the `url` crate
        if let Some(target_url) = build_get_form_url(&form.action, &field_pairs, &self.form_inputs)
        {
            self.navigate_to(target_url, true);
        }
    }

    fn ensure_image_requested(&mut self, url: &str) {
        if !self.image_cache.contains_key(url) {
            self.image_cache
                .insert(url.to_string(), ImageState::Loading);
            self.net_client.fetch_image(url);
        }
    }

    fn poll_network(&mut self, ctx: &egui::Context) {
        while let Some(result) = self.net_client.poll() {
            match result {
                FetchResult::Success(resp) => {
                    self.is_loading = false;
                    self.is_markdown_doc = false;
                    self.current_url = resp.final_url.clone();
                    self.address_input = resp.final_url;
                    self.bytes_received = resp.bytes_received;
                    self.status_message = resp.status_text;
                    self.raw_html = resp.html;
                    self.reparse_current_html();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                        "{} — Klast",
                        self.page_title
                    )));
                }
                FetchResult::Error { url, error } => {
                    self.is_loading = false;
                    self.status_message = format!("Error loading {url}");
                    self.bytes_received = 0;
                    self.dom_nodes = vec![
                        DomNode::Heading(1, "Navigation Failed".to_string()),
                        DomNode::Paragraph(vec![
                            InlineSpan::Bold("Target URL: ".to_string()),
                            InlineSpan::Code(url),
                        ]),
                        DomNode::Paragraph(vec![InlineSpan::Text(error)]),
                        DomNode::HorizontalRule,
                        DomNode::Paragraph(vec![
                            InlineSpan::Text("You can open ".to_string()),
                            InlineSpan::Link {
                                text: "about:bookmarks".to_string(),
                                href: "about:bookmarks".to_string(),
                            },
                            InlineSpan::Text(" or the offline ".to_string()),
                            InlineSpan::Link {
                                text: "about:welcome".to_string(),
                                href: "about:welcome".to_string(),
                            },
                            InlineSpan::Text(" guide.".to_string()),
                        ]),
                    ];
                }
                FetchResult::ImageLoaded {
                    url,
                    rgba,
                    width,
                    height,
                } => {
                    let color_image = ColorImage::from_rgba_unmultiplied([width, height], &rgba);
                    let texture = ctx.load_texture(&url, color_image, TextureOptions::LINEAR);
                    let display_size = Vec2::new(width as f32, height as f32);
                    self.image_cache
                        .insert(url, ImageState::Loaded(texture, display_size));
                }
                FetchResult::ImageFailed { url, error } => {
                    self.image_cache.insert(url, ImageState::Failed(error));
                }
            }
        }

        let any_image_loading = self
            .image_cache
            .values()
            .any(|s| matches!(s, ImageState::Loading));
        if self.is_loading || any_image_loading {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    fn render_top_toolbar(&mut self, ctx: &egui::Context) {
        let p = self.theme.palette();
        let frame = Frame::none()
            .fill(p.panel_bg)
            .inner_margin(Margin::symmetric(6.0, 4.0))
            .stroke(Stroke::new(1.0_f32, p.border));

        egui::TopBottomPanel::top("klast_top_toolbar")
            .exact_height(32.0)
            .frame(frame)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
                ui.horizontal_centered(|ui| {
                    // Navigation buttons
                    if ui
                        .add_enabled(self.can_go_back(), egui::Button::new("[ ◀ Back ]"))
                        .on_hover_text("Go back one page")
                        .clicked()
                    {
                        self.go_back();
                    }

                    if ui
                        .add_enabled(self.can_go_forward(), egui::Button::new("[ ▶ Forward ]"))
                        .on_hover_text("Go forward one page")
                        .clicked()
                    {
                        self.go_forward();
                    }

                    if ui
                        .button("[ ↻ Reload ]")
                        .on_hover_text("Reload current page")
                        .clicked()
                    {
                        self.reload();
                    }

                    // Mode Toggles, Bookmarks & Offline Actions aligned right so address bar fills remaining width
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("[ ⚡ WebKit Fallback ]")
                            .on_hover_text(
                                "Launch current URL in an isolated system WebView subprocess",
                            )
                            .clicked()
                        {
                            self.spawn_webkit_fallback();
                        }

                        let theme_label = format!("[ 🌗 Theme: {} ]", self.theme.label());
                        if ui
                            .button(theme_label)
                            .on_hover_text(
                                "Cycle between High-Contrast Light, Dark, and Sepia/Paper themes",
                            )
                            .clicked()
                        {
                            self.theme = self.theme.next();
                            Self::apply_flat_visuals(ctx, self.theme);
                        }

                        let img_label = if self.images_enabled {
                            "[ 🖼️ Images: ON ]"
                        } else {
                            "[ 🖼️ Images: OFF ]"
                        };
                        if ui
                            .button(img_label)
                            .on_hover_text(
                                "Toggle lazy image loading (OFF renders lightweight [IMG: alt] badges)",
                            )
                            .clicked()
                        {
                            self.images_enabled = !self.images_enabled;
                            self.reparse_current_html();
                        }

                        let redirect_label = if self.smart_redirects {
                            "[ 🔄 Smart Redirects: ON ]"
                        } else {
                            "[ 🔄 Smart Redirects: OFF ]"
                        };
                        if ui
                            .button(redirect_label)
                            .on_hover_text(
                                "Rewrite reddit.com -> old.reddit.com, en.wikipedia.org -> en.m.wikipedia.org, and strip tracking params",
                            )
                            .clicked()
                        {
                            self.smart_redirects = !self.smart_redirects;
                            if self.smart_redirects {
                                let rewritten = rewrite_url(&self.current_url, true);
                                if rewritten != self.current_url {
                                    self.navigate_to(rewritten, true);
                                }
                            }
                        }

                        let mode_label = if self.reader_mode {
                            "[ 📖 Reader ]"
                        } else {
                            "[ 📖 Full DOM ]"
                        };
                        if ui
                            .button(mode_label)
                            .on_hover_text(
                                "Toggle between clean <article>/<main> extraction and full <body>",
                            )
                            .clicked()
                        {
                            self.reader_mode = !self.reader_mode;
                            self.reparse_current_html();
                        }

                        // Offline & Bookmark controls
                        if ui
                            .button("[ 📥 Save Offline ]")
                            .on_hover_text(
                                "Save current page as plain-text Markdown in ~/.config/klast/archive/",
                            )
                            .clicked()
                        {
                            self.save_current_page_offline();
                        }

                        if ui
                            .button("[ +★ ]")
                            .on_hover_text("Bookmark current page to ~/.config/klast/bookmarks.md")
                            .clicked()
                        {
                            self.bookmark_current_page();
                        }

                        if ui
                            .button("[ ★ Bookmarks ]")
                            .on_hover_text("Open plain-text Markdown bookmarks & offline reading list")
                            .clicked()
                        {
                            self.navigate_to("about:bookmarks".to_string(), true);
                        }

                        // Live adblock counter badge
                        let blocked_badge = format!("[ 🛡️ Blocked: {} ]", self.blocked_count);
                        ui.add_enabled(false, egui::Button::new(blocked_badge))
                            .on_disabled_hover_text(
                                "Number of ad/tracker URLs and cosmetic banner/consent elements stripped on this page",
                            );

                        // Single-line Address / Search Bar taking up all remaining width
                        let available_width = ui.available_width().max(90.0);
                        let response = ui.add_sized(
                            [available_width, 22.0],
                            TextEdit::singleline(&mut self.address_input)
                                .font(FontId::new(13.5, FontFamily::Monospace))
                                .hint_text("Enter URL, about:bookmarks, or search..."),
                        );

                        if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            let target = self.address_input.clone();
                            self.navigate_to(target, true);
                        }
                    });
                });
            });
    }

    fn render_bottom_status_bar(&self, ctx: &egui::Context) {
        let p = self.theme.palette();
        let frame = Frame::none()
            .fill(p.panel_bg)
            .inner_margin(Margin::symmetric(8.0, 2.0))
            .stroke(Stroke::new(1.0_f32, p.border));

        egui::TopBottomPanel::bottom("klast_status_bar")
            .exact_height(20.0)
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let status_str = if self.is_loading {
                        "Loading..."
                    } else {
                        &self.status_message
                    };

                    let kb = self.bytes_received as f32 / 1024.0;
                    let summary = format!(
                        "{status_str}  |  DOM Nodes: {}  |  Blocked: {}  |  Payload: {:.1} KB",
                        self.dom_nodes.len(),
                        self.blocked_count,
                        kb
                    );
                    ui.label(
                        RichText::new(summary)
                            .font(FontId::new(11.5, FontFamily::Monospace))
                            .color(p.muted_fg),
                    );

                    if let Some(href) = &self.hovered_link {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("→ {href}"))
                                    .font(FontId::new(11.5, FontFamily::Monospace))
                                    .color(p.link),
                            );
                        });
                    }
                });
            });
    }

    fn render_document_view(&mut self, ctx: &egui::Context) {
        let p = self.theme.palette();
        let mut clicked_link: Option<String> = None;
        let mut submitted_form: Option<FormNode> = None;
        let mut requested_images: Vec<String> = Vec::new();
        let mut newly_previewed: Vec<(String, String)> = Vec::new();
        self.hovered_link = None;

        let central_frame = Frame::none()
            .fill(p.bg)
            .inner_margin(Margin::symmetric(24.0, 16.0));

        let dom_snapshot = self.dom_nodes.clone();

        egui::CentralPanel::default()
            .frame(central_frame)
            .show(ctx, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let max_read_width = if self.reader_mode { 760.0 } else { 980.0 };
                        ui.set_max_width(ui.available_width().min(max_read_width));
                        ui.spacing_mut().item_spacing = Vec2::new(4.0, 10.0);

                        if dom_snapshot.is_empty() && !self.is_loading {
                            ui.label(
                                RichText::new("No text content extracted from this page.")
                                    .color(p.muted_fg),
                            );
                        }

                        for (node_idx, node) in dom_snapshot.iter().enumerate() {
                            match node {
                                DomNode::Heading(level, text) => {
                                    let size = match level {
                                        1 => 24.0,
                                        2 => 20.0,
                                        3 => 17.5,
                                        4 => 15.5,
                                        5 => 14.5,
                                        _ => 13.5,
                                    };
                                    ui.add_space(4.0);
                                    ui.label(
                                        RichText::new(text)
                                            .font(FontId::new(size, FontFamily::Proportional))
                                            .strong()
                                            .color(p.fg),
                                    );
                                }
                                DomNode::Paragraph(spans) => {
                                    Self::render_inline_spans(
                                        ui,
                                        spans,
                                        p,
                                        &mut self.hovered_link,
                                        &mut clicked_link,
                                        &mut newly_previewed,
                                    );
                                    for span in spans {
                                        if let InlineSpan::ImageBadge { alt, url } = span {
                                            if self.previewed_images.contains(url) {
                                                requested_images.push(url.clone());
                                                self.render_image_widget(
                                                    ui,
                                                    url,
                                                    alt,
                                                    p,
                                                    &mut clicked_link,
                                                );
                                            }
                                        }
                                    }
                                }
                                DomNode::List(items) => {
                                    for item_spans in items {
                                        ui.horizontal_top(|ui| {
                                            ui.label(
                                                RichText::new(" • ")
                                                    .font(FontId::new(
                                                        14.5,
                                                        FontFamily::Proportional,
                                                    ))
                                                    .strong()
                                                    .color(p.muted_fg),
                                            );
                                            Self::render_inline_spans(
                                                ui,
                                                item_spans,
                                                p,
                                                &mut self.hovered_link,
                                                &mut clicked_link,
                                                &mut newly_previewed,
                                            );
                                        });
                                    }
                                }
                                DomNode::CodeBlock(code) => {
                                    Frame::none()
                                        .fill(p.code_bg)
                                        .stroke(Stroke::new(1.0_f32, p.border))
                                        .inner_margin(Margin::symmetric(10.0, 8.0))
                                        .show(ui, |ui| {
                                            ui.set_min_width(ui.available_width());
                                            ui.label(
                                                RichText::new(code)
                                                    .font(FontId::new(12.5, FontFamily::Monospace))
                                                    .color(p.fg),
                                            );
                                        });
                                }
                                DomNode::Quote(quote_text) => {
                                    ui.horizontal_top(|ui| {
                                        let (_, rect) = ui.allocate_space(Vec2::new(4.0, 20.0));
                                        ui.painter().rect_filled(rect, Rounding::ZERO, p.accent);
                                        ui.add_space(6.0);
                                        ui.label(
                                            RichText::new(quote_text)
                                                .font(FontId::new(14.5, FontFamily::Proportional))
                                                .italics()
                                                .color(p.muted_fg),
                                        );
                                    });
                                }
                                DomNode::Form(form) => {
                                    Frame::none()
                                        .fill(p.code_bg)
                                        .stroke(Stroke::new(1.0_f32, p.border))
                                        .inner_margin(Margin::symmetric(10.0, 8.0))
                                        .show(ui, |ui| {
                                            ui.push_id(format!("form_{node_idx}"), |ui| {
                                                ui.horizontal_wrapped(|ui| {
                                                    ui.spacing_mut().item_spacing =
                                                        Vec2::new(8.0, 6.0);
                                                    for (input_idx, input) in
                                                        form.inputs.iter().enumerate()
                                                    {
                                                        if input.is_hidden {
                                                            continue;
                                                        }
                                                        let val = self
                                                            .form_inputs
                                                            .entry(input.name.clone())
                                                            .or_insert_with(|| {
                                                                input.initial_value.clone()
                                                            });

                                                        let resp = ui.add_sized(
                                                            [280.0, 22.0],
                                                            TextEdit::singleline(val)
                                                                .id_source(format!(
                                                                    "input_{node_idx}_{input_idx}_{}",
                                                                    input.name
                                                                ))
                                                                .font(FontId::new(
                                                                    13.5,
                                                                    FontFamily::Monospace,
                                                                ))
                                                                .hint_text(&input.placeholder),
                                                        );

                                                        if resp.lost_focus()
                                                            && ui.input(|i| {
                                                                i.key_pressed(Key::Enter)
                                                            })
                                                        {
                                                            submitted_form = Some(form.clone());
                                                        }
                                                    }

                                                    let btn_text =
                                                        format!("[ {} ]", form.submit_label);
                                                    if ui.button(btn_text).clicked() {
                                                        submitted_form = Some(form.clone());
                                                    }
                                                });
                                            });
                                        });
                                }
                                DomNode::Image { url, alt } => {
                                    requested_images.push(url.clone());
                                    self.render_image_widget(ui, url, alt, p, &mut clicked_link);
                                }
                                DomNode::HorizontalRule => {
                                    ui.separator();
                                }
                            }
                        }

                        ui.add_space(24.0);
                    });
            });

        for (alt, url) in newly_previewed {
            self.previewed_images.insert(url.clone());
            requested_images.push(url.clone());
            self.status_message = format!("Previewing image: {alt}");
        }

        for img_url in requested_images {
            self.ensure_image_requested(&img_url);
        }

        if let Some(form) = submitted_form {
            self.submit_form(&form);
        } else if let Some(target_href) = clicked_link {
            self.navigate_to(target_href, true);
        }
    }

    fn render_image_widget(
        &self,
        ui: &mut egui::Ui,
        url: &str,
        alt: &str,
        p: ThemePalette,
        clicked_link: &mut Option<String>,
    ) {
        Frame::none()
            .fill(p.code_bg)
            .stroke(Stroke::new(1.0_f32, p.border))
            .inner_margin(Margin::symmetric(8.0, 6.0))
            .show(ui, |ui| match self.image_cache.get(url) {
                Some(ImageState::Loaded(texture, size)) => {
                    let max_w = ui.available_width().min(560.0);
                    let scale = if size.x > max_w && size.x > 0.0 {
                        max_w / size.x
                    } else {
                        1.0
                    };
                    let render_size = Vec2::new(size.x * scale, size.y * scale);
                    ui.image((texture.id(), render_size));
                    ui.label(
                        RichText::new(format!("🖼️ {alt}"))
                            .font(FontId::new(12.0, FontFamily::Monospace))
                            .color(p.muted_fg),
                    );
                }
                Some(ImageState::Failed(err)) => {
                    let resp = ui.add(
                        egui::Label::new(
                            RichText::new(format!("[IMG: {alt} — {err} (click to open URL)]"))
                                .font(FontId::new(12.5, FontFamily::Monospace))
                                .color(p.muted_fg),
                        )
                        .sense(Sense::click()),
                    );
                    if resp.clicked() {
                        *clicked_link = Some(url.to_string());
                    }
                }
                _ => {
                    ui.label(
                        RichText::new(format!("⏳ Loading image: {alt}..."))
                            .font(FontId::new(12.5, FontFamily::Monospace))
                            .color(p.muted_fg),
                    );
                }
            });
    }

    fn render_inline_spans(
        ui: &mut egui::Ui,
        spans: &[InlineSpan],
        p: ThemePalette,
        hovered_link: &mut Option<String>,
        clicked_link: &mut Option<String>,
        newly_previewed: &mut Vec<(String, String)>,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 4.0);
            for span in spans {
                match span {
                    InlineSpan::Text(text) => {
                        ui.label(
                            RichText::new(text)
                                .font(FontId::new(14.5, FontFamily::Proportional))
                                .color(p.fg),
                        );
                    }
                    InlineSpan::Bold(text) => {
                        ui.label(
                            RichText::new(text)
                                .font(FontId::new(14.5, FontFamily::Proportional))
                                .strong()
                                .color(p.fg),
                        );
                    }
                    InlineSpan::Italic(text) => {
                        ui.label(
                            RichText::new(text)
                                .font(FontId::new(14.5, FontFamily::Proportional))
                                .italics()
                                .color(p.fg),
                        );
                    }
                    InlineSpan::Code(text) => {
                        ui.label(
                            RichText::new(text)
                                .font(FontId::new(13.0, FontFamily::Monospace))
                                .background_color(p.code_bg)
                                .color(p.fg),
                        );
                    }
                    InlineSpan::Link { text, href } => {
                        let link_rich = RichText::new(text)
                            .font(FontId::new(14.5, FontFamily::Proportional))
                            .underline()
                            .color(p.link);
                        let resp = ui.add(egui::Label::new(link_rich).sense(Sense::click()));
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            *hovered_link = Some(href.clone());
                        }
                        if resp.clicked() {
                            *clicked_link = Some(href.clone());
                        }
                    }
                    InlineSpan::ImageBadge { alt, url } => {
                        let badge_text = format!("[IMG: {alt}]");
                        let badge_rich = RichText::new(badge_text)
                            .font(FontId::new(12.5, FontFamily::Monospace))
                            .background_color(p.code_bg)
                            .color(p.accent);
                        let resp = ui
                            .add(egui::Label::new(badge_rich).sense(Sense::click()))
                            .on_hover_text(format!("Click to preview image ({url})"));
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            *hovered_link = Some(url.clone());
                        }
                        if resp.clicked() {
                            newly_previewed.push((alt.clone(), url.clone()));
                        }
                    }
                }
            }
        });
    }
}

impl eframe::App for KlastApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_network(ctx);
        self.render_top_toolbar(ctx);
        self.render_bottom_status_bar(ctx);
        self.render_document_view(ctx);
    }
}
