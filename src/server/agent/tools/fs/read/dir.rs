// Directory listings: one level per directory, and each level judging its own documents.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Result, bail};

use crate::server::agent::tools::fs::lang;
use crate::server::agent::tools::fs::lang::graph::loader::{MAX_BYTES, entries_all};
use crate::server::agent::tools::fs::lang::graph::parts::plural;
use crate::server::agent::tools::workspace::canonicalize_lenient;
use crate::server::platform::time;

// Direct files at or past this count fold into counts.
const FOLD_AT: usize = 10;

// One directory's listing, as its own counts and its body. The root is special: every source tree a language claims becomes a section of its own. `docs` names the documents the user keeps current, which take no part in any level's verdict.
pub(crate) fn listing(path: &Path, cwd: &Path, docs: &[String]) -> Result<(String, String)> {
    let root = canonicalize_lenient(cwd);
    let target = canonicalize_lenient(path);
    let trees = lang::trees(&root);
    let mut list = List::new();
    if target == root {
        // One tree that is the root itself needs no section of its own: the listing already is it.
        let sections: Vec<&PathBuf> = trees
            .iter()
            .filter(|tree| tree.as_path() != root.as_path())
            .collect();
        fill(
            &mut list,
            &root,
            1,
            false,
            sections.first().map(|tree| tree.as_path()),
            docs,
        );
        for tree in &sections {
            if !list.out.is_empty() {
                list.out.push('\n');
            }
            let name = tree
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            list.line(&format!("{name}/  {}", counts(tree)));
            fill(&mut list, tree, 1, true, None, docs);
        }
        return Ok((counts(&root), list.finish()));
    }
    if !trees.iter().any(|tree| target.starts_with(tree)) {
        bail!("`read` lists directories at the session root and inside a source tree only");
    }
    fill(&mut list, &target, 1, true, None, docs);
    Ok((counts(&target), list.finish()))
}

// The listing being built: one line per entry, and folding at each level as its own budget.
struct List {
    out: String,
}

impl List {
    fn new() -> Self {
        List { out: String::new() }
    }

