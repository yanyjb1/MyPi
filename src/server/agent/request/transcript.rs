//! Stored messages to wire messages: one translation, no injection.

use std::collections::{BTreeMap, BTreeSet};

use super::message::{FunctionCall, Message, WireCall, WireMessage, complete_args};

/// State that carries across one rebuild.
#[derive(Default)]
struct Rebuild {
    /// How often one call id appeared so far; duplicates are renamed, never merged.
    seen: BTreeMap<String, u32>,
    /// A thinking chain waiting for the assistant message it belongs to.
    pending_reasoning: Option<String>,
}

/// A user message's text: the injected parts froze when it was written.
fn user_text(prefix: Option<&str>, content: &str, suffix: Option<&str>) -> String {
    let mut out = String::with_capacity(
        prefix.map_or(0, str::len) + content.len() + suffix.map_or(0, str::len),
    );
    if let Some(prefix) = prefix {
        out.push_str(prefix);
    }
    out.push_str(content);
    if let Some(suffix) = suffix {
        out.push_str(suffix);
    }
    out
}

/// Map one stored sequence onto the wire.
pub fn to_wire(messages: &[Message]) -> Vec<WireMessage> {
    walk(messages, &mut Rebuild::default())
}

fn walk(messages: &[Message], state: &mut Rebuild) -> Vec<WireMessage> {
    let mut out: Vec<WireMessage> = Vec::new();
    let mut pending_text: Option<String> = None;
    // Only calls with a result may go out: a half-open `tool_calls` gets the request rejected.
    let mut answered: BTreeSet<String> = BTreeSet::new();
    for message in messages {
        if let Message::Result { call_id, .. } = message {
            answered.insert(call_id.clone());
        }
    }

    let mut i = 0;
    while i < messages.len() {
        match &messages[i] {
            Message::User {
                prefix,
                content,
                suffix,
            } => {
                out.push(WireMessage::User {
                    content: user_text(prefix.as_deref(), content, suffix.as_deref()).into(),
                });
                i += 1;
            }
            Message::Assistant { content } => {
                if matches!(
                    messages.get(i + 1),
                    Some(Message::Call { .. } | Message::Result { .. })
                ) {
                    pending_text = Some(content.clone());
                    i += 1;
                    continue;
                }
                out.push(WireMessage::Assistant {
                    content: Some(content.clone()),
                    tool_calls: Vec::new(),
                    reasoning: state.pending_reasoning.take(),
                });
                i += 1;
            }
            Message::Reasoning { content } => {
                state.pending_reasoning = Some(content.clone());
                i += 1;
            }
            Message::Call { .. } | Message::Result { .. } => {
                let end = messages[i..]
                    .iter()
                    .position(|m| !matches!(m, Message::Call { .. } | Message::Result { .. }))
                    .map(|offset| i + offset)
                    .unwrap_or(messages.len());
                one_batch(
                    &messages[i..end],
                    &mut out,
                    state,
                    pending_text.take(),
                    &answered,
                );
                i = end;
            }
        }
    }
    out
}

/// One run of calls and results: an assistant message per batch, then its results.
fn one_batch(
    batch: &[Message],
    out: &mut Vec<WireMessage>,
    state: &mut Rebuild,
    mut opening_text: Option<String>,
    answered: &BTreeSet<String>,
) {
    // Pair the nth request with the nth result; rename duplicates; drop pairs the wire cannot take.
    let mut wire_ids: BTreeMap<(String, u32), (String, Option<String>)> = BTreeMap::new();
    let mut in_batch: BTreeMap<String, u32> = BTreeMap::new();
    for message in batch {
        if let Message::Call {
            call_id,
            name,
            args,
            ..
        } = message
        {
            let seen = *state.seen.entry(call_id.clone()).or_insert(0);
            *state.seen.get_mut(call_id).expect("just inserted") += 1;
            let local = *in_batch.entry(call_id.clone()).or_insert(0);
            *in_batch.get_mut(call_id).expect("just inserted") += 1;
            let arguments = if call_id.trim().is_empty() || name.trim().is_empty() {
                None
            } else {
                replayed_args(args)
            };
            let wire = if seen == 0 {
                call_id.clone()
            } else {
                format!("{call_id}_dup{seen}")
            };
            wire_ids.insert((call_id.clone(), local), (wire, arguments));
        }
    }

    type Group = (Option<String>, Vec<WireCall>, Vec<(String, String)>);
    let mut groups: Vec<Group> = Vec::new();
    let mut request_nth: BTreeMap<String, u32> = BTreeMap::new();
    let mut result_nth: BTreeMap<String, u32> = BTreeMap::new();

    for message in batch {
        let (call_id, is_call) = match message {
            Message::Call { call_id, .. } => (call_id.clone(), true),
            Message::Result { call_id, .. } => (call_id.clone(), false),
            _ => unreachable!("a batch holds only calls and results"),
        };
        let counter = if is_call {
            &mut request_nth
        } else {
            &mut result_nth
        };
        let nth = *counter.entry(call_id.clone()).or_insert(0);
        *counter.get_mut(&call_id).expect("just inserted") += 1;
        let (wire_id, arguments) = wire_ids
            .get(&(call_id.clone(), nth))
            .map(|(wire, arguments)| (wire.clone(), arguments.clone()))
            .unwrap_or_else(|| (call_id.clone(), None));

        match message {
            Message::Call { name, first, .. } => {
                if *first || groups.is_empty() {
                    groups.push((opening_text.take(), Vec::new(), Vec::new()));
                }
                let Some(arguments) = arguments else {
                    continue;
                };
                if !answered.contains(&call_id) {
                    continue;
                }
                groups.last_mut().expect("just pushed").1.push(WireCall {
                    id: wire_id,
                    kind: "function".into(),
                    function: FunctionCall {
                        name: name.clone(),
                        arguments,
                    },
                });
            }
            Message::Result { result, .. } => {
                // A call the wire cannot carry takes its result with it: a lone tool reply is worse.
                if arguments.is_none() {
                    continue;
                }
                if let Some(group) = groups.last_mut() {
                    group.2.push((wire_id, result.clone()));
                }
            }
            _ => unreachable!("a batch holds only calls and results"),
        }
    }

    for (opening, calls, results) in groups {
        if calls.is_empty() {
            continue;
        }
        out.push(WireMessage::Assistant {
            content: opening.filter(|t| !t.is_empty()),
            tool_calls: calls,
            // The thinking chain preceded the whole run; only its first batch gets it.
            reasoning: state.pending_reasoning.take(),
        });
        for (call_id, result) in results {
            out.push(WireMessage::Tool {
                tool_call_id: call_id,
                content: result,
            });
        }
    }
}

/// One call's arguments as the wire takes them: an empty set becomes `{}`, anything else goes out
/// byte for byte, and a row that was never a whole call stays out.
fn replayed_args(args: &str) -> Option<String> {
    if !complete_args(args) {
        return None;
    }
    Some(if args.trim().is_empty() {
        "{}".to_string()
    } else {
        args.to_string()
    })
}
