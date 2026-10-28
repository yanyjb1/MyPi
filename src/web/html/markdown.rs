use anyhow::Context as _;
use html_to_markdown_rs::convert;
use html_to_markdown_rs::options::{ConversionOptions, PreprocessingOptions, PreprocessingPreset};

pub(crate) fn html_to_markdown(html: &str) -> anyhow::Result<String> {
    let options = ConversionOptions {
        preprocessing: PreprocessingOptions {
            enabled: true,
            preset: PreprocessingPreset::Aggressive,
            ..Default::default()
        },
        ..Default::default()
    };
    let result = convert(html, Some(options)).context("html-to-markdown conversion")?;
    Ok(result.content.unwrap_or_default())
}

pub(crate) fn is_js_shell(raw_len: usize, converted: &str) -> bool {
    if converted.trim().len() < 200 && raw_len > 10_000 {
        return true;
    }
    let meta_lines = converted
        .lines()
        .filter(|l| l.starts_with("meta-") || l.starts_with("title:"))
        .count();
    let body_lines = converted
        .lines()
        .filter(|l| !l.starts_with("meta-") && !l.starts_with("title:") && !l.trim().is_empty())
        .count();
    meta_lines >= 10 && body_lines < 15
}
