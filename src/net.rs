use crate::adblock_filter::AdblockFilter;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;
use url::Url;

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";

const TRACKING_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "fbclid",
    "gclid",
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
    ImageLoaded { url: String, rgba: Vec<u8>, width: usize, height: usize },
    ImageFailed { url: String, error: String },
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

    /// Triggers a non-blocking HTTP GET on a background thread, applying smart redirects if enabled.
    pub fn fetch(&self, raw_input: &str, smart_redirects: bool) -> String {
        let normalized = normalize_input_to_url(raw_input);
        let target_url = rewrite_url(&normalized, smart_redirects);
        let tx = self.tx.clone();
        let url_for_thread = target_url.clone();

        thread::spawn(move || {
            let result = perform_request(&url_for_thread, smart_redirects);
            let _ = tx.send(result);
        });

        target_url
    }

    /// Triggers a non-blocking image download and decode on a background thread.
    pub fn fetch_image(&self, image_url: &str) {
        let tx = self.tx.clone();
        let url_for_thread = image_url.to_string();

        thread::spawn(move || {
            let result = perform_image_request(&url_for_thread);
            let _ = tx.send(result);
        });
    }

    /// Polls for completed network responses without blocking the UI thread.
    pub fn poll(&self) -> Option<FetchResult> {
        self.rx.try_recv().ok()
    }
}

/// Applies privacy and low-bloat URL rewrites:
/// - `www.reddit.com` / `reddit.com` -> `old.reddit.com`
/// - `en.wikipedia.org` -> `en.m.wikipedia.org`
/// - Strips tracking query parameters (`utm_source`, `utm_medium`, `utm_campaign`, `fbclid`, `gclid`).
pub fn rewrite_url(raw_url: &str, smart_redirects: bool) -> String {
    if !smart_redirects || raw_url.starts_with("about:") {
        return raw_url.to_string();
    }

    let Ok(mut parsed) = Url::parse(raw_url) else {
        return raw_url.to_string();
    };

    if let Some(host) = parsed.host_str().map(|h| h.to_ascii_lowercase()) {
        match host.as_str() {
            "reddit.com" | "www.reddit.com" => {
                let _ = parsed.set_host(Some("old.reddit.com"));
            }
            "en.wikipedia.org" => {
                let _ = parsed.set_host(Some("en.m.wikipedia.org"));
            }
            _ => {}
        }
    }

    // Strip common tracking query parameters while keeping legitimate query parameters intact.
    let existing_pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    if !existing_pairs.is_empty() {
        let filtered: Vec<(String, String)> = existing_pairs
            .into_iter()
            .filter(|(k, _)| {
                let key_lower = k.to_ascii_lowercase();
                !TRACKING_PARAMS.contains(&key_lower.as_str())
            })
            .collect();

        if filtered.is_empty() {
            parsed.set_query(None);
        } else {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            for (k, v) in &filtered {
                serializer.append_pair(k, v);
            }
            let new_query = serializer.finish();
            parsed.set_query(Some(&new_query));
        }
    }

    parsed.to_string()
}

