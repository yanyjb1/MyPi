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
    let Some(newest) = stamps.last() else {
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
    match estimate(list, stamps) {
        Some(tokens) => (Some(tokens), true),
        None => (baseline, baseline.is_some()),
    }
}

/// The retained tail plus the summary's reply, after a compaction.
fn estimate(list: &[Value], stamps: &[Stamp]) -> Option<u64> {
    let at = list
        .iter()
        .rposition(|row| row["is_compaction"].as_bool() == Some(true))?;
    // The summary stamp: the compaction round's own bill.
    let spans = intervals(&list[at]["ranges"]);
    let summary = stamps.iter().rev().find(|stamp| {
        spans
            .iter()
            .any(|(lo, hi)| stamp.at >= *lo && stamp.at <= *hi)
    })?;
    // The cut: where the kept intervals begin.
    let kept: Vec<(i64, i64)> = list[at + 1..]
        .iter()
        .filter(|row| row["stop_reason"].as_str() == Some("borrowed"))
        .flat_map(|row| intervals(&row["ranges"]))
        .collect();
    let cut = kept.iter().map(|(lo, _)| *lo).min()?;

    // The newest stamp before the cut stands for the tail.
    let head = stamps.iter().rev().find(|stamp| stamp.at < cut)?;
    let tail = share(head.prompt, head.at, head.at - cut + 1)?;
    Some(tail + summary.completion)
}

/// `whole` spread evenly over `whole_span` blocks, the part `at` blocks stand for.
fn share(whole: u64, whole_span: i64, at: i64) -> Option<u64> {
    if whole_span <= 0 || at <= 0 {
        return None;
    }
    Some(((whole as u128) * (at as u128) / (whole_span as u128)) as u64)
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
