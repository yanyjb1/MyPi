//! What a tool is: its parameter table, one run, its summary, and what it hands back.

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::contract::env::Env;
use super::contract::output::ToolOutput;
use super::contract::view::CallRecord;

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
    /// A JSON object.
    Object,
    /// One of an enum.
    Choice(&'static [&'static str]),
    /// An array of strings.
    TextList,
    /// An array of integers.
    Numbers,
}

/// One parameter's declaration. **Its position in the table is its index**: values are read by
/// position, the model sees the name, and the name appears nowhere else.
#[derive(Debug, Clone, Copy)]
pub struct Param {
    /// The name, for the model.
    pub name: &'static str,
    pub kind: Kind,
    /// The default, as text, converted per `kind`.
    pub default: Option<&'static str>,
    /// The description, for the model.
    pub about: &'static str,
}

impl Param {
    /// No default: an absent value is absent, and the tool decides what that means.
    pub const fn new(name: &'static str, kind: Kind, about: &'static str) -> Self {
        Self {
            name,
            kind,
            default: None,
            about,
        }
    }

    /// Absent means this text, converted per `kind`.
    pub const fn df(
        name: &'static str,
        kind: Kind,
        about: &'static str,
        default: &'static str,
    ) -> Self {
        Self {
            name,
            kind,
            default: Some(default),
            about,
        }
    }

    /// The node in the manual.
    fn json(&self) -> Value {
        let about = match self.default {
            Some(default) => format!("{}. Omitted: {default}.", self.about.trim_end_matches('.')),
            None => self.about.to_string(),
        };
        match self.kind {
            Kind::Text => json!({"type": "string", "description": about}),
            Kind::Number => json!({"type": "integer", "description": about}),
            Kind::Line => json!({"type": "integer", "description": about}),
            Kind::Flag => json!({"type": "boolean", "description": about}),
            Kind::Object => json!({"type": "object", "description": about}),
            Kind::Choice(list) => json!({"type": "string", "enum": list, "description": about}),
            Kind::TextList => json!({
                "type": "array",
                "items": {"type": "string"},
                "description": about,
            }),
            Kind::Numbers => json!({
                "type": "array",
                "items": {"type": "integer"},
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

/// One parameter table → the `input` object's properties. No `required` array: which fields a call
/// needs is stated in the tool's `input` description, per `type`.
fn properties_of(params: &[Param]) -> serde_json::Map<String, Value> {
    let mut properties = serde_json::Map::new();
    for p in params {
        properties.insert(p.name.to_string(), p.json());
    }
    properties
}

/// One tool's manual: `intent` and `input` side by side, both required. `intent` is injected here,
/// once for every tool, so no tool writes it out itself.
pub fn schema_of(tool: &Tool) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "intent".to_string(),
        json!({
            "type": "string",
            "description": "Capitalized 2-6 word present-participle intent, e.g. Reading model role settings. Shown to the user while the call runs. Emit it first."
        }),
    );
    properties.insert(
        "input".to_string(),
        json!({
            "type": "object",
            "description": tool.input,
            "properties": properties_of(tool.params),
        }),
    );
    json!({
        "type": "object",
        "properties": properties,
        "required": ["intent", "input"],
    })
}

// ---- reading values ----

/// One call's arguments, laid out by the parameter table's positions.
pub struct Args {
    params: &'static [Param],
    values: Vec<Value>,
}

impl Args {
    /// Raw text → ordered arguments: unwrap the envelope, read by table, fill defaults.
    pub fn fill(params: &'static [Param], raw: &str) -> Result<Self> {
        let text = raw.trim();
        let parsed: Value = if text.is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(text).map_err(|e| anyhow!("arguments is not valid JSON: {e}"))?
        };
        let Value::Object(mut map) = parsed else {
            bail!("arguments must be a JSON object");
        };
        let input = match map.remove("input") {
            None | Some(Value::Null) => serde_json::Map::new(),
            Some(Value::Object(inner)) => inner,
            Some(Value::String(text)) => match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(inner)) => inner,
                _ => bail!("`input` must be an object, not a string holding something else"),
            },
            Some(other) => bail!("`input` must be an object; got {other}"),
        };
        if map.keys().any(|key| key != "intent") {
            for p in params {
                if map.contains_key(p.name) && !input.contains_key(p.name) {
                    bail!(
                        "`{}` belongs inside `input`; only `intent` sits beside it",
                        p.name
                    );
                }
            }
        }
        Self::from_map(params, &input)
    }

    /// A table + one object → ordered arguments. A value for an object parameter that arrived as a
    /// string is read once as JSON, so a host that stringifies nested values costs nothing.
    fn from_map(params: &'static [Param], map: &serde_json::Map<String, Value>) -> Result<Self> {
        let mut values = Vec::with_capacity(params.len());
        for p in params {
            let mut value = map.get(p.name).cloned().unwrap_or(Value::Null);
            if value.is_null()
                && let Some(d) = p.default
            {
                value = p.typed_default(d);
            }
            if matches!(p.kind, Kind::Object)
                && let Some(parsed) = value
                    .as_str()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    .filter(Value::is_object)
            {
                value = parsed;
            }
            values.push(value);
        }
        Ok(Self { params, values })
    }

    /// An array of strings; empty when absent.
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

// ---- one tool ----

/// One call's live moments, beyond the start and the end the front door adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolProgress {
    /// More output. An increment; a front end appends it to a bounded buffer.
    Output(String),
}

/// The signature of one run: the environment, the arguments, the progress line.
pub type Run = fn(&mut Env, &Args, &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput>;

/// How one call reads in a history listing.
pub type Summary = fn(&CallRecord) -> String;

/// One tool.
#[derive(Clone, Copy)]
pub struct Tool {
    /// The name the model calls it by, and the key the profile list uses.
    pub name: &'static str,
    /// What the tool does, when to reach for it, what it answers. No field list.
    pub about: &'static str,
    /// How `input` is composed: the fields each `type` needs, and what an omission means.
    pub input: &'static str,
    /// The parameter table; position is the index.
    pub params: &'static [Param],
    /// The one entry point.
    pub run: Run,
    /// The history summary; one per tool, no fallback.
    pub summary: Summary,
}

impl Tool {
    /// The model-visible entry: name + manual + the `input` envelope (with `intent` beside it).
    pub fn definition(&self) -> crate::server::agent::request::message::ToolDef {
        crate::server::agent::request::message::ToolDef::function(
            self.name,
            self.about,
            schema_of(self),
        )
    }
}
