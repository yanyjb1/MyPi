//! File path completion — relative paths first; absolute paths and `~` both supported.
//!
//! Trigger rules (`candidate`): the word before the cursor contains `/`, or starts
//! with `~` / `.`. Ordinary typing is never interrupted; completion only fires when
//! you look like you are writing a path.
//!
//! Input form and insertion form are handled separately — the key point:
//! - typing `~/Doc` lists `$HOME` looking for `Doc*`;
//! - what goes **back into the input is still `~/Documents/`**, not the absolute
//!   expansion. The user wrote `~`; do not replace it with /home/xxx.

use std::path::{Path, PathBuf};

// How many candidates to show at most (the popup is small; more would be unreadable).
pub const MAX_VISIBLE: usize = 8;

// One completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    // Display name (files: name only; commands: `/model` with the slash).
    pub name: String,
    // Extra description (gray italic after the name for commands; empty for files).
    pub detail: String,
    // Directory flag (directories get a `/` suffix and remain completable).
    pub is_dir: bool,
    // Full text used to replace the word segment in the input.
    pub insert: String,
}

// Expand a leading `~` to the home directory. Only for **lookup**, never written back into the input.
pub fn expand_tilde(s: &str, home: &Path) -> PathBuf {
    if s == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return home.join(rest);
    }
    s.into()
}

// Extract the path-like word before the cursor.
//
// Returns `(start char index, word text)`; `None` when the trigger conditions fail.
pub fn candidate(text: &str, cursor: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());

    // Scan backward from the cursor to the nearest whitespace
    let mut start = cursor;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let word: String = chars[start..cursor].iter().collect();
    if word.is_empty() {
        return None;
    }

    // Mutual-exclusion rules:
    //   line start (start == 0) and beginning with `/` -> **command** completion only;
    //   otherwise -> **file path** completion only (words with `/`, `~`, `.` prefixes).
    // The two can never both hold; candidates never mix semantics.
    if start == 0 && word.starts_with('/') {
        return Some((start, word));
    }
    // A bare relative path (src/ma) is a path only when it is a **standalone word** —
    // preceded by whitespace (start == 0 or the previous char is whitespace; the scan
    // guarantees this). Otherwise it is part of a sentence and is not completed.
    // Explicit prefixes (./ ../ ~/ /) are exempt: they are path notation by themselves.
    let explicit = word.starts_with('.') || word.starts_with('~') || word.starts_with('/');
    if !explicit {
        // A bare relative path (src/ma) must be a **standalone word**: separated by
        // whitespace before it. start == 0 means nothing separates the word — it is the
        // beginning of a sentence (the scan folds glued prose into the word), which by
        // definition is not a path and is not completed.
        if start == 0 {
            return None;
        }
    }
    // Explicit notation (. ~ prefixes, or a leading /) is a path by itself; everything else needs a / to look like one
    let looks_like_path = word.contains('/') || word.starts_with('.') || word.starts_with('~');
    if !looks_like_path {
        return None;
    }
    Some((start, word))
}

// List candidates for `input`.
//
// `input` is the user's word (possibly with `~`); `cwd` resolves relative paths.
// An empty result means no candidates (the caller inserts a literal Tab or nothing).
//
// `at_line_start` comes from `candidate()`'s scan (is the word's start the line
// start) — only a **line-start** `/` is a command; `/tmp/...` elsewhere stays a file path.
pub fn complete(input: &str, at_line_start: bool, cwd: &Path, home: &Path) -> Vec<Completion> {
    if at_line_start && input.starts_with('/') {
        return complete_commands(input);
    }
    complete_files(input, cwd, home)
}

