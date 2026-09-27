//! Syntax highlighting for code blocks and tool payloads, on syntect.
//!
//! Ported from omp `crates/pi-natives/src/highlight.rs`: syntect parses
//! source into scope stacks; a matcher table maps each scope to one of
//! **11 semantic categories** (comment/keyword/function/variable/string/
//! number/type/operator/punctuation/inserted/deleted), and the category
//! indexes a theme color (`syntaxXxx` tokens) — omp's exact matching
//! order, so classification agrees with what omp shows.
//!
//! Two entry points:
//! - [`highlight`] — color one code block given a syntax hint;
//! - [`language_for`] — pick that hint from a file path or a tool name.
//!
//! Everything is lazily built once per process: `SyntaxSet::load_defaults_*`
//! deserializes a few MB of definitions, so it must never run per frame.
//! Unknown languages degrade to plain text — a tool card must always render.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use ratatui::text::{Line, Span};
use syntect::parsing::ScopeStack;
use syntect::parsing::{ParseState, Scope, ScopeStackOp, SyntaxSet};

use super::theme::{HistoryTheme, Token};

// ---------------------------------------------------------------------------
// semantic categories — omp's indexes
// ---------------------------------------------------------------------------

const COMMENT: usize = 0;
const KEYWORD: usize = 1;
const FUNCTION: usize = 2;
const VARIABLE: usize = 3;
const STRING: usize = 4;
const NUMBER: usize = 5;
const TYPE: usize = 6;
const OPERATOR: usize = 7;
const PUNCTUATION: usize = 8;
const INSERTED: usize = 9;
const DELETED: usize = 10;
const NOMATCH: usize = usize::MAX;

/// Category → 历史区色号。走 `HistoryTheme` 快照，和卡片同一份，
/// 换主题时一帧内全变。
fn category_token(idx: usize) -> Token {
    match idx {
        COMMENT => Token::SyntaxComment,
        KEYWORD => Token::SyntaxKeyword,
        FUNCTION => Token::SyntaxFunction,
        VARIABLE => Token::SyntaxVariable,
        STRING => Token::SyntaxString,
        NUMBER => Token::SyntaxNumber,
        TYPE => Token::SyntaxType,
        OPERATOR => Token::SyntaxOperator,
        PUNCTUATION => Token::SyntaxPunctuation,
        INSERTED => Token::DiffAdded,
        DELETED => Token::DiffRemoved,
        _ => Token::AssistantText,
    }
}

// ---------------------------------------------------------------------------
// scope matchers — omp's table & precedence, verbatim
// ---------------------------------------------------------------------------

struct ScopeMatchers {
    comment: Scope,
    // string
    string: Scope,
    constant_character: Scope,
    meta_string: Scope,
    // number
    constant_numeric: Scope,
    constant_integer: Scope,
    constant: Scope,
    // keyword
    keyword: Scope,
    storage_type: Scope,
    storage_modifier: Scope,
    // function
    entity_name_function: Scope,
    support_function: Scope,
    meta_function_call: Scope,
    variable_function: Scope,
    // type
    entity_name_type: Scope,
    support_type: Scope,
    support_class: Scope,
    entity_name_class: Scope,
    entity_name_struct: Scope,
    entity_name_enum: Scope,
    entity_name_interface: Scope,
    entity_name_trait: Scope,
    // operator
    keyword_operator: Scope,
    punctuation_accessor: Scope,
    // punctuation
    punctuation: Scope,
    // variable
    variable: Scope,
    entity_name: Scope,
    meta_path: Scope,
    // diff
    markup_inserted: Scope,
    markup_deleted: Scope,
    meta_diff_header: Scope,
    meta_diff_range: Scope,
}

