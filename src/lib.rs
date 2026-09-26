//! mypi — a terminal coding agent: session service, agent loop, TUI.
//!
//! The module graph is a contract, and `mod architecture` below enforces it.

pub mod ansi;
pub mod cli;
pub mod git;
pub mod grouping;
pub mod oneshot;
pub mod server;
pub mod tui;
#[cfg(feature = "web")]
pub mod web;
pub mod xdg;

/// Executable layering rules — the module graph as a test, not a diagram.
///
/// Cargo cannot express any of this inside a single crate, so a forbidden
/// `crate::` reference has to fail something. It fails here.
///
/// Rules are **path-prefix pairs**, so they keep working after a file moves
/// into a submodule: `("server::agent", &["server::session"])` reads "nothing
/// under `server/agent/` may name the session service". Targets match by
/// prefix too (`server::session::Session` is caught by `server::session`).
#[cfg(test)]
mod architecture {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    /// `(module prefix, module prefixes it must never reference)`.
    ///
    /// Direction is "downward only": the leaves know nothing above them, the
    /// execution engine knows nothing about the service that drives it, and
    /// the service knows nothing about any terminal. Everything not listed is
    /// allowed — `tui` is a surface and may use anything it can reach.
    const FORBIDDEN: &[(&str, &[&str])] = &[
        // 协议数据模型：只认识自己 + 模型层的类型。
        (
            "server::entry",
            &[
                "server::agent",
                "server::session",
                "server::turn",
                "server::store",
                "server::compaction",
                "server::profile",
                "server::events",
                "cli",
                "tui",
                "web",
            ],
        ),
        // 持久化：只认识数据模型与平台胶水。
        (
            "server::store",
            &[
                "server::agent",
                "server::session",
                "server::turn",
                "server::compaction",
                "server::profile",
                "server::events",
                "cli",
                "tui",
                "web",
            ],
        ),
        // 模型网关：只管 HTTP 与线格式，不知道谁在用它。
        (
            "server::ai",
            &[
                "server::agent",
                "server::session",
                "server::turn",
                "server::compaction",
                "server::profile",
                "server::events",
                "cli",
                "tui",
                "web",
            ],
        ),
        // 执行引擎：跑工具，不认识界面，也不认识驱动它的会话服务。
        (
            "server::agent",
            &[
                "server::session",
                "server::turn",
                "server::compaction",
                "server::profile",
                "server::events",
                "cli",
                "tui",
            ],
        ),
        // 会话服务与回合线程：认识下面每一层，但绝不知道终端。
        ("server::session", &["cli", "tui"]),
        ("server::turn", &["cli", "tui"]),
        ("server::compaction", &["cli", "tui"]),
        ("server::profile", &["cli", "tui"]),
        ("server::events", &["cli", "tui"]),
        // 内存日志：只认识标准库，谁都能往里写，它不认识任何人。
        (
            "server::log",
            &[
                "server::ai",
                "server::agent",
                "server::session",
                "server::turn",
                "server::store",
                "server::entry",
                "server::events",
                "server::hub",
                "server::compaction",
                "server::profile",
                "cli",
                "tui",
                "web",
            ],
        ),
        // 多会话宿主：认识会话与下面的每一层，不知道终端，也不认识别的宿主。
        ("server::hub", &["cli", "tui", "web"]),
        // web 工具域：自成一体的工具提供方。
        (
            "web",
            &["server::agent", "server::session", "server::turn", "cli", "tui"],
        ),
        // 块分组：渲染与压缩共用的纯函数，只认识数据模型。
        (
            "grouping",
            &["server::agent", "server::session", "server::turn", "cli", "tui", "web"],
        ),
        // 平台胶水与叶子。
        ("xdg", &["cli", "server", "tui", "web"]),
        ("git", &["cli", "server", "tui", "web"]),
        ("ansi", &["cli", "server", "tui", "web"]),
        // 兜底：`server` 下任何文件都不许点名终端（含刚搬进来的子模块）。
        ("server", &["tui"]),
        // TUI 是 socket 前端：只许碰协议（wire）与会话工厂类型（hub 的
        // spec），不许摸库、网关、回合引擎或 daemon 本体。
        (
            "tui",
            &[
                "server::store",
                "server::daemon",
                "server::turn",
                "server::session",
                "server::agent",
            ],
        ),
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

    /// `crate::` reference paths in a source string.
    ///
    /// Comments are skipped: module docs *name* other layers all the time
    /// ("the TUI subscribes to this") and naming is not depending. Only code
    /// counts. `use crate::{a, b::c};` is expanded so a braced import cannot
    /// hide an edge.
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
        // 只允许向下依赖：叶子不认识它上面的人，执行引擎不认识驱动它的服务，
        // 服务不认识任何终端。规则见 FORBIDDEN（路径前缀对，所以文件搬家后仍然成立）。
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
        let text = "use crate::server::store::Store;\n#[cfg(test)]\nmod tests {\n    use crate::tui::app::App;\n}\npub fn f() { let _ = crate::server::x(); }\n";
        let prod = production_source(text);
        assert!(prod.contains("crate::server::store"), "{prod}");
        assert!(prod.contains("crate::server"), "生产代码不能被吞掉: {prod}");
        assert!(!prod.contains("crate::tui"), "测试模块必须被剔除: {prod}");
        // A test-only accessor (not a module) keeps the code after it.
        let text =
            "#[cfg(test)]\npub fn for_tests() {}\npub fn real() { let _ = crate::server::entry::x(); }\n";
        let prod = production_source(text);
        assert!(prod.contains("crate::server::entry"), "{prod}");
    }
}
