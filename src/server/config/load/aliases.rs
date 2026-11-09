//! `name=command` entries of the user alias list.

/// The name before the first `=`, the command after it; both trimmed and non-empty.
pub(super) fn split(entry: &str) -> Result<(String, String), String> {
    let Some((name, command)) = entry.split_once('=') else {
        return Err("expected `name=command`".into());
    };
    let name = name.trim();
    let command = command.trim();
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        return Err("the name is empty or has whitespace".into());
    }
    if command.is_empty() {
        return Err("the command is empty".into());
    }
    Ok((name.to_string(), command.to_string()))
}