// Slash-command completion: `/mod` -> `/model`; `/model ` (with argument) -> model id list.
//
// Argument candidates are injected by the caller (the table's ArgKind resolves
// against runtime config); this function only handles static command names and
// prefix filtering.
pub fn complete_commands(input: &str) -> Vec<Completion> {
    // Exact hit: return this command alone, no longer names beside it (/q vs /quit).
    // This is the precondition for one-shot submission — /q + Enter should submit
    // directly, without confirming first.
    if let Some(c) = COMMANDS.iter().find(|c| c.name == input) {
        return vec![Completion {
            name: c.name.to_string(),
            detail: c.detail.to_string(),
            is_dir: false,
            insert: format!("{} ", c.name),
        }];
    }
    let mut out: Vec<Completion> = COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(input))
        .map(|c| Completion {
            name: c.name.to_string(),
            detail: c.detail.to_string(),
            is_dir: false,
            insert: format!("{} ", c.name),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// A slash command's argument shape: decides completion behavior and whether an argument exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgKind {
    // No argument: the command name is the final stage (`/q`, `/resume`).
    None,
    // Argument is a model id (`/model`, `/switch`): candidates come from runtime config.
    ModelId,
    // Argument is a file path (`/cdp`): reuses file path completion.
    Path,
}

// Static description of one slash command — the **single source of truth**.
//
// Dispatch (`App::submit`), completion candidates, and argument shape all read
// from this table. Adding a command touches exactly this place plus one `run_*` arm.
pub struct CommandSpec {
    pub name: &'static str,
    pub detail: &'static str,
    pub args: ArgKind,
}

// The command table: completion order and descriptions come from here.
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "/model",
        detail: "set default model (writes config.yaml)",
        args: ArgKind::ModelId,
    },
    CommandSpec {
        name: "/switch",
        detail: "switch session model (not persisted)",
        args: ArgKind::ModelId,
    },
    CommandSpec {
        name: "/name",
        detail: "name the session",
        args: ArgKind::None,
    },
    CommandSpec {
        name: "/cdp",
        detail: "change working directory (persisted)",
        args: ArgKind::Path,
    },
    CommandSpec {
        name: "/resume",
        detail: "resume a session of this project",
        args: ArgKind::None,
    },
    CommandSpec {
        name: "/compact",
        detail: "compress history into a checkpoint (optional focus)",
        args: ArgKind::None,
    },
    CommandSpec {
        name: "/profile",
        detail: "switch the system-prompt profile",
        args: ArgKind::None,
    },
    CommandSpec {
        name: "/q",
        detail: "quit (= /quit /exit)",
        args: ArgKind::None,
    },
    CommandSpec {
        name: "/quit",
        detail: "quit",
        args: ArgKind::None,
    },
    CommandSpec {
        name: "/exit",
        detail: "quit",
        args: ArgKind::None,
    },
];

// Look up a command by name.
pub fn lookup(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.name == name)
}

pub fn complete_files(input: &str, cwd: &Path, home: &Path) -> Vec<Completion> {
    // Split into the directory part + the prefix part to match
    let (dir_part, name_part) = match input.rfind('/') {
        Some(i) => (&input[..=i], &input[i + 1..]),
        None => ("", input),
    };

    // The directory part's **input form** (written back) vs its **actual path** (listed)
    let real_dir = if dir_part.is_empty() {
        cwd.to_path_buf()
    } else {
        let expanded = expand_tilde(dir_part, home);
        if expanded.is_absolute() {
            expanded
        } else {
            cwd.join(expanded)
        }
    };

    let Ok(entries) = std::fs::read_dir(&real_dir) else {
        return Vec::new();
    };

    let mut out: Vec<Completion> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Prefix filter, with dot-tolerant matching for dot-prefixed names: typing
        // "env" also hits ".env" (no hidden-file concept, by explicit decision).
        if !name.starts_with(name_part)
            && !(name.starts_with('.') && name[1..].starts_with(name_part))
        {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let suffix = if is_dir { "/" } else { "" };
        out.push(Completion {
            name: format!("{name}{suffix}"),
            detail: String::new(),
            is_dir,
            // The written-back text keeps the user's original form (`~/`, `./`, `/abs/`)
            insert: format!("{dir_part}{name}{suffix}"),
        });
    }

    // Directories first, then case-insensitive name order
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

// Common prefix of multiple candidates, for "extend a bit" (stopping at the first
// divergence).
//
// `None` means the candidates share no longer prefix.
pub fn common_prefix(items: &[Completion]) -> Option<String> {
    let first = items.first()?.insert.clone();
    let mut prefix = first;
    for it in &items[1..] {
        let mut n = 0usize;
        let a: Vec<char> = prefix.chars().collect();
        let b: Vec<char> = it.insert.chars().collect();
        while n < a.len() && n < b.len() && a[n] == b[n] {
            n += 1;
        }
        prefix = a[..n].iter().collect();
        if prefix.is_empty() {
            return None;
        }
    }
    Some(prefix)
}

// The popup's interaction result, executed by `app.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionAction {
    // Replace the input's `replace_from..cursor` segment with this text.
    Replace {
        from: usize,
        to: usize,
        text: String,
    },
    // No change needed (e.g. a single candidate already fully typed).
    None,
}

