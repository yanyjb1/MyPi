//! Completion candidates: which level the cursor is on, and what that level offers.
//!
//! Command names come from `crate::commands::COMMANDS`, model and profile names
//! from the view's pools, paths from one directory level scanned here (cached).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::commands::{self, Arg, Candidate};
use crate::server::api::types::CandidatePools;

/// How many candidates the popup shows at most.
pub const MAX_VISIBLE: usize = 8;

/// How many entries one directory listing may carry.
const MAX_ENTRIES: usize = 2000;

/// What a candidate is, for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateKind {
    /// A command name: rendered with a selection marker and a description column.
    Command,
    /// Everything else: no marker, selection shows by color.
    Plain,
}

/// One completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// What this candidate is (decides the popup's row style).
    pub kind: CandidateKind,
    /// The display name (files: name plus a `/` when a directory).
    pub name: String,
    /// Char indices in `name` the typed text matched, ascending.
    pub hits: Vec<usize>,
    /// The gray text after the name.
    pub detail: String,
    /// Directories are rendered as directories.
    pub is_dir: bool,
    /// The text that replaces the word this level occupies.
    pub insert: String,
}

/// Where a level's data comes from.
pub struct Look<'a> {
    /// The session's working directory: relative paths resolve from it.
    pub cwd: &'a Path,
    /// `~` expands to it for lookups only.
    pub home: &'a Path,
    /// Model and profile names, asked for once at startup.
    pub pools: Option<&'a CandidatePools>,
}

/// Directory listings kept between keystrokes: one `read_dir` per directory.
#[derive(Default)]
pub struct Scan {
    dirs: HashMap<PathBuf, Vec<Entry>>,
}

/// One name of a directory listing.
struct Entry {
    name: String,
    lower: String,
    is_dir: bool,
}

impl Scan {
    fn entries(&mut self, dir: &Path) -> &[Entry] {
        self.dirs
            .entry(dir.to_path_buf())
            .or_insert_with(|| list(dir))
    }
}

fn list(dir: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Entry> = read
        .flatten()
        .take(MAX_ENTRIES)
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            Entry {
                lower: name.to_lowercase(),
                name,
                is_dir: e.file_type().map(|t| t.is_dir()).unwrap_or(false),
            }
        })
        .collect();
    // Directories first, then case-insensitive name; ranks sort stably on top.
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.lower.cmp(&b.lower)));
    out
}

/// What the cursor's word offers.
pub struct Found {
    /// The word the cursor sits in, as typed.
    pub word: String,
    /// Where that word starts (char index).
    pub from: usize,
    /// Candidates for it.
    pub items: Vec<Completion>,
}

/// Candidates for the word the cursor sits in. Empty = nothing to offer.
pub fn suggest(line: &str, cursor: usize, look: &Look, scan: &mut Scan) -> Found {
    let chars: Vec<char> = line.chars().collect();
    let cursor = cursor.min(chars.len());
    if cursor == 0 {
        return Found {
            word: String::new(),
            from: 0,
            items: Vec::new(),
        };
    }
    let before: String = chars[..cursor].iter().collect();
    let word = word_before(&before).to_string();
    let from = before.chars().count() - word.chars().count();
    // A command line: the input starts with `/` and the cursor is on its first line.
    let items = if chars[0] == '/' && !before.contains('\n') {
        command_line(&before, &word, look, scan)
    } else {
        path_items(&word, look, scan)
    };
    Found { word, from, items }
}

/// The word the cursor sits in: the trailing non-whitespace run of `before`.
fn word_before(before: &str) -> &str {
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    &before[start..]
}

