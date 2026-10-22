//! mypi — a terminal coding agent: session service, agent loop, TUI.
//! The module graph is a contract, and `mod architecture` below enforces it.

pub mod ansi;
pub mod cli;
pub mod commands;
pub mod git;
pub mod ipc;
pub mod oneshot;
pub mod server;
#[cfg(feature = "web")]
pub mod web;

/// The layering rules as a test: a forbidden `crate::` reference has to fail something, and it
/// fails here. Rules are path-prefix pairs, so they survive a file moving into a submodule.
#[cfg(test)]
mod architecture {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    /// `(module prefix, module prefixes it must never reference)`. The direction is downward only;
    /// anything not listed is allowed.
    const FORBIDDEN: &[(&str, &[&str])] = &[
        // Execution engine: runs tools, knows no UI and no layer above it.
        ("server::agent", &["server::api", "cli", "tui"]),
        // Contract: command declarations and metadata only. It reads nothing (both the server and the
        // front ends read it).
        ("commands", &["server", "cli", "tui", "web", "git", "ansi"]),
        // api sits above: any of the lower layers reaching for it would close a cycle.
        ("server::storage", &["server::api"]),
        ("server::config", &["server::api"]),
        // The api surface asks downward (storage / config / platform / agent);
        // terminals, the CLI and the tool providers are none of its business.
        ("server::api", &["cli", "tui", "web", "git", "ansi"]),
        // Platform: the standard library and each other, nothing more. Anyone may ask it.
        (
            "server::platform",
            &[
                "server::agent",
                "server::api",
                "server::config",
                "server::storage",
                "cli",
                "tui",
                "web",
            ],
        ),
        // web: a self-contained tool provider.
        ("web", &["server::agent", "cli", "tui"]),
        // Leaves: the standard library only.
        ("git", &["cli", "server", "tui", "web"]),
        ("ansi", &["cli", "server", "tui", "web"]),
        // Backstop: no file under `server` may name a terminal (including the submodules just moved
        // in), and none may name the protocol layer — ipc stands between the two halves.
        ("server", &["tui", "ipc"]),
        // The protocol layer stands between the halves: it carries api's shapes, knows no UI.
        ("ipc", &["cli", "tui", "web", "git", "ansi"]),
        // The TUI is a front end: api only; never the engine's, storage's or config's innards.
        (
            "tui",
            &["server::agent", "server::storage", "server::config"],
        ),
    ];

    /// Every `.rs` file under `src/`, except sibling `tests.rs` files: a moved-out test half is not
    /// production code, and test code is allowed to reach across layers.
    fn rust_files(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "rs") {
                    // Sibling tests.rs: test code may reach across layers, so do not scan it.
                    if p.file_name().is_some_and(|n| n == "tests.rs") {
                        continue;
                    }
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    }

    /// A file's module path: `src/server/agent/tools.rs` → `server::agent::tools`,
    /// `src/server/ai/mod.rs` → `server::ai`.
    fn module_path(root: &Path, file: &Path) -> String {
        let rel = file.strip_prefix(root).unwrap_or(file);
        let mut segs: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        if let Some(last) = segs.last_mut() {
            *last = last.trim_end_matches(".rs").to_string();
        }
        if segs.last().is_some_and(|s| s == "mod") {
            segs.pop();
        }
        segs.join("::")
    }

    /// `crate::` reference paths in a source string. Comments are skipped — naming a layer is not
    /// depending on it — and a braced `use` is expanded so it cannot hide an edge.
    fn refs(src: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for line in src.lines().filter(|l| !l.trim_start().starts_with("//")) {
            let mut rest = line;
            while let Some(pos) = rest.find("crate::") {
                let after = &rest[pos + "crate::".len()..];
                if let Some(group) = after.strip_prefix('{') {
                    if let Some(close) = group.find('}') {
                        for item in group[..close].split(',') {
                            let item = item.trim();
                            if !item.is_empty() {
                                out.insert(item.replace(' ', ""));
                            }
                        }
                    }
                } else if let Some(chain) = ident_chain(after) {
                    out.insert(chain);
                }
                rest = after;
            }
        }
        out
    }

