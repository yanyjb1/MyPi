// Paths as text: the one spelling every short path the front end shows takes.

const VERBATIM: &str = r"\\?\";

// A path as this front end spells it: no windows `\\?\` prefix, forward slashes only.
fn plain(text: &str) -> String {
    let text = text.strip_prefix(VERBATIM).unwrap_or(text);
    if text.contains('\\') {
        text.replace('\\', "/")
    } else {
        text.to_string()
    }
}

// The last two components, no leading slash: `\\?\C:\Users\Arisha\AppData\Local\Temp` -> `Local/Temp`.
pub fn short(text: &str) -> String {
    let text = plain(text);
    let parts: Vec<&str> = text.split('/').filter(|part| !part.is_empty()).collect();
    match parts.len() {
        0 => "/".into(),
        1 => parts[0].into(),
        _ => format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]),
    }
}

// `path` as `base` names it: `./x` below the base, `.` for the base itself, `..` one level up, else as it stands.
pub fn relative(base: &str, path: &str) -> String {
    let text = plain(path);
    let base = plain(base);
    let base = base.trim_end_matches('/');
    if base.is_empty() {
        return text;
    }
    if let Some(rest) = text.strip_prefix(base) {
        if rest.is_empty() {
            return ".".to_string();
        }
        if let Some(rest) = rest.strip_prefix('/') {
            return format!("./{rest}");
        }
    }
    let parent = base
        .rsplit_once('/')
        .map(|(head, _)| if head.is_empty() { "/" } else { head });
    if parent == Some(text.as_str()) {
        return "..".to_string();
    }
    text
}
