// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Klast Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

use adblock::engine::Engine;
use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;

const CORE_FILTER_RULES: &[&str] = &[
    "||doubleclick.net^",
    "||googlesyndication.com^",
    "||google-analytics.com^",
    "||googletagmanager.com^",
    "||adservice.google.com^",
    "||googleadservices.com^",
    "||connect.facebook.net^",
    "||pixel.facebook.com^",
    "||analytics.twitter.com^",
    "||ads-twitter.com^",
    "||scorecardresearch.com^",
    "||zedo.com^",
    "||adnxs.com^",
    "||criteo.com^",
    "||criteo.net^",
    "||outbrain.com^",
    "||taboola.com^",
    "||amazon-adsystem.com^",
    "||moatads.com^",
    "||quantserve.com^",
    "||rubiconproject.com^",
    "||pubmatic.com^",
    "||openx.net^",
    "||casalemedia.com^",
    "||advertising.com^",
    "||hotjar.com^",
    "||mixpanel.com^",
    "||segment.io^",
    "||sentry.io^",
    "||newrelic.com^",
    "||fonts.googleapis.com^",
    "||fonts.gstatic.com^",
    "*.woff2^",
    "##.ad-container",
    "##.advertisement",
    "##.cookie-banner",
    "##.gdpr-consent",
    "##.newsletter-signup",
];

const COSMETIC_BLOCK_PATTERNS: &[&str] = &[
    "cookie",
    "gdpr",
    "consent",
    "advert",
    "promo",
    "newsletter",
    "popup",
    "modal",
];

pub struct AdblockFilter {
    engine: Engine,
}

impl Default for AdblockFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl AdblockFilter {
    pub fn new() -> Self {
        let mut filter_set = FilterSet::new(false);
        filter_set.add_filter_list(CORE_FILTER_RULES.join("\n"), ParseOptions::default());
        let engine = Engine::new_with_filter_set(filter_set);
        Self { engine }
    }

    /// Checks whether a network or link URL should be blocked given the page's source URL.
    pub fn should_block_url(&self, target_url: &str, source_url: &str, request_type: &str) -> bool {
        let lower_target = target_url.to_ascii_lowercase();
        let path_part = lower_target.split('?').next().unwrap_or(&lower_target);
        if path_part.ends_with(".woff2") {
            return true;
        }

        let effective_source =
            if source_url.starts_with("http://") || source_url.starts_with("https://") {
                source_url
            } else {
                target_url
            };

        if let Ok(req) = Request::new(target_url, effective_source, request_type, "GET") {
            if self.engine.check_network_request(&req).should_block() {
                return true;
            }
        }
        false
    }

    /// Checks whether an HTML container (`div`, `section`, `aside`, etc.) should be skipped
    /// based on its `id` or `class` matching common ad, banner, or cookie-consent patterns.
    pub fn should_block_element(&self, tag: &str, id: Option<&str>, class: Option<&str>) -> bool {
        if !matches!(
            tag,
            "div" | "section" | "aside" | "header" | "footer" | "dialog"
        ) {
            return false;
        }

        if let Some(id_val) = id {
            if matches_cosmetic_pattern(id_val) {
                return true;
            }
        }

        if let Some(class_val) = class {
            if matches_cosmetic_pattern(class_val) {
                return true;
            }
        }

        false
    }
}

fn matches_cosmetic_pattern(attr_value: &str) -> bool {
    let lower = attr_value.to_ascii_lowercase();
    COSMETIC_BLOCK_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adblock_url_and_cosmetic_matching() {
        let filter = AdblockFilter::new();
        assert!(filter.should_block_url(
            "https://ads.doubleclick.net/pagead/ads",
            "https://example.com",
            "script"
        ));
        assert!(filter.should_block_url(
            "https://cdn.example.com/fonts/inter.woff2?v=4",
            "https://example.com",
            "font"
        ));
        assert!(!filter.should_block_url(
            "https://news.ycombinator.com/news",
            "https://news.ycombinator.com",
            "document"
        ));

        assert!(filter.should_block_element("div", Some("cookie-consent-banner"), None));
        assert!(filter.should_block_element("section", None, Some("sidebar promo-box")));
        assert!(filter.should_block_element("aside", None, Some("newsletter-widget")));
        assert!(filter.should_block_element("div", Some("gdpr_modal"), None));
        assert!(!filter.should_block_element("div", Some("main-content"), Some("article-body")));
        assert!(!filter.should_block_element("p", Some("cookie-mention"), None));
    }
}