// Completion popup state.
//
// Lifecycle: Tab triggers -> opens when candidates exist -> ↑↓ move, Enter/Tab
// confirm, Esc cancels — or it closes on cursor movement or input change while
// live matching continues.
#[derive(Debug, Clone, Default)]
pub struct CompletionPopup {
    // Current candidates.
    items: Vec<Completion>,
    // Highlighted index.
    selected: usize,
    // Start char index of the replaced segment.
    from: usize,
    // Cursor position at trigger time (the replaced segment's end).
    to: usize,
    // Row height locked for this completion session.
    //
    // Candidate counts change with every keystroke; if the reserved area followed
    // them row by row the screen would jitter endlessly. The height locks once when
    // the popup opens (candidate count, capped at MAX); cascading and filtering
    // reuse it until the popup closes and unlocks.
    locked_height: Option<usize>,
}

impl CompletionPopup {
    pub fn is_open(&self) -> bool {
        !self.items.is_empty()
    }

    pub fn items(&self) -> &[Completion] {
        &self.items
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    // Row height locked for this completion session (None when closed).
    pub fn locked_height(&self) -> Option<usize> {
        self.locked_height
    }

    // The popup's displayed rows, with a scrolling window that keeps the selection
    // visible.
    //
    // `avail` is the popup's row capacity (from layout). Returns
    // `(name, selected, is_dir)` **top to bottom** — matching screen order, so ↑
    // really moves up and never inverts.
    pub fn visible(&self, avail: usize) -> Vec<(&str, &str, bool, bool)> {
        // Locked height wins: the reserved area occupies the rows from open time (empty
        // slots stay blank), so fewer candidates leave whitespace instead of collapsing
        // the block — no visual jumps.
        let avail = match self.locked_height {
            Some(h) => avail.min(h).min(MAX_VISIBLE),
            None => avail.min(MAX_VISIBLE).min(self.items.len()),
        };
        if avail == 0 {
            return Vec::new();
        }
        let start = self.window_start(avail);
        // The window must not exceed the remaining candidate count (locked height may exceed items.len())
        let end = (start + avail).min(self.items.len());
        self.items[start..end]
            .iter()
            .enumerate()
            .map(|(i, c)| {
                (
                    c.name.as_str(),
                    c.detail.as_str(),
                    start + i == self.selected,
                    c.is_dir,
                )
            })
            .collect()
    }

    // Scroll window start: guarantees the selection lands inside `[start, start + avail)`.
    //
    // No fancy "center the selection" behavior — visibility only. Fewer window moves
    // mean less eye strain; the cost is that the window jumps only at the edges.
    fn window_start(&self, avail: usize) -> usize {
        let max_start = self.items.len().saturating_sub(avail);
        if self.selected < avail {
            0
        } else {
            (self.selected - avail + 1).min(max_start)
        }
    }

    // Open the popup showing `items` (replacing the `from..to` segment).
    pub fn open(&mut self, items: Vec<Completion>, from: usize, to: usize) {
        // Closed -> open: a new completion session, lock the row height; already open
        // (cascading into the next tier, filtering): reuse the previous lock.
        let locked = self
            .locked_height
            .unwrap_or_else(|| items.len().min(MAX_VISIBLE));
        self.items = items;
        self.selected = 0;
        self.from = from;
        self.to = to;
        self.locked_height = Some(locked.max(1));
    }

    pub fn close(&mut self) {
        self.items.clear();
        self.selected = 0;
        self.locked_height = None;
    }

    // ↑ / ↓ move the highlight.
    //
    // **Stops at the ends, no wrapping.** The list is in natural order (first item
    // on top); wrapping would make ↑ at the top jump to the bottom, inverting the
    // sense of direction. Completion lists are short, so wrap-around is useless here.
    pub fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let last = self.items.len() - 1;
        let cur = self.selected as isize;
        self.selected = (cur + delta).clamp(0, last as isize) as usize;
    }

