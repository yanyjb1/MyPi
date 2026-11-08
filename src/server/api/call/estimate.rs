//! Where the context stands.
//!
//! A stamped block reports the prompt its lap carried, so a measured answer needs nothing but the
//! newest stamp. A compaction is the one gap: the next prompt is the summary plus the retained
//! tail, and only those two parts are known. The tail is estimated from the turns the truncation
//! cut into — every request reports how much its own blocks added, so a turn's share is its
//! intervals' weight.

use serde_json::Value;

use crate::server::agent::r#loop::Partial;

/// The context in use, and whether that number is an estimate.
pub(crate) fn context(
    list: &[Value],
    running: Option<&Partial>,
    baseline: Option<u64>,
    measured: Option<u64>,
    measured_at: i64,
    summary: Option<u64>,
) -> (Option<u64>, bool) {
    // A lap just landed: its prompt is the context, to the token.
    if let Some(partial) = running {
        return (Some(partial.context), false);
    }
    // The newest stamp settles it, unless a compaction round sits after its block.
    let after = list.iter().any(|row| {
        row["is_compaction"].as_bool() == Some(true)
            && intervals(&row["ranges"])
                .iter()
                .any(|(lo, hi)| measured_at >= *lo && measured_at <= *hi)
    });
    if let Some(tokens) = measured.filter(|_| !after) {
        return (Some(tokens), false);
    }

    // No measurement since the last compaction: estimate it, or fall back to the archive's floor.
    match estimate(list, baseline, summary) {
        Some(tokens) => (Some(tokens), true),
        None => (baseline, baseline.is_some()),
    }
}

/// The summary plus the retained tail, plus the archive's fixed floor.
fn estimate(list: &[Value], baseline: Option<u64>, summary: Option<u64>) -> Option<u64> {
    let at = list
        .iter()
        .rposition(|row| row["is_compaction"].as_bool() == Some(true))?;
    let summary = summary?;
    // The tail: every borrowed round after the compaction carries the intervals it kept.
    let mut kept: Vec<(i64, i64)> = Vec::new();
    for row in &list[at + 1..] {
        if row["stop_reason"].as_str() != Some("borrowed") {
            continue;
        }
        kept.extend(intervals(&row["ranges"]));
    }

    // The retained tail's own weight: the summary request reports it; until one lands, the
    // borrowed intervals stand in at their block-count share of the archive's floor.
    let floor = baseline?;
    let weight = weight_of(&list[at + 1..], &kept);
    Some(floor + summary + ((floor as f64) * weight).round() as u64)
}

/// How much of the list's post-compaction rounds a set of kept intervals covers, 0..=1 by block count.
fn weight_of(rounds: &[Value], kept: &[(i64, i64)]) -> f64 {
    let all: Vec<(i64, i64)> = rounds
        .iter()
        .flat_map(|row| intervals(&row["ranges"]))
        .collect();
    let size: i64 = all.iter().map(|(lo, hi)| hi - lo + 1).sum();
    if size <= 0 {
        return 0.0;
    }
    let held: i64 = all
        .iter()
        .map(|(lo, hi)| {
            kept.iter()
                .map(|(klo, khi)| (hi.min(khi) - lo.max(klo) + 1).max(0))
                .sum::<i64>()
        })
        .sum();
    held as f64 / size as f64
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
