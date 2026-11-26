// One commit of the files a session named, by path: the index may hold the user's own work, and it stays theirs.

use std::path::Path;

use super::run::{git, why};

// Hand one file to the index, so a file git never saw can be named by the commit that follows.
pub(crate) fn add(root: &Path, paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args = vec!["add", "--"];
    args.extend(paths.iter().map(String::as_str));
    match git(root, &args) {
        Some(out) if out.status.success() => Ok(()),
        Some(out) => Err(why(&out)),
        None => Err("git could not be run".to_string()),
    }
}

// Commit exactly these paths. `--` after the message makes git record what those files hold now, whatever the index carries for anything else.
pub(crate) fn record(root: &Path, message: &str, paths: &[String]) -> Result<(), String> {
    let mut args = vec!["commit", "--no-verify", "-m", message, "--"];
    args.extend(paths.iter().map(String::as_str));
    match git(root, &args) {
        Some(out) if out.status.success() => Ok(()),
        Some(out) => Err(why(&out)),
        None => Err("git could not be run".to_string()),
    }
}
