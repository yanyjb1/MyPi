//! Oversized tool outputs ("artifacts", 巨物) — out-of-band storage with
//! in-context references.
//!
//! A tool result above the threshold never enters the context whole. It
//! lands in the `artifacts` table and the context gets a one-line
//! placeholder:
//!
//! ```text
//! [工具输出共 51234 行，过大已存为巨物 #3。取用：#3（管道任意组合，如 #3 | grep pattern | head -50）]
//! ```
//!
//! The model pulls content back through **virtual files**: inside a bash
//! command, a standalone `#N` token resolves to a temp file holding
//! artifact N's content (written on demand, deleted when the command
//! exits). `#3 | grep me`, `head -100 #3`, `cat #3` — pipes work
//! naturally because #3 is just a path by the time bash sees it.
//!
//! Re-fetching can itself overflow (grep with no filter): that result
//! recurses through the same spill path and gets a fresh id. Termination
//! is guaranteed — the artifact set is finite, so the worst case is a
//! chain of placeholders, never an infinite loop.
//!
//! Rendering never sees artifact content: the placeholder IS the card
//! body. The block cache stays artifact-agnostic.

use anyhow::{Context as _, Result};

/// Spill threshold: results above either bound become artifacts.
/// 512 lines ≈ a screenful-of-screens; 256 KiB guards against very long
/// lines (minified JSON, base64 blobs) that stay under 512 lines.
pub const SPILL_LINES: usize = 512;
pub const SPILL_BYTES: usize = 256 * 1024;

/// One session's artifact spill channel. Clones share the same DB
/// connection handle; the session id pins ownership (artifacts of a
/// session are only resolvable within it).
#[derive(Clone)]
pub struct ArtifactStore {
    inner: std::sync::Arc<std::sync::Mutex<crate::server::store::Store>>,
    session_id: i64,
}

impl ArtifactStore {
    pub fn new(
        store: std::sync::Arc<std::sync::Mutex<crate::server::store::Store>>,
        session_id: i64,
    ) -> Self {
        Self {
            inner: store,
            session_id,
        }
    }

    /// Production constructor: the turn thread opens its **own** SQLite
    /// connection to the same WAL-mode DB (SQLite serializes writers, and
    /// artifact I/O is rare + short), so the persisted `Store` owned by
    /// the session never has to share its handle. `None` on open failure
    /// = degrade to a session without artifacts (same as before this
    /// existed).
    pub fn open(path: &std::path::Path, session_id: i64) -> Option<Self> {
        let st = crate::server::store::Store::open(path).ok()?;
        Some(Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(st)),
            session_id,
        })
    }

    /// Spill `content` into the table; returns (id, total_lines).
    pub fn spill(&self, tool_name: &str, content: &str) -> Result<(i64, usize)> {
        let lines = content.lines().count();
        let mut st = self.inner.lock().expect("store 锁中毒");
        let id = st.put_artifact(self.session_id, tool_name, content)?;
        Ok((id, lines))
    }

    /// Fetch an artifact owned by this session.
    pub fn fetch(&self, id: i64) -> Result<Option<(String, i64, String)>> {
        let st = self.inner.lock().expect("store 锁中毒");
        st.get_artifact(id, self.session_id)
    }
}

/// Does this tool output qualify as an artifact?
///
/// The bounds come from `config.yaml → tools.outputMaxLines/outputMaxBytes`
/// (defaults are [`SPILL_LINES`] / [`SPILL_BYTES`]): how much output stays part
/// of the conversation is a property of the conversation, and the same numbers
/// cap what `read` hands over before it points at the rest.
pub fn over_threshold(result: &str, limits: crate::server::ai::config::OutputLimits) -> bool {
    result.lines().count() > limits.max_lines || result.len() > limits.max_bytes
}

