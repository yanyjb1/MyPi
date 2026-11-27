// The R test support: the testthat files a batch just wrote or a batch's own file implies, run by file.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use super::format::program;
use crate::platform::stop::Stop;
use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::verdict::{Fault, Verdict};
use crate::server::agent::tools::fs::lang::{Table, Tests};

// What this language owes the tools as test support.
pub(crate) const TESTS: Tests = Tests {
    unit,
    title,
    find,
    run,
};

// Where a project's tests stand, under the root that holds them.
const DIR: &[&str] = &["tests", "testthat"];

// The program that runs one test file, fed down Rscript's own stdin: no file is written for it, and the paths that follow are its arguments.
const DRIVER: &str = r#"
args <- commandArgs(trailingOnly = TRUE)
if (length(args) == 0) quit(save = "no", status = 0)
suppressWarnings(suppressMessages({
  library(testthat)
  # A directory with no `DESCRIPTION` is no project to load: its tests are run as they stand.
  if (requireNamespace("pkgload", quietly = TRUE) && file.exists("DESCRIPTION")) {
    loaded <- try(pkgload::load_all(".", quiet = TRUE, export_all = TRUE, helpers = FALSE), silent = TRUE)
    if (inherits(loaded, "try-error")) {
      cat(sprintf("LOADFAIL\t%s\n", gsub("[\r\n\t]+", " ", as.character(loaded))))
      quit(save = "no", status = 0)
    }
  }
}))
for (f in args) {
  res <- try(testthat::test_file(f, reporter = "silent"), silent = TRUE)
  if (inherits(res, "try-error")) {
    cat(sprintf("ERROR\t%s\t%s:1\t%s\n", basename(f), f, gsub("[\r\n\t]+", " ", as.character(res))))
    next
  }
  for (one in res) {
    bad <- NULL
    skipped <- FALSE
    for (e in one$results) {
      cls <- class(e)[1]
      if (cls %in% c("expectation_failure", "expectation_error")) { bad <- e; break }
      if (cls == "expectation_skip") skipped <- TRUE
    }
    if (!is.null(bad)) {
      line <- if (length(bad$srcref) >= 1) bad$srcref[1] else 1L
      kind <- if (class(bad)[1] == "expectation_error") "ERROR" else "FAIL"
      cat(sprintf("%s\t%s\t%s:%d\t%s\n", kind, one$test, f, line, gsub("[\r\n\t]+", " ", paste(bad$message, collapse = " "))))
    } else if (skipped) {
      cat(sprintf("SKIP\t%s\t%s\n", one$test, f))
    } else {
      cat(sprintf("PASS\t%s\t%s\n", one$test, f))
    }
  }
}
"#;

// The project this file belongs to: the nearest directory above it that holds a `DESCRIPTION`, else the file's own folder. A loose script is a project of its own, so a test run never walks up into an unrelated tree.
pub(super) fn unit(path: &Path) -> Option<PathBuf> {
    let start = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    let marked = start
        .ancestors()
        .find(|dir| dir.join("DESCRIPTION").is_file())
        .map(Path::to_path_buf);
    Some(marked.unwrap_or(start))
}

// What a report calls the project: the `Package:` line its `DESCRIPTION` carries, or the folder's own name.
pub(super) fn title(root: &Path) -> String {
    let named = std::fs::read_to_string(root.join("DESCRIPTION"))
        .ok()
        .and_then(|text| package_of(&text));
    named.unwrap_or_else(|| {
        root.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    })
}

