// The notebook language: its sorter, its parser and its checker.

use crate::server::agent::tools::fs::lang::Language;

mod check;
mod format;
mod scan;
mod table;

// What this language owes the registry: its suffixes, its table, its parser and its checker. It names no project marker and no graph: a notebook is located, never modelled, so a directory of notebooks is no tree of its own.
pub(crate) static LANGUAGE: Language = Language {
    markers: &[],
    suffixes: table::SUFFIXES,
    table: table::table,
    shape: table::shape,
    relations: None,
    hub: None,
    source: true,
    format: Some(format::format),
    check: Some(check::CHECKER),
    test: None,
};
