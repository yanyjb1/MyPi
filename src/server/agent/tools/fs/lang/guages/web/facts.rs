// The web's facts for the module graph: a page is the hub, and every stylesheet and script it names hangs off it.

use std::path::{Path, PathBuf};

use super::{css, html, js};
use crate::server::agent::tools::fs::lang::Hub;
use crate::server::agent::tools::fs::lang::graph::{Decl, Domain, Loader, RawEdge, Unit};
use crate::server::agent::tools::workspace::canonicalize_lenient;

pub(crate) static HUB: Hub = Hub {
    edges,
    unit_of: Some(unit_of),
    territory: Some(territory),
    interface: Some(interface),
    domain: Some(domain),
    loose: None,
    view: Some(super::view::view),
};

// The page a directory serves, in the order a server would try them.
const PAGES: &[&str] = &["index.html", "index.htm"];

// Names one interface line lists before the rest fold into a count.
const LISTED: usize = 12;

// The page root this path is read against: the nearest directory above it that serves a page, its own directory otherwise.
fn domain(path: &Path) -> Option<Domain> {
    let path = canonicalize_lenient(path);
    let start = if path.is_dir() {
        path.clone()
    } else {
        path.parent()?.to_path_buf()
    };
    let page = start
        .ancestors()
        .find(|dir| PAGES.iter().any(|one| dir.join(one).is_file()))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| start.clone());
    Some(Domain {
        root: page.clone(),
        tree: page,
    })
}

// Where a path belongs: a page is one unit named by its own directory when it is that directory's page, a stylesheet or a script one unit named by its file.
fn unit_of(path: &Path) -> Option<Unit> {
    let path = canonicalize_lenient(path);
    if !path.is_file() {
        return None;
    }
    let dom = domain(&path)?;
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let page = PAGES.iter().any(|name| path.ends_with(name));
    let name = if page {
        path.parent()
            .and_then(|dir| dir.strip_prefix(&dom.tree).ok())
            .map(|rel| rel.to_string_lossy().into_owned())
            .filter(|rel| !rel.is_empty())
            .unwrap_or_else(|| "index".to_string())
    } else {
        path.strip_prefix(&dom.tree)
            .map(|rel| rel.to_string_lossy().into_owned())
            .unwrap_or_else(|_| stem.clone())
    };
    Some(Unit {
        name,
        place: path.clone(),
        entry: Some(path),
    })
}

// What one unit holds: the files its own text names and that exist on disk.
fn territory(unit: &Unit, loader: &Loader) -> Vec<Decl> {
    let Some(text) = loader.read(&unit.place) else {
        return Vec::new();
    };
    let mut out: Vec<Decl> = Vec::new();
    for raw in edges(&unit.place, &text, loader) {
        let Some(target) = raw.targets.first() else {
            continue;
        };
        let Some(child) = unit_of(target) else {
            continue;
        };
        if out.iter().any(|held| held.unit.place == child.place) {
            continue;
        }
        out.push(Decl {
            unit: child,
            line: Some(raw.line),
            from: Some(unit.place.clone()),
            tags: vec!["asset".to_string()],
        });
    }
    out
}

// What one file offers the rest of the page: a page's ids, a stylesheet's selectors, a script's exports.
fn interface(path: &Path, text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    match super::kind(path) {
        Some("html") => {
            let tags = html::scan(text);
            for (kind, name, _) in html::anchors(&tags) {
                if kind == '#' && !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        Some("css") => {
            let table = css::table(text);
            for (_, name, _) in css::anchors(&table) {
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        Some("js") => out = js::exports(&js::strip(text), text),
        _ => {}
    }
    if out.len() > LISTED {
        let more = out.len() - LISTED;
        out.truncate(LISTED);
        out.push(format!("[+{more} more]"));
    }
    out
}

// Every file this text names, with the line it stands on and where it lands.
fn edges(path: &Path, text: &str, _loader: &Loader) -> Vec<RawEdge> {
    let Some(dom) = domain(path) else {
        return Vec::new();
    };
    let mut out: Vec<RawEdge> = Vec::new();
    match super::kind(path) {
        Some("html") => {
            let tags = html::scan(text);
            for (line, href) in html::refs(&tags) {
                out.push(RawEdge {
                    line,
                    wrote: href.clone(),
                    symbols: Vec::new(),
                    targets: land(path, &dom.tree, &href),
                    via: Vec::new(),
                    re_export: false,
                });
            }
        }
        Some("css") => {
            for (line, href) in css::imports(&css::strip(text), text) {
                out.push(RawEdge {
                    line,
                    wrote: href.clone(),
                    symbols: Vec::new(),
                    targets: land(path, &dom.tree, &href),
                    via: Vec::new(),
                    re_export: false,
                });
            }
        }
        Some("js") => {
            for (line, href, symbols) in js::refs(&js::strip(text), text) {
                out.push(RawEdge {
                    line,
                    wrote: href.clone(),
                    symbols,
                    targets: land(path, &dom.tree, &href),
                    via: Vec::new(),
                    re_export: false,
                });
            }
        }
        _ => {}
    }
    out
}

// The files a reference lands on: a path opening with `/` is read from the page root, anything else from the referring file's own directory. A target that is not on disk lands nowhere, and the edge keeps only what it wrote.
fn land(path: &Path, root: &Path, wrote: &str) -> Vec<PathBuf> {
    if wrote.starts_with("http://") || wrote.starts_with("https://") || wrote.starts_with("//") {
        return Vec::new();
    }
    let cut = wrote.split(['?', '#']).next().unwrap_or(wrote);
    if cut.is_empty() {
        return Vec::new();
    }
    let joined = if let Some(rest) = cut.strip_prefix('/') {
        root.join(rest)
    } else {
        let Some(dir) = path.parent() else {
            return Vec::new();
        };
        dir.join(cut)
    };
    let found = canonicalize_lenient(&joined);
    found.is_file().then_some(found).into_iter().collect()
}
