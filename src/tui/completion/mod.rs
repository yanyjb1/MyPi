//! Completion domain — everything that suggests text in the input box.
//!
//! - `engine.rs`: the candidate engines — path/file completion, the slash
//!   command table, argument completion, and the shared types
//!   ([`Completion`], [`CompletionAction`], [`CompletionPopup`]);
//! - `controller.rs`: the input zone's completion controller (popup state
//!   + the word memo + model-argument wiring);
//! - `popup.rs`: the reserved-area rendering of the popup.
//!
//! Leaf-state rules live in `crate::tui::leaf` (pure); this domain wires
//! them to the editor text and the runtime config.

pub mod controller;
pub mod engine;
pub mod popup;

pub use controller::{CompletionController, InputCtx, ModelCandidate};
pub use engine::{Completion, CompletionAction, CompletionPopup};
