//! What a tool is: its parameter table, one run, what it hands back, and what it holds while running.

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::workspace::Workspace;
use crate::server::storage::{Effect, EffectKind};

// ---- parameters ----

/// A parameter's type: it decides the manual's wording and how the value is read.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    /// A string.
    Text,
    /// An integer.
    Number,
    /// A line number, 1-based.
    Line,
    /// A boolean.
    Flag,
    /// Any JSON; the tool reads it (used by `browser`'s `value`).
    Any,
    /// One of an enum.
    Choice(&'static [&'static str]),
    /// An array of strings.
    TextList,
    /// An array of objects, fields per the given table (`edit`'s `edits`, `todo`'s `list`).
    Table(&'static [Param]),
}

/// One parameter's declaration. **Its position in the table is its index**: values are read by
/// position, the model sees the name, and the name appears nowhere else.
#[derive(Debug, Clone, Copy)]
pub struct Param {
    /// The name, for the model.
    pub name: &'static str,
    pub kind: Kind,
    /// Required.
    pub required: bool,
    /// The default, as text, converted per `kind`.
    pub default: Option<&'static str>,
    /// The description, for the model.
    pub about: &'static str,
}

impl Param {
    /// Required.
    pub const fn req(name: &'static str, kind: Kind, about: &'static str) -> Self {
        Self {
            name,
            kind,
            required: true,
            default: None,
            about,
        }
    }

    /// Optional: absent means absent.
    pub const fn opt(name: &'static str, kind: Kind, about: &'static str) -> Self {
        Self {
            name,
            kind,
            required: false,
            default: None,
            about,
        }
    }

    /// Optional: absent means the default.
    pub const fn df(
        name: &'static str,
        kind: Kind,
        about: &'static str,
        default: &'static str,
    ) -> Self {
        Self {
            name,
            kind,
            required: false,
            default: Some(default),
            about,
        }
    }

    /// The node in the manual.
    fn json(&self) -> Value {
        let about = self.about;
        match self.kind {
            Kind::Text => json!({"type": "string", "description": about}),
            Kind::Number => json!({"type": "integer", "description": about}),
            Kind::Line => json!({"type": "integer", "description": about}),
            Kind::Flag => json!({"type": "boolean", "description": about}),
            Kind::Any => json!({"description": about}),
            Kind::Choice(list) => json!({"type": "string", "enum": list, "description": about}),
            Kind::TextList => json!({
                "type": "array",
                "items": {"type": "string"},
                "description": about,
            }),
            Kind::Table(fields) => json!({
                "type": "array",
                "items": object_of(fields),
                "description": about,
            }),
        }
    }

    /// The default as text, converted to a JSON value per its kind.
    fn typed_default(&self, text: &str) -> Value {
        match self.kind {
            Kind::Number | Kind::Line => {
                text.parse::<i64>().map(|n| json!(n)).unwrap_or(Value::Null)
            }
            Kind::Flag => json!(text == "true" || text == "1"),
            _ => json!(text),
        }
    }
}

