//! Shared infrastructure for the web domain. Nothing here knows about the
//! tools above it.
//!
//! - [`cdp`] — the CDP protocol layer (browser process, WS session, Target).
//! - [`session`] — the one owner of attach-or-launch, rendezvous, tab pools.
//! - [`html`] — tag-level HTML parsing helpers shared by search providers.
//! - [`url`] — URL normalization and encoding helpers.

pub mod cdp;
pub(crate) mod html;
pub mod session;
pub mod url;
