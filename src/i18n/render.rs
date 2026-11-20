// One sentence: a keyed template with its `{name}` holes filled, or a loud miss.

use serde_json::Value;

use super::table::text;

// One sentence in this language; a key no table holds answers `<key> NOT FOUND`.
pub fn render(lang: &str, key: &str, args: &Value) -> String {
    match text(lang, key) {
        Some(template) => fill(template, args),
        None => format!("{key} NOT FOUND"),
    }
}

fn fill(template: &str, args: &Value) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        let Some(end) = tail.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &tail[..end];
        match args.get(name) {
            Some(Value::String(text)) => out.push_str(text),
            Some(Value::Number(number)) => out.push_str(&number.to_string()),
            Some(Value::Bool(true)) => out.push_str("true"),
            Some(Value::Bool(false)) => out.push_str("false"),
            Some(Value::Array(items)) => {
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&text_of(item));
                }
            }
            Some(other) => out.push_str(&text_of(other)),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
