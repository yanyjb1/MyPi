// The CP language: one domain over both halves — C is the half without types and namespaces, C++ the half with them — with one sorter for both.

use crate::server::agent::tools::fs::lang::Language;

mod check;
mod facts;
mod format;
mod relations;
mod scan;
mod table;

// What this language owes the registry: four project markers, one sorter reading C++'s grammar, the `::` relation index, the file-per-unit graph, and the two clang tools. It has no tests: CP carries none of its own.
pub(crate) static LANGUAGE: Language = Language {
    markers: &[
        "compile_commands.json",
        "CMakeLists.txt",
        "Makefile",
        "meson.build",
    ],
    suffixes: table::SUFFIXES,
    table: table::table,
    shape: table::shape,
    relations: Some(&relations::RELATIONS),
    hub: Some(&facts::HUB),
    source: true,
    format: Some(format::format),
    check: Some(check::CHECKER),
    test: None,
};
