//! The three base zones + modal zones, wired to real app state.
//!
//! Each zone struct owns its own slice of state and implements [`Zone`]:
//! `wants` declares what it accepts (the readable routing table),
//! `handle` mutates only its own state and reports cross-zone effects
//! via [`Cascade`]. The app applies cascades; zones never touch zones.

use crate::tui::keys::Action;
use crate::tui::zones::{Cascade, Zone, ZoneId};

// ---------------------------------------------------------------------------
// History zone — chat scrollback. Mouse wheel: yes. Cursor keys: no.
// ---------------------------------------------------------------------------

/// Owns everything that only affects how the transcript renders.
pub struct HistoryState {
    /// Scroll-follow: false once scrolled off the bottom, true when back at it.
    /// Starts true: the default view follows the bottom.
    pub scroll_pinned: bool,
    /// History viewport offset (when unpinned; 0 = bottom).
    pub chat_scroll: usize,
    /// Global reasoning fold (Ctrl+T). false = expanded by default.
    pub reasoning_folded: bool,
    /// Global tool-output expansion (Ctrl+O). false = folded per-tool thresholds.
    pub tools_expanded: bool,
}

impl Default for HistoryState {
    fn default() -> Self {
        Self {
            scroll_pinned: true,
            chat_scroll: 0,
            reasoning_folded: false,
            tools_expanded: false,
        }
    }
}

impl Zone for HistoryState {
    fn wants(&self, action: &Action, _modal_active: bool) -> bool {
        matches!(action, Action::ToggleReasoning | Action::ToggleTools)
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

/// Mouse wheel policy, decided by hit-testing the row against the frame
/// layout. `chat_h` is the history area height (rows 0..chat_h); the
/// input container and reserved strip ignore the wheel entirely.
pub fn wheel_zone(row: u16, chat_h: u16, modal_active: bool) -> Option<ZoneId> {
    if modal_active {
        return Some(ZoneId::History); // modal takes the wheel as list scrolling
    }
    if row < chat_h {
        Some(ZoneId::History)
    } else {
        None
    }
}

/// Apply one wheel step to the history zone. Up unpin; reaching 0 re-pins.
pub fn wheel_step(h: &mut HistoryState, up: bool, amount: u16) {
    let n = amount as usize;
    if up {
        h.scroll_pinned = false;
        h.chat_scroll = h.chat_scroll.saturating_add(n);
    } else if h.chat_scroll > 0 {
        h.chat_scroll = h.chat_scroll.saturating_sub(n);
    }
    if h.chat_scroll == 0 {
        h.scroll_pinned = true;
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

        assert_eq!(
            h.handle(Action::ToggleReasoning),
            vec![Cascade::LayoutDirty]
        );
        assert!(h.reasoning_folded);
        assert_eq!(h.handle(Action::ToggleTools), vec![Cascade::LayoutDirty]);
        assert!(h.tools_expanded);
    }

    #[test]
    fn wheel_routes_by_row_and_modal() {
        // History area rows scroll; container/reserved rows do not.
        assert_eq!(wheel_zone(0, 10, false), Some(ZoneId::History));
        assert_eq!(wheel_zone(9, 10, false), Some(ZoneId::History));
        assert_eq!(wheel_zone(10, 10, false), None);
        assert_eq!(wheel_zone(20, 10, false), None);
        // A modal covers the whole screen.
        assert_eq!(wheel_zone(15, 10, true), Some(ZoneId::History));
    }

    #[test]
    fn wheel_step_unpins_and_repins() {
        let mut h = HistoryState::default();
        assert!(h.scroll_pinned);
        wheel_step(&mut h, true, 3);
        assert_eq!(h.chat_scroll, 3);
        assert!(!h.scroll_pinned);
        wheel_step(&mut h, true, 3);
        assert_eq!(h.chat_scroll, 6);
        wheel_step(&mut h, false, 3);
        assert_eq!(h.chat_scroll, 3);
        wheel_step(&mut h, false, 10);
        assert_eq!(h.chat_scroll, 0);
        assert!(h.scroll_pinned);
        // Down at 0 stays pinned, no underflow.
        wheel_step(&mut h, false, 3);
        assert_eq!(h.chat_scroll, 0);
        assert!(h.scroll_pinned);
    }
}