/// A parameter table → the manual's object (internal: no `intent` injected).
fn object_of(params: &[Param]) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for p in params {
        properties.insert(p.name.to_string(), p.json());
        if p.required {
            required.push(json!(p.name));
        }
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

/// A parameter table → the model's manual. `intent` is injected here, once for every tool, so no tool
/// writes it out itself.
pub fn schema_of(params: &[Param]) -> Value {
    let mut node = object_of(params);
    let properties = node["properties"]
        .as_object_mut()
        .expect("schema_of builds the properties map");
    properties.insert(
        "intent".to_string(),
        json!({
            "type": "string",
            "description": "Capitalized 2-6 word present-participle intent, e.g. Reading model role settings. Shown to the user while the call runs."
        }),
    );
    node["required"]
        .as_array_mut()
        .expect("schema_of builds the required array")
        .push(json!("intent"));
    node
}

// ---- reading values ----

/// One call's arguments, laid out by the parameter table's positions.
pub struct Args {
    params: &'static [Param],
    values: Vec<Value>,
    /// The model's own `intent`: not a parameter, but `Args` is the only place the raw object was seen.
    intent: String,
}

impl Args {
    /// Raw text → ordered arguments: read by table, fill defaults, report what is missing.
    pub fn fill(params: &'static [Param], raw: &str) -> Result<Self> {
        let text = raw.trim();
        let parsed: Value = if text.is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(text).map_err(|e| anyhow!("arguments is not valid JSON: {e}"))?
        };
        let Value::Object(map) = parsed else {
            bail!("arguments must be a JSON object");
        };
        Self::from_map(params, &map)
    }

    /// A table + one object → ordered arguments.
    fn from_map(params: &'static [Param], map: &serde_json::Map<String, Value>) -> Result<Self> {
        let mut missing: Vec<&str> = Vec::new();
        let mut values = Vec::with_capacity(params.len());
        for p in params {
            let mut value = map.get(p.name).cloned().unwrap_or(Value::Null);
            if value.is_null()
                && let Some(d) = p.default
            {
                value = p.typed_default(d);
            }
            if value.is_null() && p.required {
                missing.push(p.name);
            }
            values.push(value);
        }
        if !missing.is_empty() {
            bail!("missing required argument(s): {}", missing.join(", "));
        }
        let intent = map
            .get("intent")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        Ok(Self {
            params,
            values,
            intent,
        })
    }

    /// The model's own "what am I about to do"; empty when it wrote none.
    pub fn intent(&self) -> &str {
        &self.intent
    }

    /// An array of strings; empty when absent (required ones were caught in `fill`).
    pub fn str_list(&self, i: usize) -> Result<Vec<String>> {
        let Some(value) = self.maybe(i) else {
            return Ok(Vec::new());
        };
        let array = value
            .as_array()
            .ok_or_else(|| self.wrong(i, "an array of strings"))?;
        let mut out = Vec::with_capacity(array.len());
        for element in array {
            out.push(
                element
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| self.wrong(i, "an array of strings"))?,
            );
        }
        Ok(out)
    }

    /// A table of objects: each element read by the given sub-table; empty when absent.
    pub fn table(&self, i: usize) -> Result<Vec<Args>> {
        let fields = match self.params.get(i).map(|p| p.kind) {
            Some(Kind::Table(f)) => f,
            _ => bail!("`{}` is not a table of records", self.name(i)),
        };
        let Some(value) = self.maybe(i) else {
            return Ok(Vec::new());
        };
        let array = value.as_array().ok_or_else(|| self.wrong(i, "an array"))?;
        let mut rows = Vec::with_capacity(array.len());
        for element in array {
            let map = element
                .as_object()
                .ok_or_else(|| self.wrong(i, "an array of objects"))?;
            rows.push(Self::from_map(fields, map)?);
        }
        Ok(rows)
    }

    fn name(&self, i: usize) -> &'static str {
        self.params.get(i).map(|p| p.name).unwrap_or("?")
    }

    fn at(&self, i: usize) -> Result<&Value> {
        match self.values.get(i) {
            Some(v) if !v.is_null() => Ok(v),
            Some(_) => bail!("missing argument `{}`", self.name(i)),
            None => bail!("no parameter at position {i}"),
        }
    }

    /// Whether this one was given (for optional parameters).
    fn maybe(&self, i: usize) -> Option<&Value> {
        self.values.get(i).filter(|v| !v.is_null())
    }

    fn wrong(&self, i: usize, want: &str) -> anyhow::Error {
        anyhow!(
            "argument `{}` must be {want}; got {}",
            self.name(i),
            self.values.get(i).cloned().unwrap_or(Value::Null)
        )
    }

    /// A string.
    pub fn str(&self, i: usize) -> Result<String> {
        self.at(i)?
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| self.wrong(i, "a string"))
    }

    /// An integer.
    pub fn int(&self, i: usize) -> Result<i64> {
        self.at(i)?
            .as_i64()
            .ok_or_else(|| self.wrong(i, "an integer"))
    }

    /// A line number (1-based).
    pub fn line(&self, i: usize) -> Result<usize> {
        let n = self.int(i)?;
        if n < 1 {
            bail!("`{}` is 1-based; got {n}", self.name(i));
        }
        Ok(n as usize)
    }

    /// A boolean.
    pub fn flag(&self, i: usize) -> Result<bool> {
        self.at(i)?
            .as_bool()
            .ok_or_else(|| self.wrong(i, "a boolean"))
    }

    /// One of an enum.
    pub fn choice(&self, i: usize) -> Result<String> {
        let s = self.str(i)?;
        match self.params.get(i).map(|p| p.kind) {
            Some(Kind::Choice(list)) if !list.contains(&s.as_str()) => {
                bail!(
                    "`{}` must be one of {}; got `{s}`",
                    self.name(i),
                    list.join(" / ")
                )
            }
            _ => Ok(s),
        }
    }

    /// A value as is, given or not.
    pub fn opt_any(&self, i: usize) -> Option<&Value> {
        self.maybe(i)
    }

    /// An optional string.
    pub fn opt_str(&self, i: usize) -> Result<Option<String>> {
        match self.values.get(i) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_str()
                .map(|s| Some(s.to_string()))
                .ok_or_else(|| self.wrong(i, "a string")),
        }
    }

    /// An optional integer.
    pub fn opt_int(&self, i: usize) -> Result<Option<i64>> {
        match self.values.get(i) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_i64()
                .map(Some)
                .ok_or_else(|| self.wrong(i, "an integer")),
        }
    }
}

