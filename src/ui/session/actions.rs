//! The session's named actions: one per door, so no zone names a door.

use serde_json::{Value, json};

use super::Session;
use super::vocabulary::{ModelId, Retention};

impl Session {
    /// `call init`: the session's rows, structure and todo.
    pub fn init(&mut self) {
        self.act("call", "init", json!({}));
    }

    /// `call meta`: the session row.
    pub fn meta(&mut self) {
        self.act("call", "meta", json!({}));
    }

    /// `call usage`: the bill.
    pub fn usage(&mut self) {
        self.act("call", "usage", json!({}));
    }

    /// `call pools`: the model addresses and profile names.
    pub fn pools(&mut self) {
        self.act("call", "pools", json!({}));
    }

    /// `call config`: one configuration key.
    pub fn config(&mut self, key: &str) {
        self.act("call", "config", json!({"key": key}));
    }

    /// `call blocks`: closed id ranges.
    pub fn blocks(&mut self, ranges: Vec<[i64; 2]>) {
        self.act("call", "blocks", json!({"ranges": ranges}));
    }

    /// `call sessions`: the session list.
    pub fn sessions(&mut self, under: Option<&str>) {
        self.act("call", "sessions", json!({"under": under}));
    }

    /// `subscribe watch`: follow one session without being answered.
    pub fn watch(&mut self, session: i64) {
        self.send("subscribe", "watch", json!({"session": session}), false);
    }

    /// `subscribe submit`: one message leaves as a turn.
    pub fn submit(&mut self, text: &str) {
        self.act("subscribe", "submit", json!({"text": text}));
    }

    /// `post interrupt`: stop the running turn.
    pub fn interrupt(&mut self) {
        self.act("post", "interrupt", json!({}));
    }

    /// `post mailbox`: the sentence that waits in the server's slot.
    pub fn write_mailbox(&mut self, prefix: Option<&str>, text: &str, suffix: Option<&str>) {
        self.act(
            "post",
            "mailbox",
            json!({"prefix": prefix, "text": text, "suffix": suffix}),
        );
    }

    /// `post mailbox` with no sentence: the slot is cleared.
    pub fn clear_mailbox(&mut self) {
        self.act("post", "mailbox", json!({}));
    }

    /// `post name`: the session's display name.
    pub fn rename(&mut self, name: Option<&str>) {
        self.act("post", "name", json!({"name": name}));
    }

    /// `post cwd`: the directory later turns run in.
    pub fn set_cwd(&mut self, path: &str) {
        self.act("post", "cwd", json!({"path": path}));
    }

    /// `post switch`: this session's model.
    pub fn switch_model(&mut self, id: &ModelId) {
        self.act("post", "switch", model_of(id));
    }

    /// `post default_model`: the model a new session opens with; the file is the record.
    pub fn set_default_model(&mut self, id: &ModelId) {
        self.act("post", "default_model", model_of(id));
    }

    /// `subscribe compact`: one compaction turn.
    pub fn compact(&mut self, retention: Option<Retention>, focus: Option<&str>) {
        let mut content = json!({"focus": focus});
        if let (Some(map), Some(retention)) = (content.as_object_mut(), retention) {
            let retention = match retention {
                Retention::Chars(n) => json!({"chars": n}),
                Retention::Blocks(n) => json!({"blocks": n}),
            };
            map.insert("retention".to_string(), retention);
        }
        self.act("subscribe", "compact", content);
    }

    /// `post delete`: one session goes.
    pub fn delete(&mut self, session: i64) {
        self.act("post", "delete", json!({"session": session}));
    }

    /// `post rewind`: the conversation goes back to one block.
    pub fn rewind(&mut self, turn: i64, block: i64) {
        self.act("post", "rewind", json!({"turn": turn, "block": block}));
    }
}

/// `{"model": {"provider", "model"}}`.
fn model_of(id: &ModelId) -> Value {
    json!({"model": {"provider": id.provider, "model": id.model}})
}
