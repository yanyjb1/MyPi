//! git integration — only two things needed: the current branch name and
//! how many files changed.
//!
//! Counting rules (by design: count files, not lines):
//! - Needs `git add` (untracked + unstaged changes) -> `?N`
//! - Already staged -> `+N`
//!
//! A file that is both staged and modified again (porcelain `MM`) counts
//! on both sides — it genuinely exists in both todo lists.

use std::path::Path;
use std::process::Command;

/// Whether untracked directories expand into individual files.
/// `false` matches `git status` defaults (`?? src/` counts as 1);
/// `true` counts every file (adds `--untracked-files=all`).
const UNTRACKED_ALL: bool = false;

/// Result of one git query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
    /// Current branch name; None when detached or outside a repository.
    pub branch: Option<String>,
    /// Files needing `git add` -> statusline `?N`
    pub unstaged: usize,
    /// Staged file count -> statusline `+N`
    pub staged: usize,
}

/// Read the git status of a directory. Outside a repo / no git installed -> None (statusline hides it).
pub fn snapshot(dir: &Path) -> Option<GitStatus> {
    let mut cmd = Command::new("git");
    cmd.args(["status", "--porcelain=v1"]);
    if UNTRACKED_ALL {
        cmd.arg("--untracked-files=all");
    }
    let out = cmd.current_dir(dir).output().ok()?;
    if !out.status.success() {
        return None; // not a git repository
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let (staged, unstaged) = count_porcelain(&text);

    Some(GitStatus {
        branch: branch_name(dir),
        unstaged,
        staged,
    })
}

/// Current branch name. Uses `branch --show-current`: works even on an unborn HEAD (no commits yet).
fn branch_name(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

/// Parse porcelain output into counts (extracted so tests do not need a real repository).
pub fn count_porcelain(text: &str) -> (usize, usize) {
    let (mut staged, mut unstaged) = (0usize, 0usize);
    for line in text.lines() {
        let b = line.as_bytes();
        if b.len() < 2 {
            continue;
        }
        let (x, y) = (b[0] as char, b[1] as char);
        if x == '?' {
            unstaged += 1;
            continue;
        }
        if x != ' ' {
            staged += 1;
        }
        if y != ' ' {
            unstaged += 1;
        }
    }
    (staged, unstaged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_untracked_and_modified() {
        // ?? = untracked, " M" = unstaged modification, "M " = staged
        let (staged, unstaged) = count_porcelain("?? a.txt\n?? b.txt\n M c.rs\nM  d.rs\n");
        assert_eq!(unstaged, 3); // a, b, c
        assert_eq!(staged, 1); // d
    }

    #[test]
    fn modified_and_staged_counts_both() {
        // MM: staged plus new changes -> counted on both sides
        let (staged, unstaged) = count_porcelain("MM e.rs\n");
        assert_eq!(staged, 1);
        assert_eq!(unstaged, 1);
    }

    #[test]
    fn clean_tree_is_zero() {
        let (staged, unstaged) = count_porcelain("");
        assert_eq!((staged, unstaged), (0, 0));
    }

    #[test]
    fn added_and_deleted_are_staged() {
        let (staged, unstaged) = count_porcelain("A  new.rs\n D gone.rs\nD  del.rs\n");
        // A = staged addition; " D" = unstaged deletion; "D " = staged deletion
        assert_eq!(staged, 2); // A, D
        assert_eq!(unstaged, 1); // ' D'
    }
}