/// Turn an oversized result into its placeholder. `head` is a small
/// preview (first lines) so the card shows *something* before expansion.
pub fn placeholder(id: i64, tool_name: &str, total_lines: usize, result: &str) -> String {
    let preview: Vec<&str> = result.lines().take(3).collect();
    let bytes = result.len();
    let kb = bytes / 1024;
    format!(
        "[工具输出共 {total_lines} 行 / {kb}KB，过大已存为巨物 #{id}（{tool_name}）。\
取用：#{id}（等价文件内容，参与管道：#{id} | grep 关键词 | head -50；裸 #{id} 无过滤会再次巨物化）。前 3 行：\n{}]",
        preview.join("\n"),
    )
}

/// Resolve every standalone `#N` token in `cmd` to a temp file path with
/// the artifact's content. Returns the rewritten command plus the temp
/// directory to remove afterwards.
///
/// A token is "standalone" when it is delimited by shell-token boundaries
/// (start/string end, whitespace, quotes, or pipe/semicolon/redirect
/// operators). `grep '#3'`'s pattern, `foo#3`, `#3abc` do NOT resolve —
/// an id is always its own word.
pub fn resolve_refs(
    cmd: &str,
    artifacts: &ArtifactStore,
) -> Result<(String, Option<std::path::PathBuf>)> {
    let mut rewritten = String::with_capacity(cmd.len());
    let mut resolved: Vec<(i64, std::path::PathBuf)> = Vec::new();
    let bytes: Vec<char> = cmd.chars().collect();
    let is_boundary = |c: char| {
        c.is_whitespace() || matches!(c, '|' | ';' | '&' | '<' | '>' | '"' | '\'' | '(' | ')')
    };
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == '#' && (i == 0 || is_boundary(bytes[i - 1])) {
            // Parse digits.
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            let standalone = j < bytes.len() && is_boundary(bytes[j]) || j == bytes.len();
            if j > i + 1 && standalone {
                let id: i64 = bytes[i + 1..j]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .ok()
                    .context("artifact id")?;
                let path = materialize(artifacts, id, &mut resolved)?;
                // The ref stands for the artifact's CONTENT, so it rewrites
                // to `cat <file>`: a content producer any pipeline accepts.
                // Rewriting to the bare path made `#1 | grep` execute the
                // file (text, no exec bit) instead of feeding the pipe.
                rewritten.push_str(&format!("cat {}", path.display()));
                i = j;
                continue;
            }
        }
        rewritten.push(bytes[i]);
        i += 1;
    }
    // Unique temp dir for this command (first materialization created it).
    // No refs resolved → no cleanup target (None), NEVER the temp root.
    let dir = resolved
        .first()
        .map(|(_, p)| p.parent().expect("dir").to_path_buf());
    Ok((rewritten, dir))
}

