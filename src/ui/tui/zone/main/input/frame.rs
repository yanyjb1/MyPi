//! One frame in, and every read the zone takes from it.

use serde_json::Value;

use super::boot::{DEFAULT_MODEL_KEY, FX_CURRENCY_KEY, GIT_SCAN_KEY, LANG_KEY};
use super::ctx::{Context, Currency};
use super::zone::InputZone;
use crate::ui::lang::Lang;
use crate::ui::tui::zone::{Frame, Request, Shout, SubZone, name};

impl InputZone {
    /// One frame answer the zone reads for itself.
    pub(super) fn read_frame(&mut self, frame: &Frame) -> Vec<Request> {
        let mut out = Vec::new();
        match (frame.class.as_str(), frame.name.as_str()) {
            ("call", "meta") => {
                self.read_meta(&frame.content, &mut out);
            }
            ("call", "config") => {
                self.read_config(&frame.content);
            }
            ("call", "usage") => {
                self.read_usage(&frame.content);
            }
            ("post", "cwd") | ("local", "cwd") => {
                self.receipt("cwd", &frame.content, &mut out);
                self.read_cwd(&frame.content);
            }
            ("post", "name") | ("local", "name") => {
                self.receipt("name", &frame.content, &mut out);
                self.read_name(&frame.content);
            }
            ("post", "switch") | ("local", "switch") => {
                self.receipt("switch", &frame.content, &mut out);
                self.read_switch(&frame.content, &mut out);
            }
            ("post", "default_model") => {
                self.receipt("default_model", &frame.content, &mut out);
            }
            ("subscribe", "compact") => {
                self.receipt("compact", &frame.content, &mut out);
            }
            ("subscribe", "submit") => {
                if frame.content.get("error").is_some() {
                    self.refused(&frame.content);
                } else {
                    self.in_flight = None;
                }
            }
            _ => {}
        }
        out
    }

    /// The session row: name, and the model its widgets follow.
    fn read_meta(&mut self, row: &Value, out: &mut Vec<Request>) {
        if let Some(name) = row.get("name").and_then(Value::as_str) {
            self.data.name = Some(name.to_string());
        }
        let Some(provider) = row.get("provider").and_then(Value::as_str) else {
            return;
        };
        let Some(model) = row.get("model").and_then(Value::as_str) else {
            return;
        };
        let address = format!("{provider}:{model}");
        if self.data.address.as_deref() == Some(address.as_str()) {
            return;
        }
        self.model(&address, out);
    }

    /// One `/name` receipt: the name it asked for lands when the write did.
    fn read_name(&mut self, content: &Value) {
        let name = self.pending_name.take();
        if content.get("error").is_none()
            && let Some(name) = name
        {
            self.data.name = name;
        }
    }

    /// One model switch: the server answers with the row's new facts, a draft with the
    /// address itself.
    fn read_switch(&mut self, content: &Value, out: &mut Vec<Request>) {
        if content.get("error").is_some() {
            return;
        }
        match content.get("address").and_then(Value::as_str) {
            Some(address) => self.model(address, out),
            None => out.push(Request::Meta),
        }
    }

    /// The model whose facts the widgets follow: one ask, answered as a whole entry.
    pub(super) fn model(&mut self, address: &str, out: &mut Vec<Request>) {
        self.data.address = Some(address.to_string());
        self.data.model = None;
        self.data.limit = 0;
        out.push(Request::Config {
            key: format!("models.model.{address}"),
        });
    }

    /// One config answer: the statusline's own keys, and the running model's facts.
    fn read_config(&mut self, content: &Value) {
        let Some(key) = content.get("key").and_then(Value::as_str) else {
            return;
        };
        let value = content.get("value");
        if key == DEFAULT_MODEL_KEY {
            // Asked at boot and when leaving a session: it names the draft's model, and only
            // while the pill has none.
            if self.data.address.is_none()
                && let Some(address) = value
                    .and_then(Value::as_str)
                    .filter(|address| !address.is_empty())
            {
                let mut out = Vec::new();
                self.model(address, &mut out);
                self.outbox.extend(out);
            }
            return;
        }
        if key == FX_CURRENCY_KEY {
            if let Some(name) = value.and_then(Value::as_str)
                && let Some(currency) = Currency::parse(name)
            {
                self.data.money.currency = currency;
                self.shouts.push(Shout::new(
                    name::CURRENCY,
                    serde_json::json!({"currency": name}),
                ));
            }
            return;
        }
        if key == LANG_KEY {
            if let Some(name) = value.and_then(Value::as_str)
                && let Some(lang) = Lang::parse(name)
            {
                self.data.lang = lang;
                self.shouts
                    .push(Shout::new(name::LANG, serde_json::json!({"lang": name})));
            }
            return;
        }
        if key == GIT_SCAN_KEY {
            if let Some(seconds) = value.and_then(Value::as_u64) {
                self.data.git_scan = Some(std::time::Duration::from_secs(seconds.max(1)));
            }
            return;
        }
        if let Some(address) = key.strip_prefix("models.model.") {
            self.read_model(address, value);
            // Any desk hand may want the entry's display facts; the panel is one.
            if let Some(entry) = value {
                let mut content = entry.clone();
                if let Some(map) = content.as_object_mut() {
                    map.insert("address".to_string(), serde_json::json!(address));
                }
                self.shouts.push(Shout::new(name::MODEL_ENTRY, content));
            }
        }
    }

    /// The model's entry: the pill's text and the gauge's denominator.
    fn read_model(&mut self, address: &str, value: Option<&Value>) {
        if self.data.address.as_deref() != Some(address) {
            return;
        }
        let Some(entry) = value else {
            return;
        };
        self.data.model = entry
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.data.provider = entry
            .get("provider")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.data.limit = entry
            .get("contextWindow")
            .and_then(Value::as_u64)
            .unwrap_or(0);
    }

    /// What the session has cost, and where its context stands. Both are the server's to say.
    fn read_usage(&mut self, content: &Value) {
        let number = |key: &str| content.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        self.data.money.cny = number("cny");
        self.data.money.usd = number("usd");
        self.data.context = Context {
            tokens: content.get("context").and_then(Value::as_u64),
            estimated: content
                .get("estimated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        self.data.baseline = content.get("baseline_prompt").and_then(Value::as_u64);
    }

    /// The write landed: the workspace is the one the server resolved, and everyone hears it.
    fn read_cwd(&mut self, content: &Value) {
        let Some(path) = content.get("cwd").and_then(Value::as_str) else {
            return;
        };
        let content = serde_json::json!({ "path": path, "temporary": false });
        self.hear(name::CWD, &content);
        self.shouts.push(Shout::new(name::CWD, content));
    }
}
