//! The git segment — `@ <branch>`, `?N` unstaged, `+N` staged.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::text::Span;
use serde_json::Value;

use crate::git::GitStatus;
use crate::ui::tui::zone::main::input::Ctx;
use crate::ui::tui::zone::main::input::statusline::theme::{Slot, StatusTheme};
use crate::ui::tui::zone::main::input::statusline::Deco;

/// Until the config answers: `config.ui.tui.zone.main.input.deco.gitScanInterval`.
pub const DEFAULT_SCAN: Duration = Duration::from_secs(3);

/// The segment scans on its own schedule, one frame at a time; the render reads what landed.
pub struct Git {
    status: Option<GitStatus>,
    /// Where the last scan looked.
    dir: PathBuf,
    /// When it landed; `None` = the first look has not happened yet.
    at: Option<Instant>,
    /// A tool finished or the workspace moved: look again on the next frame.
    dirty: bool,
    /// A scan landed: `None` then means "not a repository", not "not looked yet".
    scanned: bool,
}

pub(super) fn make() -> Box<dyn Deco> {
    Box::new(Git::new())
}

impl Git {
    pub fn new() -> Self {
        Self {
            status: None,
            dir: PathBuf::new(),
            at: None,
            dirty: false,
            scanned: false,
        }
    }
}

impl Deco for Git {
    /// The first frame scans at once; after that one scan per configured interval, and one
    /// more whenever a round moved the tree.
    fn tick(&mut self, ctx: &Ctx) {
        if ctx.data.cwd.is_empty() {
            return;
        }
        let dir = PathBuf::from(&ctx.data.cwd);
        if dir != self.dir {
            self.dir = dir;
            self.status = None;
            self.scanned = false;
            self.dirty = true;
        }
        let interval = ctx.data.git_scan.unwrap_or(DEFAULT_SCAN);
        let due = self.at.is_none_or(|at| at.elapsed() >= interval);
        if !due && !self.dirty {
            return;
        }
        self.status = crate::git::snapshot(&self.dir);
        self.at = Some(Instant::now());
        self.scanned = true;
        self.dirty = false;
    }

    fn on_event(&mut self, event: &str, _content: &Value, _ctx: &Ctx) {
        // A round moved: tools ran, the tree may have changed.
        if event == crate::ui::tui::zone::name::BUSY {
            self.dirty = true;
        }
    }

    fn render(&self, t: &StatusTheme, budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        let Some(g) = self.status.as_ref() else {
            // Before the first scan a mark stands in; a directory with no repository stays blank.
            if ctx.data.cwd.is_empty() || self.scanned {
                return Vec::new();
            }
            return vec![t.capsule("@ ?", t.get(Slot::Muted))];
        };
        // Under 16 cells the branch stands alone, counts dropped.
        let branch_only = budget.is_some_and(|w| w < 16);
        let mut out: Vec<Span<'static>> = Vec::new();
        if let Some(b) = g.branch.as_deref() {
            let color = if g.unstaged == 0 && g.staged == 0 {
                t.get(Slot::GitClean)
            } else {
                t.get(Slot::GitDirty)
            };
            out.push(t.capsule(format!("@ {b}"), color));
        }
        if branch_only {
            return out;
        }
        // Counts are gold; their marker wears the capsule default.
        for (marker, n) in [(" ?", g.unstaged), (" +", g.staged)] {
            if n > 0 {
                out.push(t.on_capsule(marker));
                out.push(t.capsule(n.to_string(), t.get(Slot::Gold)));
            }
        }
        out
    }
}
