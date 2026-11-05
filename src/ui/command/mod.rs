//! The command contract: the shapes a front end declares and the two lookups over its table.

mod meta;

pub use meta::{Arg, Candidate, CommandMeta, lookup, path_shaped, split, valid, word, word_index};