impl ScopeMatchers {
    fn new() -> Self {
        macro_rules! s {
            ($n:literal) => {
                Scope::new($n).expect("static scope name")
            };
        }
        Self {
            comment: s!("comment"),
            string: s!("string"),
            constant_character: s!("constant.character"),
            meta_string: s!("meta.string"),
            constant_numeric: s!("constant.numeric"),
            constant_integer: s!("constant.integer"),
            constant: s!("constant"),
            keyword: s!("keyword"),
            storage_type: s!("storage.type"),
            storage_modifier: s!("storage.modifier"),
            entity_name_function: s!("entity.name.function"),
            support_function: s!("support.function"),
            meta_function_call: s!("meta.function-call"),
            variable_function: s!("variable.function"),
            entity_name_type: s!("entity.name.type"),
            support_type: s!("support.type"),
            support_class: s!("support.class"),
            entity_name_class: s!("entity.name.class"),
            entity_name_struct: s!("entity.name.struct"),
            entity_name_enum: s!("entity.name.enum"),
            entity_name_interface: s!("entity.name.interface"),
            entity_name_trait: s!("entity.name.trait"),
            keyword_operator: s!("keyword.operator"),
            punctuation_accessor: s!("punctuation.accessor"),
            punctuation: s!("punctuation"),
            variable: s!("variable"),
            entity_name: s!("entity.name"),
            meta_path: s!("meta.path"),
            markup_inserted: s!("markup.inserted"),
            markup_deleted: s!("markup.deleted"),
            meta_diff_header: s!("meta.diff.header"),
            meta_diff_range: s!("meta.diff.range"),
        }
    }

    /// omp's `compute_scope_color`: innermost-first prefix matching with a
    /// fixed precedence. The order below is load-bearing — comment wins
    /// over everything, diff categories next, then strings/numbers, then
    /// keywords, functions, types, operators, punctuation, variables.
    fn classify(&self, s: Scope) -> usize {
        if self.comment.is_prefix_of(s) {
            return COMMENT;
        }
        if self.markup_inserted.is_prefix_of(s) {
            return INSERTED;
        }
        if self.markup_deleted.is_prefix_of(s) {
            return DELETED;
        }
        if self.meta_diff_header.is_prefix_of(s) || self.meta_diff_range.is_prefix_of(s) {
            return KEYWORD;
        }
        if self.string.is_prefix_of(s)
            || self.constant_character.is_prefix_of(s)
            || self.meta_string.is_prefix_of(s)
        {
            return STRING;
        }
        if self.constant_numeric.is_prefix_of(s) || self.constant_integer.is_prefix_of(s) {
            return NUMBER;
        }
        if self.keyword.is_prefix_of(s)
            || self.storage_type.is_prefix_of(s)
            || self.storage_modifier.is_prefix_of(s)
        {
            return KEYWORD;
        }
        if self.entity_name_function.is_prefix_of(s)
            || self.support_function.is_prefix_of(s)
            || self.meta_function_call.is_prefix_of(s)
            || self.variable_function.is_prefix_of(s)
        {
            return FUNCTION;
        }
        if self.entity_name_type.is_prefix_of(s)
            || self.support_type.is_prefix_of(s)
            || self.support_class.is_prefix_of(s)
            || self.entity_name_class.is_prefix_of(s)
            || self.entity_name_struct.is_prefix_of(s)
            || self.entity_name_enum.is_prefix_of(s)
            || self.entity_name_interface.is_prefix_of(s)
            || self.entity_name_trait.is_prefix_of(s)
        {
            return TYPE;
        }
        if self.keyword_operator.is_prefix_of(s) || self.punctuation_accessor.is_prefix_of(s) {
            return OPERATOR;
        }
        if self.punctuation.is_prefix_of(s) {
            return PUNCTUATION;
        }
        if self.variable.is_prefix_of(s)
            || self.entity_name.is_prefix_of(s)
            || self.meta_path.is_prefix_of(s)
        {
            return VARIABLE;
        }
        if self.constant.is_prefix_of(s) {
            return NUMBER; // generic constant -> number (omp)
        }
        NOMATCH
    }
}

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static MATCHERS: LazyLock<ScopeMatchers> = LazyLock::new(ScopeMatchers::new);

