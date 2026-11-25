// The Python language: its sorter, its own scan, its relations, its module graph, its parser, its checker and its tests.

use crate::server::agent::tools::fs::lang::Language;

mod check;
mod facts;
mod format;
mod relations;
mod scan;
mod table;
mod test;
mod view;

pub(crate) static LANGUAGE: Language = Language {
    markers: &["pyproject.toml", "setup.cfg", "setup.py"],
    suffixes: table::SUFFIXES,
    table: table::table,
    shape: table::shape,
    relations: Some(&relations::RELATIONS),
    hub: Some(&facts::HUB),
    source: true,
    format: Some(format::format),
    check: Some(check::CHECKER),
    test: Some(test::TESTS),
};
