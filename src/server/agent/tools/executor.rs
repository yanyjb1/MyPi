//! Front door: one round of calls enters here. Three things happen exactly once — name lookup,
//! argument decode, output measurement — and a tool never re-does any of them.

use anyhow::{Result, anyhow};

use super::registry;
use super::spec::{Args, Env, Spill, ToolOutput, ToolProgress};
use crate::server::agent::request::message::ToolCall;
use crate::server::config::OutputLimits;

/// Run one tool call. Two gates: is it compiled in (the table's business), and does the active
/// profile allow it (the same list the roster read, so the two cannot disagree).
pub fn execute(
    env: &mut Env,
    call: &ToolCall,
    progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let tool = registry::spec(call.name()).ok_or_else(|| unknown(call.name()))?;
    anyhow::ensure!(
        env.allows(tool.name),
        "tool `{}` is disabled by the active profile",
        tool.name
    );
    anyhow::ensure!(
        registry::available(env, tool.name),
        "`{}` is unavailable in this session",
        tool.name
    );
    let args = Args::fill(tool.params, &call.function.arguments)?;
    let mut out = (tool.run)(env, &args, progress)?;
    let ts = crate::server::platform::time::now_stamp();
    for effect in &mut out.effects {
        effect.call_id = call.id.clone();
        effect.ts = ts.clone();
    }
    Ok(spill(tool.name, env.limits, out))
}

/// Measure: too big → split in two. The full text goes to `spill`, `text` keeps the visible part;
/// storage files the full text as an artifact and appends the reference.
fn spill(tool: &str, limits: OutputLimits, mut out: ToolOutput) -> ToolOutput {
    if out.spill.is_some() {
        return out;
    }
    if out.text.lines().count() <= limits.max_lines && out.text.len() <= limits.max_bytes {
        return out;
    }
    let head = visible(&out.text, limits);
    out.spill = Some(Spill {
        name: tool.to_string(),
        content: std::mem::replace(&mut out.text, head),
    });
    out
}

/// The visible part: whole lines, until either the line cap or the byte cap runs out. A first line
/// over the byte cap is cut at a character boundary.
fn visible(text: &str, limits: OutputLimits) -> String {
    let mut out = String::new();
    for line in text.lines().take(limits.max_lines) {
        let room = limits.max_bytes.saturating_sub(out.len() + 1);
        if room == 0 {
            break;
        }
        if line.len() <= room {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        out.push_str(&line[..floor_boundary(line, room)]);
        break;
    }
    out.trim_end_matches('\n').to_string()
}

/// The largest character boundary within the first `bytes` bytes.
fn floor_boundary(line: &str, bytes: usize) -> usize {
    let mut end = bytes.min(line.len());
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// The error for a name the table does not hold. With `web` off those three tools are absent, not
/// disabled, so a typo and an uncompiled tool look alike here; this says which one it is.
fn unknown(name: &str) -> anyhow::Error {
    if !cfg!(feature = "web") && matches!(name, "fetch" | "browser" | "search") {
        return anyhow!("tool `{name}` is not compiled in (this build has the `web` feature off)");
    }
    anyhow!("unknown tool: {name}")
}
