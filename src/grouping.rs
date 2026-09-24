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

use crate::entry::Entry;

/// A node's slice of the transcript: `entries[start..end]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: usize,
    /// Exclusive.
    pub end: usize,
}

/// Group transcript entries into renderable nodes.
///
/// The only place that knows a request+result pair is one visual node, so
/// grouping and rendering can never disagree about what "one node" is.
pub fn blocks(entries: &[Entry]) -> Vec<Range> {
    let mut out = Vec::new();
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