// ---------------------------------------------------------------------------
// 预热：把「编译一条语法」的百毫秒开销挡在渲染帧之外
// ---------------------------------------------------------------------------
//
// syntect 的语法正则是**首次用到时才编译**的（实测：bash 178ms、rust 112ms、
// python 131ms，之后每次 0.3~0.6ms）。这个开销如果落在渲染帧里，resume 时
// 第一张带代码的卡片就会把整屏卡住——首帧卡顿的成因就在这里，与哪一类
// 卡片无关。
//
// 所以：**谁的就绪状态谁记**。`highlight()` 先问一声 ready，没就绪就先出
// 无色文本（一帧都不等），同时在后台线程里编译；编译完置位，下一帧走正常的
// 延迟补色路径把颜色刷上去。预热只针对**转录里真实出现过的语言**，整个库
// 里没用到的语法永远不编译。

/// 已经编译好正则的语法（按 `SyntaxSet` 里的名字）。
static READY: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// 正在后台编译的（避免同一个语言排两次队）。
static WARMING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

fn syntax_name(lang: &str) -> Option<String> {
    find_syntax(lang).map(|s| s.name.clone())
}

/// 这条语言现在就敢用吗？没就绪的返回 `false`，调用方出无色。
fn is_ready(lang: &str) -> bool {
    let Some(name) = syntax_name(lang) else {
        // 找不到这条语法：本来也没得编译，当就绪处理（走 plain 兜底）。
        return true;
    };
    READY.lock().expect("预热表锁中毒").contains(&name)
}

/// 把一条语法交给后台线程编译。同一条语言只会有一个任务在跑。
///
/// 编译方式就是**真喂一行代码**：syntect 的正则是在第一次 parse 该语法时
/// 才构造的，没有"只编译不解析"的公开入口，所以这里用一个最小的样本来触发
/// 它。目标不是产出，是把那次构造挪出渲染帧。
pub fn warm(lang: &str) {
    let Some(name) = syntax_name(lang) else { return };
    {
        let ready = READY.lock().expect("预热表锁中毒");
        if ready.contains(&name) {
            return;
        }
    }
    {
        let mut warming = WARMING.lock().expect("预热表锁中毒");
        if !warming.insert(name.clone()) {
            return; // 已经在编译了
        }
    }
    std::thread::spawn(move || {
        // 真正触发编译：一条最小样本过一遍 parse。
        if let Some(syn) = SYNTAXES.syntaxes().iter().find(|s| s.name == name) {
            let mut st = ParseState::new(syn);
            let _ = st.parse_line("x\n", &SYNTAXES);
        }
        READY.lock().expect("预热表锁中毒").insert(name.clone());
        WARMING.lock().expect("预热表锁中毒").remove(&name);
        crate::tui::theme::request_repaint();
    });
}

/// 按给定顺序预热（前面的优先）。转录里靠下的语言离视线更近，调用方据此排序。
pub fn warm_in_order<'a>(langs: impl IntoIterator<Item = &'a str>) {
    for l in langs {
        warm(l);
    }
}

/// 这条语言就绪了没（诊断/测试用）。
#[doc(hidden)]
pub fn ready(lang: &str) -> bool {
    is_ready(lang)
}

