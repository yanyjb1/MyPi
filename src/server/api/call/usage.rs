//! `usage` — tokens per model, the cache-hit share, and the price of each.

use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};
use crate::server::config::ResolvedModel;
use crate::server::storage::Reader;

#[derive(Default)]
struct Tally {
    rounds: i64,
    prompt: i64,
    completion: i64,
    cached: i64,
    written: i64,
}

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let reader = Reader::open(api.db())?;
    let turns = reader.turns(session)?;
    drop(reader);

    let mut groups: BTreeMap<String, Tally> = BTreeMap::new();
    for turn in &turns {
        let (Some(provider), Some(model)) = (turn["provider"].as_str(), turn["model"].as_str())
        else {
            continue;
        };
        let tally = groups.entry(format!("{provider}:{model}")).or_default();
        tally.rounds += 1;
        tally.prompt += turn["prompt_tokens"].as_i64().unwrap_or(0);
        tally.completion += turn["completion_tokens"].as_i64().unwrap_or(0);
        tally.cached += turn["cached_tokens"].as_i64().unwrap_or(0);
        tally.written += turn["cache_write_tokens"].as_i64().unwrap_or(0);
    }

    let cfg = api.cfg();
    let fx = cfg
        .get_json("config.tui.fx.usdCny", None)
        .ok()
        .and_then(|value| value.as_f64())
        .filter(|rate| *rate > 0.0)
        .unwrap_or(6.0);
    let mut prompt = 0i64;
    let mut completion = 0i64;
    let mut cached = 0i64;
    let mut written = 0i64;
    for tally in groups.values() {
        prompt += tally.prompt;
        completion += tally.completion;
        cached += tally.cached;
        written += tally.written;
    }
    let total = prompt + completion;
    let mut rows: Vec<Value> = Vec::with_capacity(groups.len());
    for (address, tally) in &groups {
        let (provider, model) = address.split_once(':').unwrap_or(("", address));
        let price = cfg
            .model_by_id(address)
            .map(|resolved| quote(&resolved, tally, fx));
        rows.push(json!({
            "address": address,
            "provider": provider,
            "model": model,
            "rounds": tally.rounds,
            "prompt_tokens": tally.prompt,
            "completion_tokens": tally.completion,
            "cached_tokens": tally.cached,
            "cache_write_tokens": tally.written,
            "share": share(tally.prompt + tally.completion, total),
            "price": price,
        }));
    }
    rows.sort_by(|left, right| {
        let share = |row: &Value| row["share"].as_f64().unwrap_or(0.0);
        share(right)
            .partial_cmp(&share(left))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "cached_tokens": cached,
        "cache_write_tokens": written,
        "total_tokens": total,
        "cache_hit_rate": share(cached, total),
        "groups": rows,
    }))
}

/// What one group cost, in the model's own currency and in both display currencies.
fn quote(model: &ResolvedModel, tally: &Tally, fx: f64) -> Value {
    let per_million = |tokens: i64, price: f64| tokens as f64 / 1_000_000.0 * price;
    let cost = model.entry.cost;
    let input = per_million(tally.prompt, cost.input);
    let output = per_million(tally.completion, cost.output);
    let read = per_million(tally.cached, cost.cache_read);
    let write = per_million(tally.written, cost.cache_write);
    let total = input + output + read + write;
    let (cny, usd) = match model.entry.currency {
        crate::server::config::Currency::CNY => (total, total / fx),
        crate::server::config::Currency::USD => (total * fx, total),
    };
    json!({
        "currency": model.entry.currency.as_str(),
        "input": input,
        "output": output,
        "cache_read": read,
        "cache_write": write,
        "total": total,
        "cny": cny,
        "usd": usd,
    })
}

fn share(part: i64, whole: i64) -> f64 {
    if whole <= 0 {
        return 0.0;
    }
    part as f64 / whole as f64
}
