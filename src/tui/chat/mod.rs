//! Session-level chat features that outgrew app.rs: the slash-command
//! table and the conversation-tree navigation. Both files hold `impl App`
//! blocks (they mutate session/editor state directly); Rust allows the
//! inherent impl to live in a sibling module, so no trait indirection is
//! needed and the methods read exactly as before.

mod commands;
mod tree;
