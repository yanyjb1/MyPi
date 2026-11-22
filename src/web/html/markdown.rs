// Converts a page's HTML to Markdown, and answers whether what came back is a script shell.

use anyhow::Context as _;
use html_to_markdown_rs::convert;
use html_to_markdown_rs::options::{ConversionOptions, PreprocessingOptions, PreprocessingPreset};

use crate::web::html::tags::{drop_svg, fold_controls};

// The tags kept as HTML where they sit, so a control stays visible under its own name.
const KEEP: &[&str] = &["button", "input", "select", "textarea", "summary"];

pub(crate) fn html_to_markdown(html: &str) -> anyhow::Result<String> {
    let options = ConversionOptions {
        extract_metadata: false,
        preserve_tags: KEEP.iter().map(|tag| (*tag).to_string()).collect(),
        preprocessing: PreprocessingOptions {
            enabled: true,
            preset: PreprocessingPreset::Aggressive,
            remove_forms: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let raw = fold_controls(&drop_svg(html));
    let result = convert(&raw, Some(options)).context("html-to-markdown conversion")?;
    let markdown = strip_inline_images(&result.content.unwrap_or_default());
    Ok(collapse_blank(&markdown))
}

// A page not painted yet answers an empty body with a large document: its bytes sit in the scripts.
pub(crate) fn is_js_shell(raw_len: usize, converted: &str) -> bool {
    converted.trim().len() < 200 && raw_len > 10_000
}

// An inline `data:` image is a payload no reader can read, so only its alt text stays.
fn strip_inline_images(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut at = 0;
    while let Some(open) = markdown[at..].find("![").map(|n| at + n) {
        let Some(sep) = markdown[open + 2..].find("](").map(|n| open + 2 + n) else {
            break;
        };
        let dest = sep + 2;
        let Some(close) = markdown[dest..].find(')').map(|n| dest + n) else {
            break;
        };
        if !markdown[dest..close].starts_with("data:") {
            out.push_str(&markdown[at..dest]);
            at = dest;
            continue;
        }
        out.push_str(&markdown[at..open]);
        let label = &markdown[open + 2..sep];
        if !label.trim().is_empty() {
            out.push_str("![");
            out.push_str(label);
            out.push(']');
        }
        at = close + 1;
    }
    out.push_str(&markdown[at..]);
    out
}

// A page's gaps are line breaks, not a wall of them: no run of blank lines survives twice.
fn collapse_blank(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut blank = false;
    for line in markdown.split('\n') {
        if line.trim().is_empty() {
            if blank {
                continue;
            }
            blank = true;
        } else {
            blank = false;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}
