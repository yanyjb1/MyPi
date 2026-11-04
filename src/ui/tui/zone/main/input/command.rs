//! One command line to the door it names, its arguments parsed per the declaration.

use serde_json::{Value, json};

use crate::commands::CommandMeta;
use crate::ui::tui::zone::Request;

/// The request a command becomes; `None` = its required argument is missing or unusable.
pub(crate) fn frame(meta: &'static CommandMeta, args: &str) -> Option<Request> {
    match meta.name {
        "/name" => Some(Request::post("name", json!({ "name": text(args) }))),
        "/cdp" => {
            let path = args.trim();
            (!path.is_empty()).then(|| Request::post("cwd", json!({ "path": path })))
        }
        "/switch" => Some(Request::post("switch", pair(args)?)),
        "/model" => Some(Request::post("default_model", pair(args)?)),
        "/profile" => {
            let name = args.trim();
            (!name.is_empty()).then(|| Request::create(Some(name.to_string())))
        }
        "/compact" => Some(Request::subscribe("compact", compact(args))),
        "/delete" => Some(Request::post("delete", json!({ "session": number(args)? }))),
        _ => None,
    }
}

/// A free-text argument; empty means "none".
fn text(args: &str) -> Value {
    match args.trim() {
        "" => Value::Null,
        text => json!(text),
    }
}

/// `{"model": {"provider", "model"}}` from `provider:model`.
fn pair(args: &str) -> Option<Value> {
    let (provider, model) = args.trim().split_once(':')?;
    let (provider, model) = (provider.trim(), model.trim());
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some(json!({ "model": { "provider": provider, "model": model } }))
}

fn number(args: &str) -> Option<i64> {
    args.trim().parse().ok()
}

/// `[20k|18] [focus]`: the tail to keep, and what to keep about.
fn compact(args: &str) -> Value {
    let args = args.trim();
    if args.is_empty() {
        return json!({});
    }
    let (head, rest) = match args.split_once(char::is_whitespace) {
        Some((head, rest)) => (head, rest.trim()),
        None => (args, ""),
    };
    let Some(retention) = retention(head) else {
        return json!({ "focus": args });
    };
    match rest.is_empty() {
        true => json!({ "retention": retention }),
        false => json!({ "retention": retention, "focus": rest }),
    }
}

/// A character count (`20k`) or a block count (`18`).
fn retention(token: &str) -> Option<Value> {
    if let Some(count) = token.strip_suffix(['k', 'K']) {
        let count: u64 = count.parse().ok()?;
        return Some(json!({ "chars": count.saturating_mul(1000) }));
    }
    let count: u64 = token.parse().ok()?;
    Some(json!({ "blocks": count }))
}
