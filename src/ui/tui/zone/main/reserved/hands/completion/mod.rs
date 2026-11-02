//! Completion: candidates as a tree, one level asked at a time.
//!
//! - `engine.rs`: which level the cursor is on, and that level's candidates —
//!   command names, argument names, or one directory level;
//! - `controller.rs`: the popup state plus the completion hand itself;
//! - `popup.rs`: the reserved area's rendering of the popup.

pub(in crate::ui::tui::zone::main::reserved) mod controller;
pub(in crate::ui::tui::zone::main::reserved) mod engine;
pub(in crate::ui::tui::zone::main::reserved) mod popup;

pub(in crate::ui::tui::zone::main::reserved) use controller::CompletionService;
