//! Structured search query — a port of oh-my-pi's `web/search/query.ts`.
//!
//! Agents embed Google-style directives (`site:`, `before:`, `inurl:`,
//! `filetype:`, quoted phrases, `OR` groups, `-exclusions`) regardless of
//! whether the backing engine parses them. This module turns a raw query into
//! a [`StructuredQuery`] so each engine can:
//!
//! 1. rebuild the query with only the syntax it actually understands
//!    ([`format_query`] / [`format_scraper_query`] `+` [`QuerySyntax`]), and
//! 2. post-filter the returned hits **leniently** ([`apply_constraints`]): a
//!    constraint dimension that would eliminate every hit is dropped and
//!    reported instead of returning nothing.
//!
//! Leniency is the whole point. A query the engine cannot honor still returns
//! results, with a note saying which constraint was relaxed — the failure mode
//! it replaces is a plausible-looking empty answer (or, worse, an
//! engine-specific "no results" the model reads as "this does not exist").

use super::hit::{SearchHit, now_epoch};

/// One free-text token (everything that is not a recognized directive).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QueryTerm {
    /// Term text without quotes or operator prefixes.
    pub text: String,
    /// Quoted exact phrase (`"like this"`) or verbatim-required (`+term`).
    pub phrase: bool,
    /// Excluded via `-term` or `NOT term`.
    pub negated: bool,
    /// OR-group id: terms sharing one are alternatives (`a OR b`); terms
    /// without a group are implicitly AND-ed. Members are always contiguous.
    pub group: Option<usize>,
}

/// A raw query decomposed into free text plus every recognized constraint.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct StructuredQuery {
    /// Original query, verbatim.
    pub raw: String,
    /// Free-text remainder with recognized directives removed.
    pub text: String,
    pub terms: Vec<QueryTerm>,
    /// `site:`/`domain:`/`host:` includes — any-of. Lowercased, scheme
    /// stripped, may carry a path (`github.com/anthropics`).
    pub sites: Vec<String>,
    pub excluded_sites: Vec<String>,
    /// `inurl:`/`url:` substrings — all must appear in the URL.
    pub in_url: Vec<String>,
    pub excluded_in_url: Vec<String>,
    /// `intitle:`/`title:` substrings — all must appear in the title.
    pub in_title: Vec<String>,
    pub excluded_in_title: Vec<String>,
    /// `intext:`/`inbody:`/`inanchor:` body substrings. Not post-filterable
    /// (snippets are partial), so query-building only.
    pub in_text: Vec<String>,
    pub excluded_in_text: Vec<String>,
    /// `filetype:`/`ext:` extensions — any-of. Lowercased, no leading dot.
    pub filetypes: Vec<String>,
    pub excluded_filetypes: Vec<String>,
    /// Inclusive lower publish-date bound from `after:`/`since:`, ISO date.
    pub after: Option<String>,
    /// Exclusive upper publish-date bound from `before:`/`until:`, ISO date.
    pub before: Option<String>,
    /// Language code from `lang:`/`language:`, lowercased.
    pub lang: Option<String>,
    /// True when any directive or boolean operator was recognized.
    pub has_directives: bool,
    /// True when any post-filterable constraint is set.
    pub has_constraints: bool,
}

/// Query-syntax capabilities of a target engine, used by [`format_query`] to
/// decide which parsed features are re-emitted as query text. Everything
/// defaults to `false`: the zero value produces plain keywords.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct QuerySyntax {
    pub phrases: bool,
    pub negation: bool,
    pub or: bool,
    pub site: bool,
    pub in_url: bool,
    pub in_title: bool,
    pub in_text: bool,
    pub filetype: bool,
    pub date_range: bool,
}

/// Full Google-style syntax: the classic operator set, as parsed by Google,
/// Startpage, Ecosia, Bing (outside CN egress), Kagi and SearXNG.
pub(crate) const GOOGLE_QUERY_SYNTAX: QuerySyntax = QuerySyntax {
    phrases: true,
    negation: true,
    or: true,
    site: true,
    in_url: true,
    in_title: true,
    in_text: true,
    filetype: true,
    date_range: true,
};

/// `site:`/`inurl:` are the two operators a plain-keyword engine must not be
/// handed (they zero-match), so engines that only "sort of" parse operators
/// use this instead and let the post-filter enforce the rest.
pub(crate) const PLAIN_QUERY_SYNTAX: QuerySyntax = QuerySyntax {
    phrases: false,
    negation: false,
    or: false,
    site: false,
    in_url: false,
    in_title: false,
    in_text: false,
    filetype: false,
    date_range: false,
};

// --- Tokenizing --------------------------------------------------------------

#[derive(Debug, Clone)]
struct RawToken {
    text: String,
    quoted: bool,
    /// The quoted span was already glued onto a `name:"…"` token.
    quoted_value: bool,
}

fn is_quote(c: char) -> bool {
    c == '"' || c == '\u{201c}' || c == '\u{201d}'
}

