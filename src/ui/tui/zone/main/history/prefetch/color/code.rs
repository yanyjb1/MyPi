//! Fenced code: syntect scopes to fixed inks, one line at a time.

use std::sync::{LazyLock, Mutex};

use syntect::parsing::{ParseState, Scope, ScopeStack, ScopeStackOp, SyntaxSet};

use super::super::super::product::segment::{Kind, Segment};
use super::super::super::row::{Ink, Lead, Row, Run};
use crate::ui::tui::theme::fixed;

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

fn category_ink(idx: usize) -> Option<Ink> {
    let color = match idx {
        COMMENT => fixed::SYN_COMMENT,
        KEYWORD => fixed::SYN_KEYWORD,
        FUNCTION => fixed::SYN_FUNCTION,
        VARIABLE => fixed::SYN_VARIABLE,
        STRING => fixed::SYN_STRING,
        NUMBER => fixed::SYN_NUMBER,
        TYPE => fixed::SYN_TYPE,
        OPERATOR => fixed::SYN_OPERATOR,
        PUNCTUATION => fixed::SYN_PUNCTUATION,
        INSERTED => fixed::DIFF_ADDED,
        DELETED => fixed::DIFF_REMOVED,
        NOMATCH => return None,
        _ => return None,
    };
    Some(Ink::Fixed(color))
}

struct Matchers {
    comment: Scope,
    string: Scope,
    character: Scope,
    meta_string: Scope,
    numeric: Scope,
    integer: Scope,
    constant: Scope,
    keyword: Scope,
    storage_type: Scope,
    storage_modifier: Scope,
    name_function: Scope,
    support_function: Scope,
    function_call: Scope,
    variable_function: Scope,
    name_type: Scope,
    support_type: Scope,
    support_class: Scope,
    name_class: Scope,
    name_struct: Scope,
    name_enum: Scope,
    name_interface: Scope,
    name_trait: Scope,
    keyword_operator: Scope,
    accessor: Scope,
    punctuation: Scope,
    variable: Scope,
    name: Scope,
    meta_path: Scope,
    markup_inserted: Scope,
    markup_deleted: Scope,
    diff_header: Scope,
    diff_range: Scope,
}

impl Matchers {
    fn new() -> Matchers {
        macro_rules! s {
            ($name:literal) => {
                Scope::new($name).expect("static scope name")
            };
        }
        Matchers {
            comment: s!("comment"),
            string: s!("string"),
            character: s!("constant.character"),
            meta_string: s!("meta.string"),
            numeric: s!("constant.numeric"),
            integer: s!("constant.integer"),
            constant: s!("constant"),
            keyword: s!("keyword"),
            storage_type: s!("storage.type"),
            storage_modifier: s!("storage.modifier"),
            name_function: s!("entity.name.function"),
            support_function: s!("support.function"),
            function_call: s!("meta.function-call"),
            variable_function: s!("variable.function"),
            name_type: s!("entity.name.type"),
            support_type: s!("support.type"),
            support_class: s!("support.class"),
            name_class: s!("entity.name.class"),
            name_struct: s!("entity.name.struct"),
            name_enum: s!("entity.name.enum"),
            name_interface: s!("entity.name.interface"),
            name_trait: s!("entity.name.trait"),
            keyword_operator: s!("keyword.operator"),
            accessor: s!("punctuation.accessor"),
            punctuation: s!("punctuation"),
            variable: s!("variable"),
            name: s!("entity.name"),
            meta_path: s!("meta.path"),
            markup_inserted: s!("markup.inserted"),
            markup_deleted: s!("markup.deleted"),
            diff_header: s!("meta.diff.header"),
            diff_range: s!("meta.diff.range"),
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
        if self.diff_header.is_prefix_of(s) || self.diff_range.is_prefix_of(s) {
            return KEYWORD;
        }
        if self.string.is_prefix_of(s)
            || self.character.is_prefix_of(s)
            || self.meta_string.is_prefix_of(s)
        {
            return STRING;
        }
        if self.numeric.is_prefix_of(s) || self.integer.is_prefix_of(s) {
            return NUMBER;
        }
        if self.keyword.is_prefix_of(s)
            || self.storage_type.is_prefix_of(s)
            || self.storage_modifier.is_prefix_of(s)
        {
            return KEYWORD;
        }
        if self.name_function.is_prefix_of(s)
            || self.support_function.is_prefix_of(s)
            || self.function_call.is_prefix_of(s)
            || self.variable_function.is_prefix_of(s)
        {
            return FUNCTION;
        }
        if self.name_type.is_prefix_of(s)
            || self.support_type.is_prefix_of(s)
            || self.support_class.is_prefix_of(s)
            || self.name_class.is_prefix_of(s)
            || self.name_struct.is_prefix_of(s)
            || self.name_enum.is_prefix_of(s)
            || self.name_interface.is_prefix_of(s)
            || self.name_trait.is_prefix_of(s)
        {
            return TYPE;
        }
        if self.keyword_operator.is_prefix_of(s) || self.accessor.is_prefix_of(s) {
            return OPERATOR;
        }
        if self.punctuation.is_prefix_of(s) {
            return PUNCTUATION;
        }
        if self.variable.is_prefix_of(s)
            || self.name.is_prefix_of(s)
            || self.meta_path.is_prefix_of(s)
        {
            return VARIABLE;
        }
        if self.constant.is_prefix_of(s) {
            return NUMBER;
        }
        NOMATCH
    }
}

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static MATCHERS: LazyLock<ScopeMatchers> = LazyLock::new(Matchers::new);
static READY: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));
static WARMING: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