/// A line that starts with `/`: the command name first, then its argument positions.
fn command_line(before: &str, word: &str, look: &Look, scan: &mut Scan) -> Vec<Completion> {
    let complete: Vec<&str> = before.split_whitespace().collect();
    let ends_space = before.chars().last().is_some_and(|c| c.is_whitespace());
    let index = if ends_space {
        complete.len()
    } else {
        complete.len().saturating_sub(1)
    };
    if index == 0 {
        return command_items(word);
    }
    let Some(meta) = complete.first().and_then(|name| commands::lookup(name)) else {
        return Vec::new();
    };
    match meta.arg(index - 1) {
        Some(Arg::Model) => name_items(pools(look, true), word),
        Some(Arg::Profile) => name_items(pools(look, false), word),
        Some(Arg::Path) => path_items(word, look, scan),
        Some(Arg::Number | Arg::Text) | None => Vec::new(),
    }
}

/// The two name pools, or an empty run when the server has not answered yet.
fn pools(look: &Look, models: bool) -> &[Candidate] {
    let Some(pools) = look.pools else {
        return &[];
    };
    if models {
        &pools.models
    } else {
        &pools.profiles
    }
}

/// Command names (and aliases) starting at the word.
fn command_items(word: &str) -> Vec<Completion> {
    let mut out: Vec<(Rank, Completion)> = Vec::new();
    for meta in commands::COMMANDS {
        let names = std::iter::once(meta.name).chain(meta.aliases.iter().copied());
        for name in names {
            let Some((rank, hits)) = matches(word, name, false) else {
                continue;
            };
            // Commands with arguments take a trailing space, so the hand lands on
            // the first argument position.
            let insert = if meta.args.is_empty() {
                name.to_string()
            } else {
                format!("{name} ")
            };
            out.push((
                rank,
                Completion {
                    kind: CandidateKind::Command,
                    name: name.to_string(),
                    hits,
                    detail: meta.detail.to_string(),
                    is_dir: false,
                    insert,
                },
            ));
        }
    }
    out.sort_by(|(_, a), (_, b)| a.name.cmp(&b.name));
    out.sort_by_key(|(rank, _)| *rank);
    out.into_iter().map(|(_, c)| c).collect()
}

/// Model or profile names starting at the word.
fn name_items(names: &[Candidate], word: &str) -> Vec<Completion> {
    let mut out: Vec<(Rank, Completion)> = Vec::new();
    for item in names {
        let Some((rank, hits)) = matches(word, &item.value, false) else {
            continue;
        };
        out.push((
            rank,
            Completion {
                kind: CandidateKind::Plain,
                name: item.value.clone(),
                hits,
                detail: item.detail.clone(),
                is_dir: false,
                insert: item.value.clone(),
            },
        ));
    }
    out.sort_by(|(_, a), (_, b)| a.name.cmp(&b.name));
    out.sort_by_key(|(rank, _)| *rank);
    out.into_iter().map(|(_, c)| c).collect()
}

/// Paths under the word's directory. Nothing when the word asks for no path.
fn path_items(word: &str, look: &Look, scan: &mut Scan) -> Vec<Completion> {
    if !path_trigger(word, look, scan) {
        return Vec::new();
    }
    let (dir_part, prefix) = match word.rfind('/') {
        Some(i) => (&word[..=i], &word[i + 1..]),
        None => ("", word),
    };
    let real = if dir_part.is_empty() {
        look.cwd.to_path_buf()
    } else {
        let expanded = expand_tilde(dir_part, look.home);
        if expanded.is_absolute() {
            expanded
        } else {
            look.cwd.join(expanded)
        }
    };
    let mut out: Vec<(Rank, Completion)> = Vec::new();
    for entry in scan.entries(&real) {
        let Some((rank, hits)) = matches(prefix, entry.name.as_str(), true) else {
            continue;
        };
        let suffix = if entry.is_dir { "/" } else { "" };
        out.push((
            rank,
            Completion {
                kind: CandidateKind::Plain,
                name: format!("{}{suffix}", entry.name),
                hits,
                detail: String::new(),
                is_dir: entry.is_dir,
                insert: format!("{dir_part}{}{suffix}", entry.name),
            },
        ));
    }
    out.sort_by_key(|(rank, _)| *rank);
    out.into_iter().map(|(_, c)| c).collect()
}

