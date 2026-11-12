//! DOM to Markdown, and the one question asked of the text it yields.

use anyhow::Context as _;
use html_to_markdown_rs::convert;
use html_to_markdown_rs::options::{ConversionOptions, PreprocessingOptions, PreprocessingPreset};

/// Tags emitted as HTML where they sit, so a control stays recognizable in the prose. The
/// selector that reaches it is in the operable section, not here.
const KEEP: &[&str] = &["button", "input", "textarea", "summary"];

pub(crate) fn html_to_markdown(html: &str) -> anyhow::Result<String> {
    let options = ConversionOptions {
        extract_metadata: false,
        preserve_tags: KEEP.iter().map(|tag| (*tag).to_string()).collect(),
        preprocessing: PreprocessingOptions {
            enabled: true,
            preset: PreprocessingPreset::Aggressive,
            ..Default::default()
        },
        ..Default::default()
    };
    let result = convert(html, Some(options)).context("html-to-markdown conversion")?;
    Ok(strip_inline_images(&result.content.unwrap_or_default()))
}

/// A page navigated to but not yet painted answers with an empty body and a large document:
/// the bytes sit in its scripts. Waiting is what keeps that shell out of the answer.
pub(crate) fn is_js_shell(raw_len: usize, converted: &str) -> bool {
    converted.trim().len() < 200 && raw_len > 10_000
}

/// An inline `data:` image is a payload no reader can read, and the inline icons and sprite
/// SVGs of a marketing page outweigh its prose many times over. The alt text is the part worth
/// keeping. An image naming a real address is left as it is.
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
