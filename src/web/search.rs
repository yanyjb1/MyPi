//! Search tool — engine chain entry: plain terms → Bing direct,
//! advanced syntax → DDG in the browser.

use anyhow::anyhow;
use serde::Deserialize;

use super::{bing, ddg};

pub(super) const DEFAULT_LIMIT: usize = 8;
pub(super) const MAX_LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub snippet: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchArgs {
    /// 一句话说明这次调用要干什么，中文，会显示给用户看
    pub intent: String,
    /// 搜索词，支持 site: -词 "短语" filetype: inurl: intitle: OR 等高级语法
    pub query: String,
    /// 返回条数，默认 8，上限 20
    pub limit: Option<usize>,
}

/// Parse arguments exactly as the model sent them (already JSON string).
pub fn parse_search_args(arguments: &str) -> anyhow::Result<SearchArgs> {
    serde_json::from_str(arguments).map_err(|e| anyhow!("bad search args: {e}"))
}

// Query assembly for the **direct tier**. Bing parses Google-style operators
// natively (site:, quotes, negation, OR, filetype:, inurl:, intitle:) —
// **except from a CN egress IP**, where www.bing.com 302s into cn.bing.com
// no matter which escape-hatch parameters or cookies ride along (verified:
// ensearch/mkt/setmkt/cc/SRCHHPGUSR all ignored) and cn.bing silently drops
// every operator. The direct tier therefore serves plain-keyword queries
// only; syntax-bearing queries route to the DDG browser tier, which honors
// them all.
fn has_advanced_syntax(query: &str) -> bool {
    // Quoted phrase or negation: unambiguous operators on every engine.
    if query.contains('"') {
        return true;
    }
    // Operator whitelist — exactly the tokens DDG (and Google-style engines)
    // define. Deliberately NOT "any word followed by :": a plain-words query
    // like `hello:world` would otherwise be forced onto the slow browser
    // tier for no benefit.
    const OPERATORS: [&str; 8] =
        ["site", "inurl", "intitle", "intext", "filetype", "ext", "before", "after"];
    for token in query.split_whitespace() {
        if token.starts_with('-')
            && token.len() > 1
            && token[1..].contains(|c: char| c.is_ascii_alphanumeric())
        {
            return true; // negation: -tokio
        }
        let Some((head, tail)) = token.split_once(':') else { continue };
        if OPERATORS.contains(&head) && !tail.is_empty() {
            return true;
        }
    }
    false
}

pub fn search(args: &SearchArgs) -> anyhow::Result<Vec<SearchHit>> {
    let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    // Syntax-bearing queries bypass Bing entirely (CN egress drops every
    // operator — verified); plain keywords take the fast direct tier first.
    if !has_advanced_syntax(&args.query) {
        // Direct-tier failures (bot wall, network) fall through to DDG.
        if let Ok(hits) = bing::direct(&args.query, limit)
            && !hits.is_empty()
        {
            return Ok(hits);
        }
    }
    ddg::via_browser(&args.query, limit)
}

/// Tool-facing text rendering: numbered rows, snippet on its own line. The
/// engine name is deliberately absent — the model doesn't need it and it
/// would leak into answers.
pub fn render(hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return "no results".to_string();
    }
    let mut out = String::new();
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!("{}. {}\n   {}\n", i + 1, h.title, h.url));
        if !h.snippet.is_empty() {
            out.push_str(&format!("   {}\n", h.snippet));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;



    #[test]
    fn render_is_numbered_and_has_no_engine_name() {
        let hits = vec![SearchHit {
            title: "T".into(),
            url: "https://x/".into(),
            snippet: "s".into(),
        }];
        let out = render(&hits);
        assert!(out.starts_with("1. T\n"));
        assert!(!out.to_lowercase().contains("bing"));
    }

    #[test]
    fn syntax_detection_routes_by_operator() {
        // Operators Bing-on-CN-egress drops:
        assert!(has_advanced_syntax("site:github.com rust"));
        assert!(has_advanced_syntax("rust -tokio"));
        assert!(has_advanced_syntax("\"async runtime\""));
        assert!(has_advanced_syntax("filetype:pdf rust book"));
        // A colon inside a URL-ish word is NOT an operator (no operator head).
        assert!(!has_advanced_syntax("rust async book"));
        assert!(!has_advanced_syntax("hello:world-with-dashes"));
        // Lone dash or colon is punctuation, not an operator.
        assert!(!has_advanced_syntax("rust - "));
        assert!(!has_advanced_syntax("rust :"));
    }

}