/// Split a raw query into whitespace-delimited tokens, honoring quoted spans.
/// `name:"budget tips"` stays one token so a directive value can contain
/// spaces; a bare `"phrase"` becomes a quoted token.
fn tokenize(raw: &str) -> Vec<RawToken> {
    let chars: Vec<char> = raw.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if is_quote(c) {
            let mut buf = String::new();
            let mut j = i + 1;
            while j < chars.len() && !is_quote(chars[j]) {
                buf.push(chars[j]);
                j += 1;
            }
            if !buf.trim().is_empty() {
                tokens.push(RawToken {
                    text: buf.trim().to_string(),
                    quoted: true,
                    quoted_value: false,
                });
            }
            i = j + 1;
            continue;
        }
        // Bare word; a quote directly after `name:` swallows the quoted span
        // into the same token.
        let mut buf = String::new();
        let mut quoted_value = false;
        while i < chars.len() && !chars[i].is_whitespace() {
            let c = chars[i];
            if is_quote(c) && buf.ends_with(':') {
                let mut j = i + 1;
                while j < chars.len() && !is_quote(chars[j]) {
                    buf.push(chars[j]);
                    j += 1;
                }
                quoted_value = true;
                i = j + 1;
                continue;
            }
            if is_quote(c) {
                break; // `foo"bar`: stop the word, let the quote open a phrase
            }
            buf.push(c);
            i += 1;
        }
        if !buf.is_empty() {
            tokens.push(RawToken {
                text: buf,
                quoted: false,
                quoted_value,
            });
        }
    }
    split_parens(tokens)
}

/// Split leading `(` and unbalanced trailing `)` into standalone tokens so
/// `(react OR vue)` parses while `site:wikipedia.org/Foo_(bar)` stays whole.
fn split_parens(tokens: Vec<RawToken>) -> Vec<RawToken> {
    let mut out = Vec::new();
    for tok in tokens {
        if tok.quoted || tok.quoted_value {
            out.push(tok);
            continue;
        }
        let mut text = tok.text.as_str();
        while let Some(rest) = text.strip_prefix('(') {
            out.push(RawToken {
                text: "(".into(),
                quoted: false,
                quoted_value: false,
            });
            text = rest;
        }
        let mut trailing = 0usize;
        while let Some(body) = text.strip_suffix(')') {
            // Only strip a paren that does not close an opener inside the word.
            let mut depth = 0i32;
            for c in body.chars() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
            }
            if depth > 0 {
                break;
            }
            text = body;
            trailing += 1;
        }
        if !text.is_empty() {
            out.push(RawToken {
                text: text.to_string(),
                quoted: false,
                quoted_value: false,
            });
        }
        for _ in 0..trailing {
            out.push(RawToken {
                text: ")".into(),
                quoted: false,
                quoted_value: false,
            });
        }
    }
    out
}

// --- Directive values -------------------------------------------------------

