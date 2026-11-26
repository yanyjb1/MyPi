// The working tree's own reading: its branch, how many files it changed, and which of a set of paths already carry a change.

use std::path::Path;

use super::run::git;

// Expand untracked directories into individual files? `false` matches `git status` defaults.
const UNTRACKED_ALL: bool = false;

// Result of one git query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
    // Current branch name; None when detached or outside a repository.
    pub branch: Option<String>,

    // Files needing `git add` -> statusline `?N`
    pub unstaged: usize,

    // Staged file count -> statusline `+N`
    pub staged: usize,
}

// Read the git status of a directory. Outside a repo / no git installed -> None (statusline hides it).
pub fn snapshot(dir: &Path) -> Option<GitStatus> {
    let mut args = vec!["status", "--porcelain=v1"];
    if UNTRACKED_ALL {
        args.push("--untracked-files=all");
    }
    let out = git(dir, &args)?;
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

// Current branch name. Uses `branch --show-current`: works even on an unborn HEAD (no commits yet).
fn branch_name(dir: &Path) -> Option<String> {
    let out = git(dir, &["branch", "--show-current"])?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

// Which of these paths, named against the repository root, the working tree or the index already holds a change for. A path git does not know at all counts as unchanged here: it is new, not the user's work.
pub(crate) fn changed(root: &Path, paths: &[String]) -> Vec<String> {
    if paths.is_empty() {
        return Vec::new();
    }
    // `core.quotepath=false`: git otherwise escapes every path outside ASCII into octal, and the names this answers would never match the ones its caller holds.
    let mut args = vec![
        "-c",
        "core.quotepath=false",
        "status",
        "--porcelain=v1",
        "--",
    ];
    args.extend(paths.iter().map(String::as_str));
    let Some(out) = git(root, &args) else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    dirty(&String::from_utf8_lossy(&out.stdout))
}

// Every path a porcelain row names, with its two status letters cut away.
fn dirty(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.len() < 4 {
            continue;
        }
        let rest = line[3..].trim();
        // A rename reads `old -> new`: the file the session knows is the one it now has.
        let path = rest.rsplit(" -> ").next().unwrap_or(rest);
        let path = path.trim_matches('"');
        if !path.is_empty() {
            out.push(path.to_string());
        }
    }
    out
}

// Parse porcelain output into counts (extracted so tests do not need a real repository).
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
