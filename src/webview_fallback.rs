// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Klast Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

pub const LOW_SPEC_INIT_SCRIPT: &str = r#"
(function() {
    function applyLowSpecOptimizations() {
        if (!document.getElementById('klast-low-spec-style')) {
            const style = document.createElement('style');
            style.id = 'klast-low-spec-style';
            style.textContent = `
                *, *::before, *::after {
                    animation-duration: 0.01ms !important;
                    animation-iteration-count: 1 !important;
                    transition-duration: 0.01ms !important;
                    scroll-behavior: auto !important;
                }
                html, body, p, div, span, a, li, td, th, input, button, h1, h2, h3, h4, h5, h6 {
                    font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif !important;
                }
                pre, code, kbd, samp, tt {
                    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace !important;
                }
                #klast-return-btn {
                    position: fixed !important;
                    top: 8px !important;
                    right: 12px !important;
                    z-index: 2147483647 !important;
                    background: #141619 !important;
                    color: #e2e6eb !important;
                    border: 1px solid #5096e6 !important;
                    padding: 4px 10px !important;
                    font-family: ui-monospace, monospace !important;
                    font-size: 12px !important;
                    cursor: pointer !important;
                    box-shadow: none !important;
                    border-radius: 0 !important;
                }
                #klast-return-btn:hover {
                    background: #22262c !important;
                    color: #5fafff !important;
                }
            `;
            (document.head || document.documentElement).appendChild(style);
        }

        if (document.body && !document.getElementById('klast-return-btn')) {
            const btn = document.createElement('button');
            btn.id = 'klast-return-btn';
            btn.textContent = '[ ✕ Return to Klast Native ]';
            btn.title = 'Close WebKit Fallback (Shortcut: Esc or Alt+Q)';
            btn.addEventListener('click', function(e) {
                e.preventDefault();
                e.stopPropagation();
                if (window.ipc && typeof window.ipc.postMessage === 'function') {
                    window.ipc.postMessage('close_webview');
                } else {
                    window.close();
                }
            });
            document.body.appendChild(btn);
        }

        const videos = document.querySelectorAll('video');
        for (let i = 0; i < videos.length; i++) {
            const v = videos[i];
            v.autoplay = false;
            try { v.pause(); } catch (_) {}
        }
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', applyLowSpecOptimizations);
    } else {
        applyLowSpecOptimizations();
    }

    window.addEventListener('load', applyLowSpecOptimizations);

    window.addEventListener('keydown', function(e) {
        if (e.key === 'Escape' || (e.altKey && (e.key === 'q' || e.key === 'Q'))) {
            if (window.ipc && typeof window.ipc.postMessage === 'function') {
                window.ipc.postMessage('close_webview');
            } else {
                window.close();
            }
        }
    }, true);
})();
"#;

#[cfg(all(
    feature = "system-webview",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
mod native_webview {
    use super::LOW_SPEC_INIT_SCRIPT;
    use crate::adblock_filter::AdblockFilter;
    use tao::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder},
        window::WindowBuilder,
    };
    use wry::WebViewBuilder;

    #[derive(Debug, Clone, Copy)]
    enum FallbackEvent {
        CloseRequested,
    }

    pub fn run_webview_impl(target_url: &str) -> Result<(), Box<dyn std::error::Error>> {
        let event_loop = EventLoopBuilder::<FallbackEvent>::with_user_event().build();
        let proxy = event_loop.create_proxy();

        let window = WindowBuilder::new()
            .with_title(format!("Klast [WebKit Fallback Mode] - {target_url}"))
            .with_inner_size(LogicalSize::new(1080.0, 740.0))
            .build(&event_loop)?;

        let adblock = AdblockFilter::new();
        let source_url = target_url.to_string();

        let _webview = WebViewBuilder::new()
            .with_url(target_url)
            .with_initialization_script(LOW_SPEC_INIT_SCRIPT)
            .with_ipc_handler(move |req| {
                if req.body() == "close_webview" {
                    let _ = proxy.send_event(FallbackEvent::CloseRequested);
                }
            })
            .with_navigation_handler(move |nav_url: String| {
                let blocked = adblock.should_block_url(&nav_url, &source_url, "document");
                !blocked
            })
            .build(&window)?;

        event_loop.run(move |event, _, control_flow| {
            *control_flow = ControlFlow::Wait;

            match event {
                Event::UserEvent(FallbackEvent::CloseRequested) => {
                    *control_flow = ControlFlow::Exit;
                }
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => {
                    *control_flow = ControlFlow::Exit;
                }
                _ => {}
            }
        });
    }
}

/// Runs the isolated system WebView fallback process for a single target URL.
/// On platforms/targets built without `system-webview` (such as FreeBSD, NetBSD, OpenBSD,
/// static musl Linux, RISC-V, ARMv5/v7, or legacy Windows XP/7), gracefully delegates
/// to the system's external browser launcher.
pub fn run_webview(target_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(all(
        feature = "system-webview",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    {
        native_webview::run_webview_impl(target_url)
    }

    #[cfg(not(all(
        feature = "system-webview",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    )))]
    {
        let _ = LOW_SPEC_INIT_SCRIPT;
        launch_external_browser_fallback(target_url)
    }
}

#[cfg(not(all(
    feature = "system-webview",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
)))]
fn launch_external_browser_fallback(target_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    use std::process::Command;

    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", target_url])
            .spawn()?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(target_url).spawn()?;
        Ok(())
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        for candidate in [
            "xdg-open", "dillo", "netsurf", "falkon", "firefox", "chromium",
        ] {
            if Command::new(candidate).arg(target_url).spawn().is_ok() {
                return Ok(());
            }
        }
        Err(format!(
            "System WebView is not compiled in this build target; could not launch external browser for {target_url}"
        )
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_low_spec_init_script_contains_optimizations() {
        assert!(LOW_SPEC_INIT_SCRIPT.contains("animation-duration: 0.01ms !important"));
        assert!(LOW_SPEC_INIT_SCRIPT.contains("transition-duration: 0.01ms !important"));
        assert!(LOW_SPEC_INIT_SCRIPT.contains("klast-return-btn"));
        assert!(LOW_SPEC_INIT_SCRIPT.contains("close_webview"));
    }
}
