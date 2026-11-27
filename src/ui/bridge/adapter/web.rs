// The webui's own files: the page the bridge serves, seeded onto disk so a reader may edit it.

use std::path::{Path, PathBuf};

use include_dir::{Dir, include_dir};

// The directory the page's files live in, under the global config root.
const DIR: &str = "webui";

// The page itself, embedded whole: every file under it is served, so nothing here lists them.
static PAGE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/ui/bridge/adapter/web");

// The page, as one directory on disk: whatever is there is served, whatever is missing is written first.
pub struct Webui {
    dir: PathBuf,
}

impl Webui {
    // Seed the directory, then answer from it: an edited file stays edited, a missing one is written again.
    pub fn open(root: &Path) -> Webui {
        let dir = root.join(DIR);
        let webui = Webui { dir };
        webui.seed();
        webui
    }

    fn seed(&self) {
        for file in PAGE.files() {
            let name = name_of(file.path());
            if kind(&name).is_none() {
                continue;
            }
            let path = self.dir.join(&name);
            if path.exists() {
                continue;
            }
            if let Some(parent) = path.parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                eprintln!("mypi: bridge cannot make {}: {e}", parent.display());
                continue;
            }
            if let Err(e) = std::fs::write(&path, file.contents()) {
                eprintln!("mypi: bridge cannot write {}: {e}", path.display());
            }
        }
    }

    // One file by the name a request asks for; the empty name is the page itself.
    pub fn file(&self, name: &str) -> Option<(&'static str, Vec<u8>)> {
        let asked = if name.is_empty() { "index.html" } else { name };
        let ctype = kind(asked)?;
        let built = PAGE.get_file(asked)?;
        let body = std::fs::read(self.dir.join(asked))
            .unwrap_or_else(|_| built.contents().to_vec());
        Some((ctype, body))
    }

    // Where the files are, as one line the door says at startup.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

// A file's name as a request asks for it: its path inside the page, with one kind of separator.
fn name_of(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

// The type a name is served as: the page, its sheets, and its modules. A note beside them is not one of them.
fn kind(name: &str) -> Option<&'static str> {
    if name.ends_with(".html") {
        Some("text/html; charset=utf-8")
    } else if name.ends_with(".css") {
        Some("text/css; charset=utf-8")
    } else if name.ends_with(".js") {
        Some("text/javascript; charset=utf-8")
    } else {
        None
    }
}