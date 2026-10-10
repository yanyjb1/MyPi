//! The compat rule table: structure, loading, matching, conflict detection. A provider's shape is a
//! **fact**, so it lives in data (YAML) rather than code; a conflict fails at load, never silently.

use std::sync::OnceLock;

use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};

use super::types::Compat;

/// The built-in rule table, compiled in.
pub const BUILTIN: &str = include_str!("builtin.yaml");

/// The rule table's file name.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSet {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

fn one() -> u32 {
    1
}

/// One rule: under which conditions (`any`) which switches change (`set`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Rule {
    /// Named in a conflict message, to say which two rules clashed.
    pub name: String,
    /// Higher numbers apply later (they override lower ones). Default 0, declaration order.
    #[serde(default)]
    pub priority: i32,
    /// 0 = built-in, 1 = user. Not written in YAML: the user's table always sorts after the built-in
    /// one, so a user rule can override a built-in conclusion.
    #[serde(skip, default)]
    pub source: u8,
    /// Any selector matching counts as a match; empty = everything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any: Vec<Selector>,
    /// The switches to set when it matches. Same shape as a `compat:` block in `models.yml`.
    #[serde(default)]
    pub set: Compat,
}

/// One match condition; several keys inside are AND. Omitting all keys matches everything.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Selector {
    /// Loopback/private address: `true` matches only private, `false` only public.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_is_private: Option<bool>,
    /// The base url's host, without scheme or port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<Vec<String>>,
    /// The wire model id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
}

/// The input of one detection, normalised.
pub struct Probe<'a> {
    /// Host in lower case, no port, no path.
    pub host: &'a str,
    /// Lower-case model id.
    pub model: &'a str,
    pub is_private: bool,
}

impl Selector {
    fn matches(&self, probe: &Probe<'_>) -> bool {
        if let Some(want) = self.host_is_private
            && want != probe.is_private
        {
            return false;
        }
        if let Some(globs) = &self.host
            && !globs.iter().any(|g| glob(g, probe.host))
        {
            return false;
        }
        if let Some(globs) = &self.models
            && !globs.iter().any(|g| glob(g, probe.model))
        {
            return false;
        }
        true
    }
}

impl Rule {
    pub fn matches(&self, probe: &Probe<'_>) -> bool {
        self.any.is_empty() || self.any.iter().any(|s| s.matches(probe))
    }
}

/// The host out of a base url, lower case, port removed: `https://api.deepseek.com/v1` → `api.deepseek.com`.
pub fn host_of(base_url: &str) -> String {
    let rest = base_url.split("://").nth(1).unwrap_or(base_url);
    let authority = rest.split('/').next().unwrap_or(rest);
    // user:pass@host  — take what follows the @
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = if authority.starts_with('[') {
        // IPv6 literal: [::1]:1 — the port follows the ]
        authority.split(']').next().unwrap_or(authority).trim_start_matches('[')
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    host.to_ascii_lowercase()
}

/// `*` any run (including empty), `?` exactly one character; case-insensitive, iterative (no recursion).
pub fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// The rule table is **not** user-editable: only the `builtin.yaml` compiled into the binary. Reading
/// and checking it (empty rule, two rules claiming one switch) happens at startup.
pub fn load_from() -> anyhow::Result<Vec<Rule>> {
    let all = parse(BUILTIN, "(built-in)")?;
    for r in &all {
        if r.any.is_empty() {
            bail!(
                "compat rule `{}` has no selectors and would match everything. Either delete it, \
                 or give it an `any:`.",
                r.name
            );
        }
    }
    check_conflicts(&all)?;
    Ok(all)
}

/// An in-process cache: the table is fixed at startup, like `models.yml`.
static RULES: OnceLock<Result<Vec<Rule>, String>> = OnceLock::new();

/// Install the table. The first call wins; later calls keep what is there.
pub fn install() -> anyhow::Result<()> {
    let loaded = load_from().map_err(|e| format!("{e:#}"));
    let _ = RULES.set(loaded);
    rules().map(|_| ())
}

/// The settled table, or the read-only built-in one when nothing was installed: only config knows paths.
pub fn rules() -> anyhow::Result<&'static [Rule]> {
    match RULES.get_or_init(|| load_from().map_err(|e| format!("{e:#}"))) {
        Ok(v) => Ok(v),
        Err(e) => bail!("{e}"),
    }
}

/// Could two rules match the same request? A **conservative** answer: "maybe" checks field conflicts,
/// "no" skips. Calling an intersecting pair disjoint only under-reports conflicts.
fn may_overlap(a: &Rule, b: &Rule) -> bool {
    if a.any.is_empty() || b.any.is_empty() {
        return true;
    }
    a.any
        .iter()
        .any(|x| b.any.iter().any(|y| selectors_may_overlap(x, y)))
}

fn selectors_may_overlap(x: &Selector, y: &Selector) -> bool {
    if let (Some(p), Some(q)) = (x.host_is_private, y.host_is_private)
        && p != q
    {
        return false;
    }
    globs_may_overlap(&x.host, &y.host) && globs_may_overlap(&x.models, &y.models)
}

fn globs_may_overlap(x: &Option<Vec<String>>, y: &Option<Vec<String>>) -> bool {
    match (x, y) {
        // One side without this constraint covers all its values, so they may overlap.
        (None, _) | (_, None) => true,
        (Some(x), Some(y)) => x
            .iter()
            .any(|g| y.iter().any(|h| globs_intersect(g, h))),
    }
}

fn globs_intersect(g: &str, h: &str) -> bool {
    let (g, h) = (g.to_ascii_lowercase(), h.to_ascii_lowercase());
    if g.contains(&h) || h.contains(&g) {
        return true;
    }
    // Compare the literal prefix before `*`: `deepseek-*` and `deepseek-r1` share one.
    let pg = g.split('*').next().unwrap_or("");
    let ph = h.split('*').next().unwrap_or("");
    pg.starts_with(ph) || ph.starts_with(pg)
}

/// Same priority, possibly both matching, writing different values to one field → error.
pub fn check_conflicts(rules: &[Rule]) -> anyhow::Result<()> {
    for (i, a) in rules.iter().enumerate() {
        for b in &rules[i + 1..] {
            // A different source or priority means the author already decided; not a conflict.
            if a.source != b.source || a.priority != b.priority || !may_overlap(a, b) {
                continue;
            }
            let (va, vb) = (
                serde_json::to_value(&a.set).context("failed to serialize the rule set")?,
                serde_json::to_value(&b.set).context("failed to serialize the rule set")?,
            );
            let (Some(oa), Some(ob)) = (va.as_object(), vb.as_object()) else {
                continue;
            };
            for (key, x) in oa {
                // `null` here means "this rule says nothing", not "set it to null".
                if x.is_null() {
                    continue;
                }
                match ob.get(key) {
                    Some(y) if !y.is_null() && y != x => bail!(
                        "compat rule conflict: `{key}` is set to different values at the same priority by `{a}` ({x}) \
                         and `{b}` ({y}). Give one of them a `priority`, or drop one.",
                        a = a.name,
                        b = b.name,
                    ),
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

/// Parse one rule table's text; `origin` is only used in error messages.
pub fn parse(text: &str, origin: &str) -> anyhow::Result<Vec<Rule>> {
    let set: RuleSet =
        serde_yaml::from_str(text).with_context(|| format!("failed to parse the compat rule table {origin}"))?;
    if set.version != 1 {
        bail!(
            "compat rule table {origin} has version {}, but this build only understands 1",
            set.version
        );
    }
    Ok(set.rules)
}
