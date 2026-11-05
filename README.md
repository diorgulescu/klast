# Klast — Ultra-Lightweight Hybrid Web Browser

**Klast** is a fast, non-bloated, text-first hybrid web browser written in Rust for older hardware, retro machines, SBCs (ARM, RISC-V), BSD workstations, and low-resource systems.

Licensed under the **GNU General Public License v3.0 or later ([`GPL-3.0-or-later`](LICENSE))**.

---

## Architecture

Instead of embedding a multi-hundred-megabyte browser engine by default, Klast operates in two modes:

1. **Primary Mode (Native DOM-to-UI):**
   - Fetches HTML over non-blocking background threads (`reqwest` + `rustls`).
   - Blocks ad/tracker domains, `.woff2` web fonts, and cookie/GDPR/newsletter DOM containers via `adblock-rust` (`[ 🛡️ Blocked: X ]`).
   - Rewrites bloated URLs (`reddit.com` -> `old.reddit.com`, `en.wikipedia.org` -> `en.m.wikipedia.org`) and strips `utm_*`, `fbclid`, and `gclid` parameters (`[ 🔄 Smart Redirects ]`).
   - Translates HTML and plain-text Markdown into a lightweight semantic `DomNode` AST rendered directly as native `egui` widgets (headings, paragraphs, clickable inline links, interactive `GET` forms, code blocks, quotes, lists, and optional lazy images).
   - Stores bookmarks and offline reading list articles as human-readable Markdown files in `~/.config/klast/`.
2. **Fallback Mode (Subprocess-Isolated System WebView / External Browser):**
   - Triggered only on demand via `[ ⚡ WebKit Fallback ]` (`klast --webview <url>`).
   - Runs in an isolated OS child process so all WebView memory is immediately reclaimed by the OS kernel when closed (`Esc` or `Alt+Q`).

---

## Building Across Platforms & Architectures

### Standard Desktop Build (Linux glibc, macOS, Windows 10/11)
```sh
cargo build --release
```

### Ultra-Portable / Low-Spec Build (musl, FreeBSD, NetBSD, OpenBSD, ARMv5/v7, RISC-V, Windows XP/7)
Building with `--no-default-features` disables the optional `wry`/`tao` WebKit2GTK/WebView2 bindings, producing a pure `eframe`/`glow` binary that compiles cleanly on static `musl`, BSDs, embedded ARM/RISC-V, and legacy Windows:
```sh
cargo build --release --no-default-features
```

### Running on Older GPUs / Software OpenGL (Mesa `llvmpipe` / `softpipe`)
On vintage laptops or headless/low-end boards without hardware OpenGL 2.0+:
```sh
LIBGL_ALWAYS_SOFTWARE=1 klast
```

### Legacy Windows (Windows XP SP3 / Windows 7+)
Use the `i686-pc-windows-gnu` or `x86_64-pc-windows-gnu` target with `--no-default-features` (configured automatically in [`.github/workflows/release.yml`](.github/workflows/release.yml) with PE subsystem `5.01`). For running Rust binaries on unmodified Windows XP / early Windows 7 kernels missing newer `bcrypt`/`kernel32` imports, pair with [`rust7`](https://github.com/OneLoneCoder/rust7) or [`thunk-rs`](https://github.com/Chuyu-Team/thunk).

---

## License

Copyright (C) 2026 Klast Contributors.

This program is free software: you can redistribute it and/or modify it under the terms of the **GNU General Public License** as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. See [`LICENSE`](LICENSE) for full details.