    // Confirm the highlighted candidate -> produce a replace action.
    //
    // With a single candidate it is used directly, ignoring `selected` (which is
    // always 0 then). The returned `Replace.to` is always `self.to` — the cursor
    // position from before this completion.
    pub fn accept(&mut self) -> CompletionAction {
        let Some(item) = self.items.get(self.selected).cloned() else {
            return CompletionAction::None;
        };
        let action = CompletionAction::Replace {
            from: self.from,
            to: self.to,
            text: item.insert.clone(),
        };
        // Directory completion continues downward, so the popup closes after the
        // choice and the user keeps typing; the next Tab re-matches against the new text.
        self.close();
        action
    }

    // Tab's "extend a bit": complete the input up to the candidates' common prefix.
    //
    // `current` is the **actual** word in the input. Deriving it by truncating
    // `items[0].insert` was wrong — candidates are completion **results**, not the
    // input, and the derived length never matched.
    //
    // Returns `None` when the common prefix is no longer than the input; the caller
    // then confirms the highlighted candidate instead.
    pub fn accept_common_prefix(&mut self, current: &str) -> Option<CompletionAction> {
        if self.items.len() < 2 {
            return None;
        }
        let prefix = common_prefix(&self.items)?;
        if prefix.chars().count() <= current.chars().count() {
            return None; // no longer prefix to extend
        }
        Some(CompletionAction::Replace {
            from: self.from,
            to: self.to,
            text: prefix,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mypi-path-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    // ---- trigger conditions ----

    #[test]
    fn candidate_triggers_on_slash() {
        let (start, word) = candidate("看下 src/tui/ma", 13).unwrap(); // CJK prose then a path
        assert_eq!(word, "src/tui/ma");
        assert_eq!(start, 3); // the CJK prefix is 3 characters
    }

    #[test]
    fn candidate_triggers_on_dot_and_tilde() {
        assert!(candidate("./a", 3).is_some());
        assert!(candidate("~/b", 3).is_some());
        assert!(candidate(".env", 4).is_some());
    }

    #[test]
    fn candidate_ignores_plain_words() {
        assert_eq!(candidate("hello wor", 9), None);
        assert_eq!(candidate("", 0), None);
        assert_eq!(candidate("看下 ", 3), None); // pure prose never completes
    }

    #[test]
    fn candidate_stops_at_whitespace() {
        let (start, word) = candidate("foo bar/baz", 11).unwrap();
        assert_eq!(word, "bar/baz");
        assert_eq!(start, 4);
    }

    // ---- directory listing ----

    #[test]
    fn lists_matching_entries_dirs_first() {
        let d = tmpdir("list");
        fs::create_dir(d.join("alpha")).unwrap();
        fs::write(d.join("almond.txt"), "x").unwrap();
        fs::write(d.join("beta.txt"), "x").unwrap();

        let got = complete("al", true, &d, &d);
        let names: Vec<&str> = got.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["alpha/", "almond.txt"],
            "directories first, al* filter only"
        );
        assert!(got[0].is_dir);
        assert!(!got[1].is_dir);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn no_hidden_file_filtering() {
        let d = tmpdir("hidden");
        fs::write(d.join(".env"), "x").unwrap();
        fs::write(d.join("env.txt"), "x").unwrap();

        let got = complete("", true, &d, &d);
        let names: Vec<&str> = got.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names.len(),
            2,
            "no hidden-file filtering, everything listed"
        );

        // Prefix matching works as usual: typing "." matches only .env (a name prefix), not a hidden rule
        let got = complete(".", true, &d, &d);
        let names: Vec<&str> = got.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec![".env"]);
        // Typing "e" yields both .env and env.txt — the point of ignoring hidden files
        let got = complete("e", true, &d, &d);
        let names: Vec<&str> = got.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names.len(), 2, "prefix e hits both .env and env.txt");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn subdirectory_path_keeps_user_notation() {
        let d = tmpdir("sub");
        fs::create_dir_all(d.join("src")).unwrap();
        fs::write(d.join("src/main.rs"), "x").unwrap();

        // The user wrote "src/ma" -> the written-back text is "src/main.rs", not an absolute path
        let got = complete("src/ma", true, &d, &d);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].insert, "src/main.rs");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn tilde_is_expanded_for_lookup_but_not_for_insert() {
        let home = tmpdir("home");
        fs::create_dir_all(home.join("Documents")).unwrap();
        fs::write(home.join("Downloads.txt"), "x").unwrap();

        let got = complete("~/Do", true, &home, &home);
        let inserts: Vec<&str> = got.iter().map(|c| c.insert.as_str()).collect();
        assert_eq!(inserts, vec!["~/Documents/", "~/Downloads.txt"]);
        assert!(
            !inserts[0].contains(&home.display().to_string()),
            "written back as ~, never expanded to an absolute path"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn absolute_path_works() {
        let d = tmpdir("abs");
        fs::write(d.join("only.txt"), "x").unwrap();
        let input = format!("{}/on", d.display());
        let got = complete(&input, false, &d, &d);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].insert, format!("{}/only.txt", d.display()));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn nonexistent_dir_yields_nothing_without_panic() {
        let d = tmpdir("nodir");
        assert!(complete("no/such/dir/x", true, &d, &d).is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn empty_prefix_lists_everything() {
        let d = tmpdir("all");
        fs::write(d.join("a.txt"), "x").unwrap();
        fs::write(d.join("b.txt"), "x").unwrap();
        assert_eq!(complete("", true, &d, &d).len(), 2);
        let _ = fs::remove_dir_all(&d);
    }

    // ---- common prefix ----

    #[test]
    fn common_prefix_of_siblings() {
        let items = vec![
            Completion {
                detail: String::new(),
                name: "main.rs".into(),
                is_dir: false,
                insert: "src/main.rs".into(),
            },
            Completion {
                detail: String::new(),
                name: "mainfest".into(),
                is_dir: false,
                insert: "src/mainfest".into(),
            },
        ];
        assert_eq!(common_prefix(&items), Some("src/main".into()));
    }

    #[test]
    fn common_prefix_none_on_divergence() {
        let items = vec![
            Completion {
                detail: String::new(),
                name: "a".into(),
                is_dir: false,
                insert: "a".into(),
            },
            Completion {
                detail: String::new(),
                name: "b".into(),
                is_dir: false,
                insert: "b".into(),
            },
        ];
        assert_eq!(common_prefix(&items), None);
    }

    #[test]
    fn common_prefix_single_item_is_itself() {
        let items = vec![Completion {
            name: "x".into(),
            detail: String::new(),
            is_dir: false,
            insert: "only/x".into(),
        }];
        assert_eq!(common_prefix(&items), Some("only/x".into()));
    }

    // ---- expand_tilde ----

    #[test]
    fn tilde_expansion() {
        let home = Path::new("/home/u");
        assert_eq!(expand_tilde("~", home), Path::new("/home/u"));
        assert_eq!(expand_tilde("~/a/b", home), Path::new("/home/u/a/b"));
        assert_eq!(
            expand_tilde("~x", home),
            Path::new("~x"),
            "~user is not expanded"
        );
        assert_eq!(expand_tilde("src", home), Path::new("src"));
    }

    // ---- popup state machine ----

    fn cand(name: &str, is_dir: bool) -> Completion {
        Completion {
            name: name.to_string(),
            detail: String::new(),
            is_dir,
            insert: name.to_string(),
        }
    }

    #[test]
    fn popup_opens_and_closes() {
        let mut p = CompletionPopup::default();
        assert!(!p.is_open());
        p.open(vec![cand("a", false)], 0, 1);
        assert!(p.is_open());
        p.close();
        assert!(!p.is_open());
    }

    #[test]
    fn selection_clamps_at_both_ends() {
        let mut p = CompletionPopup::default();
        p.open(
            vec![cand("a", false), cand("b", false), cand("c", false)],
            0,
            1,
        );
        assert_eq!(p.selected(), 0);
        // ↑ at the top: stop, no wrapping
        p.move_selection(-1);
        assert_eq!(p.selected(), 0);
        p.move_selection(1);
        assert_eq!(p.selected(), 1);
        p.move_selection(1);
        p.move_selection(1);
        assert_eq!(p.selected(), 2, "stops at the end, no wrap-around");
    }

    #[test]
    fn selection_stays_within_items() {
        let mut p = CompletionPopup::default();
        let items: Vec<Completion> = (0..20).map(|i| cand(&format!("f{i}"), false)).collect();
        p.open(items, 0, 1);
        for _ in 0..50 {
            p.move_selection(1);
        }
        assert_eq!(p.selected(), 19, "highlight never passes the last item");
    }

    #[test]
    fn window_scrolls_to_keep_selection_visible() {
        let mut p = CompletionPopup::default();
        let items: Vec<Completion> = (0..20).map(|i| cand(&format!("f{i}"), false)).collect();
        p.open(items, 0, 1);
        let avail = MAX_VISIBLE;
        // Selection inside the window
        let vis = p.visible(avail);
        assert_eq!(vis.len(), avail);
        assert!(vis.iter().any(|(_, _, sel, _)| *sel));
        // After many downward steps the selection must still be visible
        for step in 0..20 {
            p.move_selection(1);
            let vis = p.visible(avail);
            assert!(
                vis.iter().any(|(_, _, sel, _)| *sel),
                "selection escaped the window after step {step}"
            );
        }
    }

    #[test]
    fn visible_is_in_natural_order() {
        // First item on top: ↑ really means up
        let mut p = CompletionPopup::default();
        p.open(vec![cand("first", false), cand("second", false)], 0, 1);
        let vis = p.visible(2);
        assert_eq!(vis[0].0, "first");
        assert_eq!(vis[1].0, "second");
        assert!(vis[0].2, "first entry highlighted by default");
    }

    #[test]
    fn accept_replaces_the_candidate_span() {
        let mut p = CompletionPopup::default();
        p.open(vec![cand("main.rs", false)], 4, 6); // "src/ma|" → from=4, to=6
        let action = p.accept();
        assert_eq!(
            action,
            CompletionAction::Replace {
                from: 4,
                to: 6,
                text: "main.rs".into()
            }
        );
        assert!(!p.is_open(), "popup closes after confirm");
    }

    #[test]
    fn accept_on_empty_popup_is_none() {
        let mut p = CompletionPopup::default();
        assert_eq!(p.accept(), CompletionAction::None);
    }

    #[test]
    fn accept_uses_highlighted_item() {
        let mut p = CompletionPopup::default();
        p.open(vec![cand("alpha", true), cand("beta", false)], 0, 2);
        p.move_selection(1);
        let action = p.accept();
        assert_eq!(
            action,
            CompletionAction::Replace {
                from: 0,
                to: 2,
                text: "beta".into()
            }
        );
    }

    #[test]
    fn common_prefix_fills_to_divergence() {
        let mut p = CompletionPopup::default();
        // Input "sr", candidates src/ and srclib/ (common prefix "src", stops there)
        p.open(
            vec![
                Completion {
                    detail: String::new(),
                    name: "src/".into(),
                    is_dir: true,
                    insert: "src/".into(),
                },
                Completion {
                    detail: String::new(),
                    name: "srcx".into(),
                    is_dir: false,
                    insert: "srcx".into(),
                },
            ],
            0,
            2,
        );
        let action = p.accept_common_prefix("sr").unwrap();
        assert_eq!(
            action,
            CompletionAction::Replace {
                from: 0,
                to: 2,
                text: "src".into()
            }
        );
    }

    #[test]
    fn common_prefix_is_noop_when_already_complete() {
        let mut p = CompletionPopup::default();
        // Input already "src", common prefix "src" -> no action
        p.open(
            vec![
                Completion {
                    detail: String::new(),
                    name: "src/".into(),
                    is_dir: true,
                    insert: "src/".into(),
                },
                Completion {
                    detail: String::new(),
                    name: "srcx".into(),
                    is_dir: false,
                    insert: "srcx".into(),
                },
            ],
            0,
            3,
        );
        assert_eq!(p.accept_common_prefix("src"), None);
    }

    #[test]
    fn common_prefix_needs_multiple_candidates() {
        let mut p = CompletionPopup::default();
        p.open(vec![cand("only", false)], 0, 2);
        assert_eq!(
            p.accept_common_prefix("on"),
            None,
            "single candidate goes through accept, not common prefix"
        );
    }

    #[test]
    fn visible_caps_at_max() {
        let mut p = CompletionPopup::default();
        let items: Vec<Completion> = (0..20).map(|i| cand(&format!("f{i}"), false)).collect();
        p.open(items, 0, 1);
        assert_eq!(p.visible(MAX_VISIBLE).len(), MAX_VISIBLE);
        // First entry highlighted
        assert!(p.visible(MAX_VISIBLE)[0].2);
        assert!(!p.visible(MAX_VISIBLE)[1].2);
    }

    // ---- command / argument completion ----

    #[test]
    fn command_completion_prefix_matches() {
        let got = complete_commands("/mod");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "/model");
        assert_eq!(got[0].insert, "/model ");
        assert!(
            got[0].detail.contains("default model"),
            "detail present: {}",
            got[0].detail
        );
    }

    #[test]
    fn command_completion_lists_all_on_slash() {
        let got = complete_commands("/");
        let names: Vec<String> = got.iter().map(|c| c.name.clone()).collect();
        assert_eq!(
            names,
            vec![
                "/cdp", "/compact", "/exit", "/model", "/name", "/profile", "/q", "/quit",
                "/resume", "/switch"
            ],
            "按名字排序"
        );
    }

    #[test]
    fn candidate_line_start_slash_is_command_not_path() {
        // Leading / -> command semantics (candidate does not discriminate; from==0 carries the flag)
        let (from, word) = candidate("/mod", 4).unwrap();
        assert_eq!(from, 0);
        assert_eq!(word, "/mod");
    }

    #[test]
    fn candidate_mid_line_slash_stays_path() {
        // / after a space -> file path semantics
        let (from, word) = candidate("看 src/ma", 9).unwrap();
        assert_eq!(word, "src/ma");
        assert!(from > 0, "非行首");
    }

    #[test]
    fn files_never_fire_on_line_start_slash_word() {
        // A line-start "/" word would list the root directory if treated as a file path
        // — absolutely not allowed. complete's at_line_start dispatch guarantees this.
        let cwd = std::env::temp_dir();
        let got = complete("/mod", true, &cwd, &cwd);
        assert!(!got.is_empty());
        assert!(got[0].name.starts_with("/"), "应是命令候选");
        assert!(
            !got.iter().any(|c| c.name.ends_with("tmp/")),
            "不该混入文件"
        );
    }

    #[test]
    fn command_cascade_after_accept() {
        // "/mo" single candidate -> accept writes back "/model " (trailing space)
        let got = complete_commands("/mo");
        assert_eq!(got.len(), 1);
        assert!(
            got[0].insert.ends_with(' '),
            "命令 insert 带尾随空格，触发参数级联"
        );
    }

    #[test]
    fn exact_file_match_means_leaf() {
        // Final-stage detection: single candidate == current word -> complete still returns it (app closes the popup on equality)
        let d = tmpdir("leaf");
        fs::write(d.join("zstd.h"), "x").unwrap();
        let got = complete_files("zstd.h", &d, &d);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].insert, "zstd.h");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn directory_completed_still_lists_children() {
        // After the directory completes ("src" -> "src/") the next tier must list; that is what makes cascading work
        let d = tmpdir("dir");
        fs::create_dir_all(d.join("src").join("tui")).unwrap();
        let got = complete_files("src/", &d, &d);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].insert, "src/tui/");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn height_locks_on_open_and_survives_filtering() {
        let mut p = CompletionPopup::default();
        p.open(
            vec![
                cand("a", false),
                cand("b", false),
                cand("c", false),
                cand("d", false),
            ],
            0,
            1,
        );
        assert_eq!(p.locked_height(), Some(4), "打开时锁定为候选数");

        // Cascading into the next tier: 8 candidates, locked value unchanged
        p.open(
            (0..8).map(|i| cand(&format!("n{i}/"), true)).collect(),
            0,
            5,
        );
        assert_eq!(p.locked_height(), Some(4), "级联沿用锁定值");
        // The visible window is capped at MAX_VISIBLE=8; locked at 4 -> only 4 shown
        assert_eq!(p.visible(99).len(), 4, "锁定行高截断可见行数");
    }