/// 预热并**等到真的就绪**。给"断言颜色"的测试用。
///
/// 生产路径从不等待：没就绪就出无色、后台编译、下一帧补色。测试要断言
/// "上色对不对"，就必须先让它就绪。
#[doc(hidden)]
pub fn warm_and_wait_for_test(lang: &str) {
    warm(lang);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !is_ready(lang) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Map a scope stack to a category, walking innermost → outermost.
fn scope_to_category(stack: &ScopeStack) -> usize {
    for s in stack.as_slice().iter().rev() {
        let idx = MATCHERS.classify(*s);
        if idx != NOMATCH {
            return idx;
        }
    }
    NOMATCH
}

// ---------------------------------------------------------------------------
// language resolution
// ---------------------------------------------------------------------------

/// omp's alias table: (aliases, syntect syntax name).
const LANG_ALIASES: &[(&[&str], &str)] = &[
    (&["ts", "mts", "cts", "typescript"], "TypeScript"),
    (&["js", "jsx", "javascript", "mjs", "cjs"], "JavaScript"),
    (&["py", "python"], "Python"),
    (&["rb", "ruby"], "Ruby"),
    (&["rs", "rust"], "Rust"),
    (&["go", "golang"], "Go"),
    (&["java"], "Java"),
    (&["kt", "kotlin"], "Java"),
    (&["c", "h"], "C"),
    (&["cpp", "cc", "cxx", "c++", "hpp", "hxx", "hh"], "C++"),
    (&["cs", "csharp"], "C#"),
    (&["php"], "PHP"),
    (&["sh", "bash", "zsh", "shell"], "Bash"),
    (&["ps1", "powershell"], "PowerShell"),
    (&["html", "htm", "vue", "svelte"], "HTML"),
    (&["css"], "CSS"),
    (&["scss"], "SCSS"),
    (&["sass"], "Sass"),
    (&["less"], "LESS"),
    (&["json"], "JSON"),
    (&["yaml", "yml"], "YAML"),
    (&["toml"], "TOML"),
    (&["xml"], "XML"),
    (&["md", "markdown"], "Markdown"),
    (&["sql"], "SQL"),
    (&["lua"], "Lua"),
    (&["r"], "R"),
    (&["scala"], "Scala"),
    (&["ex", "exs"], "Elixir"),
    (&["erl"], "Erlang"),
    (&["hs", "haskell"], "Haskell"),
    (&["ml", "ocaml"], "OCaml"),
    (&["vim"], "VimL"),
    (&["graphql", "gql"], "GraphQL"),
    (&["proto", "protobuf"], "Protocol Buffers"),
    (&["tf", "hcl", "terraform"], "Terraform"),
    (&["dockerfile", "docker", "containerfile"], "Dockerfile"),
    (&["makefile", "make", "just", "justfile"], "Makefile"),
    (&["ini", "cfg", "conf", "config", "properties"], "INI"),
    (&["diff", "patch"], "Diff"),
];

fn find_syntax(lang: &str) -> Option<&'static syntect::parsing::SyntaxReference> {
    let ss = &*SYNTAXES;
    if let Some(syn) = ss.find_syntax_by_token(lang) {
        return Some(syn);
    }
    if let Some(syn) = ss.find_syntax_by_extension(lang) {
        return Some(syn);
    }
    let target = LANG_ALIASES
        .iter()
        .find(|(aliases, _)| aliases.iter().any(|a| lang.eq_ignore_ascii_case(a)))
        .map(|(_, t)| *t)?;
    ss.find_syntax_by_name(target)
        .or_else(|| ss.find_syntax_by_token(target))
}

// ---------------------------------------------------------------------------
// highlight — syntect parse → category → theme color
// ---------------------------------------------------------------------------

