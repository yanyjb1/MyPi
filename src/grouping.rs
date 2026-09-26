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
//! the renderer (`tui`), the compactor (`server`) and the store (which writes
//! one block per node, [`chunks`]) share it without the server layer having to
//! depend on the terminal layer.

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

/// Group transcript entries into nodes, in **arrival order** — the storage
/// shape. One range per node; request+result pairs glue, everything else stands
/// alone (the only place that knows what "one node" is, so grouping, storage
/// and rendering can never disagree).
///
/// This is what a block is on disk: written where it happened, with no
/// reordering. Pinned notices keep their arrival position here; hoisting them
/// is a display act ([`blocks`]), never something storage should freeze — a
/// stored block id must not depend on how a later front end decides to draw.
pub fn chunks(entries: &[Entry]) -> Vec<Range> {
    let mut out: Vec<Range> = Vec::with_capacity(entries.len());
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
            out.push(Range {
                start: i,
                end: i + 2,
            });
            i += 2;
            continue;
        }
        out.push(Range {
            start: i,
            end: i + 1,
        });
        i += 1;
    }
    out
}

/// [`chunks`] in **render order**: pinned notices hoisted to the head, in
/// arrival order among themselves. The hoist is a *slot*, not a freeze: the
/// pinned block is an ordinary node in the array — wheeling up scrolls past it
/// like any other; it simply always renders first whenever the viewport covers
/// the array top.
pub fn blocks(entries: &[Entry]) -> Vec<Range> {
    let mut out: Vec<Range> = Vec::with_capacity(entries.len());
    let mut rest: Vec<Range> = Vec::with_capacity(entries.len());
    for r in chunks(entries) {
        if matches!(entries[r.start], Entry::System { pin: true, .. }) {
            out.push(r);
        } else {
            rest.push(r);
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
