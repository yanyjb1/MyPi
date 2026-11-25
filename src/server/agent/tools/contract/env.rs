// One session's conditions for its whole life: budgets, shell, folders, and the reads it holds.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::view::{Export, Source};
use crate::server::agent::tools::workspace::Workspace;
#[cfg(feature = "web")]
use crate::server::config::rules::Rules;

// What a tool reads: one session's conditions, built when it was booked.
pub struct Env {
    // The working directory handle every call reads.
    pub workspace: Workspace,
    pub limits: Limits,
    // What `grep` expands and what it folds.
    pub grep: GrepLimits,
    pub shell: ShellEnv,
    // The session's stop signal; a tool that blocks registers what stopping it means.
    pub stop: Arc<crate::server::platform::stop::Stop>,
    pub session: i64,
    pub batch_len: usize,
    pub tools: BTreeSet<String>,
    // The roster a child may be named: the models flagged for children, plus the session's own.
    pub models: ModelNames,
    pub aliases: Aliases,
    // The budget a spawned sub-session gets when the call names none.
    pub default_budget: u32,
    // Documents the user keeps current: `.md` stems, upper-cased; a listing never dates them.
    pub persistent_docs: Arc<[String]>,
    // Whether the session's model takes image parts on the wire.
    pub vision: bool,
    pub export: Arc<dyn Export>,
    pub paths: Paths,
    pub source: Arc<dyn Source>,
    // The site rules `web` narrows a page with: one read, when the config was loaded.
    #[cfg(feature = "web")]
    pub rules: Arc<Rules>,
    // What this conversation's reads have already shown the model.
    pub seen: Seen,
    // The batch now running, so a tool can merge the siblings that touch its own file.
    pub batch: Vec<crate::server::agent::request::contract::ToolCall>,
    // The call now running; empty outside a run.
    pub current: String,
    // What a merged run has already produced for the siblings after it, by call id.
    pub handed: BTreeMap<String, Answer>,
    // One written call per shell: the audit hangs the batch's check lines there.
    pub shells: Vec<Shell>,
}

// Where the batch's audit hangs one written call's check lines.
#[derive(Clone)]
pub struct Slot {
    // The lines the edit now occupies in the written file.
    pub landing: Option<(usize, usize)>,
    // The line of the answer's text the audit's lines hang after.
    pub insert_at: usize,
}

// One written call, as the batch's audit sees it.
#[derive(Clone)]
pub struct Shell {
    // The call whose answer the audit finishes.
    pub call_id: String,
    // The written file, as the compiler names it against its crate root.
    pub mine: String,
    // The crate the written file belongs to.
    pub root: PathBuf,
    // The written file itself.
    pub file: PathBuf,
    pub unfold: bool,
    // The whole file was written: the fault's own seam stands under the answer.
    pub seam: bool,
    // Where the audit's lines hang.
    pub slots: Vec<Slot>,
}

// One finished call's answer, kept for its own turn in the batch.
#[derive(Debug, Clone)]
pub struct Answer {
    pub text: String,
    pub memo: Option<serde_json::Value>,
    // One outcome for the whole file: a single failed range makes it failed.
    pub items: Vec<crate::server::agent::request::contract::ItemOk>,
    // Why the call was refused before any of its ranges could be read.
    pub refusal: Option<String>,
}

// The session's reading memory: one module card per directory, and the stamp of every file it has read. A compaction clears it, because the conversation no longer holds what was shown.
#[derive(Debug, Default)]
pub struct Seen {
    // Module directories whose context card has been shown.
    pub dirs: std::collections::HashSet<PathBuf>,
    // Every file the model has read in this conversation: its license to write it.
    pub files: BTreeMap<PathBuf, License>,
}

impl Seen {
    // Record one file's text as the version this conversation has seen.
    pub(crate) fn stamp(&mut self, path: PathBuf, text: &str) {
        self.files.insert(path, License::of(text));
    }

    // Take away one file's licence to be written until it is read again.
    pub(crate) fn revoke(&mut self, path: &Path) {
        if let Some(license) = self.files.get_mut(path) {
            license.approval = false;
        }
    }
}

// One file's licence to be written by `splice`: a read granted it, a batch that is not whole or a drift on disk can revoke it.
#[derive(Debug, Clone)]
pub struct License {
    // A read or a clean write granted it; a batch that is not whole revokes it.
    pub approval: bool,
    // The content hash at the granting moment; a write is refused if the file no longer matches.
    pub hash: String,
}

impl License {
    // A licence over one text's UTF-8 bytes, granted now.
    pub fn of(text: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        text.as_bytes().hash(&mut hasher);
        Self {
            approval: true,
            hash: format!("{:016x}", hasher.finish()),
        }
    }
}

impl Env {
    // Is this tool in the session's list? The profile gate reads the same list.
    pub fn allows(&self, name: &str) -> bool {
        self.tools.contains(name)
    }
}

// Aliases in two layers: the user's, from config, above the session's, from the database.
pub struct Aliases {
    user: BTreeMap<String, String>,
    session: BTreeMap<String, String>,
}

