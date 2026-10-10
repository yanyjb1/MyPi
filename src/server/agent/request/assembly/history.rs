//! Stored messages → wire messages: a pure mapping, nothing injected, with a half tool round repaired.

use crate::server::agent::request::message::{
    FunctionCall, Message, ToolCall, WireMessage,
};

/// The state that carries across a rebuild.
#[derive(Default)]
pub(crate) struct Rebuild {
    /// How often one tool-call id appears in the **whole** history (forked or imported duplicates may
    /// sit rounds apart).
    id_seen: std::collections::BTreeMap<String, u32>,
    /// A thinking chain not yet attached to its assistant message.
    pending_reasoning: Option<String>,
}

/// One user message's text: prefix + body + suffix, concatenated. The injected parts froze when it
/// was written.
fn assemble_user_text(prefix: Option<&str>, content: &str, suffix: Option<&str>) -> String {
    let cap = prefix.map_or(0, str::len) + content.len() + suffix.map_or(0, str::len);
    let mut out = String::with_capacity(cap);
    if let Some(p) = prefix {
        out.push_str(p);
    }
    out.push_str(content);
    if let Some(s) = suffix {
        out.push_str(s);
    }
    out
}

/// Map a run of messages, one by one, into protocol messages.
pub(crate) fn to_wire(
    messages: &[Message],
    st: &mut Rebuild,
) -> Vec<WireMessage> {
    let mut rebuilt: Vec<WireMessage> = Vec::new();
    // Only calls with a result count: a call without one is a dangling tail, not a half-open `tool_calls`.
    let mut complete: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for e in messages {
        if let Message::ToolResult { call_id, .. } = e {
            complete.insert(call_id.clone());
        }
    }
    let mut i = 0;
    while i < messages.len() {
        match &messages[i] {
            Message::User {
                content,
                prefix,
                suffix,
            } => {
                rebuilt.push(WireMessage::User {
                    content: assemble_user_text(prefix.as_deref(), content, suffix.as_deref())
                        .into(),
                });
                i += 1;
            }
            Message::Assistant { content, .. } => {
                rebuilt.push(WireMessage::Assistant {
                    content: Some(content.clone()),
                    tool_calls: Vec::new(),
                    reasoning: st.pending_reasoning.take(),
                });
                i += 1;
            }
            // The thinking chain is its own record right before its message; whether it goes out is
            // `messages_to_wire`'s call.
            Message::Reasoning { content } => {
                st.pending_reasoning = Some(content.clone());
                i += 1;
            }
            // A run of tool messages is one assistant message plus its results, rebuilt in the original
            // shape (bytes matter, or the prefix cache goes cold).
            Message::ToolRequest { .. } | Message::ToolResult { .. } => {
                let end = messages[i..]
                    .iter()
                    .position(|e| {
                        !matches!(e, Message::ToolRequest { .. } | Message::ToolResult { .. })
                    })
                    .map(|p| i + p)
                    .unwrap_or(messages.len());
                let block = &messages[i..end];
                // Group by call: a `first` request opens a group, the rest of the batch follows.
                type Group = (
                    String,
                    Vec<ToolCall>,
                    Vec<(String, String)>,
                );
                // Two things that get the **whole request** rejected: blank ids/names (skipped with
                // their result) and duplicate ids.
                let mut plan: std::collections::BTreeMap<(String, u32), (String, bool)> =
                    Default::default();
                // Keyed by `(call_id, occurrence in this block)`, the block being the unit below; the
                // rename itself uses the global counter.
                let mut in_block: std::collections::BTreeMap<String, u32> = Default::default();
                for e in block {
                    if let Message::ToolRequest { call_id, name, .. } = e {
                        let global = *st.id_seen.entry(call_id.clone()).or_insert(0);
                        *st.id_seen.get_mut(call_id).expect("just inserted") += 1;
                        let local = *in_block.entry(call_id.clone()).or_insert(0);
                        *in_block.get_mut(call_id).expect("just inserted") += 1;
                        let malformed = call_id.trim().is_empty() || name.trim().is_empty();
                        let wire = if global == 0 {
                            call_id.clone()
                        } else {
                            format!("{call_id}_dup{global}")
                        };
                        plan.insert((call_id.clone(), local), (wire, malformed));
                    }
                }

                let mut groups: Vec<Group> = Vec::new();
                // Requests and results count occurrences separately: the nth request pairs with the nth result.
                let mut req_nth: std::collections::BTreeMap<String, u32> = Default::default();
                let mut res_nth: std::collections::BTreeMap<String, u32> = Default::default();
                for e in block {
                    let (call_id, is_request) = match e {
                        Message::ToolRequest { call_id, .. } => (call_id.clone(), true),
                        Message::ToolResult { call_id, .. } => (call_id.clone(), false),
                        _ => unreachable!("block holds only tool messages"),
                    };
                    let counter = if is_request { &mut req_nth } else { &mut res_nth };
                    let n = *counter.entry(call_id.clone()).or_insert(0);
                    *counter.get_mut(&call_id).expect("just inserted") += 1;
                    let (wire_id, malformed) = plan
                        .get(&(call_id.clone(), n))
                        .map(|(w, m)| (w.clone(), *m))
                        .unwrap_or_else(|| (call_id.clone(), false));

                    match e {
                        Message::ToolRequest {
                            name,
                            args,
                            text,
                            first,
                            ..
                        } => {
                            if *first || groups.is_empty() {
                                groups.push((text.clone(), Vec::new(), Vec::new()));
                            }
                            if malformed {
                                continue;
                            }
                            if complete.contains(&call_id) {
                                let call = ToolCall {
                                    id: wire_id,
                                    kind: "function".into(),
                                    function: FunctionCall {
                                        name: name.clone(),
                                        // `args` is carried verbatim: the model
                                        // must see the call it actually made.
                                        arguments: args.clone(),
                                    },
                                };
                                groups.last_mut().expect("just pushed").1.push(call);
                            }
                        }
                        Message::ToolResult { result, .. } => {
                            if malformed {
                                continue;
                            }
                            if let Some(g) = groups.last_mut() {
                                g.2.push((wire_id, result.clone()));
                            }
                        }
                        _ => unreachable!("block holds only tool messages"),
                    }
                }
                for (text, calls, results) in groups {
                    // A group whose calls all dangle has nothing legal to emit: drop it.
                    if calls.is_empty() {
                        continue;
                    }
                    rebuilt.push(WireMessage::Assistant {
                        content: (!text.is_empty()).then_some(text),
                        tool_calls: calls,
                        // `take` gives it to the first group only: the thinking preceded the run.
                        reasoning: st.pending_reasoning.take(),
                    });
                    for (call_id, result) in results {
                        rebuilt.push(WireMessage::Tool {
                            tool_call_id: call_id,
                            content: result,
                        });
                    }
                }
                i = end;
            }
        }
    }
    rebuilt
}
