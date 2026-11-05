use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;
use url::Url;

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";

const BLOCKED_DOMAINS: &[&str] = &[
    "doubleclick.net",
    "googlesyndication.com",
    "google-analytics.com",
    "googletagmanager.com",
    "adservice.google.com",
    "connect.facebook.net",
    "analytics.twitter.com",
    "scorecardresearch.com",
    "zedo.com",
    "adnxs.com",
    "criteo.com",
    "outbrain.com",
    "taboola.com",
];

#[derive(Debug, Clone)]
pub struct FetchResponse {
    pub final_url: String,
    pub status_text: String,
    pub html: String,
    pub bytes_received: usize,
}

#[derive(Debug, Clone)]
pub enum FetchResult {
    Success(FetchResponse),
    Error { url: String, error: String },
}

pub struct NetworkClient {
    tx: Sender<FetchResult>,
    rx: Receiver<FetchResult>,
}

impl Default for NetworkClient {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkClient {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx }
    }

    /// Triggers a non-blocking HTTP GET on a background thread.
    pub fn fetch(&self, raw_input: &str) -> String {
        let target_url = normalize_input_to_url(raw_input);
        let tx = self.tx.clone();
        let url_for_thread = target_url.clone();

        thread::spawn(move || {
            let result = perform_request(&url_for_thread);
            let _ = tx.send(result);
        });

        target_url
    }

    /// Polls for completed network responses without blocking the UI thread.
    pub fn poll(&self) -> Option<FetchResult> {
        self.rx.try_recv().ok()
    }
}

/// Converts raw address bar input into either an internal `about:` URL,
/// a normalized `http(s)://` URL, or a DuckDuckGo HTML search query URL.
pub fn normalize_input_to_url(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "about:welcome".to_string();
    }

    if trimmed.starts_with("about:") {
        return trimmed.to_string();
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }

    // Treat as a domain/URL if it contains a dot, has no whitespace, and isn't just a trailing dot.
    let looks_like_domain = !trimmed.contains(char::is_whitespace)
        && trimmed.contains('.')
        && !trimmed.starts_with('.')
        && !trimmed.ends_with('.');

    if looks_like_domain {
        return format!("https://{trimmed}");
    }

    // Otherwise route as a search query to DuckDuckGo HTML edition.
    match Url::parse_with_params("https://html.duckduckgo.com/html/", &[("q", trimmed)]) {
        Ok(url) => url.to_string(),
        Err(_) => format!(
            "https://html.duckduckgo.com/html/?q={}",
            trimmed.replace(' ', "+")
        ),
    }
}

/// Resolves a potentially relative `href` against a base URL.
pub fn resolve_url(base_url: Option<&Url>, href: &str) -> Option<String> {
    let trimmed = href.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("javascript:")
        || trimmed.starts_with("mailto:")
        || trimmed.starts_with("tel:")
        || trimmed.starts_with("data:")
    {
        return None;
    }

    if trimmed.starts_with("about:") {
        return Some(trimmed.to_string());
    }

    if let Ok(abs) = Url::parse(trimmed) {
        return Some(abs.to_string());
    }

    if let Some(base) = base_url {
        if let Ok(joined) = base.join(trimmed) {
            return Some(joined.to_string());
        }
    }

    None
}

/// Checks if a URL belongs to a known ad or tracker domain.
pub fn is_blocked_url(raw_url: &str) -> bool {
    let Ok(parsed) = Url::parse(raw_url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host_lower = host.to_ascii_lowercase();
    BLOCKED_DOMAINS
        .iter()
        .any(|domain| host_lower == *domain || host_lower.ends_with(&format!(".{domain}")))
}

fn perform_request(target_url: &str) -> FetchResult {
    if target_url.starts_with("about:") {
        let html = build_internal_page(target_url);
        let bytes_received = html.len();
        return FetchResult::Success(FetchResponse {
            final_url: target_url.to_string(),
            status_text: "200 OK (Internal)".to_string(),
            html,
            bytes_received,
        });
    }

    if is_blocked_url(target_url) {
        return FetchResult::Error {
            url: target_url.to_string(),
            error: "Blocked by Klast URL-level ad/tracker filter.".to_string(),
        };
    }

    let client = match reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
    {
        Ok(c) => c,
        Err(err) => {
            return FetchResult::Error {
                url: target_url.to_string(),
                error: format!("Failed to initialize HTTP client: {err}"),
            };
        }
    };

    let response = match client
        .get(target_url)
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .send()
    {
        Ok(resp) => resp,
        Err(err) => {
            return FetchResult::Error {
                url: target_url.to_string(),
                error: format!("Network error: {err}"),
            };
        }
    };

    let status_code = response.status().as_u16();
    let status_text = format!(
        "{} {}",
        status_code,
        response.status().canonical_reason().unwrap_or("OK")
    );
    let final_url = response.url().to_string();

    match response.text() {
        Ok(body) => {
            let bytes_received = body.len();
            FetchResult::Success(FetchResponse {
                final_url,
                status_text,
                html: body,
                bytes_received,
            })
        }
        Err(err) => FetchResult::Error {
            url: target_url.to_string(),
            error: format!("Failed reading response body: {err}"),
        },
    }
}

pub fn build_internal_page(about_url: &str) -> String {
    match about_url {
        "about:welcome" | _ => r#"<!DOCTYPE html>
<html>
<head>
    <title>Welcome to Klast Browser</title>
</head>
<body>
    <main>
        <h1>Klast — Ultra-Lightweight Hybrid Web Browser</h1>
        <p>Welcome to <strong>Klast</strong>, a fast, non-bloated, text-first web browser built in Rust for low-resource systems and distraction-free reading.</p>
        <hr />
        <h2>Quick Links</h2>
        <ul>
            <li><a href="https://news.ycombinator.com">Hacker News (news.ycombinator.com)</a> — Tech &amp; startup links</li>
            <li><a href="https://lobste.rs">Lobsters (lobste.rs)</a> — Computing-focused community</li>
            <li><a href="https://en.wikipedia.org/wiki/Rust_(programming_language)">Wikipedia: Rust Programming Language</a></li>
            <li><a href="https://lite.cnn.com">CNN Lite (lite.cnn.com)</a> — Text-only news feed</li>
            <li><a href="https://html.duckduckgo.com/html/?q=rust+programming">DuckDuckGo HTML Search</a></li>
        </ul>
        <h2>Browser Controls &amp; Modes</h2>
        <ul>
            <li><strong>Address / Search Bar:</strong> Enter any URL or type keywords to search immediately via <code>DuckDuckGo HTML</code>.</li>
            <li><strong>[ 📖 Reader / Full DOM ]:</strong> Toggle between clean article extraction (<code>&lt;article&gt;</code> / <code>&lt;main&gt;</code>, stripping navigation clutter) and the full document body.</li>
            <li><strong>[ 🌗 Theme ]:</strong> Cycle between <em>High-Contrast Light</em>, <em>Dark</em>, and <em>Sepia / Paper</em> reading themes.</li>
            <li><strong>[ ⚡ WebKit Fallback ]:</strong> On-demand escalation trigger for JavaScript-heavy SPA sites (logs target URL in Phase 1).</li>
        </ul>
        <blockquote>"Simplicity is prerequisite for reliability." — Edsger W. Dijkstra</blockquote>
        <pre><code>Architecture: HTTP (reqwest) -> HTML AST (scraper) -> Semantic DomNode -> Native egui Widgets</code></pre>
    </main>
</body>
</html>"#
            .to_string(),
    }
}
