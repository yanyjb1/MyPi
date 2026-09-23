//! Per-site parsers — for pages whose useful content is NOT the readable
//! body (the aggressive preset strips it or the DOM-to-markdown default
//! mangles it). One function per site, dispatched by host suffix.
//!
//! Adding a site: write a `fn <site>(html: &str) -> Option<String>` that
//! returns the model-facing markdown (None = "not my page, use the
//! default conversion"), then register it in [`parse`]. That's the whole
//! interface — the tier engine and output shaping upstream are untouched.

use url::Url;

/// Host-suffix dispatch: first match wins. Keep the table tiny and
/// ordered most-specific-first.
pub(crate) fn parse(url: &str, html: &str) -> Option<anyhow::Result<String>> {
    let host = Url::parse(url).ok()?.host_str()?.to_string();
    for (suffix, parser) in TABLE {
        if host.ends_with(suffix) {
            return parser(html);
        }
    }
    None
}

type Parser = fn(&str) -> Option<anyhow::Result<String>>;

const TABLE: &[(&str, Parser)] = &[
    // (host suffix, parser) — register new sites here.
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_dispatch_matches_suffix_and_misses_cleanly() {
        // No registered parsers yet: every URL falls through to None.
        assert!(parse("https://example.com/a", "<html></html>").is_none());
        assert!(parse("not a url", "<html></html>").is_none());
    }

    // Site parsers register their own FIXTURE tests here, mirroring
    // search/providers/bing.rs.
}