/// Highlight `code` using the given syntax name (`"bash"`, `"python"`, …).
///
/// The result is themed through the global theme's `syntaxXxx` tokens, so
/// a mid-session theme switch recolors new renders with no rebuild.
/// Unknown syntaxes and parse hiccups fall back to unstyled lines — a
/// tool card must always render.
pub fn highlight(code: &str, language: Option<&str>, t: &HistoryTheme) -> Vec<Line<'static>> {
    let Some(lang) = language else {
        return plain(code);
    };
    // 没预热过就先出无色，并把编译排到后台——绝不在这帧里等那几百毫秒。
    // 编译完会敲一次重绘，下一帧这里就返回带色的。
    if !is_ready(lang) {
        warm(lang);
        return plain(code);
    }
    let Some(syntax) = find_syntax(lang) else {
        return plain(code);
    };

    let mut parse_state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out: Vec<Line<'static>> = Vec::new();

    for line in syntect::util::LinesWithEndings::from(code) {
        let Ok(ops) = parse_state.parse_line(line, &SYNTAXES) else {
            out.push(Line::from(line.trim_end_matches('\n').to_string()));
            continue;
        };
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut prev_end = 0usize;
        for (offset, op) in ops {
            let offset = offset.min(line.len());
            if offset > prev_end {
                let text = &line[prev_end..offset];
                let cat = scope_to_category(&stack);
                spans.push(spans_for(t, cat, text));
                prev_end = offset;
            }
            match op {
                ScopeStackOp::Push(scope) => stack.push(scope),
                ScopeStackOp::Pop(count) => {
                    for _ in 0..count {
                        stack.pop();
                    }
                }
                _ => {}
            }
        }
        if prev_end < line.len() {
            let text = &line[prev_end..];
            let cat = scope_to_category(&stack);
            spans.push(spans_for(t, cat, text));
        }
        // Trim the trailing newline the syntect line iteration keeps.
        if let Some(last) = spans.last_mut() {
            let mut s = last.content.to_string();
            while s.ends_with('\n') || s.ends_with('\r') {
                s.pop();
            }
            last.content = s.into();
            if last.content.is_empty() {
                spans.pop();
            }
        }
        out.push(Line::from(spans));
    }
    out
}

/// A code block being colored a few lines at a time.
///
/// The reason this exists: syntect's cost is dominated by **line length**, not
/// line count. Measured on the dirty bench block, one 10k-character line costs
/// ~100ms and one 200×CJK line ~112ms, while 46 ordinary lines cost 3.5ms
/// together. Coloring a whole fenced segment to paint a 21-row viewport
/// therefore paid for every wide line in it, whether or not any of them was on
/// screen — the measured 200ms frame peaks were two such lines.
///
/// So the segment is colored **line by line**, and the caller decides how many
/// lines one frame may pay for. `ParseState` carries the lexer context across
/// lines and is cheap to clone (a small `Vec` of context ids), so the walk can
/// stop anywhere and resume on the next frame without re-parsing what it
/// already did — the colors are identical to coloring the segment in one go,
/// because the parse still happens in source order from line 0.
pub struct HighlightCursor {
    state: ParseState,
    stack: ScopeStack,
    /// Source lines still to color, in order.
    lines: std::vec::IntoIter<String>,
    /// Index of `lines`' next element == how many source lines are done.
    done: usize,
    t: HistoryTheme,
}

impl HighlightCursor {
    /// Start coloring `code` as `lang`. Returns `None` when the language is
    /// unknown — the caller then owes plain rows, which cost nothing.
    pub fn new(code: &str, lang: Option<&str>, t: &HistoryTheme) -> Option<Self> {
        let lang = lang?;
        let syntax = find_syntax(lang)?;
        Some(Self {
            state: ParseState::new(syntax),
            stack: ScopeStack::new(),
            lines: split_keep_empty(code),
            done: 0,
            t: *t,
        })
    }

    pub fn done(&self) -> usize {
        self.done
    }

    /// Total source lines — known up front, so a caller can budget.
    pub fn total(&self) -> usize {
        self.done + self.lines.len()
    }

    /// True when there is nothing left to color.
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 0
    }

    /// Byte length of the next source line, without coloring it.
    ///
    /// A peek, not a pop: `IntoIter` has no cheap "front" access, so this is
    /// served from a clone-free `as_slice` — worth it because the caller uses
    /// it to decide whether the next line is cheap enough to skip over or
    /// expensive enough to defer until it is actually on screen.
    pub fn peek_len(&self) -> usize {
        self.lines.as_slice().first().map(|l| l.len()).unwrap_or(0)
    }

    /// Color up to `budget` more lines, **ignoring** the parse cost of each
    /// one. Returns the styled rows produced, in source order.
    ///
    /// A line can be arbitrarily expensive, so the budget alone cannot bound
    /// the time — the caller pairs this with a wall-clock check via
    /// [`Self::is_empty`] and stops issuing calls. One line is always colored
    /// per call when lines remain, so progress is guaranteed.
    pub fn take(&mut self, budget: usize) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        for _ in 0..budget {
            let Some(line) = self.lines.next() else { break };
            self.done += 1;
            out.push(color_one_line(
                &line,
                &mut self.state,
                &mut self.stack,
                &self.t,
            ));
        }
        out
    }
}

