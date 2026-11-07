//! The product layer: a block in, segments or rows out, kept by block.

pub(crate) mod card;
pub(crate) mod fold;
pub(crate) mod natural;
mod notice;
mod opts;
mod products;
pub(crate) mod segment;
pub(crate) mod table;
mod text;
mod todo;
mod user;

pub(crate) use notice::rows as notice_rows;
pub(crate) use opts::Opts;
pub(crate) use products::{Product, Products, of};
pub(crate) use segment::Segment;
pub(crate) use todo::{Task, TodoPanel};
