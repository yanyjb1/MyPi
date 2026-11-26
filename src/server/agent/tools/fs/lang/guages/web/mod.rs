// The web: one language over its four suffixes — a page, a stylesheet, a script — so that a query can cross all of them.

use std::path::Path;

use crate::server::agent::tools::fs::lang::{Language, Line, Table};

mod check;
mod css;
mod facts;
mod format;
mod html;
mod js;
mod layout;
mod relations;
mod view;

// The suffixes this language answers to.
pub(crate) const SUFFIXES: &[&str] = &["html", "htm", "css", "js"];

// What this language owes the registry: the suffixes it answers to, its sorter, and the tools it drives. It names no project marker: a page's own directory is the only root it needs, so a sweep of the workspace never stops at one.
pub(crate) static LANGUAGE: Language = Language {
    markers: &[],
    suffixes: SUFFIXES,
    table,
    shape,
    relations: Some(&relations::RELATIONS),
    hub: Some(&facts::HUB),
    source: true,
    format: Some(format::format),
    layout: Some(layout::layout),
    check: Some(check::CHECKER),
    test: None,
};

// One file's line table, by the kind its own suffix names.
pub(crate) fn table(path: &Path, text: &str) -> Table {
    match kind(path) {
        Some("css") => css::table(text),
        Some("js") => js::table(text),
        _ => html::table(text),
    }
}

// The kind of piece this line opens, in the file's own words.
pub(crate) fn shape(line: &Line) -> Option<&str> {
    line.label.as_deref()
}

// Which of the three kinds this path is: `css`, `js`, or a page.
pub(super) fn kind(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "css" => Some("css"),
        "js" => Some("js"),
        "html" | "htm" => Some("html"),
        _ => None,
    }
}
