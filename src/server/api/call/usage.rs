//! The `/usage` bill: tokens per (provider, model), priced here from the stored tokens and the
//! price list `models.yml` carries. The front end draws; it never computes.

use anyhow::Result;
use rusqlite::Connection;

use crate::server::config::Config;
use crate::server::storage::RoundUsage;

/// What one model charges, per **million** tokens, in one display currency.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PriceSheet {
    /// Input that missed the cache.
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    /// What the host charges for tokens written into its cache. Hosts that
    /// never bill cache creation report nothing, and this cell stays zero.
    pub cache_write: f64,
    /// The currency symbol the four numbers are in (`¥` / `$`).
    pub symbol: String,
}

/// One model's tokens for a session, and the price sheet found for it (`None` = no price list).
#[derive(Debug, Clone, PartialEq)]
pub struct Rated {
    pub provider: Option<String>,
    pub model: String,
    /// How many rounds were asked.
    pub rounds: u64,
    pub usage: RoundUsage,
    pub sheet: Option<PriceSheet>,
}

/// One `/usage` row: **one model's** cumulative usage and bill within a session.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelUsage {
    /// The config id; even unrecognized, a front end can still draw the row.
    pub id: String,
    /// Owning provider; `None` = this row has no provider record.
    pub provider: Option<String>,
    /// How many rounds were asked.
    pub rounds: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    /// The reasoning part of `completion_tokens` (thinking models only).
    pub reasoning_tokens: u64,
    /// The part of `prompt_tokens` written into the cache (hosts that bill
    /// cache creation report it).
    pub cache_write_tokens: u64,
    /// What each of the four token kinds cost; `None` = no price sheet found.
    pub cost: Option<UsageCost>,
    /// The pricing currency's display symbol.
    pub currency: Option<String>,
}

/// The `/usage` bill's four cells.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct UsageCost {
    /// Input that missed the cache.
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// This session's tokens aggregated by (provider, model), in the order the models were first asked.
/// A round with no usage still counts into `rounds` with zero tokens.
pub fn tokens_by_model(conn: &Connection, session: i64) -> Result<Vec<Rated>> {
    let mut stmt = conn.prepare(
        "SELECT r.provider, r.model, COUNT(*),
                COALESCE(SUM(t.prompt_tokens), 0),
                COALESCE(SUM(t.completion_tokens), 0),
                COALESCE(SUM(t.cached_tokens), 0),
                COALESCE(SUM(t.reasoning_tokens), 0),
                COALESCE(SUM(t.cache_write_tokens), 0)
           FROM turns t JOIN session_turns s ON s.turn_id = t.turn_id
           JOIN request_meta r ON r.request_id = t.request_id
          WHERE s.session_id = ?1
          GROUP BY r.provider, r.model
          ORDER BY MIN(s.idx)",
    )?;
    let found = stmt.query_map([session], |r| {
        Ok(Rated {
            provider: r.get(0)?,
            model: r.get(1)?,
            rounds: r.get::<_, i64>(2)?.max(0) as u64,
            usage: RoundUsage {
                prompt_tokens: r.get::<_, i64>(3)?.max(0) as u64,
                completion_tokens: r.get::<_, i64>(4)?.max(0) as u64,
                cached_tokens: r.get::<_, i64>(5)?.max(0) as u64,
                reasoning_tokens: r.get::<_, i64>(6)?.max(0) as u64,
                cache_write_tokens: r.get::<_, i64>(7)?.max(0) as u64,
            },
            sheet: None,
        })
    })?;
    found
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

/// The bill for one session: tokens read from the store, prices read from the
/// config, the arithmetic done here.
pub fn bill_for(cfg: &Config, conn: &Connection, session: i64) -> Result<Vec<ModelUsage>> {
    let mut rated = tokens_by_model(conn, session)?;
    for r in &mut rated {
        r.sheet = sheet_of(cfg, r);
    }
    Ok(bill(&rated))
}

/// One model's price sheet, folded into the bill's display currency (`None` = no rates).
fn sheet_of(cfg: &Config, r: &Rated) -> Option<PriceSheet> {
    let provider = r.provider.as_deref()?;
    let rm = cfg.model_by_id(&format!("{provider}:{}", r.model)).ok()?;
    let cost = rm.entry.cost;
    if cost.input == 0.0 && cost.output == 0.0 && cost.cache_read == 0.0 && cost.cache_write == 0.0
    {
        return None;
    }
    let fx = &cfg.app.tui.fx;
    let (input, symbol) = fx.to_display(cost.input, rm.entry.currency);
    let (output, _) = fx.to_display(cost.output, rm.entry.currency);
    let (cache_read, _) = fx.to_display(cost.cache_read, rm.entry.currency);
    let (cache_write, _) = fx.to_display(cost.cache_write, rm.entry.currency);
    Some(PriceSheet {
        input,
        output,
        cache_read,
        cache_write,
        symbol: symbol.to_string(),
    })
}

/// Price a run of rated models: one row per model, four cells each.
pub fn bill(rated: &[Rated]) -> Vec<ModelUsage> {
    rated
        .iter()
        .map(|r| ModelUsage {
            id: r.model.clone(),
            provider: r.provider.clone(),
            rounds: r.rounds,
            prompt_tokens: r.usage.prompt_tokens,
            completion_tokens: r.usage.completion_tokens,
            cached_tokens: r.usage.cached_tokens,
            reasoning_tokens: r.usage.reasoning_tokens,
            cache_write_tokens: r.usage.cache_write_tokens,
            cost: r.sheet.as_ref().map(|s| cost_of(s, r.usage)),
            currency: r.sheet.as_ref().map(|s| s.symbol.clone()),
        })
        .collect()
}

/// What one round's tokens cost under one sheet: the prompt splits into cache-read, cache-write and
/// the rest; rates come per million tokens.
pub fn cost_of(sheet: &PriceSheet, u: RoundUsage) -> UsageCost {
    const PER_MILLION: f64 = 1_000_000.0;
    let cached = u.cached_tokens.min(u.prompt_tokens);
    let written = u
        .cache_write_tokens
        .min(u.prompt_tokens.saturating_sub(cached));
    let fresh = u
        .prompt_tokens
        .saturating_sub(cached)
        .saturating_sub(written);
    UsageCost {
        input: fresh as f64 * sheet.input / PER_MILLION,
        output: u.completion_tokens as f64 * sheet.output / PER_MILLION,
        cache_read: cached as f64 * sheet.cache_read / PER_MILLION,
        cache_write: written as f64 * sheet.cache_write / PER_MILLION,
    }
}