/// Split source into lines the way syntect's `LinesWithEndings` does: every
/// line keeps its terminator, and a trailing newline does **not** produce a
/// final empty element. Kept in sync with [`highlight`] by construction — the
/// row count must match or the geometry the cache holds becomes a lie.
fn split_keep_empty(code: &str) -> std::vec::IntoIter<String> {
    let mut v: Vec<String> = syntect::util::LinesWithEndings::from(code)
        .map(|l| l.to_string())
        .collect();
    if v.is_empty() {
        v.push(String::new());
    }
    v.into_iter()
}

/// Color one already-line-split source line, advancing the shared state.
fn color_one_line(
    line: &str,
    state: &mut ParseState,
    stack: &mut ScopeStack,
    t: &HistoryTheme,
) -> Line<'static> {
    let Ok(ops) = state.parse_line(line, &SYNTAXES) else {
        return Line::from(line.trim_end_matches(['\n', '\r']).to_string());
    };
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut prev_end = 0usize;
    for (offset, op) in ops {
        let offset = offset.min(line.len());
        if offset > prev_end {
            let text = &line[prev_end..offset];
            let cat = scope_to_category(stack);
            spans.push(spans_for(t, cat, text));
            prev_end = offset;
        }
        match op {
            ScopeStackOp::Push(scope) => stack.push(scope),
            ScopeStackOp::Pop(count) => {
                for _ in 0..count {
                    stack.pop();
                }
            }
            _ => {}
        }
    }
    if prev_end < line.len() {
        let text = &line[prev_end..];
        let cat = scope_to_category(stack);
        spans.push(spans_for(t, cat, text));
    }
    // Trim the trailing newline the syntect line iteration keeps.
    if let Some(last) = spans.last_mut() {
        let mut s = last.content.to_string();
        while s.ends_with('\n') || s.ends_with('\r') {
            s.pop();
        }
        last.content = s.into();
        if last.content.is_empty() {
            spans.pop();
        }
    }
    Line::from(spans)
}

/// Build one styled span: colored when the category resolves, plain text
/// otherwise (NOMATCH inherits the card's default look).
fn spans_for(t: &HistoryTheme, cat: usize, text: &str) -> Span<'static> {
    if cat == NOMATCH {
        return Span::raw(text.to_string());
    }
    t.fg(text.to_string(), category_token(cat))
}

/// 不上色的同一份文本：行数与文本与 [`highlight`] 完全一致（上色只改样式，
/// 折行只依赖文本宽度），所以"先出纯文本、颜色随后补"不会动几何。
pub(crate) fn plain(code: &str) -> Vec<Line<'static>> {
    code.lines().map(|l| Line::from(l.to_string())).collect()
}

/// Syntax hint for a path (extension only — the file is never read).
pub fn language_for_path(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?;
    let ext = if ext == path { "" } else { ext };
    Some(match ext {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescript",
        "jsx" => "javascript",
        "go" => "go",
        "java" => "java",
        "kt" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" | "hxx" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "sh" | "bash" | "zsh" => "bash",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "xml" => "xml",
        "md" => "markdown",
        "sql" => "sql",
        "lua" => "lua",
        "hs" => "haskell",
        "ex" | "exs" => "elixir",
        "erl" => "erlang",
        "swift" => "swift",
        _ => return None,
    })
}