type ScopeMatchers = Matchers;

/// A fenced segment's rows at this width.
pub(crate) fn rows(seg: &Segment, width: usize) -> Vec<Row> {
    natural(seg)
        .into_iter()
        .flat_map(|row| super::fold_row(row, width))
        .collect()
}

/// A fenced segment, opening and closing fences included, before any folding.
fn natural(seg: &Segment) -> Vec<Row> {
    debug_assert_eq!(seg.kind, Kind::Code);
    let mut out: Vec<Row> = Vec::new();
    let lines: Vec<&str> = seg.source.split_inclusive('\n').collect();
    if lines.is_empty() {
        return out;
    }
    let fence = |line: &str| {
        let trimmed = line.trim_start();
        trimmed.starts_with("```") || trimmed.starts_with("~~~")
    };
    let closed = lines.len() > 1 && fence(lines[lines.len() - 1]);
    let body_end = if closed { lines.len() - 1 } else { lines.len() };
    out.push(fence_row(if closed { lines[0] } else { lines[0] }));
    let mut painter = Painter::new(seg.lang.as_deref());
    for line in &lines[1..body_end] {
        out.push(painter.push(line));
    }
    if closed {
        out.push(fence_row(lines[lines.len() - 1]));
    }
    out
}

fn fence_row(line: &str) -> Row {
    Row::of(
        Lead::First,
        vec![Run::fixed(
            line.trim_end_matches('\n').to_string(),
            fixed::MD_CODE_BLOCK_BORDER,
        )],
    )
}

/// One line at a time, keeping the syntax state between calls.
pub(crate) struct Painter {
    state: Option<(ParseState, ScopeStack)>,
}

impl Painter {
    pub(crate) fn new(lang: Option<&str>) -> Painter {
        let state = lang
            .and_then(ready_syntax)
            .map(|syntax| (ParseState::new(syntax), ScopeStack::new()));
        Painter { state }
    }

    pub(crate) fn push(&mut self, line: &str) -> Row {
        Row::of(Lead::First, self.runs(line))
    }

    /// The same line with no lead: card rows supply their own left edge.
    pub(crate) fn runs(&mut self, line: &str) -> Vec<Run> {
        let Some((parse, stack)) = self.state.as_mut() else {
            return vec![Run::plain(line.trim_end_matches(['\n', '\r']).to_string())];
        };
        match parse.parse_line(line, &SYNTAXES) {
            Ok(ops) => line_runs(line, ops, stack),
            Err(_) => vec![Run::plain(line.trim_end_matches(['\n', '\r']).to_string())],
        }
    }
}

