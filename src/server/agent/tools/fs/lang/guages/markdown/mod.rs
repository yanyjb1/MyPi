// The Markdown language: its sorter, its references, its note graph and its narration.

use crate::server::agent::tools::fs::lang::Language;

mod facts;
mod links;
mod table;
mod view;

// The suffixes this sorter answers to.
pub(crate) const SUFFIXES: &[&str] = &["md", "markdown"];

// What this language owes the registry: its suffixes, its table, and the note graph. It names no project marker — a directory holding notes is a tree of its own — and it has no parser, checker or tests: a document is never laid out by a tool.
pub(crate) static LANGUAGE: Language = Language {
    markers: &[],
    suffixes: SUFFIXES,
    table: table::table,
    shape: table::shape,
    relations: None,
    hub: Some(&facts::HUB),
    source: false,
    format: None,
    check: None,
    test: None,
};