fn iso_date(year: i64, month: i64, day: i64) -> Option<String> {
    if !(1000..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// Parse a `before:`/`after:` value into an ISO `YYYY-MM-DD` date. Accepts
/// `YYYY`, `YYYY-MM`, `YYYY-MM-DD` (also `/` and `.` separators) and
/// `MM/DD/YYYY` (day-first assumed when the first field exceeds 12). Bare
/// years/months resolve to the first day of the period, matching Google's
/// `after:2024` = `after:2024-01-01`.
pub(crate) fn parse_date_value(value: &str) -> Option<String> {
    let t = value.trim();
    let nums = |s: &str| -> Vec<i64> {
        s.split(['-', '/', '.'])
            .map(|p| p.parse::<i64>().unwrap_or(-1))
            .collect()
    };
    // YYYY[-MM[-DD]]
    if t.len() >= 4 && t.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        let parts = nums(t);
        if (1..=3).contains(&parts.len()) && parts.iter().all(|&p| p >= 0) && t.len() >= 4 {
            let year = parts[0];
            // A 4-digit leading field is a year; anything else falls through
            // to the MM/DD/YYYY shape below.
            if (1000..=9999).contains(&year) {
                return iso_date(
                    year,
                    parts.get(1).copied().unwrap_or(1),
                    parts.get(2).copied().unwrap_or(1),
                );
            }
        }
    }
    // MM/DD/YYYY — day-first when the first field cannot be a month.
    let parts = nums(t);
    if parts.len() == 3 && (1000..=9999).contains(&parts[2]) {
        let (mut month, mut day) = (parts[0], parts[1]);
        if month > 12 && day <= 12 {
            std::mem::swap(&mut month, &mut day);
        }
        return iso_date(parts[2], month, day);
    }
    None
}

/// Lowercase a `site:` value and strip scheme, `*.` wildcard, and trailing
/// slash/dot.
fn normalize_site(value: &str) -> String {
    let mut site = value.trim().to_lowercase();
    if let Some(pos) = site.find("://") {
        // Only strip a real scheme prefix, not a path colon.
        if site[..pos]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+')
        {
            site = site[pos + 3..].to_string();
        }
    }
    if let Some(rest) = site.strip_prefix("*.") {
        site = rest.to_string();
    }
    while site.ends_with('/') || site.ends_with('.') {
        site.pop();
    }
    site
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Field {
    Site,
    InUrl,
    InTitle,
    InText,
    Filetype,
    Before,
    After,
    Lang,
}

/// Directive names mapped to their canonical field.
fn directive_field(name: &str) -> Option<Field> {
    Some(match name {
        "site" | "domain" | "host" => Field::Site,
        "inurl" | "url" => Field::InUrl,
        "intitle" | "title" => Field::InTitle,
        "intext" | "inbody" | "inanchor" => Field::InText,
        "filetype" | "ext" => Field::Filetype,
        "before" | "until" => Field::Before,
        "after" | "since" => Field::After,
        "lang" | "language" => Field::Lang,
        _ => return None,
    })
}

/// `allin*:` directives that capture every following plain term.
fn all_mode(name: &str) -> Option<Field> {
    Some(match name {
        "allintitle" => Field::InTitle,
        "allinurl" => Field::InUrl,
        "allintext" => Field::InText,
        _ => return None,
    })
}

/// `name:value` / `-name:value` / `+name:value`, with `name` the leading
/// alphabetic-hyphen run before the first colon.
fn split_directive(token: &str) -> Option<(char, String, String)> {
    let (sign, rest) = match token.as_bytes().first() {
        Some(b'-') => ('-', &token[1..]),
        Some(b'+') => ('+', &token[1..]),
        _ => ('\0', token),
    };
    let colon = rest.find(':')?;
    let (name, value) = rest.split_at(colon);
    let value = &value[1..];
    let mut chars = name.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    if !name.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
        return None;
    }
    Some((sign, name.to_lowercase(), value.to_string()))
}

/// True for operator/paren tokens and recognized directives — anything a bare
/// `name:` must not adopt as its value (`site: example.com`).
fn is_reserved_token(text: &str) -> bool {
    if matches!(
        text,
        "(" | ")" | "OR" | "AND" | "NOT" | "|" | "||" | "&&" | "!"
    ) {
        return true;
    }
    match split_directive(text) {
        Some((_, name, _)) => directive_field(&name).is_some() || all_mode(&name).is_some(),
        None => false,
    }
}

/// Parse a raw query into a [`StructuredQuery`].
///
/// Lenient by construction: unknown `name:value` tokens (URLs, `C:\paths`,
/// `TS2345:`, jargon) stay in the free text verbatim, and a directive with an
/// unparseable value (`before:someday`) degrades to a plain term.
pub(crate) fn parse(raw: &str) -> StructuredQuery {
    let mut q = StructuredQuery {
        raw: raw.to_string(),
        ..Default::default()
    };
    let tokens = tokenize(raw);

    let mut negate_next = false;
    let mut or_pending = false;
    let mut last_was_term = false;
    let mut group_seq = 0usize;
    let mut all: Option<Field> = None;

    fn push_constraint(
        q: &mut StructuredQuery,
        all: Option<Field>,
        field: Field,
        value: &str,
        negated: bool,
    ) {
        q.has_directives = true;
        let v = value.trim();
        if v.is_empty() {
            return;
        }
        match field {
            Field::Site => {
                let site = normalize_site(v);
                if !site.is_empty() {
                    if negated {
                        q.excluded_sites.push(site);
                    } else {
                        q.sites.push(site);
                    }
                }
            }
            Field::InUrl => {
                if negated {
                    q.excluded_in_url.push(v.to_string());
                } else {
                    q.in_url.push(v.to_string());
                }
            }
            Field::InTitle => {
                if negated {
                    q.excluded_in_title.push(v.to_string());
                } else {
                    q.in_title.push(v.to_string());
                }
            }
            Field::InText => {
                if negated {
                    q.excluded_in_text.push(v.to_string());
                } else {
                    q.in_text.push(v.to_string());
                }
            }
            Field::Filetype => {
                let ext = v.trim_start_matches('.').to_lowercase();
                if !ext.is_empty() {
                    if negated {
                        q.excluded_filetypes.push(ext);
                    } else {
                        q.filetypes.push(ext);
                    }
                }
            }
            Field::Before | Field::After | Field::Lang => unreachable!("handled by the caller"),
        }
        let _ = all;
    }

    for (idx, tok) in tokens.iter().enumerate() {
        if tok.quoted {
            // A quoted token is a phrase, unless an `allin*:` mode is active.
            if let Some(field) = all {
                push_constraint(&mut q, all, field, &tok.text, negate_next);
                negate_next = false;
                last_was_term = false;
                continue;
            }
            let mut term = QueryTerm {
                text: tok.text.clone(),
                phrase: true,
                negated: negate_next,
                group: None,
            };
            negate_next = false;
            if or_pending
                && last_was_term
                && let Some(prev) = q.terms.last_mut()
            {
                let g = *prev.group.get_or_insert_with(|| {
                    group_seq += 1;
                    group_seq
                });
                term.group = Some(g);
            }
            or_pending = false;
            last_was_term = true;
            q.terms.push(term);
            continue;
        }

        // Boolean operators and grouping parens.
        match tok.text.as_str() {
            "(" | ")" => continue,
            "OR" | "|" | "||" => {
                or_pending = true;
                q.has_directives = true;
                continue;
            }
            "AND" | "&&" => {
                q.has_directives = true;
                continue;
            }
            "NOT" | "!" => {
                negate_next = true;
                q.has_directives = true;
                continue;
            }
            "-" | "+" => {
                // The tokenizer splits `-"exact phrase"` into `-` + phrase.
                if tok.text == "-" && tokens.get(idx + 1).is_some_and(|t| t.quoted) {
                    negate_next = true;
                }
                continue;
            }
            _ => {}
        }

        let directive = split_directive(&tok.text);

        if let Some((sign, name, value)) = directive.clone() {
            if let Some(field) = all_mode(&name) {
                all = Some(field);
                q.has_directives = true;
                if !value.trim().is_empty() {
                    push_constraint(&mut q, all, field, &value, sign == '-');
                }
                or_pending = false;
                last_was_term = false;
                continue;
            }
            if let Some(field) = directive_field(&name) {
                let mut value = value.trim().to_string();
                // `site: example.com` — lenient: adopt the next plain token.
                if value.is_empty()
                    && let Some(next) = tokens.get(idx + 1)
                    && (next.quoted || !is_reserved_token(&next.text))
                {
                    value = next.text.trim().to_string();
                }
                if value.is_empty() {
                    q.has_directives = true;
                    continue;
                }
                let negated = sign == '-' || negate_next;
                negate_next = false;
                match field {
                    Field::Before | Field::After => match parse_date_value(&value) {
                        None => q.terms.push(QueryTerm {
                            text: tok.text.clone(),
                            phrase: false,
                            negated: false,
                            group: None,
                        }),
                        Some(iso) => {
                            if field == Field::Before {
                                q.before = Some(iso);
                            } else {
                                q.after = Some(iso);
                            }
                            q.has_directives = true;
                            or_pending = false;
                            last_was_term = false;
                        }
                    },
                    Field::Lang => {
                        q.lang = Some(value.to_lowercase());
                        q.has_directives = true;
                        or_pending = false;
                        last_was_term = false;
                    }
                    other => {
                        push_constraint(&mut q, all, other, &value, negated);
                        or_pending = false;
                        last_was_term = false;
                    }
                }
                continue;
            }
        }

        // Plain term with an optional +/- prefix.
        let mut text = tok.text.clone();
        if let Some(rest) = text.strip_prefix('-') {
            if !rest.is_empty() {
                negate_next = true;
                q.has_directives = true;
                let rest = rest.trim_start_matches('-').to_string();
                if rest.is_empty() {
                    continue;
                }
                // `- site:x` arrives split; re-check for a directive.
                if let Some((_, name, value)) = split_directive(&rest)
                    && let Some(field) = directive_field(&name)
                    && !matches!(field, Field::Before | Field::After | Field::Lang)
                {
                    negate_next = false;
                    push_constraint(&mut q, all, field, value.trim(), true);
                    continue;
                }
                text = rest;
            }
        } else if let Some(rest) = text.strip_prefix('+')
            && !rest.is_empty()
        {
            // Legacy Google `+term`: verbatim/required — an exact phrase.
            q.has_directives = true;
            let term = QueryTerm {
                text: rest.to_string(),
                phrase: true,
                negated: false,
                group: None,
            };
            q.terms.push(term);
            negate_next = false;
            or_pending = false;
            last_was_term = true;
            continue;
        }

        if let Some(field) = all {
            push_constraint(&mut q, all, field, &text, negate_next);
            negate_next = false;
            last_was_term = false;
            continue;
        }

        let mut term = QueryTerm {
            text,
            phrase: false,
            negated: negate_next,
            group: None,
        };
        negate_next = false;
        if or_pending
            && last_was_term
            && let Some(prev) = q.terms.last_mut()
        {
            let g = *prev.group.get_or_insert_with(|| {
                group_seq += 1;
                group_seq
            });
            term.group = Some(g);
        }
        or_pending = false;
        last_was_term = true;
        q.terms.push(term);
    }

    q.text = render_terms(
        &q.terms,
        QuerySyntax {
            phrases: true,
            negation: true,
            or: true,
            ..Default::default()
        },
    );
    q.has_constraints = !q.sites.is_empty()
        || !q.excluded_sites.is_empty()
        || !q.in_url.is_empty()
        || !q.excluded_in_url.is_empty()
        || !q.in_title.is_empty()
        || !q.excluded_in_title.is_empty()
        || !q.filetypes.is_empty()
        || !q.excluded_filetypes.is_empty()
        || q.before.is_some()
        || q.after.is_some();
    q
}

// --- Rebuilding a query for one engine --------------------------------------

/// Quote a directive value when it contains whitespace.
fn quote_value(value: &str) -> String {
    if value.chars().any(char::is_whitespace) {
        format!("\"{value}\"")
    } else {
        value.to_string()
    }
}

fn render_term(term: &QueryTerm, syntax: QuerySyntax) -> Option<String> {
    if term.negated && !syntax.negation {
        return None;
    }
    let body = if term.phrase && syntax.phrases {
        format!("\"{}\"", term.text)
    } else {
        term.text.clone()
    };
    Some(if term.negated {
        format!("-{body}")
    } else {
        body
    })
}

fn render_terms(terms: &[QueryTerm], syntax: QuerySyntax) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < terms.len() {
        let term = &terms[i];
        if let Some(group) = term.group
            && syntax.or
        {
            let mut members: Vec<String> = Vec::new();
            let mut j = i;
            while j < terms.len() && terms[j].group == Some(group) {
                if let Some(rendered) = render_term(&terms[j], syntax) {
                    members.push(rendered);
                }
                j += 1;
            }
            i = j;
            if members.len() > 1 {
                parts.push(format!("({})", members.join(" OR ")));
            } else if let Some(only) = members.pop() {
                parts.push(only);
            }
            continue;
        }
        if let Some(rendered) = render_term(term, syntax) {
            parts.push(rendered);
        }
        i += 1;
    }
    parts.join(" ")
}