// ---- what a tool hands back ----

/// What one tool call hands back: model text, a front-end memo, the overflow, side effects, and
/// whether this result ends the round.
#[derive(Debug)]
pub struct ToolOutput {
    pub text: String,
    pub memo: Option<Value>,
    pub spill: Option<Spill>,
    pub effects: Vec<Effect>,
    pub stop: bool,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            memo: None,
            spill: None,
            effects: Vec::new(),
            stop: false,
        }
    }

    pub fn with_memo(text: impl Into<String>, memo: Value) -> Self {
        Self {
            text: text.into(),
            memo: Some(memo),
            spill: None,
            effects: Vec::new(),
            stop: false,
        }
    }

    /// Add a side effect; the front door fills `call_id` and `ts`.
    pub fn effect(mut self, kind: EffectKind) -> Self {
        self.effects.push(Effect {
            call_id: String::new(),
            ts: String::new(),
            kind,
        });
        self
    }

    /// Mark this result as ending the round.
    pub fn stop(mut self) -> Self {
        self.stop = true;
        self
    }
}

/// The part of a tool result that exceeds the output budget: the full text. Tools ignore the budget;
/// the front door measures and moves the overflow here.
#[derive(Debug)]
pub struct Spill {
    /// The name (the tool's), for people only.
    pub name: String,
    /// The full text.
    pub content: String,
}

/// A tool's own progress event, beyond the start and finish every call has. The loop forwards it as is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolProgress {
    /// More output. An increment; a front end appends it to a bounded buffer.
    Output(String),
}

// ---- what a tool holds while running ----

/// Artifact fetch: a block id in, a temp file path out. Implemented by the session's own state.
pub trait Artifacts: Send + Sync {
    fn path_for(&self, id: i64) -> Result<std::path::PathBuf>;
}

/// What a memo read asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoReadOp {
    List,
    Show,
    History,
}

/// One stored memo version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoVersion {
    pub id: i64,
    pub content: String,
    pub ts: String,
}

/// What a memo read answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoView {
    Names(Vec<String>),
    Content(String),
    History(Vec<MemoVersion>),
}

/// Memo reads for the `memo` tool; implemented by the caller, never by the tool.
pub trait MemoRead: Send + Sync {
    fn read(
        &self,
        session: i64,
        scope: crate::server::storage::MemoScope,
        op: MemoReadOp,
        name: Option<&str>,
    ) -> Result<MemoView>;
}

