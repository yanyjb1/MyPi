//! The git segment — `@ <branch>`, `?N` unstaged, `+N` staged.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ratatui::text::Span;

use crate::git::GitStatus;
use crate::ui::tui::zone::main::input::statusline::theme::{StatusTheme, Token};
use crate::ui::tui::zone::main::input::statusline::{Deco, Setup};
use crate::ui::tui::zone::{Ctx, Event};

/// Fallback scan interval: `config.ui.tui.zone.main.input.deco.gitScanInterval`.
pub const DEFAULT_SCAN: Duration = Duration::from_secs(3);

/// The segment's background thread scans on its own; the render reads a slot.
pub struct Git {
    status: Arc<Mutex<Option<GitStatus>>>,
    dir: Arc<Mutex<PathBuf>>,
    /// A tool finished, or the workspace moved.
    dirty: Arc<AtomicBool>,
}

pub(super) fn make(setup: &Setup) -> Box<dyn Deco> {
    Box::new(Git::new(setup.git_scan))
}

impl Git {
    pub fn new(interval: Duration) -> Self {
        let status = Arc::new(Mutex::new(None));
        let dir = Arc::new(Mutex::new(PathBuf::new()));
        let dirty = Arc::new(AtomicBool::new(false));
        let (s, d, w) = (status.clone(), dir.clone(), dirty.clone());
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(interval);
                if !w.load(Ordering::Relaxed) {
                    continue;
                }
                let path = d.lock().map(|p| p.clone()).unwrap_or_default();
                let snap = (!path.as_os_str().is_empty())
                    .then(|| crate::git::snapshot(&path))
                    .flatten();
                if let Ok(mut slot) = s.lock() {
                    *slot = snap;
                }
                w.store(false, Ordering::Relaxed);
            }
        });
        Self { status, dir, dirty }
    }

    /// Aim the scan at another directory and schedule a look.
    fn set_dir(&self, dir: PathBuf) {
        if let Ok(mut slot) = self.dir.lock() {
            *slot = dir;
        }
        self.dirty.store(true, Ordering::Relaxed);
    }
}

impl Deco for Git {
    fn on_event(&mut self, ev: Event, ctx: &Ctx) {
        // The workspace comes from the view; a temporary `cd` does not move it.
        if !ctx.view.cwd_temporary && !ctx.view.cwd.is_empty() {
            let dir = PathBuf::from(ctx.view.cwd);
            let moved = self.dir.lock().map(|p| *p != dir).unwrap_or(false);
            if moved {
                self.set_dir(dir);
            }
        }
        if matches!(ev, Event::ToolFinished) {
            self.dirty.store(true, Ordering::Relaxed);
        }
    }

    fn render(&self, t: &StatusTheme, budget: Option<usize>, _ctx: &Ctx) -> Vec<Span<'static>> {
        let Ok(slot) = self.status.lock() else {
            return Vec::new();
        };
        let Some(g) = slot.as_ref() else {
            return Vec::new();
        };
        // Under 16 cells the branch stands alone, counts dropped.
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
        // Counts are gold; their marker wears the capsule default.
        for (marker, n) in [(" ?", g.unstaged), (" +", g.staged)] {
            if n > 0 {
                out.push(t.on_capsule(marker));
                out.push(t.capsule(n.to_string(), t.get(Token::Gold)));
            }
        }
        out
    }
}