    /// Identifiers joined by `::`, up to the first non-path character.
    fn ident_chain(after: &str) -> Option<String> {
        let mut chain = String::new();
        let mut chars = after.chars().peekable();
        loop {
            let ident: String = chars
                .by_ref()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if ident.is_empty() {
                return None;
            }
            chain.push_str(&ident);
            let mut clone = chars.clone();
            if clone.next() == Some(':') && clone.next() == Some(':') {
                chars = clone;
                chain.push_str("::");
            } else {
                return Some(chain);
            }
        }
    }

    /// Does `path` fall under `prefix` (`server::session` ⊄ `server::session2`)?
    fn under(path: &str, prefix: &str) -> bool {
        path == prefix || path.starts_with(&format!("{prefix}::"))
    }

    /// A file's production half: `#[cfg(test)] mod …` blocks are dropped,
    /// because test code legitimately reaches across layers.
    fn production_source(text: &str) -> String {
        const MARK: &str = "#[cfg(test)]";
        let mut out = String::new();
        let mut rest = text;
        loop {
            let Some(pos) = rest.find(MARK) else {
                out.push_str(rest);
                return out;
            };
            out.push_str(&rest[..pos]);
            let after = &rest[pos + MARK.len()..];
            if !after.trim_start().starts_with("mod ") {
                // An item that only exists for tests (a test-only accessor):
                // keep the code, drop just the attribute from the scan.
                rest = after;
                continue;
            }
            // Skip the whole module by matching braces.
            let Some(open) = after.find('{') else {
                return out;
            };
            let mut depth = 0usize;
            let mut end = None;
            for (i, c) in after[open..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(open + i + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            match end {
                Some(e) => rest = &after[e..],
                None => return out,
            }
        }
    }

    /// `(file, source module, referenced path)` for every forbidden edge.
    fn violations(root: &Path) -> Vec<String> {
        let mut found = Vec::new();
        for file in rust_files(root) {
            let module = module_path(root, &file);
            let forbidden: Vec<&str> = FORBIDDEN
                .iter()
                .filter(|(m, _)| under(&module, m))
                .flat_map(|(_, f)| f.iter().copied())
                .collect();
            if forbidden.is_empty() {
                continue;
            }
            let src = production_source(&std::fs::read_to_string(&file).unwrap());
            for target in refs(&src) {
                if forbidden.iter().any(|f| under(&target, f)) {
                    found.push(format!(
                        "{}: {module} -> {target}",
                        file.strip_prefix(root).unwrap_or(&file).display()
                    ));
                }
            }
        }
        found
    }

    fn src_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    #[test]
    fn the_layer_matrix_holds() {
        // Downward dependencies only: leaves do not know what is above them, the engine does not know
        // the service driving it, and the service knows no terminal. Rules in FORBIDDEN.
        let root = src_root();
        let found = violations(&root);
        assert!(
            found.is_empty(),
            "分层被打破（只允许向下依赖）：\n  {}",
            found.join("\n  ")
        );
    }

    #[test]
    fn the_server_never_learns_about_the_terminal() {
        // The split rests on this: nothing under `src/server/` names a terminal type.
        let root = src_root();
        let found: Vec<String> = violations(&root)
            .into_iter()
            .filter(|v| v.starts_with("server/"))
            .collect();
        assert!(
            found.is_empty(),
            "server 不得依赖 tui（Telegram 桥接会作为第二个订阅端接入）：\n  {}",
            found.join("\n  ")
        );
    }

    #[test]
    fn the_stripper_ignores_test_modules_only() {
        // Guard the guard: if `production_source` swallowed production code the
        // rules above would pass vacuously.
        let text = "use crate::server::storage::Store;\n#[cfg(test)]\nmod tests {\n    use crate::tui::app::App;\n}\npub fn f() { let _ = crate::server::x(); }\n";
        let prod = production_source(text);
        assert!(prod.contains("crate::server::storage"), "{prod}");
        assert!(prod.contains("crate::server"), "生产代码不能被吞掉: {prod}");
        assert!(!prod.contains("crate::tui"), "测试模块必须被剔除: {prod}");
        // A test-only accessor (not a module) keeps the code after it.
        let text =
            "#[cfg(test)]\npub fn for_tests() {}\npub fn real() { let _ = crate::server::entry::x(); }\n";
        let prod = production_source(text);
        assert!(prod.contains("crate::server::entry"), "{prod}");
    }
}
