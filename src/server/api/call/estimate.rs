//! Where the context stands: the newest stamp, or after a compaction a difference estimate.

use serde_json::Value;

use crate::server::agent::r#loop::Partial;

/// One lap's bill, stamped on the last block it landed.
pub(crate) struct Stamp {
    pub at: i64,
    pub prompt: u64,
    pub completion: u64,
}

/// The context in use, and whether that number is an estimate.
pub(crate) fn context(
    list: &[Value],
    running: Option<&Partial>,
    stamps: &[Stamp],
    baseline: Option<u64>,
) -> (Option<u64>, bool) {
    // A lap just landed: its prompt is the context, to the token.
    if let Some(partial) = running {
        return (Some(partial.context), false);
    }
    // The newest stamp by block id, not by arrival: a compaction's borrowed rounds hold old
    // blocks, and they are read after the summary that replaced them.
    let Some(newest) = stamps.iter().max_by_key(|stamp| stamp.at) else {
        return (baseline, baseline.is_some());
    };
    // The newest stamp settles it, unless a compaction round sits after its block.
    let cut = list.iter().any(|row| {
        row["is_compaction"].as_bool() == Some(true)
            && intervals(&row["ranges"])
                .iter()
                .any(|(lo, hi)| newest.at >= *lo && newest.at <= *hi)
    });
    if !cut {
        return (Some(newest.prompt), false);
    }
    match estimate(list, stamps, baseline) {
        Some(tokens) => (Some(tokens), true),
        None => (baseline, baseline.is_some()),
    }
}

/// The tail the compaction kept, plus the summary that replaced the rest.
///
/// The compaction round measured two things: what it sent (the region it replaced, plus the
/// system prompt and the manuals), and what the summary cost. The newest measurement before it
/// is the context as it stood. Subtracting the region from that leaves the tail, and the summary
/// is added back on top.
fn estimate(list: &[Value], stamps: &[Stamp], baseline: Option<u64>) -> Option<u64> {
    let at = list
        .iter()
        .rposition(|row| row["is_compaction"].as_bool() == Some(true))?;
    let spans = intervals(&list[at]["ranges"]);
    // The compaction round's own bill: the only stamped block in it is the summary.
    let summary = stamps.iter().rev().find(|stamp| {
        spans
            .iter()
            .any(|(lo, hi)| stamp.at >= *lo && stamp.at <= *hi)
    })?;
    // The newest measurement before this round: the context as it stood before compaction.
    let first = spans.iter().map(|(lo, _)| *lo).min()?;
    let head = stamps.iter().rev().find(|stamp| stamp.at < first)?;

    // What the compaction sent, less the part every request carries anyway.
    let overhead = baseline.unwrap_or(0);
    let region = summary.prompt.saturating_sub(overhead);
    Some(head.prompt.saturating_sub(region) + summary.completion)
}

/// `[[lo, hi], ...]` as pairs.
fn intervals(raw: &Value) -> Vec<(i64, i64)> {
    raw.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let pair = item.as_array()?;
                    Some((pair.first()?.as_i64()?, pair.get(1)?.as_i64()?))
                })
                .collect()
        })
        .unwrap_or_default()
}