/// Syntax hint for a tool's own payload language.
///
/// `bash` payloads are shell; edit-family payloads are file content (the
/// caller passes the path so the extension decides).
pub fn language_for_tool(tool: &str) -> Option<&'static str> {
    match tool {
        "bash" => Some("bash"),
        _ => None,
    }
}

/// 围栏里写的语言标记 → 我们要喂给 `highlight` 的那个名字。
///
/// 别名表（`LANG_ALIASES`）认得的才返回 `Some`；认不得的**返回 `None`**，
/// 预热侧据此跳过——预编译一条根本不存在的语法是白花时间。
///
/// 返回值是**别名本身**（不是 syntect 的语法名）：`highlight()` 的参数就是
/// 围栏里那个词，用它查同一条语法，预热与使用才对得上。
pub fn language_for_alias(alias: &str) -> Option<&'static str> {
    let a = alias.trim();
    if a.is_empty() {
        return None;
    }
    // 只有别名表里写死的那些（以及 syntect 直接认得 token）才值得预热。
    // 返回 `&'static str` 需要一个静态名字，所以这里从别名表里回一个规范名。
    LANG_ALIASES
        .iter()
        .find(|(aliases, _)| aliases.iter().any(|x| x.eq_ignore_ascii_case(a)))
        .map(|(aliases, _)| aliases[0])
}