/// Rebuild a query string for an engine with the given [`QuerySyntax`].
///
/// Constraints whose syntax the engine lacks are omitted (the caller relies on
/// [`apply_constraints`]). Never returns an empty string for a non-empty
/// input: a directives-only query falls back to its constraint values as
/// keywords, then to the raw query — an engine searching *something* beats an
/// empty-query error.
pub(crate) fn format_query(q: &StructuredQuery, syntax: QuerySyntax) -> String {
    let mut parts: Vec<String> = Vec::new();
    let text = render_terms(&q.terms, syntax);
    if !text.is_empty() {
        parts.push(text);
    }

    if syntax.site {
        if q.sites.len() > 1 && syntax.or {
            let sites: Vec<String> = q.sites.iter().map(|s| format!("site:{s}")).collect();
            parts.push(format!("({})", sites.join(" OR ")));
        } else {
            parts.extend(q.sites.iter().map(|s| format!("site:{s}")));
        }
        parts.extend(q.excluded_sites.iter().map(|s| format!("-site:{s}")));
    }
    if syntax.in_url {
        parts.extend(q.in_url.iter().map(|v| format!("inurl:{}", quote_value(v))));
        parts.extend(
            q.excluded_in_url
                .iter()
                .map(|v| format!("-inurl:{}", quote_value(v))),
        );
    }
    if syntax.in_title {
        parts.extend(
            q.in_title
                .iter()
                .map(|v| format!("intitle:{}", quote_value(v))),
        );
        parts.extend(
            q.excluded_in_title
                .iter()
                .map(|v| format!("-intitle:{}", quote_value(v))),
        );
    }
    if syntax.in_text {
        parts.extend(
            q.in_text
                .iter()
                .map(|v| format!("intext:{}", quote_value(v))),
        );
        parts.extend(
            q.excluded_in_text
                .iter()
                .map(|v| format!("-intext:{}", quote_value(v))),
        );
    }
    if syntax.filetype {
        if q.filetypes.len() > 1 && syntax.or {
            let exts: Vec<String> = q
                .filetypes
                .iter()
                .map(|f| format!("filetype:{f}"))
                .collect();
            parts.push(format!("({})", exts.join(" OR ")));
        } else {
            parts.extend(q.filetypes.iter().map(|f| format!("filetype:{f}")));
        }
        parts.extend(
            q.excluded_filetypes
                .iter()
                .map(|f| format!("-filetype:{f}")),
        );
    }
    if syntax.date_range {
        if let Some(after) = &q.after {
            parts.push(format!("after:{after}"));
        }
        if let Some(before) = &q.before {
            parts.push(format!("before:{before}"));
        }
    }

    let result = parts.join(" ").trim().to_string();
    if !result.is_empty() {
        return result;
    }
    // Directives-only query and no directive syntax: search the constraint
    // values as plain keywords.
    let fallback: Vec<String> = q
        .sites
        .iter()
        .chain(q.in_title.iter())
        .chain(q.in_url.iter())
        .chain(q.in_text.iter())
        .chain(q.filetypes.iter())
        .cloned()
        .collect();
    let fallback = fallback.join(" ").trim().to_string();
    if !fallback.is_empty() {
        return fallback;
    }
    q.raw.trim().to_string()
}

