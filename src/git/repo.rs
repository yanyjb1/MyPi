// Where the repository is, and what it knows about one path: whether git tracks it, whether a rule ignores it, and how a file under the session's directory is named against the root.

use std::path::{Path, PathBuf};

use super::run::git;

// The repository above a directory. `None` = no git, or no repository anywhere above it.
pub(crate) fn root(dir: &Path) -> Option<PathBuf> {
    let out = git(dir, &["rev-parse", "--show-toplevel"])?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(PathBuf::from(text))
}

// How a file is named against the repository root, with slashes: the form every git command here takes.
pub(crate) fn relative(root: &Path, file: &Path) -> Option<String> {
    let rest = file.strip_prefix(root).ok()?;
    let text = rest.to_string_lossy().replace('\\', "/");
    (!text.is_empty()).then_some(text)
}

// Whether git holds this file: a file it never saw is untracked, and a commit cannot name it before an `add`.
pub(crate) fn tracked(root: &Path, path: &str) -> bool {
    git(root, &["ls-files", "--error-unmatch", "--", path]).is_some_and(|out| out.status.success())
}

// Whether git holds this file, and whether it already carried an uncommitted change of the user's; `(false, false)` outside a repository or off its tree.
pub(crate) fn facts(file: &Path) -> (bool, bool) {
    let Some(dir) = file.parent() else {
        return (false, false);
    };
    let Some(root) = root(dir) else {
        return (false, false);
    };
    let Some(name) = relative(&root, file) else {
        return (false, false);
    };
    let tracked = tracked(&root, &name);
    let dirty = tracked
        && crate::git::changed(&root, std::slice::from_ref(&name))
            .iter()
            .any(|held| held == &name);
    (tracked, dirty)
}

// Whether a rule in `.gitignore` refuses this path: `git add` would fail on it, and the file stays the user's own.
pub(crate) fn ignored(root: &Path, path: &str) -> bool {
    git(root, &["check-ignore", "--quiet", "--", path]).is_some_and(|out| out.status.success())
}