// Which layer an alias came from.
#[derive(Clone, Copy)]
pub enum AliasSource {
    Config,
    Session,
}

impl AliasSource {
    pub fn tag(self) -> &'static str {
        match self {
            AliasSource::Config => "config",
            AliasSource::Session => "session",
        }
    }
}

impl Aliases {
    pub fn new(user: BTreeMap<String, String>, session: BTreeMap<String, String>) -> Self {
        Self { user, session }
    }

    // The config layer wins: a session row under a config name is never read.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.user
            .get(name)
            .or_else(|| self.session.get(name))
            .map(String::as_str)
    }

    // Every alias, config layer first; session rows under a config name are dropped.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str, AliasSource)> {
        let user = self
            .user
            .iter()
            .map(|(name, command)| (name.as_str(), command.as_str(), AliasSource::Config));
        let session = self
            .session
            .iter()
            .filter(|(name, _)| !self.user.contains_key(name.as_str()))
            .map(|(name, command)| (name.as_str(), command.as_str(), AliasSource::Session));
        user.chain(session)
    }

    // Is this name fixed by the config? A session write cannot take it.
    pub fn fixed(&self, name: &str) -> bool {
        self.user.contains_key(name)
    }

    // A write lands in the session layer only.
    pub fn set(&mut self, name: String, command: String) {
        self.session.insert(name, command);
    }

    // Removing only reaches the session layer; a config name stays.
    pub fn unset(&mut self, name: &str) -> Option<String> {
        self.session.remove(name)
    }
}

// Inline budget for one tool result.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub lines: usize,
    pub bytes: usize,
}

// What `grep` folds by: two counts, one for the items and one for the fold.
#[derive(Debug, Clone, Copy)]
pub struct GrepLimits {
    // Items an answer expands whole, every line of them.
    pub deep: usize,
    // Items it sketches; more than this folds to per-file counts.
    pub fold: usize,
}

// The model roster a session offers its children: address → display name.
pub type ModelNames = std::collections::BTreeMap<String, String>;

// One model as the table holds it: its display name and whether `task` may offer it.
#[derive(Debug, Clone)]
pub struct ModelFacts {
    pub name: String,
    pub subagent: bool,
}

// The whole model table: address → facts, as the config knew it when the loop was built.
pub type Models = std::collections::BTreeMap<String, ModelFacts>;

// The roster one session sees: every model flagged for children, plus its own.
pub fn roster(models: &Models, own: &str) -> ModelNames {
    let mut out: ModelNames = models
        .iter()
        .filter(|(_, facts)| facts.subagent)
        .map(|(address, facts)| (address.clone(), facts.name.clone()))
        .collect();
    let name = models
        .get(own)
        .map(|facts| facts.name.clone())
        .unwrap_or_else(|| own.to_string());
    out.insert(own.to_string(), name);
    out
}

// What every session's [`Env`] starts from: the process-level facts, read once.
#[derive(Clone)]
pub struct Defaults {
    pub limits: Limits,
    pub grep: GrepLimits,
    pub shell: ShellEnv,
    pub aliases: BTreeMap<String, String>,
    pub models: Models,
    pub default_budget: u32,
    pub persistent_docs: Arc<[String]>,
    pub export: Arc<dyn Export>,
    pub paths: Paths,
    pub source: Arc<dyn Source>,
    #[cfg(feature = "web")]
    pub rules: Arc<Rules>,
}

impl Defaults {
    // One session's conditions: its own directory, tools, roster, aliases and stop flag.
    pub fn session(
        &self,
        session: i64,
        cwd: super::call::Cwd,
        tools: Vec<String>,
        stop: Arc<crate::server::platform::stop::Stop>,
        vision: bool,
        own: &str,
    ) -> anyhow::Result<Env> {
        Ok(Env {
            workspace: Workspace::new(cwd),
            limits: self.limits,
            grep: self.grep,
            shell: self.shell.clone(),
            stop,
            session,
            batch_len: 0,
            tools: tools.into_iter().collect(),
            models: roster(&self.models, own),
            aliases: Aliases::new(self.aliases.clone(), self.source.aliases(session)?),
            default_budget: self.default_budget,
            persistent_docs: Arc::clone(&self.persistent_docs),
            vision,
            export: self.export.clone(),
            paths: self.paths.clone(),
            source: self.source.clone(),
            #[cfg(feature = "web")]
            rules: Arc::clone(&self.rules),
            seen: Seen::default(),
            batch: Vec::new(),
            current: String::new(),
            handed: BTreeMap::new(),
            shells: Vec::new(),
        })
    }
}

// What `shell` runs with.
#[derive(Clone)]
pub struct ShellEnv {
    pub timeout: Duration,
    pub backend: crate::server::platform::shell::Shell,
}

// The one folder a sub-session reads from that is not the project's own: the user's templates. The project's `.mypi` is taken from the live working directory at call time.
#[derive(Debug, Clone)]
pub struct Paths {
    // The templates that belong to the user, not to this project.
    pub global_agents: PathBuf,
}
