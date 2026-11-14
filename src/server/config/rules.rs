//! Site rules for the web tool: where the file lives, what it says, and what stands in for it.
//!
//! The file sits in the config directory beside `config.yaml`. It is read once, when the
//! configuration is loaded: a file that cannot be read is reported there, not on first use.

use std::path::Path;

use anyhow::Context as _;
use serde::Deserialize;

use super::notice::{ConfigNotice, warn};
use super::paths::ConfigPaths;

/// The file's name inside the config directory.
const FILE: &str = "web-rules.json";

/// One page pattern, and the parts of that page which are its prose.
#[derive(Debug, Deserialize)]
pub struct Rule {
    /// Matched against `host + path`; the first rule that matches is the one used. A regex.
    #[serde(rename = "match")]
    matcher: String,
    keep: Vec<Part>,
}

/// One kept region: a selector, and what to leave out of it.
#[derive(Debug, Deserialize)]
pub struct Part {
    /// Searched from the page body; the first element it names is the region.
    selector: String,
    /// Removed from inside that region only.
    #[serde(default)]
    drop: Vec<String>,
}

/// One entry of the manifest: a rule written in place, or a path to a file of them.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Entry {
    /// Relative to the manifest's own directory, forward slashes only.
    Ref(String),
    Rule(Rule),
}

/// A file a manifest names: one array of rules, and no entries of its own.
#[derive(Debug, Deserialize)]
#[serde(transparent)]
struct Set(Vec<Rule>);

/// The rules in force: the manifest with every set it names flattened in place.
#[derive(Debug)]
pub struct Rules(Vec<Rule>);

/// The site sets compiled in beside the built-in manifest, under the name the manifest uses.
macro_rules! sets {
    ($($site:literal),* $(,)?) => {
        &[$((concat!("web-rulesets/", $site), include_str!(concat!("web-rulesets/", $site)))),*]
    };
}

const SETS: &[(&str, &str)] = sets![
    "cac.json",
    "court.json",
    "thepaper.json",
    "techcrunch.json",
    "21jingji.json",
    "36kr.json",
    "bjnews.json",
    "cn-sec.json",
    "cnbc.json",
    "forbes.json",
    "geekwire.json",
    "gov.cn.json",
    "huxiu.json",
    "jiemian.json",
    "mrjjxw.json",
    "pbc.json",
    "poetryfoundation.json",
    "qq.json",
    "sina.json",
    "siliconangle.json",
    "sohu.json",
    "stdaily.json",
    "tmtpost.json",
    "yicai.json",
];

impl Rules {
    /// The regions to keep for this page, in order, or nothing when no rule matches.
    pub fn regions(&self, host: &str, path: &str) -> &[Part] {
        let subject = format!("{host}{path}");
        self.0
            .iter()
            .find(|rule| matches(&rule.matcher, &subject))
            .map_or(&[], |rule| rule.keep.as_slice())
    }
}

impl Part {
    pub fn selector(&self) -> &str {
        &self.selector
    }

    pub fn drops(&self) -> &[String] {
        &self.drop
    }
}

/// A pattern that cannot be read matches nothing: a broken rule must not take the page with it.
fn matches(pattern: &str, subject: &str) -> bool {
    regex::Regex::new(pattern).is_ok_and(|re| re.is_match(subject))
}

/// The rules in force. A machine without the file runs the built-in set (`install` writes it out at
/// bootstrap); a file that cannot be read is reported as a config notice and the built-in set
/// answers instead.
pub fn load(paths: &ConfigPaths, notices: &mut Vec<ConfigNotice>) -> Rules {
    let Some(root) = paths.global_root() else {
        return builtin();
    };
    let file = root.join(FILE);
    if !file.is_file() {
        return builtin();
    }
    match read(&file, notices) {
        Ok(rules) => rules,
        Err(problem) => {
            notices.push(warn(
                FILE,
                &file.display().to_string(),
                &problem,
                "内置规则",
            ));
            builtin()
        }
    }
}

/// One read of the manifest: unreadable text and unreadable JSON are the same answer. A set it
/// names that cannot be read is one notice and one hole, not a reason to lose the whole file.
fn read(file: &Path, notices: &mut Vec<ConfigNotice>) -> Result<Rules, String> {
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    let entries: Vec<Entry> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let dir = file.parent().unwrap_or(Path::new("."));
    let (rules, failed) = flatten(entries, |name| set(dir, name));
    for (name, problem) in failed {
        notices.push(warn(FILE, &name, &format!("引用未读：{problem}"), "跳过"));
    }
    Ok(Rules(rules))
}

/// One file a manifest names, read from beside it.
fn set(dir: &Path, name: &str) -> Result<Set, String> {
    let text = std::fs::read_to_string(dir.join(name)).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// The manifest's entries in order, each set flattened in where it was named. A set that fails is
/// answered back as one hole; the rules around it still hold.
fn flatten(
    entries: Vec<Entry>,
    load_set: impl Fn(&str) -> Result<Set, String>,
) -> (Vec<Rule>, Vec<(String, String)>) {
    let mut out = Vec::new();
    let mut failed = Vec::new();
    for entry in entries {
        match entry {
            Entry::Rule(rule) => out.push(rule),
            Entry::Ref(name) => match load_set(&name) {
                Ok(set) => out.extend(set.0),
                Err(problem) => failed.push((name, problem)),
            },
        }
    }
    (out, failed)
}

/// What a new machine starts from: the manifest, and every set it names, thrown out beside it.
/// A file already there is left alone.
pub fn install(root: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(root)
        .with_context(|| format!("failed to create {}", root.display()))?;
    let manifest = root.join(FILE);
    if !manifest.exists() {
        std::fs::write(&manifest, MANIFEST)
            .with_context(|| format!("failed to write {}", manifest.display()))?;
        println!("mypi: wrote {}", manifest.display());
    }
    for (name, text) in SETS {
        let file = root.join(name);
        if file.exists() {
            continue;
        }
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }
        std::fs::write(&file, text)
            .with_context(|| format!("failed to write {}", file.display()))?;
        println!("mypi: wrote {}", file.display());
    }
    Ok(())
}

/// The built-in manifest's own text, and the copy a new machine is given.
const MANIFEST: &str = include_str!("web-rules.json");

/// What a new machine starts from when its file cannot be read: the manifest and every set it
/// names, both read from the binary. A name neither can answer is a build error, not a page.
fn builtin() -> Rules {
    let entries: Vec<Entry> = serde_json::from_str(MANIFEST)
        .expect("the built-in manifest parses: it is compiled in");
    let (rules, failed) = flatten(entries, compiled);
    if let Some((name, problem)) = failed.into_iter().next() {
        panic!("the built-in manifest names {name}, which the binary cannot answer: {problem}");
    }
    Rules(rules)
}

/// One compiled-in set by the name the built-in manifest uses.
fn compiled(name: &str) -> Result<Set, String> {
    let text = SETS
        .iter()
        .find(|(set, _)| *set == name)
        .map(|(_, text)| *text)
        .ok_or_else(|| "not compiled in".to_string())?;
    serde_json::from_str(text).map_err(|e| e.to_string())
}