/// What the caller hands in: tools never touch the database.
pub struct Inbox {
    /// For `todo`: this session's list.
    pub todo: Vec<super::memo::todo::state::TodoPhase>,
    /// For `context`: the window to look at.
    pub window: super::bash::context::Window,
    /// For `bash`: artifact fetch. `None` = no channel, so `#<ref>` stays as written.
    pub artifacts: Option<std::sync::Arc<dyn Artifacts>>,
    /// For `memo`: the session's id and its read port.
    pub session: i64,
    pub memo: Option<std::sync::Arc<dyn MemoRead>>,
    /// For `kairos`: the session's background job service.
    pub jobs: Option<std::sync::Arc<crate::server::agent::tools::jobs::JobService>>,
    /// For `task` and `final`: the session's subagent service.
    pub subagent: Option<std::sync::Arc<crate::server::agent::tools::subagent::Subagent>>,
}

impl Default for Inbox {
    fn default() -> Self {
        Self {
            todo: Vec::new(),
            window: Default::default(),
            artifacts: None,
            session: 0,
            memo: None,
            jobs: None,
            subagent: None,
        }
    }
}

/// The two things `bash` needs.
pub struct BashEnv {
    /// A command's wall-clock limit (config `tools.bashTimeoutSecs`, default 600).
    pub timeout: std::time::Duration,
    /// The session's stop flag; `None` = the deadline only.
    pub interrupt: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Default for BashEnv {
    fn default() -> Self {
        Self {
            timeout: std::time::Duration::from_secs(
                crate::server::config::ToolsConfig::default().bash_timeout,
            ),
            interrupt: None,
        }
    }
}

/// What a tool holds during one execution. Built by the loop; a tool never goes looking.
pub struct Env {
    /// The working directory and its permitted area.
    pub workspace: Workspace,
    /// The output budget (`read` and `bash`).
    pub limits: crate::server::config::OutputLimits,
    /// What the caller handed in.
    pub inbox: Inbox,
    /// `bash`'s timeout and stop flag.
    pub bash: BashEnv,
    /// The web tool's browser config.
    pub browser: crate::server::config::BrowserConfig,
    /// The profile gate: `None` = every tool; `Some(list)` = only those. The roster and the front door
    /// read the same list.
    pub enabled: Option<std::collections::BTreeSet<String>>,
    /// The aliases visible to `bash` this round.
    pub aliases: std::collections::BTreeMap<String, String>,
    /// The MCP bridge, if any server is configured.
    pub mcp: Option<std::sync::Arc<dyn super::McpBridge>>,
}

impl Env {
    pub fn new(cwd: impl Into<std::path::PathBuf>) -> Self {
        Self {
            workspace: Workspace::new(cwd),
            limits: crate::server::config::OutputLimits::default(),
            inbox: Inbox::default(),
            bash: BashEnv::default(),
            browser: Default::default(),
            enabled: None,
            aliases: Default::default(),
            mcp: None,
        }
    }

    /// Is this tool allowed? The built-in layer is the table's business; this is the profile.
    pub fn allows(&self, name: &str) -> bool {
        match &self.enabled {
            None => true,
            Some(set) => set.contains(name),
        }
    }

    /// Install the profile's list; `None` allows everything.
    pub fn with_enabled(mut self, names: Option<Vec<String>>) -> Self {
        self.enabled = names.map(|v| v.into_iter().collect());
        self
    }

    /// Install the aliases visible to `bash`.
    pub fn with_aliases(mut self, aliases: std::collections::BTreeMap<String, String>) -> Self {
        self.aliases = aliases;
        self
    }
}

// ---- one tool ----

/// The signature of one run: the environment, the arguments, the progress line.
pub type Run = fn(&mut Env, &Args, &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput>;

/// One tool.
#[derive(Clone, Copy)]
pub struct Tool {
    /// The name the model calls it by, and the key the profile list uses.
    pub name: &'static str,
    /// The manual the model reads.
    pub about: &'static str,
    /// The parameter table; position is the index.
    pub params: &'static [Param],
    /// The one entry point.
    pub run: Run,
}

impl Tool {
    /// The model-visible entry: name + manual + parameter table (with `intent` injected).
    pub fn definition(&self) -> crate::server::agent::request::message::ToolDef {
        crate::server::agent::request::message::ToolDef::function(
            self.name,
            self.about,
            schema_of(self.params),
        )
    }
}
