// The adapters the bridge serves: who may come, from where, and what they may never say.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

// The file, inside the global config directory, and nowhere else.
pub const FILE: &str = "adapters.yml";

// The port the door listens on when the file names none.
const PORT: u16 = 8737;

// Everything the file says: the port every adapter shares, and the adapters themselves.
pub struct Manifest {
    pub port: u16,
    adapters: BTreeMap<String, Adapter>,
}

// One declared front end: what it is fed, and what it is not allowed to ask for.
#[derive(Debug, Clone)]
pub struct Adapter {
    pub name: String,
    // Refuse a client from anywhere but this machine, whatever the port was bound to.
    pub local_only: bool,
    pub thinking: bool,
    pub tools: bool,
    // The most characters one text frame may carry; 0 is no limit.
    pub chunk: usize,
    // A program the bridge starts beside itself and stops on the way out.
    pub command: Option<String>,
    forbid: Vec<Rule>,
}

// One name this adapter may not say: a whole kind, or one name inside it.
#[derive(Debug, Clone)]
struct Rule {
    kind: String,
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default = "port")]
    port: u16,
    adapters: BTreeMap<String, Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Entry {
    #[serde(default)]
    local_only: bool,
    #[serde(default = "yes")]
    thinking: bool,
    #[serde(default = "yes")]
    tools: bool,
    #[serde(default)]
    chunk: usize,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    forbid: Vec<String>,
}

impl Manifest {
    // Read once, at startup: a bridge with no list of adapters serves nobody, so nothing is guessed.
    pub fn load(path: &Path) -> Result<Manifest> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let file: File =
            serde_yaml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        if file.adapters.is_empty() {
            bail!("{} names no adapters", path.display());
        }
        let mut adapters = BTreeMap::new();
        for (name, entry) in file.adapters {
            let mut forbid = Vec::new();
            for rule in &entry.forbid {
                let parsed = Rule::parse(rule)
                    .with_context(|| format!("{}: adapter {name}", path.display()))?;
                forbid.push(parsed);
            }
            adapters.insert(
                name.clone(),
                Adapter {
                    name,
                    local_only: entry.local_only,
                    thinking: entry.thinking,
                    tools: entry.tools,
                    chunk: entry.chunk,
                    command: entry.command,
                    forbid,
                },
            );
        }
        Ok(Manifest {
            port: file.port,
            adapters,
        })
    }

    pub fn adapter(&self, name: &str) -> Option<&Adapter> {
        self.adapters.get(name)
    }

    pub fn adapters(&self) -> impl Iterator<Item = &Adapter> {
        self.adapters.values()
    }
}

impl Adapter {
    // Whether this adapter may say this request; a rule on the kind holds whatever the name is.
    pub fn forbids(&self, kind: &str, name: &str) -> bool {
        self.forbid.iter().any(|rule| {
            rule.kind == kind
                && match &rule.name {
                    Some(held) => held.as_str() == name,
                    None => true,
                }
        })
    }
}

impl Rule {
    // `kind` or `kind/name`, and no deeper: the bridge reads a kind and a name, never a third word.
    fn parse(text: &str) -> Result<Rule> {
        let mut parts = text.split('/');
        let kind = parts.next().unwrap_or_default();
        if !matches!(kind, "call" | "post" | "subscribe") {
            bail!("`{kind}` is not one of call, post, subscribe");
        }
        let name = parts.next().map(str::to_string);
        if let Some(name) = &name
            && name.is_empty()
        {
            bail!("nothing follows `{kind}/`");
        }
        if parts.next().is_some() {
            bail!("a rule names a kind and at most one name");
        }
        Ok(Rule {
            kind: kind.to_string(),
            name,
        })
    }
}

fn port() -> u16 {
    PORT
}

fn yes() -> bool {
    true
}