    #[test]
    fn height_unlocks_on_close() {
        let mut p = CompletionPopup::default();
        p.open(vec![cand("a", false), cand("b", false)], 0, 1);
        p.close();
        assert_eq!(p.locked_height(), None);
        // Reopened: re-locked to the new candidate count
        p.open(vec![cand("x", false)], 0, 1);
        assert_eq!(p.locked_height(), Some(1));
    }

    #[test]
    fn locked_height_capped_at_max_visible() {
        let mut p = CompletionPopup::default();
        p.open(
            (0..20).map(|i| cand(&format!("f{i}"), false)).collect(),
            0,
            1,
        );
        assert_eq!(
            p.locked_height(),
            Some(MAX_VISIBLE),
            "锁定值封顶 MAX_VISIBLE"
        );
    }

    #[test]
    fn inline_sentence_with_slash_does_not_trigger() {
        // A glued-in-sentence pseudo path does not trigger — the word boundary fails
        assert_eq!(candidate("看src/ma", 8), None, "CJK 粘连");
        assert_eq!(candidate("abcsrc/ma", 9), None, "字母粘连同样不算词");
        // Only a standalone word (whitespace before it) triggers
        let (from, word) = candidate("看下 src/ma", 11).unwrap();
        assert_eq!(word, "src/ma");
        assert!(from > 0);
    }