/// Whether a path completion was asked for: `./`, `../`, `~/`, a `/`-rooted word
/// longer than the slash, or a first segment that names a directory of the workspace.
fn path_trigger(word: &str, look: &Look, scan: &mut Scan) -> bool {
    if word.starts_with("./") || word.starts_with("../") || word.starts_with("~/") {
        return true;
    }
    if word.starts_with('/') {
        return word.chars().count() > 1;
    }
    match word.split_once('/') {
        Some((first, _)) if !first.is_empty() => {
            let dir = look.cwd.to_path_buf();
            scan.entries(&dir)
                .iter()
                .any(|e| e.is_dir && e.name == first)
        }
        _ => false,
    }
}

/// Expand a leading `~` to the home directory. For lookups only: what goes back
/// into the input keeps the user's own form.
pub fn expand_tilde(s: &str, home: &Path) -> PathBuf {
    if s == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return home.join(rest);
    }
    s.into()
}

/// How the typed text hit a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    Prefix,
    Fuzzy,
}

/// A hit of `typed` in `name`: the accent positions, and how it was hit.
///
/// `dots` also matches past one leading dot, so `env` reaches `.env`.
fn matches(typed: &str, name: &str, dots: bool) -> Option<(Rank, Vec<usize>)> {
    let count = typed.chars().count();
    let typed = typed.to_lowercase();
    let name_lower = name.to_lowercase();
    let bare = if dots {
        typed.strip_prefix('.').unwrap_or(&typed).to_string()
    } else {
        typed.clone()
    };
    let mut forms: Vec<&str> = vec![name_lower.as_str()];
    if dots && name_lower.starts_with('.') {
        forms.push(&name_lower[1..]);
    }
    for form in forms {
        if form.starts_with(&typed) || (dots && form.starts_with(&bare)) {
            return Some((Rank::Prefix, (0..count).collect()));
        }
    }
    is_subsequence(&typed, &name_lower)
        .or_else(|| is_bag_match(&typed, &name_lower))
        .map(|hits| (Rank::Fuzzy, hits))
}

/// Positions of `needle`'s characters in `hay`, in order.
fn is_subsequence(needle: &str, hay: &str) -> Option<Vec<usize>> {
    let mut hits = Vec::new();
    let mut it = hay.char_indices();
    for n in needle.chars() {
        loop {
            match it.next() {
                Some((i, h)) if h == n => {
                    hits.push(hay[..i].chars().count());
                    break;
                }
                Some(_) => continue,
                None => return None,
            }
        }
    }
    Some(hits)
}

/// Positions of `needle`'s characters in `hay`, order ignored: covers swaps such
/// as `mian` for `main.rs`.
fn is_bag_match(needle: &str, hay: &str) -> Option<Vec<usize>> {
    let mut hay_chars: Vec<char> = hay.chars().collect();
    let mut hits = Vec::new();
    for n in needle.chars() {
        let pos = hay_chars.iter().position(|&h| h == n)?;
        hits.push(pos);
        // Consume the occurrence: repeated letters need repeated characters.
        hay_chars[pos] = '\0';
    }
    hits.sort_unstable();
    Some(hits)
}

/// The column the candidate list aligns to: right after the last `/` of the word.
pub fn anchor_column(text: &str, cursor: usize, cursor_col: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let before: String = chars[..cursor].iter().collect();
    let word = word_before(&before);
    if chars.first() == Some(&'/') && !before.contains('\n') {
        return cursor_col.saturating_sub(crate::ui::tui::text::display_width(word));
    }
    let segment = match word.rfind('/') {
        Some(i) => &word[i + 1..],
        None => word,
    };
    cursor_col.saturating_sub(crate::ui::tui::text::display_width(segment))
}

/// The candidates' common prefix, for "extend a bit". `None` = no longer prefix.
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

/// What confirming a candidate means: a replacement in the input, or nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionAction {
    /// Replace the input's `from..to` segment with this text.
    Replace {
        from: usize,
        to: usize,
        text: String,
    },
    /// No change needed.
    None,
}

