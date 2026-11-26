// The R language: its sorter, its own scan, its relations, its parser and its tests.

use crate::server::agent::tools::fs::lang::Language;

mod format;
mod relations;
mod scan;
mod table;
mod test;

pub(crate) static LANGUAGE: Language = Language {
    // R names a package by `DESCRIPTION` alone, and the marker registry claims a directory only for a language with a graph; this one has none, so it claims nothing.
    markers: &[],
    suffixes: table::SUFFIXES,
    table: table::table,
    shape: table::shape,
    relations: Some(&relations::RELATIONS),
    hub: None,
    source: true,
    format: Some(format::format),
    check: None,
    test: Some(test::TESTS),
};
