//! Syntax coloring on syntect: scopes map to eleven categories, warmed in the background.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use ratatui::text::{Line, Span};
use syntect::parsing::ScopeStack;
use syntect::parsing::{ParseState, Scope, ScopeStackOp, SyntaxSet};

use super::theme::{HistoryTheme, Token};

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

struct ScopeMatchers {
    comment: Scope,
    string: Scope,
    constant_character: Scope,
    meta_string: Scope,
    constant_numeric: Scope,
    constant_integer: Scope,
    constant: Scope,
    keyword: Scope,
    storage_type: Scope,
    storage_modifier: Scope,
    entity_name_function: Scope,
    support_function: Scope,
    meta_function_call: Scope,
    variable_function: Scope,
    entity_name_type: Scope,
    support_type: Scope,
    support_class: Scope,
    entity_name_class: Scope,
    entity_name_struct: Scope,
    entity_name_enum: Scope,
    entity_name_interface: Scope,
    entity_name_trait: Scope,
    keyword_operator: Scope,
    punctuation_accessor: Scope,
    punctuation: Scope,
    variable: Scope,
    entity_name: Scope,
    meta_path: Scope,
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

static READY: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

static WARMING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

fn syntax_name(lang: &str) -> Option<String> {
    find_syntax(lang).map(|s| s.name.clone())
}

fn is_ready(lang: &str) -> bool {
    let Some(name) = syntax_name(lang) else {
        return true;
    };
    READY
        .lock()
        .expect("warm-up table lock poisoned")
        .contains(&name)
}

pub fn warm(lang: &str) {
    let Some(name) = syntax_name(lang) else {
        return;
    };
    {
        let ready = READY.lock().expect("warm-up table lock poisoned");
        if ready.contains(&name) {
            return;
        }
    }
    {
        let mut warming = WARMING.lock().expect("warm-up table lock poisoned");
        if !warming.insert(name.clone()) {
            return; // 已经在编译了
        }
    }
    std::thread::spawn(move || {
        if let Some(syn) = SYNTAXES.syntaxes().iter().find(|s| s.name == name) {
            let mut st = ParseState::new(syn);
            let _ = st.parse_line("x\n", &SYNTAXES);
        }
        READY
            .lock()
            .expect("warm-up table lock poisoned")
            .insert(name.clone());
        WARMING
            .lock()
            .expect("warm-up table lock poisoned")
            .remove(&name);
        crate::ui::tui::theme::request_repaint();
    });
}

pub fn warm_in_order<'a>(langs: impl IntoIterator<Item = &'a str>) {
    for l in langs {
        warm(l);
    }
}

fn scope_to_category(stack: &ScopeStack) -> usize {
    for s in stack.as_slice().iter().rev() {
        let idx = MATCHERS.classify(*s);
        if idx != NOMATCH {
            return idx;
        }
    }
    NOMATCH
}

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

pub fn highlight(code: &str, language: Option<&str>, t: &HistoryTheme) -> Vec<Line<'static>> {
    let Some(lang) = language else {
        return plain(code);
    };
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

pub struct HighlightCursor {
    state: ParseState,
    stack: ScopeStack,
    lines: std::vec::IntoIter<String>,
    done: usize,
    t: HistoryTheme,
}

impl HighlightCursor {
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

    pub fn total(&self) -> usize {
        self.done + self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.len() == 0
    }

    pub fn peek_len(&self) -> usize {
        self.lines.as_slice().first().map(|l| l.len()).unwrap_or(0)
    }

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

fn split_keep_empty(code: &str) -> std::vec::IntoIter<String> {
    let mut v: Vec<String> = syntect::util::LinesWithEndings::from(code)
        .map(|l| l.to_string())
        .collect();
    if v.is_empty() {
        v.push(String::new());
    }
    v.into_iter()
}

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

fn spans_for(t: &HistoryTheme, cat: usize, text: &str) -> Span<'static> {
    if cat == NOMATCH {
        return Span::raw(text.to_string());
    }
    t.fg(text.to_string(), category_token(cat))
}

pub(crate) fn plain(code: &str) -> Vec<Line<'static>> {
    code.lines().map(|l| Line::from(l.to_string())).collect()
}

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

pub fn language_for_tool(tool: &str) -> Option<&'static str> {
    match tool {
        "bash" => Some("bash"),
        _ => None,
    }
}

pub fn language_for_alias(alias: &str) -> Option<&'static str> {
    let a = alias.trim();
    if a.is_empty() {
        return None;
    }
    LANG_ALIASES
        .iter()
        .find(|(aliases, _)| aliases.iter().any(|x| x.eq_ignore_ascii_case(a)))
        .map(|(aliases, _)| aliases[0])
}

pub fn fenced_languages(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("```") else {
            continue;
        };
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