    fn line(&mut self, text: &str) {
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn finish(self) -> String {
        self.out
    }
}

// One level: files carry their size and their stamp, directories their entry counts.
fn fill(
    list: &mut List,
    dir: &Path,
    depth: usize,
    expand: bool,
    skip: Option<&Path>,
    docs: &[String],
) {
    let pad = "  ".repeat(depth);
    for row in level(dir, skip, docs) {
        if !row.dir {
            list.line(&format!("{pad}{}  {}", row.label(), row.size));
        } else if expand {
            expand_one(list, &row, depth, docs);
        } else {
            list.line(&format!("{pad}{}/  {}", row.name, counts(&row.path)));
        }
    }
}

// A child directory, one level deep: few files are listed, many are counted.
fn expand_one(list: &mut List, dir: &Entry, depth: usize, docs: &[String]) {
    let rows = level(&dir.path, None, docs);
    let files: Vec<&Entry> = rows.iter().filter(|row| !row.dir).collect();
    let dirs: Vec<&Entry> = rows.iter().filter(|row| row.dir).collect();
    let pad = "  ".repeat(depth);
    let folded = files.len() >= FOLD_AT;
    list.line(&format!(
        "{pad}{}/  {} files, {} dirs",
        dir.name,
        files.len(),
        dirs.len()
    ));
    if folded {
        return;
    }
    let inner = "  ".repeat(depth + 1);
    for file in files {
        list.line(&format!("{inner}{}  {}", file.label(), file.size));
    }
    for child in dirs {
        list.line(&format!("{inner}{}/  {}", child.name, counts(&child.path)));
    }
}

// One level's entries with the level's own verdict applied: a level whose documents are all newer than everything beside them keeps no stamps and marks the documents instead.
fn level(dir: &Path, skip: Option<&Path>, docs: &[String]) -> Vec<Entry> {
    let mut rows: Vec<Entry> = entries_all(dir)
        .into_iter()
        .filter(|entry| skip.is_none_or(|skip| skip != entry.path.as_path()))
        .map(|entry| row(entry.name, entry.path, entry.dir, docs))
        .collect();
    let current = current_docs(&rows);
    for row in &mut rows {
        row.tag = match row.kind {
            Kind::Doc if current => "(latest)",
            Kind::Pinned => "(persistent)",
            _ => "",
        };
        if current {
            row.at = None;
        }
    }
    rows
}

// Whether the documents this level holds are newer than every other file beside them, so no stamp is needed to know they stand as written. A level holding no such document answers yes.
fn current_docs(rows: &[Entry]) -> bool {
    let mut oldest_doc: Option<SystemTime> = None;
    let mut newest_other: Option<SystemTime> = None;
    for row in rows.iter().filter(|row| !row.dir) {
        let at = match row.at {
            Some(at) => at,
            None if matches!(row.kind, Kind::Pinned) => continue,
            None => return false,
        };
        match row.kind {
            Kind::Doc => oldest_doc = Some(oldest_doc.map_or(at, |old| old.min(at))),
            Kind::Plain => newest_other = Some(newest_other.map_or(at, |old| old.max(at))),
            Kind::Pinned => {}
        }
    }
    match (oldest_doc, newest_other) {
        (Some(doc), Some(other)) => doc > other,
        _ => true,
    }
}

// One entry: what it is, how big, when it was last written, and the tag its level gave it.
struct Entry {
    name: String,
    path: PathBuf,
    dir: bool,
    kind: Kind,
    // The file's last write, in the process's zone; cleared when the level vouches for its docs.
    at: Option<SystemTime>,
    size: String,
    // `(latest)`, `(persistent)`, or nothing.
    tag: &'static str,
}

impl Entry {
    // The name as one line shows it: the stamp first when the level carries stamps, then the name and the tag its level gave it.
    fn label(&self) -> String {
        let named = format!("{}{}", self.name, self.tag);
        match self.at {
            Some(at) => format!("{}  {named}", time::stamp_at(at)),
            None => named,
        }
    }
}

// What one file is to the freshness rule.
#[derive(Clone, Copy)]
enum Kind {
    // A document the model would have to weigh: upper case, and not one the user declared.
    Doc,
    // A document the user keeps current himself: never weighed, never dated.
    Pinned,
    // Anything else, and every directory.
    Plain,
}

fn row(name: String, path: PathBuf, dir: bool, docs: &[String]) -> Entry {
    if dir {
        return Entry {
            name,
            path,
            dir,
            kind: Kind::Plain,
            at: None,
            size: String::new(),
            tag: "",
        };
    }
    let (at, size) = facts(&path);
    Entry {
        kind: classify(&path, docs),
        name,
        path,
        dir,
        at,
        size,
        tag: "",
    }
}

// What one name is to the freshness rule: a document is a file its language does not call source; only the stem is compared with the configured ones. Upper case means no lower-case letter and at least one upper-case one.
fn classify(path: &Path, docs: &[String]) -> Kind {
    let Some(one) = lang::owner(path) else {
        return Kind::Plain;
    };
    if one.source {
        return Kind::Plain;
    }
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return Kind::Plain;
    };
    if docs.iter().any(|doc| doc == stem) {
        return Kind::Pinned;
    }
    let mut letters = stem.chars().filter(char::is_ascii_alphabetic);
    let head = letters.next().is_some_and(|c| c.is_ascii_uppercase());
    let rest = letters.all(|c| c.is_ascii_uppercase());
    if head && rest { Kind::Doc } else { Kind::Plain }
}

// A file's stamp and size from one look at its metadata.
// Blocking: one file at a time, up to MAX_BYTES read per file, so a wide directory stalls the listing.
#[allow(clippy::naive_bytecount)]
fn facts(path: &Path) -> (Option<SystemTime>, String) {
    use std::io::Read as _;

    let Ok(meta) = std::fs::metadata(path) else {
        return (None, "unknown".to_string());
    };
    let at = meta.modified().ok();
    if meta.len() > MAX_BYTES {
        return (at, "large".to_string());
    }
    let Ok(file) = std::fs::File::open(path) else {
        return (at, "unknown".to_string());
    };
    // A whole-file read here cost one heap block per listed file; this scan holds 8 KiB.
    let mut reader = std::io::BufReader::new(file);
    let mut buf = [0u8; 8192];
    let mut newlines = 0usize;
    let mut last = 0u8;
    let mut seen = 0usize;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                newlines += buf[..n].iter().filter(|byte| **byte == b'\n').count();
                last = buf[n - 1];
                seen += n;
            }
            Err(_) => return (at, "unknown".to_string()),
        }
    }
    if seen == 0 {
        return (at, "0 lines".to_string());
    }
    let lines = if last == b'\n' {
        newlines
    } else {
        newlines + 1
    };
    (at, format!("{lines} lines"))
}

// A directory's own entry counts, without opening what it holds.
fn counts(dir: &Path) -> String {
    let kids = entries_all(dir);
    let files = kids.iter().filter(|kid| !kid.dir).count();
    let dirs = kids.len() - files;
    format!("{}, {}", plural(files, "file"), plural(dirs, "dir"))
}
