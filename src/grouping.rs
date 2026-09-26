//! Transcript grouping — how entries become renderable nodes.
//!
//! A **block** is one transcript node as a human sees it: a user card, an
//! assistant reply (with its thinking), a glued tool exchange, a system
//! notice. This is the single authority on that shape: request+result pairs
//! (`tool_request` + its result, same call id) glue into one node; everything
//! else stands alone.
//!
//! Lives outside `tui` on purpose. The grouping is pure domain logic — it
//! reads entries and returns index ranges, touching nothing graphical — so
//! both the renderer (`tui`) and the compactor (`server`) share it without
//! the server layer having to depend on the terminal layer.

use crate::server::entry::Entry;

/// A node's slice of the transcript: `entries[start..end]`.
///
/// 这就是一个块的**身份**：块的序号会因置顶重排而漂移，`(start, end)`
/// 不会。缓存拿它当键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Range {
    pub start: usize,
    /// Exclusive.
    pub end: usize,
}

/// Group transcript entries into renderable nodes.
///
/// The only place that knows a request+result pair is one visual node, so
/// grouping and rendering can never disagree about what "one node" is.
///
/// **Pinned notices**: a pinned `Entry::System` (the only kind allowed to
/// declare pinning) is hoisted to the head of the returned list, in arrival
/// order among themselves. Everything else keeps arrival order. The hoist
/// is a *slot*, not a freeze: the pinned block is an ordinary node in the
/// array — wheeling up scrolls past it like any other; it simply always
/// renders first whenever the viewport covers the array top.
pub fn blocks(entries: &[Entry]) -> Vec<Range> {
    // Pass 1: group in arrival order, remembering which nodes are pinned.
    struct Node {
        range: Range,
        pinned: bool,
    }
    let mut nodes: Vec<Node> = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        // A result immediately following its request, same id: one exchange.
        if let (
            Entry::ToolRequest { call_id, name, .. },
            Some(Entry::ToolResult {
                call_id: rid,
                name: rname,
                ..
            }),
        ) = (&entries[i], entries.get(i + 1))
            && call_id == rid
            && name == rname
        {
            nodes.push(Node {
                range: Range {
                    start: i,
                    end: i + 2,
                },
                pinned: false,
            });
            i += 2;
            continue;
        }
        let pinned = matches!(
            &entries[i],
            Entry::System { pin: true, .. }
        );
        nodes.push(Node {
            range: Range {
                start: i,
                end: i + 1,
            },
            pinned,
        });
        i += 1;
    }
    // Pass 2: stable partition — pinned first (their relative arrival
    // order preserved by the stable drain), then the unpinned queue.
    let mut out: Vec<Range> = Vec::with_capacity(nodes.len());
    let mut rest: Vec<Range> = Vec::with_capacity(nodes.len());
    for n in nodes {
        if n.pinned {
            out.push(n.range);
        } else {
            rest.push(n.range);
        }
    }
    out.extend(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::entry::Align;

    fn user(s: &str) -> Entry {
        Entry::User { content: s.into() }
    }

    fn req(id: &str) -> Entry {
        Entry::ToolRequest {
            call_id: id.into(),
            name: "bash".into(),
            args: "{}".into(),
            intent: String::new(),
            text: String::new(),
            first: true,
        }
    }

    fn res(id: &str) -> Entry {
        Entry::ToolResult {
            call_id: id.into(),
            name: "bash".into(),
            ok: true,
            result: "out".into(),
            details: None,
            duration_ms: 0,
        }
    }

    #[test]
    fn request_result_pairs_glue_into_one_block() {
        let es = vec![user("问"), req("c1"), res("c1"), user("再问")];
        let b = blocks(&es);
        assert_eq!(b.len(), 3);
        assert_eq!((b[1].start, b[1].end), (1, 3));
    }

    #[test]
    fn unpaired_result_is_its_own_block() {
        let es = vec![user("问"), res("c1")];
        let b = blocks(&es);
        assert_eq!(b.len(), 2);
        assert_eq!((b[1].start, b[1].end), (1, 2));
    }

    #[test]
    fn mismatched_ids_do_not_glue() {
        let es = vec![req("c1"), res("c2")];
        let b = blocks(&es);
        assert_eq!(b.len(), 2);
    }

    fn sys(text: &str, pin: bool) -> Entry {
        Entry::System {
            text: text.into(),
            align: Align::Center,
            pin,
        }
    }

    #[test]
    fn pinned_notice_hoists_to_head_in_arrival_order() {
        let es = vec![
            user("一"),
            sys("置顶甲", true),
            user("二"),
            sys("置顶乙", true),
        ];
        let b = blocks(&es);
        // Both pinned nodes lead, keeping first-come order among themselves.
        assert_eq!(b[0], Range { start: 1, end: 2 });
        assert_eq!(b[1], Range { start: 3, end: 4 });
        // The unpinned queue follows in arrival order.
        assert_eq!(b[2], Range { start: 0, end: 1 });
        assert_eq!(b[3], Range { start: 2, end: 3 });
    }

    #[test]
    fn unpinned_system_queues_like_everything_else() {
        let es = vec![user("一"), sys("普通通知", false)];
        let b = blocks(&es);
        assert_eq!(b[0], Range { start: 0, end: 1 }, "未置顶不插队");
        assert_eq!(b[1], Range { start: 1, end: 2 });
    }

    #[test]
    fn only_system_may_pin_others_queue() {
        // Even a hypothetical flood of pinned notices cannot reorder the
        // unpinned majority: user/tool entries never hoist.
        let es = vec![user("一"), req("c1"), res("c1"), sys("置顶", true)];
        let b = blocks(&es);
        assert_eq!(b[0], Range { start: 3, end: 4 }, "置顶系统消息在队首");
        assert_eq!(b[1], Range { start: 0, end: 1 });
        assert_eq!((b[2].start, b[2].end), (1, 3), "工具对仍粘合");
    }
}