/// Build the engine query for a credential-free HTML engine.
///
/// Canonicalizes directives via [`format_query`] with the engine's syntax,
/// after demoting the two operators that zero-match across the scraper set:
/// engines only match `site:` against a bare domain (a path yields zero
/// results everywhere), and DuckDuckGo ignores `inurl:` entirely. The raw URL
/// as a plain term matches fine, so bare-domain `site:` filters are kept while
/// path-carrying `site:` and all `inurl:` values become plain keywords.
/// Negated forms pass through untouched — demoting `-site:` would invert an
/// exclusion into a search term. Directive-free queries pass through byte
/// identical.
pub(crate) fn format_scraper_query(
    query: &str,
    parsed: Option<&StructuredQuery>,
    syntax: QuerySyntax,
) -> String {
    let owned;
    let parsed = match parsed {
        Some(p) => p,
        None => {
            owned = parse(query);
            &owned
        }
    };
    if !parsed.has_directives {
        return query.to_string();
    }
    let mut demoted: Vec<QueryTerm> = Vec::new();
    let mut sites: Vec<String> = Vec::new();
    for site in &parsed.sites {
        if site.contains('/') {
            demoted.push(QueryTerm {
                text: site.clone(),
                phrase: false,
                negated: false,
                group: None,
            });
        } else {
            sites.push(site.clone());
        }
    }
    for value in &parsed.in_url {
        demoted.push(QueryTerm {
            text: value.clone(),
            phrase: false,
            negated: false,
            group: None,
        });
    }
    let mut downgraded = parsed.clone();
    downgraded.sites = sites;
    downgraded.in_url = Vec::new();
    downgraded.terms.extend(demoted);
    format_query(&downgraded, syntax)
}

// --- Lenient post-filter ----------------------------------------------------

/// Hostname (lowercased) and pathname of a URL, or `None` when unparsable.
fn host_and_path(url: &str) -> Option<(String, String)> {
    let u = url::Url::parse(url).ok()?;
    Some((u.host_str()?.to_lowercase(), u.path().to_string()))
}

/// `site:` matcher: exact host or subdomain; when `site` carries a path
/// (`github.com/anthropics`), the URL path must start with it.
pub(crate) fn matches_site(url: &str, site: &str) -> bool {
    let Some((host, path)) = host_and_path(url) else {
        return false;
    };
    let (site_host, site_path) = match site.split_once('/') {
        Some((h, p)) => (h, format!("/{}", p.trim_start_matches('/'))),
        None => (site, String::new()),
    };
    if host != site_host && !host.ends_with(&format!(".{site_host}")) {
        return false;
    }
    if site_path != "/"
        && !site_path.is_empty()
        && !path.to_lowercase().starts_with(&site_path.to_lowercase())
    {
        return false;
    }
    true
}

/// `filetype:` matcher: URL pathname ends with `.ext`.
fn matches_filetype(url: &str, ext: &str) -> bool {
    host_and_path(url)
        .map(|(_, path)| path.to_lowercase().ends_with(&format!(".{ext}")))
        .unwrap_or(false)
}

/// Relative publish dates ("2 days ago", "3h ago") some engines render
/// instead of a timestamp. Returns the age in seconds.
fn parse_relative_age(s: &str) -> Option<u64> {
    let lower = s.trim().to_lowercase();
    let rest = lower.strip_suffix("ago")?.trim_end();
    // `\d+\s*unit` — the engines write both "3 h" and "3h".
    let digits_end = rest.find(|c: char| !c.is_ascii_digit())?;
    if digits_end == 0 {
        return None;
    }
    let count: u64 = rest[..digits_end].parse().ok()?;
    let unit = rest[digits_end..].trim();
    if unit.is_empty() || unit.split_whitespace().count() != 1 {
        return None;
    }
    let secs = match unit.trim_end_matches('s') {
        "m" | "min" | "minute" => 60,
        "h" | "hr" | "hour" => 3_600,
        "d" | "day" => 86_400,
        "w" | "week" => 604_800,
        "mo" | "month" => 2_592_000,
        "y" | "yr" | "year" => 31_536_000,
        _ => return None,
    };
    Some(count * secs)
}

