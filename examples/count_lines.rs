// Rough line counter for AI.md iron rule #2: total rs lines in src/ after stripping comments and blank lines must stay under 50,000.
// Prints the top 10 files, the line total of each first-level dir under src/, and the final OK/OVER status against the budget.
use std::cmp::Reverse;
use std::fs;
use std::path::{Path, PathBuf};

const BUDGET: usize = 50_000; // AI.md iron rule #2
const EXEMPT: [&str; 2] = ["examples", "tests"];

fn main() {
    let root = find_src_root();
    let mut files: Vec<(String, usize)> = Vec::new();
    collect_files(&root, "", &mut files);

    files.sort_by_key(|f| Reverse(f.1));
    println!("top 10 files");
    for (rel, lines) in files.iter().take(10) {
        println!("{:<32}{:>8}", rel, fmt(lines));
    }

    let mut dirs: Vec<(String, usize)> = Vec::new();
    let mut root_files = 0usize;
    for (rel, lines) in &files {
        if let Some((dir, _)) = rel.split_once('/') {
            match dirs.iter_mut().find(|entry| entry.0 == dir) {
                Some(entry) => entry.1 += *lines,
                None => dirs.push((dir.to_string(), *lines)),
            }
        } else {
            root_files += *lines;
        }
    }
    dirs.sort_by_key(|d| Reverse(d.1));
    println!();
    println!("lines per first-level dir");
    for (name, lines) in &dirs {
        println!("{:<32}{:>8}", format!("{name}/"), fmt(*lines));
    }
    println!("{:<32}{:>8}", "(root files)", fmt(root_files));

    let total: usize = files.iter().map(|f| f.1).sum();
    println!();
    println!("{:<44}{}", "effective lines total", fmt(total));
    println!("{:<44}{}", "AI.md iron rule #2 budget", fmt(BUDGET));
    let left = BUDGET as isize - total as isize;
    let status = if left >= 0 {
        format!("OK, {} lines left", fmt(left as usize))
    } else {
        format!("OVER by {} lines", fmt((-left) as usize))
    };
    println!("{:<44}{}", "status", status);
}

// Walk up from cwd to locate the src/ dir, so the example runs from any depth.
fn find_src_root() -> PathBuf {
    let mut dir = std::env::current_dir().expect("current dir");
    loop {
        let candidate = dir.join("src");
        if candidate.is_dir() {
            return candidate;
        }
        if !dir.pop() {
            panic!("no src/ found in any parent of cwd");
        }
    }
}

// Count effective lines of one file: whole-line comments (//) and blank lines stripped. Trailing comments count as code.
fn count_file(path: &Path) -> usize {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return 0, // unreadable file counts as zero, do not fail the whole run
    };
    text.lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.is_empty() && !t.starts_with("//")
        })
        .count()
}

// Collect every rs file under dir as (path relative to src/, effective lines); exempt dirs are skipped at any depth.
fn collect_files(dir: &Path, prefix: &str, out: &mut Vec<(String, usize)>) {
    for entry in fs::read_dir(dir).expect("readable dir") {
        let entry = entry.expect("readable entry");
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if path.is_dir() {
            if EXEMPT.contains(&name.as_str()) {
                continue;
            }
            collect_files(&path, &rel, out);
        } else if name.ends_with(".rs") {
            out.push((rel, count_file(&path)));
        }
    }
}

fn fmt(n: impl std::fmt::Display) -> String {
    // Thousand separators, done by hand to stay std-only.
    let raw = n.to_string();
    let (sign, digits) = match raw.strip_prefix('-') {
        Some(d) => ("-", d),
        None => ("", raw.as_str()),
    };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}")
}
