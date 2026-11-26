// The Go language: its sorter, its own scan, its relations, its module graph, its parser, its checker and its tests.

use crate::server::agent::tools::fs::lang::Language;

mod check;
mod facts;
mod format;
mod layout;
mod relations;
mod scan;
mod table;
mod test;

pub(crate) static LANGUAGE: Language = Language {
    markers: &["go.mod"],
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