/// The popup's state: candidates, highlight, replaced segment, locked height.
#[derive(Debug, Clone, Default)]
pub struct CompletionPopup {
    items: Vec<Completion>,
    selected: usize,
    /// Start char index of the replaced segment.
    from: usize,
    /// Where the replaced segment ends (the cursor at trigger time).
    to: usize,
    /// Row height locked for this completion session.
    locked_height: Option<usize>,
}

/// One row of the popup, as the renderer needs it.
pub struct VisibleRow<'a> {
    pub kind: CandidateKind,
    pub name: &'a str,
    /// Char indices in `name` the typed text matched.
    pub hits: &'a [usize],
    pub detail: &'a str,
    pub selected: bool,
    pub is_dir: bool,
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

    /// Row height locked for this session; `None` when closed.
    pub fn locked_height(&self) -> Option<usize> {
        self.locked_height
    }

    /// The rows to draw, windowed so the selection stays visible. Top to bottom,
    /// matching screen order.
    pub fn visible(&self, avail: usize) -> Vec<VisibleRow<'_>> {
        let avail = match self.locked_height {
            Some(h) => avail.min(h).min(MAX_VISIBLE),
            None => avail.min(MAX_VISIBLE).min(self.items.len()),
        };
        if avail == 0 {
            return Vec::new();
        }
        let start = self.window_start(avail);
        let end = (start + avail).min(self.items.len());
        self.items[start..end]
            .iter()
            .enumerate()
            .map(|(i, c)| VisibleRow {
                kind: c.kind,
                name: c.name.as_str(),
                hits: c.hits.as_slice(),
                detail: c.detail.as_str(),
                selected: start + i == self.selected,
                is_dir: c.is_dir,
            })
            .collect()
    }

    /// Window start: the selection always lands inside `[start, start + avail)`.
    fn window_start(&self, avail: usize) -> usize {
        let max_start = self.items.len().saturating_sub(avail);
        if self.selected < avail {
            0
        } else {
            (self.selected - avail + 1).min(max_start)
        }
    }

    /// Open showing `items`, replacing the `from..to` segment.
    pub fn open(&mut self, items: Vec<Completion>, from: usize, to: usize) {
        // A fresh session locks the height; filtering and descending reuse it.
        let locked = self
            .locked_height
            .unwrap_or_else(|| items.len().min(MAX_VISIBLE));
        // The same batch keeps the highlight; a different batch starts at the top.
        let same = self.items.len() == items.len()
            && self
                .items
                .iter()
                .zip(items.iter())
                .all(|(a, b)| a.insert == b.insert && a.name == b.name);
        let selected = if same {
            self.selected.min(items.len().saturating_sub(1))
        } else {
            0
        };
        self.items = items;
        self.selected = selected;
        self.from = from;
        self.to = to;
        self.locked_height = Some(locked.max(1));
    }

    pub fn close(&mut self) {
        self.items.clear();
        self.selected = 0;
        self.locked_height = None;
    }

    /// ↑ / ↓ move the highlight, stopping at the ends.
    pub fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let last = self.items.len() - 1;
        let cur = self.selected as isize;
        self.selected = (cur + delta).clamp(0, last as isize) as usize;
    }

    /// Confirm the highlighted candidate.
    pub fn accept(&mut self) -> CompletionAction {
        let Some(item) = self.items.get(self.selected).cloned() else {
            return CompletionAction::None;
        };
        let action = CompletionAction::Replace {
            from: self.from,
            to: self.to,
            text: item.insert.clone(),
        };
        self.close();
        action
    }

    /// Extend the input to the candidates' common prefix, if that is longer than
    /// what is already typed.
    pub fn accept_common_prefix(&mut self, current: &str) -> Option<CompletionAction> {
        if self.items.len() < 2 {
            return None;
        }
        let prefix = common_prefix(&self.items)?;
        if prefix.chars().count() <= current.chars().count() {
            return None;
        }
        Some(CompletionAction::Replace {
            from: self.from,
            to: self.to,
            text: prefix,
        })
    }
}