/// One line with no state behind it: bash commands, JSON arguments.
pub(crate) fn line(text: &str, lang: Option<&str>) -> Row {
    let mut painter = Painter::new(lang);
    Row::of(Lead::None, painter.runs(text))
}

fn line_runs(line: &str, ops: Vec<(usize, ScopeStackOp)>, stack: &mut ScopeStack) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut prev = 0usize;
    for (offset, op) in ops {
        let offset = offset.min(line.len());
        if offset > prev {
            let text = &line[prev..offset];
            runs.push(run_for(scope_category(stack), text));
            prev = offset;
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
    if prev < line.len() {
        runs.push(run_for(scope_category(stack), &line[prev..]));
    }
    if let Some(last) = runs.last_mut() {
        while last.text.ends_with('\n') || last.text.ends_with('\r') {
            last.text.pop();
        }
        if last.text.is_empty() {
            runs.pop();
        }
    }
    runs
}

fn run_for(category: usize, text: &str) -> Run {
    match category_ink(category) {
        Some(ink) => Run::with(text.to_string(), Some(ink), Default::default()),
        None => Run::plain(text.to_string()),
    }
}

fn scope_category(stack: &ScopeStack) -> usize {
    for scope in stack.as_slice().iter().rev() {
        let idx = MATCHERS.classify(*scope);
        if idx != NOMATCH {
            return idx;
        }
    }
    NOMATCH
}

fn ready_syntax(lang: &str) -> Option<&'static syntect::parsing::SyntaxReference> {
    let syntax = find_syntax(lang)?;
    if !is_ready(lang) {
        warm(lang);
        return None;
    }
    Some(syntax)
}

fn is_ready(lang: &str) -> bool {
    let Some(name) = find_syntax(lang).map(|s| s.name.clone()) else {
        return true;
    };
    READY
        .lock()
        .expect("warm-up table lock poisoned")
        .contains(&name)
}

/// Compile a language in the background so its first paint is colored.
pub(crate) fn warm(lang: &str) {
    let Some(name) = find_syntax(lang).map(|s| s.name.clone()) else {
        return;
    };
    if READY
        .lock()
        .expect("warm-up table lock poisoned")
        .contains(&name)
    {
        return;
    }
    {
        let mut warming = WARMING.lock().expect("warm-up table lock poisoned");
        if !warming.insert(name.clone()) {
            return;
        }
    }
    std::thread::spawn(move || {
        if let Some(syn) = SYNTAXES.syntaxes().iter().find(|s| s.name == name) {
            let mut state = ParseState::new(syn);
            let _ = state.parse_line("x\n", &SYNTAXES);
        }
        READY
            .lock()
            .expect("warm-up table lock poisoned")
            .insert(name.clone());
        WARMING
            .lock()
            .expect("warm-up table lock poisoned")
            .remove(&name);
    });
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
    let set = &*SYNTAXES;
    if let Some(syntax) = set.find_syntax_by_token(lang) {
        return Some(syntax);
    }
    if let Some(syntax) = set.find_syntax_by_extension(lang) {
        return Some(syntax);
    }
    let target = LANG_ALIASES
        .iter()
        .find(|(aliases, _)| aliases.iter().any(|a| lang.eq_ignore_ascii_case(a)))
        .map(|(_, target)| *target)?;
    set.find_syntax_by_name(target)
        .or_else(|| set.find_syntax_by_token(target))
}

/// The language a path's extension names, for pre-highlighted tool output.
pub(crate) fn language_for_path(path: &str) -> Option<&'static str> {
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
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" => "markdown",
        "sql" => "sql",
        "lua" => "lua",
        "html" | "htm" => "html",
        "css" => "css",
        "xml" => "xml",
        "diff" | "patch" => "diff",
        _ => "",
    })
}
