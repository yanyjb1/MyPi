//! The working directory — `[D] <path>` in the workspace, `[T] <path>` when the
//! model has temporarily migrated elsewhere.
//!
//! Update sources, both external notifications:
//! - `WorkspaceChanged` — `/cdp` (or a resume/tree restore): the user's
//!   persistent workspace moved. Also moves the displayed directory, because
//!   the session's cwd moves with it.
//! - `CwdMigrated` — the model's `cd` tool: a *temporary* migration.
//!
//! The `[D]` / `[T]` decision belongs to this component, not to the notifier:
//! the sender says where the cwd is, the component's verification decides which
//! of the two states that is. See [`in_workspace`] for the seam.

use std::path::Path;

use ratatui::text::Span;

use crate::tui::zone::main::input::statusline::theme::{ColorPolicy, StatusTheme, Token};
use crate::tui::zone::main::input::statusline::{Side, StatusComponent, StatusEvent};

#[derive(Debug, Default)]
pub struct Cwd {
    current: std::path::PathBuf,
    workspace: std::path::PathBuf,
}

impl Cwd {
    fn marker(&self) -> &'static str {
        if in_workspace(&self.current, &self.workspace) {
            "[D]"
        } else {
            "[T]"
        }
    }
}

/// Is the current directory still "in the workspace"?
///
/// **Provisional rule**: the same directory. The real rule (does a
/// subdirectory of the workspace count? what about a symlinked path?) is a
/// later decision — this function is the only place that has to change, and the
/// `[D]`/`[T]` marker plus the git target both follow it.
fn in_workspace(current: &Path, workspace: &Path) -> bool {
    current == workspace
}

impl StatusComponent for Cwd {
    fn id(&self) -> &'static str {
        "cwd"
    }

    fn side(&self) -> Side {
        Side::Left
    }

    fn order(&self) -> u8 {
        20
    }

    fn priority(&self) -> u8 {
        40
    }

    fn colors(&self) -> ColorPolicy {
        ColorPolicy::Tokens(&[Token::Text, Token::Capsule])
    }

    fn on_event(&mut self, ev: &StatusEvent<'_>) {
        match ev {
            StatusEvent::WorkspaceChanged(p) => {
                self.workspace = p.to_path_buf();
                self.current = p.to_path_buf();
            }
            StatusEvent::CwdMigrated(p) => self.current = p.to_path_buf(),
            _ => {}
        }
    }

    fn render(&self, t: &StatusTheme, _budget: Option<usize>) -> Vec<Span<'static>> {
        if self.current.as_os_str().is_empty() {
            return Vec::new();
        }
        vec![t.capsule(
            format!("{} {}", self.marker(), short_path(&self.current)),
            t.get(Token::Text),
        )]
    }
}

/// Directory: at most one parent level. `/home/Arisha/Utility/MyPi` ->
/// `Utility/MyPi`.
fn short_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    let parts: Vec<&str> = s.split('/').filter(|s| !s.is_empty()).collect();
    match parts.len() {
        0 => "/".into(),
        1 => format!("/{}", parts[0]),
        _ => format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_shortens_to_one_parent_level() {
        assert_eq!(short_path(Path::new("/home/Arisha/Utility/MyPi")), "Utility/MyPi");
        assert_eq!(short_path(Path::new("/home")), "/home");
        assert_eq!(short_path(Path::new("/")), "/");
    }

    #[test]
    fn marker_follows_the_workspace_verification() {
        let mut c = Cwd::default();
        c.on_event(&StatusEvent::WorkspaceChanged(Path::new("/work/a")));
        assert_eq!(c.marker(), "[D]");
        c.on_event(&StatusEvent::CwdMigrated(Path::new("/tmp/b")));
        assert_eq!(c.marker(), "[T]");
        // A workspace move resets both, so the marker comes back.
        c.on_event(&StatusEvent::WorkspaceChanged(Path::new("/work/c")));
        assert_eq!(c.marker(), "[D]");
        assert_eq!(c.current, std::path::PathBuf::from("/work/c"));
    }
}
