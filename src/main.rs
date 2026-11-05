mod adblock_filter;
mod app;
mod dom;
mod net;
mod webview_fallback;

use app::KlastApp;
use eframe::egui;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|arg| arg == "--webview") {
        let target_url = args
            .get(pos + 1)
            .map(String::as_str)
            .unwrap_or("https://news.ycombinator.com");
        if let Err(err) = webview_fallback::run_webview(target_url) {
            eprintln!("Klast WebView fallback error: {err}");
        }
        return Ok(());
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Klast — Ultra-Lightweight Hybrid Web Browser")
            .with_inner_size([1120.0, 740.0])
            .with_min_inner_size([560.0, 340.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Klast",
        native_options,
        Box::new(|cc| Ok(Box::new(KlastApp::new(cc)))),
    )
}
