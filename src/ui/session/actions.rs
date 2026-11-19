// The session's named actions: one per door, so no zone names a door.

use std::path::Path;

use serde_json::{Value, json};

use super::Session;
use super::vocabulary::{Effort, ModelId, Retention};

impl Session {
    // `call init`: the session's rows, structure and todo.
    pub fn init(&mut self) {
        self.act("call", "init", json!({}));
    }

    // `call meta`: the session row.
    pub fn meta(&mut self) {
        self.act("call", "meta", json!({}));
    }

    // `call usage`: the bill.
    pub fn usage(&mut self) {
        self.act("call", "usage", json!({}));
    }

    // `call pools`: the model addresses and profile names.
    pub fn pools(&mut self) {
        self.act("call", "pools", json!({}));
    }

    // `call pools` for a directory no session stands in yet: the draft's own.
    pub fn pools_at(&mut self, cwd: &Path) {
        self.send("call", "pools", json!({"cwd": cwd.to_string_lossy()}), true);
    }

    // `call config`: one configuration key. Not an action on the attached session, so no session field rides along.
    pub fn config(&mut self, key: &str) {
        self.send("call", "config", json!({"key": key}), true);
    }

    // `call blocks`: closed id ranges.
    pub fn blocks(&mut self, ranges: Vec<[i64; 2]>) {
        self.read_blocks(json!({"ranges": ranges}));
    }

    // `call sessions`: the session list.
    pub fn sessions(&mut self, under: Option<&str>) {
        self.send("call", "sessions", json!({"under": under}), true);
    }

    // `call memo`: every name in both scopes.
    pub fn memo_list(&mut self) {
        self.act("call", "memo", json!({"op": "list"}));
    }

    // `call memo`: one name's versions.
    pub fn memo_history(&mut self, scope: &str, name: &str) {
        self.act(
            "call",
            "memo",
            json!({"op": "history", "scope": scope, "name": name}),
        );
    }

    // `post memo`: one version appended.
    pub fn memo_write(&mut self, op: &str, scope: &str, name: &str, content: &str) {
        self.act(
            "post",
            "memo",
            json!({"op": op, "scope": scope, "name": name, "content": content}),
        );
    }

    // `post memo`: every version of that name removed.
    pub fn memo_delete(&mut self, scope: &str, name: &str) {
        self.act(
            "post",
            "memo",
            json!({"op": "del", "scope": scope, "name": name}),
        );
    }

    // `subscribe watch`: follow one session without being answered.
    pub fn watch(&mut self, session: i64) {
        self.send("subscribe", "watch", json!({"session": session}), false);
    }

    // `subscribe watch` with no session: stop following whatever this connection followed.
    pub fn unwatch(&mut self) {
        self.send("subscribe", "watch", json!({"session": Value::Null}), false);
    }

    // `subscribe submit`: one message leaves as a turn, with the context the command built.
    pub fn submit(&mut self, prefix: Option<&str>, text: &str) {
        self.act(
            "subscribe",
            "submit",
            json!({"prefix": prefix, "text": text}),
        );
    }

    // `post interrupt`: stop the running turn.
    pub fn interrupt(&mut self) {
        self.act("post", "interrupt", json!({}));
    }

    // `post mailbox`: the sentence that waits in the server's slot.
    pub fn write_mailbox(&mut self, prefix: Option<&str>, text: &str, suffix: Option<&str>) {
        self.act(
            "post",
            "mailbox",
            json!({"prefix": prefix, "text": text, "suffix": suffix}),
        );
    }

    // `post mailbox` with no sentence: the slot is cleared.
    pub fn clear_mailbox(&mut self) {
        self.act("post", "mailbox", json!({}));
    }

    // `post mailbox` with no sentence for a session this layer is leaving: the slot is cleared without attaching.
    pub(super) fn clear_remote_mailbox(&mut self, session: i64) {
        self.send("post", "mailbox", json!({"session": session}), false);
    }

    // `register bye`: the connection signs off.
    pub(super) fn sign_off(&mut self) {
        self.send("register", "bye", json!({}), false);
    }

    // `post meta name`: the session's display name.
    pub fn rename(&mut self, name: Option<&str>) {
        self.meta_write("name", json!({"name": name}));
    }

    // `post meta cwd`: the directory later turns run in.
    pub fn set_cwd(&mut self, path: &str) {
        self.meta_write("cwd", json!({"path": path}));
    }

    // `post meta switch`: this session's model.
    pub fn switch_model(&mut self, id: &ModelId) {
        self.meta_write("switch", model_of(id));
    }

    // `post default_model`: the model a new session opens with; the file is the record.
    pub fn set_default_model(&mut self, id: &ModelId) {
        self.act("post", "default_model", model_of(id));
    }

    // `post meta effort`: how hard this session thinks; the server keeps it in memory only.
    pub fn set_effort(&mut self, effort: Effort) {
        self.meta_write("effort", json!({"effort": effort.as_str()}));
    }

    // `subscribe compact`: one compaction turn.
    pub fn compact(&mut self, retention: Option<Retention>, focus: Option<&str>) {
        let mut content = json!({"focus": focus});
        if let Some(retention) = retention {
            content["retention"] = match retention {
                Retention::Chars(n) => json!({"chars": n}),
                Retention::Blocks(n) => json!({"blocks": n}),
            };
        }
        self.act("subscribe", "compact", content);
    }

    // `post delete`: one session goes.
    pub fn delete(&mut self, session: i64) {
        self.act("post", "delete", json!({"session": session}));
    }

    // `post meta rewind`: the conversation goes back to one block.
    pub fn rewind(&mut self, turn: i64, block: i64) {
        self.meta_write("rewind", json!({"turn": turn, "block": block}));
    }
}

// `{"model": {"provider", "model"}}`.
fn model_of(id: &ModelId) -> Value {
    json!({"model": {"provider": id.provider, "model": id.model}})
}