    #[test]
    fn explicit_prefixes_still_trigger_mid_word() {
        // Explicit path notation ignores the word-boundary rule (they usually have a space anyway)
        assert!(candidate("./a", 3).is_some());
        assert!(candidate("../b", 4).is_some());
        assert!(candidate("~/c", 3).is_some());
        assert!(candidate(".env", 4).is_some(), "隐藏文件名");
    }

    #[test]
    fn locked_height_larger_than_items_does_not_panic() {
        let mut p = CompletionPopup::default();
        // Locked at 6 rows (/usr root candidates); cascading filters down to 1
        p.open(
            (0..6).map(|i| cand(&format!("d{i}/"), true)).collect(),
            0,
            1,
        );
        assert_eq!(p.locked_height(), Some(6));
        p.open(vec![cand("only/", true)], 0, 5);
        assert_eq!(p.locked_height(), Some(6), "级联沿用锁定");
        let v = p.visible(99);
        assert_eq!(v.len(), 1, "只返回实际候选数，不越界");
    }

    #[test]
    fn exact_command_hit_lists_only_itself() {
        // /q exact hit: one-shot — Enter submits directly, no popup
        let got = complete_commands("/q");
        assert_eq!(got.len(), 1);
        // Under the /quit prefix both /q and /quit list (still typing; normal listing)
        let got = complete_commands("/qu");
        let names: Vec<String> = got.iter().map(|c| c.name.clone()).collect();
        assert_eq!(names, vec!["/quit"], "non-exact input still prefix-filters");
    }
}
