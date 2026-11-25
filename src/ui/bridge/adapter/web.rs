// The webui's own files: the page the bridge serves, seeded onto disk so a reader may edit it.

use std::path::{Path, PathBuf};

// The directory the page's files live in, under the global config root.
const DIR: &str = "webui";

// One built-in file: the name a request asks for, and the body written out when disk has none.
struct Seed {
    name: &'static str,
    body: &'static str,
}

const SEEDS: &[Seed] = &[
    Seed {
        name: "index.html",
        body: include_str!("web/index.html"),
    },
    Seed {
        name: "css/base/base.css",
        body: include_str!("web/css/base/base.css"),
    },
    Seed {
        name: "css/base/bits.css",
        body: include_str!("web/css/base/bits.css"),
    },
    Seed {
        name: "css/base/layout.css",
        body: include_str!("web/css/base/layout.css"),
    },
    Seed {
        name: "css/chat/deck.css",
        body: include_str!("web/css/chat/deck.css"),
    },
    Seed {
        name: "css/chat/dial.css",
        body: include_str!("web/css/chat/dial.css"),
    },
    Seed {
        name: "css/chat/sheet.css",
        body: include_str!("web/css/chat/sheet.css"),
    },
    Seed {
        name: "css/panel/panel.css",
        body: include_str!("web/css/panel/panel.css"),
    },
    Seed {
        name: "css/panel/bill.css",
        body: include_str!("web/css/panel/bill.css"),
    },
    Seed {
        name: "css/rail/rail.css",
        body: include_str!("web/css/rail/rail.css"),
    },
    Seed {
        name: "js/base/dom.js",
        body: include_str!("web/js/base/dom.js"),
    },
    Seed {
        name: "js/base/model.js",
        body: include_str!("web/js/base/model.js"),
    },
    Seed {
        name: "js/base/note.js",
        body: include_str!("web/js/base/note.js"),
    },
    Seed {
        name: "js/base/read.js",
        body: include_str!("web/js/base/read.js"),
    },
    Seed {
        name: "js/base/state.js",
        body: include_str!("web/js/base/state.js"),
    },
    Seed {
        name: "js/base/theme.js",
        body: include_str!("web/js/base/theme.js"),
    },
    Seed {
        name: "js/base/wire.js",
        body: include_str!("web/js/base/wire.js"),
    },
    Seed {
        name: "js/base/write.js",
        body: include_str!("web/js/base/write.js"),
    },
    Seed {
        name: "js/chat/composer/bar.js",
        body: include_str!("web/js/chat/composer/bar.js"),
    },
    Seed {
        name: "js/chat/composer/ring.js",
        body: include_str!("web/js/chat/composer/ring.js"),
    },
    Seed {
        name: "js/chat/composer/key.js",
        body: include_str!("web/js/chat/composer/key.js"),
    },
    Seed {
        name: "js/chat/composer/offer.js",
        body: include_str!("web/js/chat/composer/offer.js"),
    },
    Seed {
        name: "js/chat/composer/path.js",
        body: include_str!("web/js/chat/composer/path.js"),
    },
    Seed {
        name: "js/chat/composer/say.js",
        body: include_str!("web/js/chat/composer/say.js"),
    },
    Seed {
        name: "js/chat/composer/summon.js",
        body: include_str!("web/js/chat/composer/summon.js"),
    },
    Seed {
        name: "js/chat/composer/talk.js",
        body: include_str!("web/js/chat/composer/talk.js"),
    },
    Seed {
        name: "js/chat/dial/dial.js",
        body: include_str!("web/js/chat/dial/dial.js"),
    },
    Seed {
        name: "js/chat/dial/words.js",
        body: include_str!("web/js/chat/dial/words.js"),
    },
    Seed {
        name: "js/chat/history/card.js",
        body: include_str!("web/js/chat/history/card.js"),
    },
    Seed {
        name: "js/chat/history/enter.js",
        body: include_str!("web/js/chat/history/enter.js"),
    },
    Seed {
        name: "js/chat/history/inline.js",
        body: include_str!("web/js/chat/history/inline.js"),
    },
    Seed {
        name: "js/chat/history/live.js",
        body: include_str!("web/js/chat/history/live.js"),
    },
    Seed {
        name: "js/chat/history/liveturn.js",
        body: include_str!("web/js/chat/history/liveturn.js"),
    },
    Seed {
        name: "js/chat/history/mark.js",
        body: include_str!("web/js/chat/history/mark.js"),
    },
    Seed {
        name: "js/chat/history/paint.js",
        body: include_str!("web/js/chat/history/paint.js"),
    },
    Seed {
        name: "js/chat/history/place.js",
        body: include_str!("web/js/chat/history/place.js"),
    },
    Seed {
        name: "js/chat/history/rounds.js",
        body: include_str!("web/js/chat/history/rounds.js"),
    },
    Seed {
        name: "js/chat/history/rows.js",
        body: include_str!("web/js/chat/history/rows.js"),
    },
    Seed {
        name: "js/chat/history/segment.js",
        body: include_str!("web/js/chat/history/segment.js"),
    },
    Seed {
        name: "js/chat/history/sheet.js",
        body: include_str!("web/js/chat/history/sheet.js"),
    },
    Seed {
        name: "js/chat/history/watched.js",
        body: include_str!("web/js/chat/history/watched.js"),
    },
    Seed {
        name: "js/chat/history/window.js",
        body: include_str!("web/js/chat/history/window.js"),
    },
    Seed {
        name: "js/chat/history/cap.js",
        body: include_str!("web/js/chat/history/cap.js"),
    },
    Seed {
        name: "js/chat/history/trim.js",
        body: include_str!("web/js/chat/history/trim.js"),
    },
    Seed {
        name: "js/main.js",
        body: include_str!("web/js/main.js"),
    },
    Seed {
        name: "js/panel/bill.js",
        body: include_str!("web/js/panel/bill.js"),
    },
    Seed {
        name: "js/panel/panels.js",
        body: include_str!("web/js/panel/panels.js"),
    },
    Seed {
        name: "js/panel/plugins/child.js",
        body: include_str!("web/js/panel/plugins/child.js"),
    },
    Seed {
        name: "js/panel/plugins/memo.js",
        body: include_str!("web/js/panel/plugins/memo.js"),
    },
    Seed {
        name: "js/panel/plugins/setup.js",
        body: include_str!("web/js/panel/plugins/setup.js"),
    },
    Seed {
        name: "js/panel/plugins/todo.js",
        body: include_str!("web/js/panel/plugins/todo.js"),
    },
    Seed {
        name: "js/panel/plugins/tree.js",
        body: include_str!("web/js/panel/plugins/tree.js"),
    },
    Seed {
        name: "js/panel/plugins/usage.js",
        body: include_str!("web/js/panel/plugins/usage.js"),
    },
    Seed {
        name: "js/rail/ask.js",
        body: include_str!("web/js/rail/ask.js"),
    },
    Seed {
        name: "js/rail/draft.js",
        body: include_str!("web/js/rail/draft.js"),
    },
    Seed {
        name: "js/rail/rail.js",
        body: include_str!("web/js/rail/rail.js"),
    },
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
            "" | "index.html" => &SEEDS[0],
            other => SEEDS.iter().find(|seed| seed.name == other)?,
        };
        let body = std::fs::read(self.dir.join(seed.name))
            .unwrap_or_else(|_| seed.body.as_bytes().to_vec());
        Some((kind(seed.name), body))
    }

    // Where the files are, as one line the door says at startup.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

// The type a name is served as: the page, its sheets, and its modules.
fn kind(name: &str) -> &'static str {
    if name.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if name.ends_with(".css") {
        "text/css; charset=utf-8"
    } else {
        "text/javascript; charset=utf-8"
    }
}