/// Constructs a GET form target URL by combining the resolved form `action` URL
/// with hidden fields and user-entered form values.
pub fn build_get_form_url(
    action_url: &str,
    field_names: &[(String, String)],
    user_values: &HashMap<String, String>,
) -> Option<String> {
    let mut parsed = Url::parse(action_url).ok()?;
    {
        let mut query_pairs = parsed.query_pairs_mut();
        query_pairs.clear();
        for (field_key, name) in field_names {
            if name.is_empty() {
                continue;
            }
            let val = user_values.get(field_key).map(String::as_str).unwrap_or("");
            query_pairs.append_pair(name, val);
        }
    }
    Some(parsed.to_string())
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

/// Resolves a potentially relative `href` or `action` or `src` against a base URL.
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

fn perform_request(target_url: &str, smart_redirects: bool) -> FetchResult {
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

    let filter = AdblockFilter::new();
    if filter.should_block_url(target_url, target_url, "document") {
        return FetchResult::Error {
            url: target_url.to_string(),
            error: "Blocked by Klast adblock-rust engine.".to_string(),
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
    let final_url = rewrite_url(response.url().as_str(), smart_redirects);

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

fn perform_image_request(image_url: &str) -> FetchResult {
    let client = match reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return FetchResult::ImageFailed {
                url: image_url.to_string(),
                error: e.to_string(),
            }
        }
    };

    let resp = match client.get(image_url).send() {
        Ok(r) => r,
        Err(e) => {
            return FetchResult::ImageFailed {
                url: image_url.to_string(),
                error: e.to_string(),
            }
        }
    };

    let bytes = match resp.bytes() {
        Ok(b) => b,
        Err(e) => {
            return FetchResult::ImageFailed {
                url: image_url.to_string(),
                error: e.to_string(),
            }
        }
    };

    match image::load_from_memory(&bytes) {
        Ok(decoded) => {
            let rgba_img = decoded.to_rgba8();
            let (w, h) = rgba_img.dimensions();
            FetchResult::ImageLoaded {
                url: image_url.to_string(),
                rgba: rgba_img.into_raw(),
                width: w as usize,
                height: h as usize,
            }
        }
        Err(e) => FetchResult::ImageFailed {
            url: image_url.to_string(),
            error: format!("Image decode error: {e}"),
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
        <form action="https://html.duckduckgo.com/html/" method="get">
            <input type="search" name="q" placeholder="Search the web via DuckDuckGo HTML..." value="" />
            <button type="submit">Search Web</button>
        </form>
        <hr />
        <h2>Quick Links</h2>
        <ul>
            <li><a href="https://news.ycombinator.com">Hacker News (news.ycombinator.com)</a> — Tech &amp; startup links</li>
            <li><a href="https://lobste.rs">Lobsters (lobste.rs)</a> — Computing-focused community</li>
            <li><a href="https://www.reddit.com/r/rust">Reddit /r/rust (auto-rewrites to old.reddit.com)</a></li>
            <li><a href="https://en.wikipedia.org/wiki/Rust_(programming_language)">Wikipedia: Rust (auto-rewrites to en.m.wikipedia.org)</a></li>
            <li><a href="https://html.duckduckgo.com/html/">DuckDuckGo HTML Search Home</a></li>
        </ul>
        <h2>Phase 2 Capabilities &amp; Controls</h2>
        <ul>
            <li><strong>Interactive Native Forms:</strong> Fill in search boxes and submit <code>GET</code> forms directly inside pages like DuckDuckGo HTML and Wikipedia.</li>
            <li><strong>[ 🔄 Smart Redirects ]:</strong> Automatically rewrites <code>reddit.com</code> to <code>old.reddit.com</code>, <code>en.wikipedia.org</code> to <code>en.m.wikipedia.org</code>, and strips <code>utm_*</code>, <code>fbclid</code>, and <code>gclid</code> tracking params.</li>
            <li><strong>[ 🛡️ Blocked: X ]:</strong> Powered by <code>adblock-rust</code> and cosmetic DOM filters blocking ad domains, cookie consent modals, and newsletter popups.</li>
            <li><strong>[ 🖼️ Images: OFF / ON ]:</strong> Defaults to <code>OFF</code> with compact <code>[IMG: alt]</code> badges; toggle <code>ON</code> for lazy image loading.</li>
        </ul>
        <blockquote>"Simplicity is prerequisite for reliability." — Edsger W. Dijkstra</blockquote>
        <pre><code>Architecture: HTTP (reqwest) -> Adblock &amp; Smart Redirects -> HTML AST (scraper) -> Native egui Widgets</code></pre>
    </main>
</body>
</html>"#
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smart_redirects_and_utm_stripping() {
        let reddit = rewrite_url(
            "https://www.reddit.com/r/rust?utm_source=share&utm_medium=web2x&sort=top",
            true,
        );
        assert_eq!(reddit, "https://old.reddit.com/r/rust?sort=top");

        let wiki = rewrite_url(
            "https://en.wikipedia.org/wiki/Rust_(programming_language)?fbclid=12345",
            true,
        );
        assert_eq!(
            wiki,
            "https://en.m.wikipedia.org/wiki/Rust_(programming_language)"
        );

        let disabled = rewrite_url("https://www.reddit.com/r/rust?utm_source=test", false);
        assert_eq!(disabled, "https://www.reddit.com/r/rust?utm_source=test");
    }
}
