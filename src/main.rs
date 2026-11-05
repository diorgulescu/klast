mod adblock_filter;
mod app;
mod dom;
mod net;

use app::KlastApp;
use eframe::egui;

fn main() -> eframe::Result<()> {
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
