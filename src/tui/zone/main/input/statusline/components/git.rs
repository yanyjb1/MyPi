//! The git segment — `@ <branch>`, plus `?N` (needs `git add`) and `+N`
//! (staged).
//!
//! Update source: `Git`, carrying a snapshot the caller already took. The
//! caller decides *when* to look (turn end / tool result) and *where*
//! (`/cdp`'s workspace — a temporary `cd` never re-targets this segment, which
//! is why the component ignores `CwdMigrated` entirely). The counting rules
//! live in `crate::git`; this component only decides what is worth showing.
//!
//! Nothing to show = nothing rendered: outside a repository, or on a detached
//! HEAD with a clean tree, the segment (and its connector) disappears.

use ratatui::text::Span;

use crate::git::GitStatus;
use crate::tui::zone::main::input::statusline::theme::{ColorPolicy, StatusTheme, Token};
use crate::tui::zone::main::input::statusline::{Side, StatusComponent, StatusEvent};

#[derive(Debug, Default)]
pub struct Git {
    status: Option<GitStatus>,
}

impl StatusComponent for Git {
    fn id(&self) -> &'static str {
        "git"
    }

    fn side(&self) -> Side {
        Side::Left
    }

    fn order(&self) -> u8 {
        30
    }

    fn priority(&self) -> u8 {
        55
    }

    fn colors(&self) -> ColorPolicy {
        ColorPolicy::Tokens(&[
            Token::GitClean,
            Token::GitDirty,
            Token::Gold,
            Token::Capsule,
        ])
    }

    fn on_event(&mut self, ev: &StatusEvent<'_>) {
        if let StatusEvent::Git(s) = ev {
            self.status = s.cloned();
        }
    }

    fn render(&self, t: &StatusTheme, budget: Option<usize>) -> Vec<Span<'static>> {
        let Some(g) = self.status.as_ref() else {
            return Vec::new();
        };
        // 两档显示：分到空间就带计数（`?N +N`），分不到就只留分支胶囊。
        // 无 budget = 引擎在量宽，报全形让引擎看到真实开销。
        let branch_only = budget.is_some_and(|w| w < 16);
        let mut out: Vec<Span<'static>> = Vec::new();
        if let Some(b) = g.branch.as_deref() {
            let color = if g.unstaged == 0 && g.staged == 0 {
                t.get(Token::GitClean)
            } else {
                t.get(Token::GitDirty)
            };
            out.push(t.capsule(format!("@ {b}"), color));
        }
        if branch_only {
            return out;
        }
        // Counts are gold, their marker stays capsule-default.
        for (marker, n) in [(" ?", g.unstaged), (" +", g.staged)] {
            if n > 0 {
                out.push(t.on_capsule(marker));
                out.push(t.capsule(n.to_string(), t.get(Token::Gold)));
            }
        }
        out
    }
}
