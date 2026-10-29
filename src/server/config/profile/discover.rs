use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::super::notice::{ConfigNotice, warn};
use super::super::paths::ConfigPaths;

pub fn locate(paths: &ConfigPaths, name: &str, notices: &mut Vec<ConfigNotice>) -> Option<PathBuf> {
    let (found, unusable) = collect(paths, notices);
    if unusable.contains(name) {
        return None;
    }
    found.get(name).cloned()
}

pub fn list(paths: &ConfigPaths, notices: &mut Vec<ConfigNotice>) -> Vec<String> {
    let (found, unusable) = collect(paths, notices);
    let mut names: Vec<String> = found
        .keys()
        .filter(|n| !unusable.contains(*n))
        .cloned()
        .collect();
    for name in [DEFAULT_PROFILE, OMP_PROFILE] {
        if !names.iter().any(|n| n.as_str() == name) {
            names.push(name.to_string());
        }
    }
    names.sort();
    names
}

pub(super) fn collect(
    paths: &ConfigPaths,
    notices: &mut Vec<ConfigNotice>,
) -> (BTreeMap<String, PathBuf>, BTreeSet<String>) {
    let mut found: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut unusable: BTreeSet<String> = BTreeSet::new();
    for (_, root) in paths.profiles_dirs() {
        let mut local: BTreeMap<String, PathBuf> = BTreeMap::new();
        for dir in scan(&root) {
            let name = dir_name(&root, &dir);
            if let Some(_first) = local.get(&name) {
                unusable.insert(name.clone());
                notices.push(warn(
                    &format!("profile.{name}"),
                    &root.display().to_string(),
                    "duplicate profile name in one root",
                    "name unusable",
                ));
            } else {
                local.insert(name, dir);
            }
        }
        for (name, dir) in local {
            if unusable.contains(&name) {
                continue;
            }
            if found.contains_key(&name) {
                notices.push(warn(
                    &format!("profile.{name}"),
                    &dir.display().to_string(),
                    "the same profile name exists in a lower root",
                    "higher-priority copy",
                ));
            } else {
                found.insert(name, dir);
            }
        }
    }
    for name in &unusable {
        found.remove(name);
    }
    (found, unusable)
}

fn scan(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(root) else {
        return out;
    };
    walk(read, &mut out);
    out.sort();
    out
}

fn walk(read: std::fs::ReadDir, out: &mut Vec<PathBuf>) {
    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join(paths::SYSTEM_FILE).is_file() {
            out.push(path.clone());
        }
        if let Ok(sub) = std::fs::read_dir(&path) {
            walk(sub, out);
        }
    }
}

fn dir_name(root: &Path, dir: &Path) -> String {
    dir.strip_prefix(root)
        .unwrap_or(dir)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(".")
}
