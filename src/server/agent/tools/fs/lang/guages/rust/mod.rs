// The Rust language: its sorter, its own scan, its parser and its checker.

use crate::server::agent::tools::fs::lang::Language;

mod check;
mod facts;
mod format;
mod layout;
mod relations;
mod scan;
mod table;
mod test;
mod view;

pub(crate) static LANGUAGE: Language = Language {
    markers: &["Cargo.toml"],
    suffixes: table::SUFFIXES,
    table: table::table,
    shape: table::shape,
    relations: Some(&relations::RELATIONS),
    hub: Some(&facts::HUB),
    source: true,
    format: Some(format::format),
    layout: Some(layout::layout),
    check: Some(check::CHECKER),
    test: Some(test::TESTS),
};
