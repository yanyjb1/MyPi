//! One frame in, and every read the zone takes from it.

use serde_json::Value;

use super::boot::{AUTO_PREPARE_KEY, DEFAULT_MODEL_KEY, FX_CURRENCY_KEY, GIT_SCAN_KEY, LANG_KEY};
use super::ctx::{Context, Currency};
use super::notice::Notice;
use super::zone::InputZone;
use crate::ui::tui::lang::Lang;
use crate::ui::tui::zone::{Frame, Shout, name};

impl InputZone {
    /// One frame answer the zone reads for itself.
    pub(super) fn read_frame(&mut self, frame: &Frame) {
        match (frame.kind.as_str(), frame.name.as_str()) {
            ("call", "init") => {
                self.read_init(&frame.payload);
            }
            ("call", "meta") => {
                self.read_meta(&frame.payload);
            }
            ("call", "config") => {
                self.read_config(&frame.payload);
            }
            ("call", "usage") => {
                self.read_usage(&frame.payload);
            }
            ("post", "meta") => {
                // A meta write's own answer: only a refusal has anything to say.
                if frame.payload.get("error").is_some() {
                    self.say_refused(&frame.payload);
                }
            }
            ("local", "cwd") => {
                // A draft's own refusal: the path stayed where it was, so say why.
                if let Some(path) = frame
                    .payload
                    .get("error")
                    .and_then(|error| error.get("path"))
                    .and_then(Value::as_str)
                {
                    self.say(&Notice::NoDir(path.to_string()));
                }
            }
            ("local", "refused") => {
                // The row never opened: the sentence that opened it comes back.
                self.refused(&frame.payload);
            }
            ("post", "default_model") => {
                self.receipt("default_model", &frame.payload);
            }
            ("subscribe", "compact") => {
                self.receipt("compact", &frame.payload);
            }
            ("subscribe", "submit") => {
                if frame.payload.get("error").is_some() {
                    self.refused(&frame.payload);
                } else {
                    self.in_flight = None;
                }
            }
            _ => {}
        }
    }

    /// The session's opening word: the effort is memory-only, so this is the one place a
    /// fresh front end learns it.
    fn read_init(&mut self, content: &Value) {
        if let Some(effort) = content.get("effort").and_then(Value::as_str) {
            self.data.effort = effort.to_ascii_lowercase();
        }
    }

    /// The session row: name, and the model its widgets follow.
    fn read_meta(&mut self, row: &Value) {
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
        self.model(&address);
    }

    /// The model whose facts the widgets follow: the entry, and the wire behind it.
    pub(super) fn model(&mut self, address: &str) {
        // A model change drops the rung: the new model declares its own, and `Auto` is the one
        // state every model shares.
        if self.data.address.is_some() {
            self.data.effort = "auto".to_string();
        }
        self.data.address = Some(address.to_string());
        self.data.model = None;
        self.data.limit = 0;
        self.data.efforts.clear();
        self.want(&format!("models.model.{address}"));
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
                self.model(address);
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
        if key == AUTO_PREPARE_KEY {
            self.data.auto_prepare = value.and_then(Value::as_bool).unwrap_or(false);
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
        self.data.efforts = entry
            .get("reasoningEfforts")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(|rung| rung.trim().to_ascii_lowercase())
                    .filter(|rung| rung != "auto")
                    .collect()
            })
            .unwrap_or_default();
    }

    /// What the session has cost, and where its context stands. Both are the server's to say.
    fn read_usage(&mut self, content: &Value) {
        let number = |key: &str| content.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        self.data.money.cny = number("cny");
        self.data.money.usd = number("usd");
        self.data.money.sub = content
            .get("subagent_price")
            .filter(|price| !price.is_null())
            .map(|price| match self.data.money.currency {
                Currency::Cny => price.get("cny").and_then(Value::as_f64).unwrap_or(0.0),
                Currency::Usd => price.get("usd").and_then(Value::as_f64).unwrap_or(0.0),
            })
            .filter(|sub| *sub > 0.0);
        self.data.context = Context {
            tokens: content.get("context").and_then(Value::as_u64),
            estimated: content
                .get("estimated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        self.data.baseline = content.get("baseline_prompt").and_then(Value::as_u64);
    }
}
