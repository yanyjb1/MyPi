//! The product layer: a block in, segments or rows out, kept by block.

pub(crate) mod card;
pub(crate) mod compact;
pub(crate) mod fold;
pub(crate) mod natural;
mod notice;
mod opts;
pub(crate) mod pin;
mod products;
pub(crate) mod segment;
pub(crate) mod table;
mod text;
mod todo;
mod user;
mod wake;

pub(crate) use notice::rows as notice_rows;
pub(crate) use opts::Opts;
pub(crate) use pin::{esc_row, rows as pin_rows};
pub(crate) use products::{Phase, Product, Shape, of};
pub(crate) use todo::{Task, TodoPanel};