// The name a `DESCRIPTION` gives itself.
fn package_of(manifest: &str) -> Option<String> {
    for line in manifest.lines() {
        let Some(rest) = line.strip_prefix("Package:") else {
            continue;
        };
        let name = rest.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

// The tests this batch must run: the test file it wrote, or the one a source file's name implies. A file whose own test does not exist names nothing, and no run is asked for.
fn find(path: &Path, _text: &str, _table: &Table, _ranges: &[(usize, usize)]) -> Vec<String> {
    let Some(root) = unit(path) else {
        return Vec::new();
    };
    let Some(name) = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return Vec::new();
    };
    if !is_r(&name) {
        return Vec::new();
    }
    let dir = DIR.iter().fold(root.clone(), |at, part| at.join(part));
    if path.starts_with(&dir) {
        if is_test(&name) {
            return vec![path.to_string_lossy().into_owned()];
        }
        return Vec::new();
    }
    let Some(stem) = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
    else {
        return Vec::new();
    };
    let mut asked: Vec<String> = Vec::new();
    for suffix in ["R", "r"] {
        let candidate = dir.join(format!("test-{stem}.{suffix}"));
        if !candidate.is_file() {
            continue;
        }
        // One name on disk answers both spellings where the file system folds case, so a path an earlier candidate already named is dropped.
        let named = candidate.to_string_lossy().into_owned();
        if asked
            .iter()
            .any(|held| held.to_lowercase() == named.to_lowercase())
        {
            continue;
        }
        asked.push(named);
    }
    asked
}

fn is_r(name: &str) -> bool {
    name.ends_with(".R") || name.ends_with(".r")
}

fn is_test(name: &str) -> bool {
    name.starts_with("test-") && is_r(name)
}

// Run exactly these test files in one project, the driver down Rscript's stdin.
fn run(root: &Path, _files: &[PathBuf], names: &[String], stop: &Arc<Stop>) -> Verdict {
    let mut targets: Vec<PathBuf> = Vec::new();
    for name in names {
        let path = PathBuf::from(name);
        if path.is_file() && !targets.contains(&path) {
            targets.push(path);
        }
    }
    if targets.is_empty() {
        return Verdict::Unchecked("no test file is on disk".to_string());
    }
    let Some(bin) = program("Rscript") else {
        return Verdict::Unchecked(
            "Rscript was found neither beside this executable nor on PATH".to_string(),
        );
    };
    let mut cmd = Command::new(bin);
    cmd.arg("-").args(&targets).current_dir(root);
    let ran = match gate::run(cmd, Some(DRIVER.as_bytes()), None, stop) {
        Ok(ran) if ran.stopped => return Verdict::Unchecked("the turn was stopped".to_string()),
        Ok(ran) => ran,
        Err(_) => return Verdict::Unchecked("Rscript did not run".to_string()),
    };
    outcome(root, &ran.out, &ran.err)
}

// What one test run said: the blocks that passed, one fault per block that failed, and the skips as a note.
fn outcome(root: &Path, out: &str, err: &str) -> Verdict {
    let mut passed = 0usize;
    let mut skipped = 0usize;
    let mut faults: Vec<Fault> = Vec::new();
    for line in out.lines() {
        let mut parts = line.splitn(4, '\t');
        let Some(kind) = parts.next() else {
            continue;
        };
        let block = parts.next().unwrap_or("").to_string();
        // A project that cannot be loaded fails every test in it for that one reason, so the run names that reason instead of blaming the tests.
        if kind == "LOADFAIL" {
            return Verdict::Unchecked(format!("the project did not load: {block}"));
        }
        match kind {
            "PASS" => passed += 1,
            "SKIP" => skipped += 1,
            "FAIL" | "ERROR" => {
                let place = parts.next().unwrap_or("");
                let said = parts.next().unwrap_or("").trim().to_string();
                let (file, at) = match place.rsplit_once(':') {
                    Some((file, number)) => (file, number.parse().unwrap_or(1)),
                    None => (place, 1),
                };
                faults.push(Fault {
                    file: parts::rel(root, Path::new(file)),
                    line: at,
                    message: format!("{block}: {said}"),
                    helps: Vec::new(),
                });
            }
            _ => {}
        }
    }
    if !faults.is_empty() {
        let mut summary = format!("{} of {} passed", passed, passed + faults.len());
        if skipped > 0 {
            summary.push_str(&format!(", {skipped} skipped"));
        }
        return Verdict::Failed { faults, summary };
    }
    if passed > 0 {
        return Verdict::Passed {
            ran: passed,
            note: (skipped > 0).then(|| format!("{skipped} skipped")),
        };
    }
    // A file that holds no block at all, or a run whose driver said nothing, is no pass.
    let why = err.lines().find(|line| !line.trim().is_empty());
    Verdict::Unchecked(match why {
        Some(line) => line.trim().to_string(),
        None => "no test block ran".to_string(),
    })
}
