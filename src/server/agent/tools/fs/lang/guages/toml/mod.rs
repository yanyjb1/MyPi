// The TOML language: its sorter and its parser. A document has no graph, no checker and no layout.

use crate::server::agent::tools::fs::lang::Language;

mod format;
mod scan;
mod table;

// The suffixes this sorter answers to.
pub(crate) const SUFFIXES: &[&str] = &["toml"];

// What this language owes the registry: its suffix, its table and its parser. TOML names no project marker and no unit, and a commit leaves it as the model wrote it.
pub(crate) static LANGUAGE: Language = Language {
    markers: &[],
    suffixes: SUFFIXES,
    table: table::table,
    shape: table::shape,
    relations: None,
    hub: None,
    source: true,
    format: Some(format::format),
    layout: None,
    check: None,
    test: None,
};
