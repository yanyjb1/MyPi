//! The history zone: the block rows, the streaming tail, the local section, the viewport.

mod cache;
mod live;
mod local;
mod render;
mod segment;
mod shape;
mod view;
pub(in crate::ui::tui::zone::main) mod zone;
