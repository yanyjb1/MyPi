//! The command vocabulary: a name, its argument kinds, one line of text, one candidate shape.
//! Each front end declares its own table; this file never names a command.

/// Where the candidates for one argument position come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg {
    /// A filesystem path.
    Path,
    /// A model, addressed as `<provider>:<id>`.
    Model,
    /// A profile name.
    Profile,
    /// A number. No candidates.
    Number,
    /// Free text. No candidates.
    Text,
}

/// One command's declaration. The field order is fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandMeta {
    /// The name with its leading slash.
    pub name: &'static str,
    /// Alternate spellings of the same command.
    pub aliases: &'static [&'static str],
    /// The argument positions, in order.
    pub args: &'static [Arg],
    /// One line for help and completion.
    pub detail: &'static str,
}

impl CommandMeta {
    /// The argument position `level` (0-based); `None` = past the last one.
    pub fn arg(&self, level: usize) -> Option<Arg> {
        self.args.get(level).copied()
    }
}

/// One completion candidate for one level.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    /// The text this candidate inserts for this level.
    pub value: String,
    /// What the popup shows next to the value.
    pub detail: String,
}

/// Look a name (with its slash) up by name or alias.
pub fn lookup<'a>(table: &'a [CommandMeta], name: &str) -> Option<&'a CommandMeta> {
    table
        .iter()
        .find(|c| same(c.name, name) || c.aliases.iter().any(|a| same(a, name)))
}

/// The word the cursor sits in: where it starts (char index) and what it holds.
///
/// A word is a run of non-space characters, so `src/main.rs` is one word here even though
/// word delete stops at the slash.
pub fn word(line: &str, cursor: usize) -> (usize, String) {
    let chars: Vec<char> = line.chars().collect();
    let cursor = cursor.min(chars.len());
    let mut start = cursor;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    (start, chars[start..cursor].iter().collect())
}

/// The index of the word being typed on a line split by whitespace.
pub fn word_index(before: &str) -> usize {
    let words = before.split_whitespace().count();
    match before.chars().last() {
        Some(last) if last.is_whitespace() => words,
        _ => words.saturating_sub(1),
    }
}

/// A word shaped like a path: anything holding a slash except the bare root.
pub fn path_shaped(word: &str) -> bool {
    word.contains('/') && word != "/"
}

/// Split one line into a known command and its argument text.
pub fn split<'t, 'i>(
    table: &'t [CommandMeta],
    input: &'i str,
) -> Option<(&'t CommandMeta, &'i str)> {
    let input = input.trim();
    let rest = input.strip_prefix('/')?;
    if rest.is_empty() {
        return None;
    }
    let (word, args) = match rest.split_once(char::is_whitespace) {
        Some((word, args)) => (word, args.trim()),
        None => (rest, ""),
    };
    table
        .iter()
        .find(|c| bare(c.name) == Some(word) || c.aliases.iter().any(|a| bare(a) == Some(word)))
        .map(|c| (c, args))
}

fn bare(name: &str) -> Option<&str> {
    name.strip_prefix('/')
}

fn same(a: &str, b: &str) -> bool {
    a.as_bytes() == b.as_bytes()
}

/// The table rules, checked while the table is built.
pub const fn valid(table: &[CommandMeta]) -> bool {
    let mut i = 0;
    while i < table.len() {
        let c = table[i];
        if c.name.is_empty() || c.name.as_bytes()[0] != b'/' {
            return false;
        }
        if c.detail.is_empty() {
            return false;
        }
        let mut k = 0;
        while k < c.args.len() {
            if k + 1 < c.args.len() && matches!(c.args[k], Arg::Text) {
                return false;
            }
            k += 1;
        }
        let mut j = 0;
        while j < c.aliases.len() {
            if !slash(c.aliases[j]) {
                return false;
            }
            let mut m = 0;
            while m < table.len() {
                if eq(c.aliases[j], table[m].name) {
                    return false;
                }
                let mut n = 0;
                while n < table[m].aliases.len() {
                    if (m < i || (m == i && n < j)) && eq(c.aliases[j], table[m].aliases[n]) {
                        return false;
                    }
                    n += 1;
                }
                m += 1;
            }
            j += 1;
        }
        let mut m = 0;
        while m < i {
            if eq(c.name, table[m].name) {
                return false;
            }
            m += 1;
        }
        i += 1;
    }
    true
}

const fn slash(name: &str) -> bool {
    !name.is_empty() && name.as_bytes()[0] == b'/'
}

const fn eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
