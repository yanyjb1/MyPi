// The webui's own files: the page the bridge serves, seeded onto disk so a reader may edit it.

use std::path::{Path, PathBuf};

// The directory the page's files live in, under the global config root.
const DIR: &str = "webui";

// One built-in file: the name a request asks for, its path on disk, and the type it is served as.
struct Seed {
    name: &'static str,
    body: &'static str,
    kind: &'static str,
}

const INDEX: Seed = Seed {
    name: "index.html",
    body: include_str!("web/index.html"),
    kind: "text/html; charset=utf-8",
};

const BASE: Seed = Seed {
    name: "css/base.css",
    body: include_str!("web/css/base.css"),
    kind: "text/css; charset=utf-8",
};

const RAIL: Seed = Seed {
    name: "css/rail.css",
    body: include_str!("web/css/rail.css"),
    kind: "text/css; charset=utf-8",
};

const MAIN: Seed = Seed {
    name: "css/main.css",
    body: include_str!("web/css/main.css"),
    kind: "text/css; charset=utf-8",
};

const PANEL: Seed = Seed {
    name: "css/panel.css",
    body: include_str!("web/css/panel.css"),
    kind: "text/css; charset=utf-8",
};

const WIRE: Seed = Seed {
    name: "js/wire.js",
    body: include_str!("web/js/wire.js"),
    kind: "text/javascript; charset=utf-8",
};

const TEXT: Seed = Seed {
    name: "js/text.js",
    body: include_str!("web/js/text.js"),
    kind: "text/javascript; charset=utf-8",
};

const STATE: Seed = Seed {
    name: "js/state.js",
    body: include_str!("web/js/state.js"),
    kind: "text/javascript; charset=utf-8",
};

const VIEW: Seed = Seed {
    name: "js/view.js",
    body: include_str!("web/js/view.js"),
    kind: "text/javascript; charset=utf-8",
};

const APP: Seed = Seed {
    name: "js/app.js",
    body: include_str!("web/js/app.js"),
    kind: "text/javascript; charset=utf-8",
};

const SEEDS: [&Seed; 10] = [
    &INDEX, &BASE, &RAIL, &MAIN, &PANEL, &WIRE, &TEXT, &STATE, &VIEW, &APP,
];

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
        for seed in SEEDS {
            let path = self.dir.join(seed.name);
            if path.exists() {
                continue;
            }
            if let Some(parent) = path.parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                eprintln!("mypi: bridge cannot make {}: {e}", parent.display());
                continue;
            }
            if let Err(e) = std::fs::write(&path, seed.body) {
                eprintln!("mypi: bridge cannot write {}: {e}", path.display());
            }
        }
    }

    // One file by the name a request asks for; the empty name is the page itself.
    pub fn file(&self, name: &str) -> Option<(&'static str, Vec<u8>)> {
        let seed = match name {
            "" | "index.html" => &INDEX,
            other => *SEEDS.iter().find(|seed| seed.name == other)?,
        };
        let body = std::fs::read(self.dir.join(seed.name))
            .unwrap_or_else(|_| seed.body.as_bytes().to_vec());
        Some((seed.kind, body))
    }

    // Where the files are, as one line the door says at startup.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}
