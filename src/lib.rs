//! mypi — a terminal coding agent: session service, agent loop, TUI.
//!
//! The module graph is a contract, and `mod architecture` below enforces it.

pub mod agent;
pub mod ai;
pub mod ansi;
pub mod cli;
pub mod entry;
pub mod git;
pub mod grouping;
pub mod server;
pub mod store;
pub mod tui;
#[cfg(feature = "web")]
pub mod web;
pub mod xdg;

/// Executable layering rules — the module graph as a test, not a diagram.
///
/// Cargo cannot express any of this inside a single crate, so a forbidden
/// `crate::` reference has to fail something. It fails here.
#[cfg(test)]
mod architecture {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    /// `(module, modules it must never reference)`.
    ///
    /// Direction is "downward only": the leaves know nothing above them, the
    /// agent layer knows nothing about surfaces, and the session service knows
    /// nothing about a terminal. Everything not listed is allowed — `tui` is the
    /// surface and may use anything.
    const FORBIDDEN: &[(&str, &[&str])] = &[
        // Leaves: data model, persistence, config, platform glue.
        ("entry", &["agent", "cli", "server", "tui", "web"]),
        ("grouping", &["agent", "cli", "server", "tui", "web"]),
        ("ansi", &["agent", "cli", "server", "tui", "web"]),
        ("git", &["agent", "cli", "server", "tui", "web"]),
        ("xdg", &["agent", "cli", "server", "tui", "web"]),
        ("store", &["agent", "cli", "server", "tui", "web"]),
        ("ai", &["agent", "cli", "server", "tui", "web"]),
        // The agent loop runs tools; it has no idea what draws them.
        ("agent", &["cli", "server", "tui"]),
        // The web domain is a self-contained tool provider.
        ("web", &["agent", "cli", "server", "tui"]),
        // The session service is a service: surfaces subscribe to it, never the
        // other way round.
        ("server", &["tui"]),
    ];

    /// Every `.rs` file under `src/`.
    fn rust_files(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "rs") {
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    }

    /// The top-level module a file belongs to (`src/server/turn.rs` → `server`).
    fn top_module(root: &Path, file: &Path) -> String {
        let rel = file.strip_prefix(root).unwrap_or(file);
        match rel.components().count() {
            0 | 1 => rel.file_stem().unwrap().to_string_lossy().to_string(),
            _ => rel
                .components()
                .next()
                .unwrap()
                .as_os_str()
                .to_string_lossy()
                .to_string(),
        }
    }

    /// `crate::<module>` references in a source string.
    ///
    /// Full-line comments are skipped: module docs *name* other layers all the
    /// time ("the TUI subscribes to this") and naming is not depending. Only
    /// code counts.
    fn refs(src: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for line in src.lines().filter(|l| !l.trim_start().starts_with("//")) {
            let mut rest = line;
            while let Some(pos) = rest.find("crate::") {
                let after = &rest[pos + "crate::".len()..];
                let ident: String = after
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !ident.is_empty() {
                    out.insert(ident);
                }
                rest = after;
            }
        }
        out
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

    /// `(file, source module, referenced module)` for every forbidden edge.
    fn violations(root: &Path) -> Vec<String> {
        let mut found = Vec::new();
        for file in rust_files(root) {
            let top = top_module(root, &file);
            let Some((_, forbidden)) = FORBIDDEN.iter().find(|(m, _)| *m == top) else {
                continue;
            };
            let src = production_source(&std::fs::read_to_string(&file).unwrap());
            for target in refs(&src) {
                if forbidden.contains(&target.as_str()) {
                    found.push(format!(
                        "{}: {top} -> {target}",
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
        let found = violations(&src_root());
        assert!(
            found.is_empty(),
            "分层被打破（只允许向下依赖）：\n  {}",
            found.join("\n  ")
        );
    }

    #[test]
    fn the_server_never_learns_about_the_terminal() {
        // The rule the whole split rests on: `server` is a service that any
        // surface can drive — a Telegram bridge, a headless driver, the TUI. It
        // stays true only while nothing under `src/server/` names a terminal
        // type, so this is asserted on its own with a name that says why.
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
        let text = "use crate::store::Store;\n#[cfg(test)]\nmod tests {\n    use crate::tui::app::App;\n}\npub fn f() { let _ = crate::server::x(); }\n";
        let prod = production_source(text);
        assert!(prod.contains("crate::store"), "{prod}");
        assert!(prod.contains("crate::server"), "生产代码不能被吞掉: {prod}");
        assert!(!prod.contains("crate::tui"), "测试模块必须被剔除: {prod}");
        // A test-only accessor (not a module) keeps the code after it.
        let text =
            "#[cfg(test)]\npub fn for_tests() {}\npub fn real() { let _ = crate::entry::x(); }\n";
        let prod = production_source(text);
        assert!(prod.contains("crate::entry"), "{prod}");
    }
}