/// Write artifact `id`'s content into the shared temp dir (once per id
/// per command) and return the file path.
fn materialize(
    artifacts: &ArtifactStore,
    id: i64,
    seen: &mut Vec<(i64, std::path::PathBuf)>,
) -> Result<std::path::PathBuf> {
    if let Some((_, p)) = seen.iter().find(|(k, _)| *k == id) {
        return Ok(p.clone());
    }
    let Some((name, _lines, content)) = artifacts.fetch(id)? else {
        anyhow::bail!("巨物 #{id} 不存在（或不属于当前会话）");
    };
    // Per-command dir; the caller removes it after the command exits.
    let dir = std::env::temp_dir().join(format!(
        "mypi-art-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).context("create artifact temp dir")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let file = dir.join(format!("{id}-{name}"));
    std::fs::write(&file, content).with_context(|| format!("write artifact #{id}"))?;
    seen.push((id, file.clone()));
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_store() -> ArtifactStore {
        let path = std::env::temp_dir().join(format!("mypi-art-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut st = crate::server::store::Store::open(&path).unwrap();
        st.create_session("t", "/tmp").unwrap();
        let arc = std::sync::Arc::new(std::sync::Mutex::new(st));
        ArtifactStore::new(arc, 1)
    }

    #[test]
    fn threshold_is_lines_or_bytes() {
        let d = crate::server::ai::config::OutputLimits::default();
        assert!(!over_threshold(&"行\n".repeat(100), d));
        assert!(over_threshold(&"行\n".repeat(600), d));
        // Long lines: 300 KB single line, under 512 lines but over bytes.
        assert!(over_threshold(&"x".repeat(SPILL_BYTES + 1), d));
        // …and the bounds are the configured ones, not the defaults.
        let small = crate::server::ai::config::OutputLimits {
            max_lines: 10,
            max_bytes: 1024,
        };
        assert!(over_threshold(&"行\n".repeat(11), small));
        assert!(!over_threshold(&"行\n".repeat(9), small));
    }

    #[test]
    fn placeholder_carries_id_and_preview() {
        let s = placeholder(7, "tree", 900, "a\nb\nc\nd\ne");
        assert!(s.contains("#7"));
        assert!(s.contains("900"));
        assert!(s.contains("a\nb\nc"));
        assert!(!s.contains("\nd\n"), "预览最多 3 行（第四行内容不得出现）");
    }

    #[test]
    fn refs_resolve_to_files_only_on_boundaries() {
        let art = mem_store();
        let (id, _) = art.spill("tree", "hello\nworld").unwrap();
        // Standalone: resolves.
        let (cmd, dir) = resolve_refs(&format!("grep me #{id} | head"), &art).unwrap();
        assert!(!cmd.contains(&format!("#{id}")));
        assert!(cmd.contains("grep me"));
        let dir = dir.expect("有引用必须有清理目录");
        assert!(dir.starts_with(std::env::temp_dir()));
        assert!(dir.to_string_lossy().contains("mypi-art-"));
        let _ = std::fs::remove_dir_all(dir);
        // Embedded: untouched.
        let (cmd, dir) = resolve_refs(&format!("echo 'x#{id}y'"), &art).unwrap();
        assert!(cmd.contains(&format!("x#{id}y")));
        assert!(dir.is_none(), "未命中引用不该有清理目标");
        // Repeated id: one materialization.
        let (cmd, dir) = resolve_refs(&format!("wc -l #{id}; grep o #{id}"), &art).unwrap();
        assert!(!cmd.contains(&format!("#{id}")), "引用必须被替换成路径");
        let dir = dir.expect("有引用必须有清理目录");
        assert!(
            std::fs::read_dir(&dir).unwrap().count() == 1,
            "同一 id 只落一个文件"
        );
        // Unknown id: hard error, the model sees the message.
        let e = resolve_refs("cat #999", &art).unwrap_err().to_string();
        assert!(e.contains("#999"));
    }
    #[test]
    fn open_path_constructor_roundtrips() {
        // Production constructor: own connection, same DB. Spill through
        // one handle, read back through another.
        let path = std::env::temp_dir().join(format!("mypi-art-open-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut setup = crate::server::store::Store::open(&path).unwrap();
        setup.create_session("t", "/tmp").unwrap();
        drop(setup);

        let a = ArtifactStore::open(&path, 1).expect("文件库必须能打开");
        let (id, lines) = a.spill("bash", "one\ntwo\nthree").unwrap();
        assert_eq!(lines, 3);

        let b = ArtifactStore::open(&path, 1).expect("第二连接也必须能打开");
        let (name, total, content) = b
            .inner
            .lock()
            .unwrap()
            .get_artifact(id, 1)
            .unwrap()
            .expect("跨连接可见");
        assert_eq!(name, "bash");
        assert_eq!(total, 3);
        assert!(content.contains("two"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn open_fails_cleanly_on_a_bogus_path() {
        assert!(
            ArtifactStore::open(std::path::Path::new("/nonexistent-root-xyz/a.db"), 1).is_none()
        );
    }
}
