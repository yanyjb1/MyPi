// The sorter registry: a suffix names a language, and a language answers one line table.

pub(crate) mod graph;
mod guages;
mod registry;
mod table;
pub(crate) mod verdict;

pub(crate) use registry::{
    Checker, Format, Hub, Language, Relation, Relations, Row, Source, Tests, checker, files,
    format, hub, layout, owner, shape, suffixes, table, tests, trees,
};
pub(crate) use table::{Line, Table};