/// 正文里 ``` 围栏声明的语言，按出现顺序、去重。
///
/// 只认「行首（可有空格）+ 三个反引号 + 语言名」这一种形状。这是预热用的
/// **提示**，不是解析器：认漏了大不了那一刻现编，认错了也只是白编一条。
pub fn fenced_languages(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("```") else {
            continue;
        };
        // ``` 后面紧跟语言名；带别的字符（比如 ```rust,no_run）取到第一个
        // 非标识符为止。
        let lang: String = rest
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '-' || *c == '#')
            .collect();
        if !lang.is_empty() && !out.iter().any(|x| x == &lang) {
            out.push(lang);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    /// 等一条语法预热完（测试要断言颜色，就必须先让它就绪）。
    ///
    /// 生产路径**不等**：没就绪就出无色、后台编译，下一帧补色。这里同步等，
    /// 是因为测试断言的对象是"上色对不对"，不是"首帧快不快"。
    fn warmed(lang: &str) {
        warm(lang);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !ready(lang) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(ready(lang), "{lang} 预热超时");
    }

    #[test]
    fn unknown_language_renders_plain() {
        let t = HistoryTheme::resolve();
        let lines = highlight("just text\n", Some("no-such-lang-xyz"), &t);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans.iter().all(|s| s.style.fg.is_none()));
    }

    /// 围栏语言扫描：只认行首 ``` + 语言名，去重，认不出的丢掉。
    #[test]
    fn fenced_languages_picks_up_markers() {
        let text = "前言\n```rust\nfn a(){}\n```\n\n```python\nx=1\n```\n```rust\n\n```\n";
        assert_eq!(fenced_languages(text), vec!["rust", "python"]);
        // 没有语言标记的围栏不算。
        assert!(fenced_languages("```\ncode\n```\n").is_empty());
        // 带逗号参数（GitHub 风格 info string）取第一段。
        assert_eq!(fenced_languages("```bash,no_run\nls\n```\n"), vec!["bash"]);
    }

    /// 没预热过的语言：这一帧出无色，**并立即排一个后台编译**。
    ///
    /// 这条是把首帧卡顿钉死的关键：`highlight` 在没就绪时绝不允许同步编译。
    #[test]
    fn an_unwarmed_language_paints_plain_and_schedules_warming() {
        let t = HistoryTheme::resolve();
        // 挑一条本项目肯定没预热的语法，避免测试间互相影响。
        let lang = "haskell";
        assert!(!ready(lang), "前置条件：这条语法此刻不该是就绪的");
        let lines = highlight("main = putStrLn \"hi\"\n", Some(lang), &t);
        assert!(lines[0].spans.iter().all(|s| s.style.fg.is_none()), "没预热就先出无色");

        // 等后台编译完：颜色必须补上，而且不能再是"先出无色"。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !ready(lang) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(ready(lang), "后台预热必须在合理时间内完成");

        // 就绪之后这一帧**同步**出彩色（不再返回 plain）。
        let colored = highlight("main = putStrLn \"hi\"\n", Some(lang), &t);
        assert!(
            colored[0].spans.iter().any(|s| s.style.fg.is_some()),
            "预热完之后必须出彩色：{:?}",
            colored[0].spans
        );
    }

    /// 预热的语言名与围栏别名对得上（否则预热和实际使用是两套名字，
    /// 编了也用不上）。
    #[test]
    fn alias_resolution_matches_what_highlight_uses() {
        for (alias, _) in [("rs", "rs"), ("bash", "sh"), ("py", "python")] {
            let got = language_for_alias(alias).expect("别名表必须认");
            // 用别名喂 highlight，和用这个规范名喂，得到的语法必须同一条。
            let a = find_syntax(alias).map(|s| s.name.clone());
            let b = find_syntax(got).map(|s| s.name.clone());
            assert_eq!(a, b, "别名 {alias} 与规范名 {got} 解析到不同语法");
        }
        assert!(language_for_alias("").is_none());
        assert!(language_for_alias("no-such-lang-xyz").is_none());
    }

    #[test]
    fn none_language_renders_plain() {
        let t = HistoryTheme::resolve();
        let lines = highlight("x = 1", None, &t);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn rust_keywords_and_strings_get_theme_colors() {
        let t = HistoryTheme::resolve();
        warmed("rust");
        let lines = highlight("let s = \"hi\";\n", Some("rust"), &t);
        let all: Vec<Color> = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.style.fg.unwrap_or(Color::Reset)))
            .collect();
        assert!(
            all.contains(&t.get(Token::SyntaxKeyword)),
            "keyword 色缺失: {all:?}"
        );
        assert!(
            all.contains(&t.get(Token::SyntaxString)),
            "string 色缺失: {all:?}"
        );
    }

    #[test]
    fn comments_classify_as_comment() {
        let t = HistoryTheme::resolve();
        warmed("rust");
        let lines = highlight("// hello\n", Some("rust"), &t);
        let fgs: Vec<Color> = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.style.fg.unwrap_or(Color::Reset)))
            .collect();
        assert!(fgs.contains(&t.get(Token::SyntaxComment)), "{fgs:?}");
    }

    #[test]
    fn diff_view_maps_inserted_and_deleted() {
        // diff text through syntect: + lines are markup.inserted.diff.
        let t = HistoryTheme::resolve();
        warmed("diff");
        let lines = highlight("+ added line\n- removed line\n", Some("diff"), &t);
        let fgs: Vec<Color> = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.style.fg.unwrap_or(Color::Reset)))
            .collect();
        assert!(
            fgs.contains(&t.get(Token::DiffAdded)),
            "inserted 色缺失: {fgs:?}"
        );
        assert!(
            fgs.contains(&t.get(Token::DiffRemoved)),
            "deleted 色缺失: {fgs:?}"
        );
    }

    #[test]
    fn every_line_survives_roundtrip() {
        let t = HistoryTheme::resolve();
        let src = "fn main() {\n    println!(\"hi\");\n}\n";
        let lines = highlight(src, Some("rust"), &t);
        let joined: String = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("fn main()"), "{joined}");
        assert!(joined.contains("println!(\"hi\");"), "{joined}");
    }

    #[test]
    fn language_for_path_by_extension() {
        assert_eq!(language_for_path("a/b/c.rs"), Some("rust"));
        assert_eq!(language_for_path("no-ext"), None);
        assert_eq!(language_for_path("x.JSON"), None); // 大小写留给 alias 表（highlight 侧处理）
    }
}
