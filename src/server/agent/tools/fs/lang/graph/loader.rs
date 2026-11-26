// One read-only view of the tree: entries and text, with the junk list and the size cap in one place.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

// A file past this size is not read: the one cap every reader of the tree shares.
pub(crate) const MAX_BYTES: u64 = 4 * 1024 * 1024;

// One directory entry, already sorted.
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) dir: bool,
}

// Every file under one root, by file stem: a language that names a file by name alone reads this.
pub(crate) struct Names {
    root: PathBuf,
    by_stem: HashMap<String, Vec<PathBuf>>,
}

impl Names {
    // Every file under the root whose stem is `stem`, in path order; a name no file carries answers empty.
    pub(crate) fn find(&self, stem: &str) -> &[PathBuf] {
        self.by_stem.get(stem).map(Vec::as_slice).unwrap_or(&[])
    }
}

fn gather(dir: &Path, into: &mut HashMap<String, Vec<PathBuf>>) {
    for entry in entries(dir) {
        if entry.dir {
            gather(&entry.path, into);
            continue;
        }
        if let Some(stem) = entry.path.file_stem() {
            into.entry(stem.to_string_lossy().into_owned())
                .or_default()
                .push(entry.path);
        }
    }
}

// The only door to the disk the machine and the languages share. It never writes.
pub(crate) struct Loader {
    cache: RefCell<HashMap<PathBuf, Option<String>>>,
    // The one name index this loader built: a loader lives a single read call, so nothing written later can leave it stale.
    names: RefCell<Option<Rc<Names>>>,
}

impl Loader {
    pub(crate) fn new() -> Self {
        Loader {
            cache: RefCell::new(HashMap::new()),
            names: RefCell::new(None),
        }
    }

    // One file's text; a missing, unreadable or oversized file answers `None`.
    pub(crate) fn read(&self, path: &Path) -> Option<String> {
        if let Some(hit) = self.cache.borrow().get(path) {
            return hit.clone();
        }
        let text = text(path);
        self.cache
            .borrow_mut()
            .insert(path.to_path_buf(), text.clone());
        text
    }

    // One directory's entries, junk skipped, directories first, names case-insensitive.
    pub(crate) fn entries(dir: &Path) -> Vec<Entry> {
        entries(dir)
    }

    // The files under `root` by stem: walked once for this loader, and again only when a later call names another root.
    pub(crate) fn named(&self, root: &Path) -> Rc<Names> {
        if let Some(hit) = self.names.borrow().as_ref()
            && hit.root == root
        {
            return Rc::clone(hit);
        }
        let built = Rc::new(Names {
            root: root.to_path_buf(),
            by_stem: {
                let mut by_stem: HashMap<String, Vec<PathBuf>> = HashMap::new();
                gather(root, &mut by_stem);
                by_stem
            },
        });
        *self.names.borrow_mut() = Some(Rc::clone(&built));
        built
    }
}

impl Default for Loader {
    fn default() -> Self {
        Loader::new()
    }
}

// One file's text, capped: the only way the tree hands out a whole file.
pub(crate) fn text(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

// One directory's entries for a walk: junk skipped, directories first, names case-insensitive.
pub(crate) fn entries(dir: &Path) -> Vec<Entry> {
    entries_where(dir, false)
}

// One directory's entries for a listing: every name, junk included.
pub(crate) fn entries_all(dir: &Path) -> Vec<Entry> {
    entries_where(dir, true)
}

fn entries_where(dir: &Path, all: bool) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Entry> = read
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !all && junk(&name) {
                return None;
            }
            let dir = entry.file_type().ok()?.is_dir();
            Some(Entry {
                name,
                path: entry.path(),
                dir,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

// A name no reader opens: a build output, a dependency tree, or anything hidden.
pub(crate) fn junk(name: &str) -> bool {
    matches!(name, "target" | "node_modules") || name.starts_with('.')
}
