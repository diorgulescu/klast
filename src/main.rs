// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Klast Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

mod adblock_filter;
mod app;
mod dom;
mod net;
mod storage;
mod webview_fallback;

use app::KlastApp;
use eframe::egui;

const GPL_NOTICE: &str = "\
Klast 0.4.0 — Ultra-Lightweight Hybrid Web Browser
Copyright (C) 2026 Klast Contributors
License GPLv3+: GNU GPL version 3 or later <https://gnu.org/licenses/gpl.html>
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law.";

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args
        .iter()
        .any(|a| a == "--version" || a == "-V" || a == "--license")
    {
        println!("{GPL_NOTICE}");
        return Ok(());
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{GPL_NOTICE}\n");
        println!("Usage:");
        println!("  klast                      Launch Klast in primary native GUI mode");
        println!("  klast --webview <URL>      Launch URL in isolated WebView fallback subprocess");
        println!("  klast --version            Print version and GNU GPLv3 license notice");
        return Ok(());
    }

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
