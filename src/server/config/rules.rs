//! Site rules for the web tool: where the file lives, what it says, and what stands in for it.
//!
//! The file sits in the config directory beside `config.yaml`. It is read once, when the
//! configuration is loaded: a file that cannot be read is reported there, not on first use.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::notice::{ConfigNotice, warn};
use super::paths::ConfigPaths;

/// The file's name inside the config directory.
const FILE: &str = "web-rules.json";

/// One page pattern, and the parts of that page which are its prose.
#[derive(Debug, Deserialize, Serialize)]
pub struct Rule {
    /// Matched against `host + path`; the first rule that matches is the one used. A regex.
    #[serde(rename = "match")]
    matcher: String,
    keep: Vec<Part>,
}

/// One kept region: a selector, and what to leave out of it.
#[derive(Debug, Deserialize, Serialize)]
pub struct Part {
    /// Searched from the page body; the first element it names is the region.
    selector: String,
    /// Removed from inside that region only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    drop: Vec<String>,
}

/// The whole file: one array, ordered so a specific path may precede its site's catch-all.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Rules(Vec<Rule>);

impl Rules {
    /// The regions to keep for this page, in order, or nothing when no rule matches.
    pub fn regions(&self, host: &str, path: &str) -> &[Part] {
        let subject = format!("{host}{path}");
        self.0
            .iter()
            .find(|rule| matches(&rule.matcher, &subject))
            .map_or(&[], |rule| rule.keep.as_slice())
    }

    /// The file's own text, for the copy written on a machine that has none.
    fn text(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
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

/// The rules in force. A machine without the file is given the built-in set, once; a file that
/// cannot be read is reported as a config notice and the built-in set answers instead.
pub fn load(paths: &ConfigPaths, notices: &mut Vec<ConfigNotice>) -> Rules {
    let Some(file) = paths.global_root().map(|root| root.join(FILE)) else {
        return builtin();
    };
    if !file.is_file() {
        let builtin = builtin();
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&file, builtin.text());
        return builtin;
    }
    match read(&file) {
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

/// One read of the file: unreadable text and unreadable JSON are the same answer.
fn read(file: &Path) -> Result<Rules, String> {
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// What a new machine starts from.
fn builtin() -> Rules {
    serde_json::from_str(include_str!("web-rules.json")).unwrap_or_default()
}
