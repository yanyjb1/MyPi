//! The three base zones + modal zones, wired to real app state.
//!
//! Each zone struct owns its own slice of state and implements [`Zone`]:
//! `wants` declares what it accepts (the readable routing table),
//! `handle` mutates only its own state and reports cross-zone effects
//! via [`Cascade`]. The app applies cascades; zones never touch zones.

use crate::tui::keys::Action;
use crate::tui::zones::{Cascade, Zone};

// ---------------------------------------------------------------------------
// History zone — chat scrollback. Mouse wheel: yes. Cursor keys: no.
// ---------------------------------------------------------------------------

/// Owns everything that only affects how the transcript renders.
#[derive(Default)]
pub struct HistoryState {
    /// Scroll-follow: false once scrolled off the bottom, true when back at it.
    pub scroll_pinned: bool,
    /// History viewport offset (when unpinned; 0 = bottom).
    pub chat_scroll: usize,
    /// Global reasoning fold (Ctrl+T). false = expanded by default.
    pub reasoning_folded: bool,
    /// Global tool-output expansion (Ctrl+O). false = folded per-tool thresholds.
    pub tools_expanded: bool,
}

impl Zone for HistoryState {
    fn wants(&self, action: &Action, _modal_active: bool) -> bool {
        matches!(
            action,
            Action::ToggleReasoning | Action::ToggleTools
        )
    }

    fn handle(&mut self, action: Action) -> Vec<Cascade> {
        match action {
            Action::ToggleReasoning => {
                self.reasoning_folded = !self.reasoning_folded;
                vec![Cascade::LayoutDirty]
            }
            Action::ToggleTools => {
                self.tools_expanded = !self.tools_expanded;
                vec![Cascade::LayoutDirty]
            }
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_takes_only_its_toggles() {
        let mut h = HistoryState::default();
        assert!(h.wants(&Action::ToggleReasoning, false));
        assert!(h.wants(&Action::ToggleTools, false));
        assert!(!h.wants(&Action::Up, false));
        assert!(!h.wants(&Action::Insert('x'), false));

        assert_eq!(h.handle(Action::ToggleReasoning), vec![Cascade::LayoutDirty]);
        assert!(h.reasoning_folded);
        assert_eq!(h.handle(Action::ToggleTools), vec![Cascade::LayoutDirty]);
        assert!(h.tools_expanded);
    }
}