/// Best-effort publish time (epoch seconds) of a hit, from `age_seconds`, an
/// ISO timestamp, or relative text.
fn source_time(hit: &SearchHit) -> Option<i64> {
    if let Some(age) = hit.age_seconds {
        return Some(now_epoch() as i64 - age as i64);
    }
    let raw = hit.published_date.as_deref()?;
    if let Some(age) = parse_relative_age(raw) {
        return Some(now_epoch() as i64 - age as i64);
    }
    super::hit::iso_to_epoch(raw)
}

/// One constraint dimension: its directive rendering (for the relaxation
/// note) and its predicate.
struct Dimension {
    label: String,
    pred: Box<dyn Fn(&SearchHit) -> bool>,
}

fn constraint_dimensions(q: &StructuredQuery) -> Vec<Dimension> {
    let mut dims: Vec<Dimension> = Vec::new();
    let lower = |s: &str| s.to_lowercase();

    if !q.sites.is_empty() {
        let sites = q.sites.clone();
        dims.push(Dimension {
            label: sites
                .iter()
                .map(|s| format!("site:{s}"))
                .collect::<Vec<_>>()
                .join(" OR "),
            pred: Box::new(move |hit| sites.iter().any(|s| matches_site(&hit.url, s))),
        });
    }
    if !q.excluded_sites.is_empty() {
        let sites = q.excluded_sites.clone();
        dims.push(Dimension {
            label: sites
                .iter()
                .map(|s| format!("-site:{s}"))
                .collect::<Vec<_>>()
                .join(" "),
            pred: Box::new(move |hit| !sites.iter().any(|s| matches_site(&hit.url, s))),
        });
    }
    if !q.in_url.is_empty() {
        let vals: Vec<String> = q.in_url.iter().map(|v| lower(v)).collect();
        dims.push(Dimension {
            label: q
                .in_url
                .iter()
                .map(|v| format!("inurl:{v}"))
                .collect::<Vec<_>>()
                .join(" "),
            pred: Box::new(move |hit| {
                let url = hit.url.to_lowercase();
                vals.iter().all(|v| url.contains(v))
            }),
        });
    }
    if !q.excluded_in_url.is_empty() {
        let vals: Vec<String> = q.excluded_in_url.iter().map(|v| lower(v)).collect();
        dims.push(Dimension {
            label: q
                .excluded_in_url
                .iter()
                .map(|v| format!("-inurl:{v}"))
                .collect::<Vec<_>>()
                .join(" "),
            pred: Box::new(move |hit| {
                let url = hit.url.to_lowercase();
                !vals.iter().any(|v| url.contains(v))
            }),
        });
    }
    if !q.in_title.is_empty() {
        let vals: Vec<String> = q.in_title.iter().map(|v| lower(v)).collect();
        dims.push(Dimension {
            label: q
                .in_title
                .iter()
                .map(|v| format!("intitle:{v}"))
                .collect::<Vec<_>>()
                .join(" "),
            pred: Box::new(move |hit| {
                let title = hit.title.to_lowercase();
                vals.iter().all(|v| title.contains(v))
            }),
        });
    }
    if !q.excluded_in_title.is_empty() {
        let vals: Vec<String> = q.excluded_in_title.iter().map(|v| lower(v)).collect();
        dims.push(Dimension {
            label: q
                .excluded_in_title
                .iter()
                .map(|v| format!("-intitle:{v}"))
                .collect::<Vec<_>>()
                .join(" "),
            pred: Box::new(move |hit| {
                let title = hit.title.to_lowercase();
                !vals.iter().any(|v| title.contains(v))
            }),
        });
    }
    if !q.filetypes.is_empty() {
        let exts = q.filetypes.clone();
        dims.push(Dimension {
            label: exts
                .iter()
                .map(|f| format!("filetype:{f}"))
                .collect::<Vec<_>>()
                .join(" OR "),
            pred: Box::new(move |hit| exts.iter().any(|ext| matches_filetype(&hit.url, ext))),
        });
    }
    if !q.excluded_filetypes.is_empty() {
        let exts = q.excluded_filetypes.clone();
        dims.push(Dimension {
            label: exts
                .iter()
                .map(|f| format!("-filetype:{f}"))
                .collect::<Vec<_>>()
                .join(" "),
            pred: Box::new(move |hit| !exts.iter().any(|ext| matches_filetype(&hit.url, ext))),
        });
    }
    if q.after.is_some() || q.before.is_some() {
        let after_ms = q.after.as_deref().and_then(super::hit::iso_to_epoch);
        let before_ms = q.before.as_deref().and_then(super::hit::iso_to_epoch);
        let label = [
            q.after.as_ref().map(|a| format!("after:{a}")),
            q.before.as_ref().map(|b| format!("before:{b}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        dims.push(Dimension {
            label,
            pred: Box::new(move |hit| {
                let Some(time) = source_time(hit) else {
                    return true; // undated → cannot prove a violation
                };
                if let Some(after) = after_ms
                    && time < after
                {
                    return false;
                }
                if let Some(before) = before_ms
                    && time >= before
                {
                    return false;
                }
                true
            }),
        });
    }
    dims
}

/// Outcome of the lenient filter.
pub(crate) struct ConstraintFilter {
    /// Hits surviving the filter — never empty when the input was non-empty.
    pub hits: Vec<SearchHit>,
    /// Directive renderings of the dimensions that matched nothing and were
    /// therefore relaxed instead of enforced.
    pub dropped: Vec<String>,
}

/// Lenient post-filter: apply each constraint dimension in turn, skipping (and
/// reporting) any dimension that would eliminate every remaining hit. A
/// mis-scoped directive degrades to unfiltered results plus a note instead of
/// a dead search.
pub(crate) fn apply_constraints(hits: &[SearchHit], q: &StructuredQuery) -> ConstraintFilter {
    let mut current: Vec<SearchHit> = hits.to_vec();
    let mut dropped = Vec::new();
    if current.is_empty() {
        return ConstraintFilter {
            hits: current,
            dropped,
        };
    }
    for dim in constraint_dimensions(q) {
        let kept: Vec<SearchHit> = current.iter().filter(|h| (dim.pred)(h)).cloned().collect();
        if kept.is_empty() {
            dropped.push(dim.label);
        } else {
            current = kept;
        }
    }
    ConstraintFilter {
        hits: current,
        dropped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(title: &str, url: &str) -> SearchHit {
        SearchHit::new("Test", title.into(), url.into(), String::new())
    }

    #[test]
    fn directives_land_on_their_fields() {
        let q = parse(
            "rust async site:github.com -site:gist.github.com filetype:pdf before:2024-06 intitle:book inurl:guide lang:EN",
        );
        assert_eq!(q.sites, vec!["github.com"]);
        assert_eq!(q.excluded_sites, vec!["gist.github.com"]);
        assert_eq!(q.filetypes, vec!["pdf"]);
        assert_eq!(q.before.as_deref(), Some("2024-06-01"));
        assert_eq!(q.in_title, vec!["book"]);
        assert_eq!(q.in_url, vec!["guide"]);
        assert_eq!(q.lang.as_deref(), Some("en"));
        assert!(q.has_directives && q.has_constraints);
        // Free text keeps the plain words, in order.
        assert_eq!(q.terms.len(), 2);
        assert_eq!(q.terms[0].text, "rust");
        assert_eq!(q.terms[1].text, "async");
    }

    #[test]
    fn aliases_normalize_to_the_same_fields() {
        let q = parse("domain:example.com since:2020 after:2021-03-04 ext:PDF -inbody:noise");
        // `after:` wins over `since:` (both map to the same field, last write).
        assert_eq!(q.after.as_deref(), Some("2021-03-04"));
        assert_eq!(q.sites, vec!["example.com"]);
        assert_eq!(q.filetypes, vec!["pdf"]);
        assert_eq!(q.excluded_in_text, vec!["noise"]);
    }

    #[test]
    fn dates_accept_the_shapes_agents_write() {
        assert_eq!(parse_date_value("2024").as_deref(), Some("2024-01-01"));
        assert_eq!(parse_date_value("2024-06").as_deref(), Some("2024-06-01"));
        assert_eq!(
            parse_date_value("2024-06-15").as_deref(),
            Some("2024-06-15")
        );
        assert_eq!(parse_date_value("2024/6/5").as_deref(), Some("2024-06-05"));
        assert_eq!(parse_date_value("2024.6.5").as_deref(), Some("2024-06-05"));
        // MM/DD/YYYY, day-first when the first field cannot be a month
        assert_eq!(
            parse_date_value("06/15/2024").as_deref(),
            Some("2024-06-15")
        );
        assert_eq!(
            parse_date_value("15/06/2024").as_deref(),
            Some("2024-06-15")
        );
        // Unparseable → None, and the token degrades to a plain term.
        assert_eq!(parse_date_value("someday"), None);
        let q = parse("before:someday rust");
        assert!(!q.has_constraints);
        assert_eq!(q.terms.len(), 2, "无法解析的指令退化成普通词");
    }

    #[test]
    fn site_values_are_normalized() {
        let q = parse("site:https://*.GitHub.com/ site:example.com/");
        assert_eq!(q.sites, vec!["github.com", "example.com"]);
    }

    #[test]
    fn phrases_negation_and_or_groups_survive() {
        let q = parse("\"exact phrase\" -tokio react OR vue OR svelte");
        assert_eq!(q.terms[0].text, "exact phrase");
        assert!(q.terms[0].phrase);
        assert_eq!(q.terms[1].text, "tokio");
        assert!(q.terms[1].negated);
        // One OR group holding the three alternatives.
        let grouped: Vec<&str> = q
            .terms
            .iter()
            .filter(|t| t.group.is_some())
            .map(|t| t.text.as_str())
            .collect();
        assert_eq!(grouped, vec!["react", "vue", "svelte"]);
        assert!(q.terms[2].group == q.terms[3].group && q.terms[3].group == q.terms[4].group);
    }

    #[test]
    fn unknown_colon_tokens_stay_plain_terms() {
        // URLs, Windows paths and error codes are not directives: none of
        // them may be swallowed as a `name:value`.
        let q = parse("see https://example.com/a C:\\tmp\\x TS2345: error");
        assert!(!q.has_directives);
        assert_eq!(q.terms.len(), 5, "{:?}", q.terms);
        assert_eq!(q.terms[1].text, "https://example.com/a");
        assert_eq!(q.terms[2].text, "C:\\tmp\\x");
        assert_eq!(q.terms[3].text, "TS2345:");
        // A bare `name:` with no value and no following token is dropped, not
        // turned into an empty term.
        let q = parse("site: rust");
        assert_eq!(q.sites, vec!["rust"], "空值指令借下一个词当值");
    }

    #[test]
    fn format_query_only_emits_what_the_engine_parses() {
        let q = parse("rust site:github.com filetype:pdf before:2024-01-01 -gist");
        let google = format_query(&q, GOOGLE_QUERY_SYNTAX);
        assert!(google.contains("site:github.com"), "{google}");
        assert!(google.contains("filetype:pdf"), "{google}");
        assert!(google.contains("before:2024-01-01"), "{google}");
        assert!(google.contains("-gist"), "{google}");

        // A plain-keyword engine gets the terms only; constraints stay out and
        // are enforced by the post-filter instead.
        let plain = format_query(&q, PLAIN_QUERY_SYNTAX);
        assert_eq!(plain, "rust", "{plain}");
        // Negation is dropped, not inverted into a search term.
        assert!(!plain.contains("gist"), "{plain}");
    }

    #[test]
    fn directives_only_query_never_formats_empty() {
        let q = parse("site:arxiv.org");
        assert_eq!(format_query(&q, PLAIN_QUERY_SYNTAX), "arxiv.org");
        assert_eq!(format_query(&q, GOOGLE_QUERY_SYNTAX), "site:arxiv.org");
        // And a query with no recognized directive passes through verbatim.
        assert_eq!(
            format_scraper_query("rust async", None, GOOGLE_QUERY_SYNTAX),
            "rust async"
        );
    }

    #[test]
    fn scraper_query_demotes_path_sites_and_inurl() {
        let q = parse("rust site:github.com/rust-lang site:tokio.rs inurl:issues");
        let out = format_scraper_query(&q.raw, Some(&q), GOOGLE_QUERY_SYNTAX);
        // 裸域名的 site: 过滤保留；带路径的 site: 与全部 inurl: 降级成普通词
        //（它们在所有抓取型引擎上都是零命中）。
        assert!(out.contains("site:tokio.rs"), "{out}");
        assert!(!out.contains("site:github.com/rust-lang"), "{out}");
        assert!(!out.contains("inurl:"), "{out}");
        assert!(out.contains("github.com/rust-lang"), "{out}");
        assert!(out.contains("issues"), "{out}");
    }

    #[test]
    fn lenient_filter_relaxes_only_the_impossible_dimension() {
        let hits = vec![
            hit("Rust book", "https://doc.rust-lang.org/book/"),
            hit("Async rust", "https://tokio.rs/"),
        ];
        // site: cannot match either hit → relaxed and reported.
        let f = apply_constraints(&hits, &parse("site:example.com"));
        assert_eq!(f.hits.len(), 2, "全部命中被放宽的维度时保留原结果");
        assert_eq!(f.dropped, vec!["site:example.com"]);

        // A dimension that does match is enforced.
        let f = apply_constraints(&hits, &parse("site:tokio.rs"));
        assert_eq!(f.hits.len(), 1);
        assert!(
            f.hits[0].url.starts_with("https://tokio.rs"),
            "{:?}",
            f.hits
        );
        assert!(f.dropped.is_empty());

        // -site: excludes, and does not get relaxed away.
        let f = apply_constraints(&hits, &parse("-site:tokio.rs"));
        assert_eq!(f.hits.len(), 1);
        assert!(f.hits[0].url.contains("rust-lang.org"));
    }

    #[test]
    fn filetype_and_title_and_url_constraints() {
        let hits = vec![
            hit("The Rust Book", "https://doc.rust-lang.org/book.pdf"),
            hit("Rust reference", "https://doc.rust-lang.org/reference.html"),
        ];
        let f = apply_constraints(&hits, &parse("filetype:pdf"));
        assert_eq!(f.hits.len(), 1);
        assert!(f.hits[0].url.ends_with(".pdf"));

        let f = apply_constraints(&hits, &parse("intitle:reference inurl:reference"));
        assert_eq!(f.hits.len(), 1);
        assert!(f.hits[0].title.contains("reference"));

        let f = apply_constraints(&hits, &parse("-filetype:html"));
        assert_eq!(f.hits.len(), 1);
        assert!(f.hits[0].url.ends_with(".pdf"));
    }

    #[test]
    fn date_bounds_ignore_undated_hits_but_filter_dated_ones() {
        let mut old = hit("Old", "https://a.test/old");
        old.published_date = Some("2020-01-01".into());
        old.age_seconds = super::super::hit::age_seconds_of("2020-01-01");
        let mut recent = hit("Recent", "https://a.test/recent");
        recent.published_date = Some("2999-01-01".into()); // no age: future
        let undated = hit("Undated", "https://a.test/undated");

        // Dated-and-too-old is dropped; undated passes (cannot prove a
        // violation); the fresh one survives.
        let f = apply_constraints(&[old.clone(), undated.clone()], &parse("after:2024-01-01"));
        assert_eq!(f.hits.len(), 1);
        assert_eq!(f.hits[0].url, "https://a.test/undated");
        assert!(f.dropped.is_empty(), "有结果就不该放宽: {:?}", f.dropped);

        // Everything-violating → the dimension is relaxed, and the note names it.
        let f = apply_constraints(&[old], &parse("after:2999-01-01"));
        assert_eq!(f.hits.len(), 1);
        assert_eq!(f.dropped, vec!["after:2999-01-01"]);
    }

    #[test]
    fn relative_ages_parse_for_constraint_filtering() {
        assert_eq!(parse_relative_age("2 days ago"), Some(172_800));
        assert_eq!(parse_relative_age("3h ago"), Some(10_800));
        assert_eq!(parse_relative_age("1 month ago"), Some(2_592_000));
        assert_eq!(parse_relative_age("recently"), None);
        assert_eq!(parse_relative_age("2 fortnights ago"), None);
    }
}
