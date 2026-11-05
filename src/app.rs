use crate::dom::{parse_html, DomNode, InlineSpan};
use crate::net::{FetchResult, NetworkClient};
use eframe::egui::{
    self, Color32, FontFamily, FontId, Frame, Key, Margin, RichText, Rounding, ScrollArea, Sense,
    Stroke, TextEdit, Vec2, Visuals,
};

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

pub struct KlastApp {
    address_input: String,
    current_url: String,
    page_title: String,
    raw_html: String,
    dom_nodes: Vec<DomNode>,
    bytes_received: usize,

    history: Vec<String>,
    history_index: Option<usize>,

    reader_mode: bool,
    theme: BrowserTheme,

    is_loading: bool,
    status_message: String,
    hovered_link: Option<String>,

    net_client: NetworkClient,
}

impl KlastApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = BrowserTheme::Dark;
        Self::apply_flat_visuals(&cc.egui_ctx, theme);

        let mut app = Self {
            address_input: DEFAULT_HOME_URL.to_string(),
            current_url: DEFAULT_HOME_URL.to_string(),
            page_title: "Klast Browser".to_string(),
            raw_html: String::new(),
            dom_nodes: Vec::new(),
            bytes_received: 0,
            history: Vec::new(),
            history_index: None,
            reader_mode: false,
            theme,
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

    fn navigate_to(&mut self, raw_input: String, record_history: bool) {
        let resolved_url = self.net_client.fetch(&raw_input);
        self.current_url = resolved_url.clone();
        self.address_input = resolved_url.clone();
        self.is_loading = true;
        self.status_message = format!("Loading {resolved_url}...");

        if record_history {
            if let Some(idx) = self.history_index {
                self.history.truncate(idx + 1);
            }
            if self.history.last() != Some(&resolved_url) {
                self.history.push(resolved_url);
                self.history_index = Some(self.history.len() - 1);
            }
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

    fn reparse_current_html(&mut self) {
        if self.raw_html.is_empty() {
            return;
        }
        let parsed = parse_html(&self.raw_html, &self.current_url, self.reader_mode);
        self.page_title = parsed.title;
        self.dom_nodes = parsed.nodes;
    }

    fn poll_network(&mut self, ctx: &egui::Context) {
        while let Some(result) = self.net_client.poll() {
            self.is_loading = false;
            match result {
                FetchResult::Success(resp) => {
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
                            InlineSpan::Text("You can open the offline ".to_string()),
                            InlineSpan::Link {
                                text: "about:welcome".to_string(),
                                href: "about:welcome".to_string(),
                            },
                            InlineSpan::Text(" guide or try another URL.".to_string()),
                        ]),
                    ];
                }
            }
        }

        if self.is_loading {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    fn render_top_toolbar(&mut self, ctx: &egui::Context) {
        let p = self.theme.palette();
        let frame = Frame::none()
            .fill(p.panel_bg)
            .inner_margin(Margin::symmetric(8.0, 4.0))
            .stroke(Stroke::new(1.0_f32, p.border));

        egui::TopBottomPanel::top("klast_top_toolbar")
            .exact_height(32.0)
            .frame(frame)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 0.0);
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

                    // Mode Toggles aligned to the right so the address bar fills remaining width
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("[ ⚡ WebKit Fallback ]")
                            .on_hover_text(
                                "Escalate current URL to system WebView (Phase 1 logs URL to stdout)",
                            )
                            .clicked()
                        {
                            println!("{}", self.current_url);
                            self.status_message =
                                format!("WebKit fallback requested for {}", self.current_url);
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

                        let mode_label = if self.reader_mode {
                            "[ 📖 Reader Mode ]"
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

                        // Single-line Address / Search Bar taking up all remaining width
                        let available_width = ui.available_width().max(120.0);
                        let response = ui.add_sized(
                            [available_width, 22.0],
                            TextEdit::singleline(&mut self.address_input)
                                .font(FontId::new(13.5, FontFamily::Monospace))
                                .hint_text("Enter URL or search DuckDuckGo..."),
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
                        "{status_str}  |  DOM Nodes: {}  |  Payload: {:.1} KB (JS/Ads Off)",
                        self.dom_nodes.len(),
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
        self.hovered_link = None;

        let central_frame = Frame::none()
            .fill(p.bg)
            .inner_margin(Margin::symmetric(24.0, 16.0));

        egui::CentralPanel::default()
            .frame(central_frame)
            .show(ctx, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let max_read_width = if self.reader_mode { 760.0 } else { 980.0 };
                        ui.set_max_width(ui.available_width().min(max_read_width));
                        ui.spacing_mut().item_spacing = Vec2::new(4.0, 10.0);

                        if self.dom_nodes.is_empty() && !self.is_loading {
                            ui.label(
                                RichText::new("No text content extracted from this page.")
                                    .color(p.muted_fg),
                            );
                        }

                        for node in &self.dom_nodes {
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
                                    );
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
                                DomNode::HorizontalRule => {
                                    ui.separator();
                                }
                            }
                        }

                        ui.add_space(24.0);
                    });
            });

        if let Some(target_href) = clicked_link {
            self.navigate_to(target_href, true);
        }
    }

    fn render_inline_spans(
        ui: &mut egui::Ui,
        spans: &[InlineSpan],
        p: ThemePalette,
        hovered_link: &mut Option<String>,
        clicked_link: &mut Option<String>,
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
