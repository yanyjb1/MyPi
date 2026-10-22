//! Search tool — routing over the engine modules in `providers/`.
//!
//! Two ways to reach the keyless engines:
//!
//! - **chain** (`engine: "auto"`, the default): try them one at a time,
//!   fastest first, and stop at the first engine that answers. Cheap, and it
//!   is what a single query wants.
//! - **aggregate** (`engine: "public"`): fan out to all of them in parallel,
//!   then merge by cross-engine consensus — omp's `public` provider. Slower
//!   per call and better per query, because agreement between independent
//!   indexes is the cheapest relevance signal available without a ranker.
//!
//! Either way the hits then pass through the lenient constraint filter, and
//! whichever constraints had to be relaxed are reported ahead of the results.

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::hit::{SearchHit, dedup_key};
use super::providers::{self, Ctx, Recency};
use super::query::{self, StructuredQuery};
use crate::server::ai::config::BrowserConfig;

pub(super) const DEFAULT_LIMIT: usize = 8;
pub(super) const MAX_LIMIT: usize = 20;

/// Aggregate deadlines, omp's numbers: return as soon as every engine has
/// settled or the soft deadline passes with something in hand, and never hold
/// the tool call past the hard deadline because one engine is slow.
const SOFT_DEADLINE: Duration = Duration::from_secs(5);
const HARD_DEADLINE: Duration = Duration::from_secs(30);

/// Which engine(s) to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// Chain: try each engine in turn, stop at the first that answers.
    Auto,
    /// Aggregate: query every keyless engine in parallel and merge.
    Public,
    Bing,
    Duckduckgo,
    Google,
    Ecosia,
    Startpage,
    Mojeek,
}

/// The keyless engines, fastest and most reliable first. The chain walks it in
/// order; the aggregate uses the position to break ranking ties.
const CHAIN: [Engine; 6] = [
    Engine::Bing,
    Engine::Duckduckgo,
    Engine::Startpage,
    Engine::Google,
    Engine::Ecosia,
    Engine::Mojeek,
];

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Engine::Auto => "auto",
            Engine::Public => "public",
            Engine::Bing => providers::bing::LABEL,
            Engine::Duckduckgo => providers::duckduckgo::LABEL,
            Engine::Google => providers::google::LABEL,
            Engine::Ecosia => providers::ecosia::LABEL,
            Engine::Startpage => providers::startpage::LABEL,
            Engine::Mojeek => providers::mojeek::LABEL,
        }
    }

    /// Run one engine. The two routing pseudo-engines have no implementation
    /// of their own — they are expanded by [`search`].
    fn run(self, ctx: &Ctx<'_>) -> anyhow::Result<Vec<SearchHit>> {
        match self {
            Engine::Bing => providers::bing::search(ctx),
            Engine::Duckduckgo => providers::duckduckgo::search(ctx),
            Engine::Google => providers::google::search(ctx),
            Engine::Ecosia => providers::ecosia::search(ctx),
            Engine::Startpage => providers::startpage::search(ctx),
            Engine::Mojeek => providers::mojeek::search(ctx),
            Engine::Auto | Engine::Public => {
                anyhow::bail!("{}: not a single engine", self.label())
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchArgs {
    /// 一句话说明这次调用要干什么，中文，会显示给用户看
    pub intent: String,
    /// 搜索词，支持 site: -词 "短语" filetype: inurl: intitle: before:/after: OR 等高级语法
    pub query: String,
    /// 返回条数，默认 8，上限 20
    pub limit: Option<usize>,
    /// 只看某个时间窗内发布的结果（day/week/month/year）
    pub recency: Option<Recency>,
    /// 引擎：auto（默认，逐个尝试）/ public（全部并行、按共识合并）/ 单个引擎名
    pub engine: Option<Engine>,
}

/// What the tool hands back: the hits, plus the notes the model should read
/// first (engines that were skipped, constraints that were relaxed).
pub struct SearchOutcome {
    pub hits: Vec<SearchHit>,
    pub notes: Vec<String>,
}

/// Parse arguments exactly as the model sent them (already JSON string).
pub fn parse_search_args(arguments: &str) -> anyhow::Result<SearchArgs> {
    serde_json::from_str(arguments).map_err(|e| anyhow::anyhow!("bad search args: {e}"))
}

/// One line of note, clipped so a chatty engine error cannot crowd out the
/// results it is annotating.
fn clipped(message: &str, max: usize) -> String {
    let cleaned: String = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= max {
        return cleaned;
    }
    let cut: String = cleaned.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

fn ctx_for<'a>(
    query: &'a str,
    parsed: &'a StructuredQuery,
    limit: usize,
    recency: Option<Recency>,
    browser: &'a BrowserConfig,
) -> Ctx<'a> {
    Ctx {
        query,
        parsed,
        limit,
        recency,
        browser,
    }
}

/// Sequential fallback, omp's role chain in miniature: advance on any failure
/// or empty result, remember what was skipped, stop at the first answer.
fn chain(
    query: &str,
    parsed: &StructuredQuery,
    limit: usize,
    recency: Option<Recency>,
    browser: &BrowserConfig,
) -> (Vec<SearchHit>, Vec<String>) {
    let mut failures: Vec<String> = Vec::new();
    for engine in CHAIN {
        // Bing is handed plain keywords and drops any operator it is given,
        // so a directive-bearing query is not its job at all.
        if engine == Engine::Bing && parsed.has_directives {
            continue;
        }
        let ctx = ctx_for(query, parsed, limit, recency, browser);
        match engine.run(&ctx) {
            Ok(hits) if !hits.is_empty() => {
                let notes = failures
                    .iter()
                    .map(|f| format!("{f} — these results come from {}", engine.label()))
                    .collect();
                return (hits, notes);
            }
            Ok(_) => failures.push(format!("{}: no results", engine.label())),
            Err(e) => failures.push(clipped(&e.to_string(), 90)),
        }
    }
    (Vec::new(), failures)
}

/// One engine's contribution to the aggregate, as the worker thread reports it.
type EngineReport = (Engine, anyhow::Result<Vec<SearchHit>>);

/// Fan out over every keyless engine in parallel and merge by consensus.
///
/// Ranking is omp's: how many engines returned the URL (descending), then the
/// best per-engine rank (ascending), then engine order and first-seen order so
/// ties stay deterministic. A URL's label follows the engine that ranked it
/// best, which is what makes the merged list still nameable.
fn aggregate(
    query: &str,
    parsed: &StructuredQuery,
    limit: usize,
    recency: Option<Recency>,
    browser: &BrowserConfig,
) -> (Vec<SearchHit>, Vec<String>) {
    let (tx, rx) = mpsc::channel::<EngineReport>();
    for engine in CHAIN {
        let tx = tx.clone();
        let query = query.to_string();
        let parsed = parsed.clone();
        let browser = browser.clone();
        std::thread::spawn(move || {
            let ctx = ctx_for(&query, &parsed, limit, recency, &browser);
            let _ = tx.send((engine, engine.run(&ctx)));
        });
    }
    drop(tx);

    // Collect until the soft deadline — but only "whatever we have" counts
    // once something succeeded: with zero successes the wait extends to the
    // hard deadline so a slow field degrades to fewer engines rather than to
    // an empty answer. Stragglers are detached; each has its own transport
    // deadline, and their late sends fail harmlessly once `rx` is dropped.
    let started = Instant::now();
    let mut reports: Vec<EngineReport> = Vec::with_capacity(CHAIN.len());
    let mut succeeded = false;
    while reports.len() < CHAIN.len() {
        let elapsed = started.elapsed();
        let budget = if succeeded {
            SOFT_DEADLINE
        } else {
            HARD_DEADLINE
        };
        let Some(remaining) = budget.checked_sub(elapsed) else {
            break;
        };
        match rx.recv_timeout(remaining) {
            Ok((engine, result)) => {
                succeeded |= result.as_ref().is_ok_and(|hits| !hits.is_empty());
                reports.push((engine, result));
            }
            Err(_) => break,
        }
    }

    let mut merged: Vec<Merged> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut notes: Vec<String> = Vec::new();
    // Merge in engine-priority order (not settlement order) so tie-breaks are
    // deterministic regardless of which request finished first.
    for engine in CHAIN {
        match reports.iter().find(|(e, _)| *e == engine) {
            Some((_, Ok(hits))) if !hits.is_empty() => merge(&mut merged, &mut index, hits),
            Some((_, Ok(_))) => notes.push(format!("{}: no results", engine.label())),
            Some((_, Err(e))) => notes.push(clipped(&e.to_string(), 90)),
            None => notes.push(format!("{}: timed out", engine.label())),
        }
    }
    if merged.is_empty() {
        return (Vec::new(), notes);
    }
    merged.sort_by(|a, b| {
        b.engines
            .cmp(&a.engines)
            .then(a.best_rank.cmp(&b.best_rank))
            .then(a.order.cmp(&b.order))
    });
    let hits = merged.into_iter().take(limit).map(|m| m.hit).collect();
    (hits, notes)
}

/// One URL across engines: how many found it, where the best-ranked engine
/// put it, and when it was first seen.
struct Merged {
    hit: SearchHit,
    engines: usize,
    best_rank: usize,
    order: usize,
}

fn merge(acc: &mut Vec<Merged>, index: &mut HashMap<String, usize>, hits: &[SearchHit]) {
    for (rank, hit) in hits.iter().enumerate() {
        let key = dedup_key(&hit.url);
        match index.get(&key) {
            Some(&at) => {
                let entry = &mut acc[at];
                entry.engines += 1;
                if rank < entry.best_rank {
                    entry.best_rank = rank;
                    entry.hit.title = hit.title.clone();
                    entry.hit.url = hit.url.clone();
                    entry.hit.provider = hit.provider;
                }
                // Keep the most informative snippet regardless of which
                // engine ranked it best.
                if hit.snippet.len() > entry.hit.snippet.len() {
                    entry.hit.snippet = hit.snippet.clone();
                }
                if entry.hit.published_date.is_none() {
                    entry.hit.published_date = hit.published_date.clone();
                    entry.hit.age_seconds = hit.age_seconds;
                }
            }
            None => {
                index.insert(key, acc.len());
                acc.push(Merged {
                    hit: hit.clone(),
                    engines: 1,
                    best_rank: rank,
                    order: acc.len(),
                });
            }
        }
    }
}

pub fn search(args: &SearchArgs, browser: &BrowserConfig) -> anyhow::Result<SearchOutcome> {
    let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let parsed = query::parse(&args.query);
    let engine = args.engine.unwrap_or(Engine::Auto);

    let (hits, mut notes) = match engine {
        Engine::Auto => chain(&args.query, &parsed, limit, args.recency, browser),
        Engine::Public => aggregate(&args.query, &parsed, limit, args.recency, browser),
        single => {
            let ctx = ctx_for(&args.query, &parsed, limit, args.recency, browser);
            let hits = single
                .run(&ctx)
                .map_err(|e| anyhow::anyhow!("{}", clipped(&e.to_string(), 160)))?;
            (hits, Vec::new())
        }
    };

    // Every engine failed: say so per engine rather than reporting "no
    // results", which would read as "nothing exists".
    if hits.is_empty() && !notes.is_empty() {
        anyhow::bail!("all search engines failed: {}", notes.join("; "));
    }

    // Lenient constraint pass over whatever came back: enforce the directives
    // the engines could not (or only partially) honor, relaxing any dimension
    // that would wipe out every hit.
    let filtered = query::apply_constraints(&hits, &parsed);
    for dropped in filtered.dropped {
        notes.push(format!(
            "no results matched `{dropped}`; the constraint was relaxed"
        ));
    }
    Ok(SearchOutcome {
        hits: filtered.hits,
        notes,
    })
}

/// Tool-facing text rendering: notes first, then numbered rows with the engine
/// and, when the engine published one, the age.
pub fn render(outcome: &SearchOutcome) -> String {
    let mut out = String::new();
    for note in &outcome.notes {
        out.push_str(&format!("Note: {note}\n"));
    }
    if outcome.hits.is_empty() {
        out.push_str("no results\n");
        return out;
    }
    for (i, hit) in outcome.hits.iter().enumerate() {
        let meta = match hit.age_label() {
            Some(age) => format!("{} · {}", hit.provider, age),
            None => hit.provider.to_string(),
        };
        out.push_str(&format!(
            "{}. {} [{}]\n   {}\n",
            i + 1,
            hit.title,
            meta,
            hit.url
        ));
        if !hit.snippet.is_empty() {
            out.push_str(&format!("   {}\n", hit.snippet));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(provider: &'static str, title: &str, url: &str) -> SearchHit {
        SearchHit::new(
            provider,
            title.into(),
            url.into(),
            format!("{provider} snippet"),
        )
    }

    #[test]
    fn merge_ranks_by_consensus_then_rank_then_engine_order() {
        let mut merged = Vec::new();
        let mut index = HashMap::new();
        // `a` appears in two engines, `b` in one but ranked first there.
        merge(
            &mut merged,
            &mut index,
            &[
                hit("Bing", "A", "https://a.test/"),
                hit("Bing", "B", "https://b.test/"),
            ],
        );
        merge(
            &mut merged,
            &mut index,
            &[hit("Google", "A again", "https://www.a.test/")],
        );
        merged.sort_by(|x, y| {
            y.engines
                .cmp(&x.engines)
                .then(x.best_rank.cmp(&y.best_rank))
                .then(x.order.cmp(&y.order))
        });
        assert_eq!(merged.len(), 2, "同一 URL 折叠成一条");
        assert_eq!(merged[0].hit.url, "https://a.test/");
        assert_eq!(merged[0].engines, 2);
        assert_eq!(merged[1].hit.url, "https://b.test/");
    }

    #[test]
    fn merge_keeps_the_label_of_the_best_ranked_engine() {
        let mut merged = Vec::new();
        let mut index = HashMap::new();
        merge(
            &mut merged,
            &mut index,
            &[hit("Bing", "T", "https://a.test/")],
        );
        merge(
            &mut merged,
            &mut index,
            &[hit("Google", "T", "https://a.test/")],
        );
        assert_eq!(merged[0].hit.provider, "Bing", "最好的名次属于先到的引擎");
        assert_eq!(merged[0].engines, 2);
        // A longer snippet wins regardless of which engine ranked it best.
        assert_eq!(merged[0].hit.snippet, "Google snippet");
    }

    #[test]
    fn merge_fills_dates_from_whichever_engine_has_them() {
        let mut merged = Vec::new();
        let mut index = HashMap::new();
        merge(
            &mut merged,
            &mut index,
            &[hit("Bing", "T", "https://a.test/")],
        );
        let dated = hit("DuckDuckGo", "T", "https://a.test/").dated(Some("2020-01-01".into()));
        merge(&mut merged, &mut index, &[dated]);
        assert_eq!(merged[0].hit.published_date.as_deref(), Some("2020-01-01"));
    }

    #[test]
    fn render_names_the_engine_and_lists_notes_first() {
        let outcome = SearchOutcome {
            hits: vec![
                SearchHit::new("Google", "T".into(), "https://x/".into(), "s".into()),
                SearchHit::new("Mojeek", "U".into(), "https://y/".into(), String::new())
                    .dated(Some("2020-01-01".into())),
            ],
            notes: vec!["bing: soft-blocked the direct request".into()],
        };
        let out = render(&outcome);
        assert!(out.starts_with("Note: bing: soft-blocked"), "{out}");
        assert!(out.contains("1. T [Google]\n"), "{out}");
        assert!(
            out.contains("2. U [Mojeek · "),
            "带日期的行必须显示年龄: {out}"
        );
        assert!(out.contains("   https://x/\n"), "{out}");
        assert!(out.contains("   s\n"), "{out}");
    }

    #[test]
    fn render_says_no_results_without_inventing_an_engine() {
        let out = render(&SearchOutcome {
            hits: Vec::new(),
            notes: Vec::new(),
        });
        assert_eq!(out, "no results\n");
    }

    #[test]
    fn clipped_keeps_notes_to_one_short_line() {
        assert_eq!(clipped("a\n  b   c", 40), "a b c");
        let long = "x".repeat(200);
        let cut = clipped(&long, 20);
        assert!(cut.chars().count() <= 20, "{cut}");
        assert!(cut.ends_with('…'));
        // Multi-byte input must not be sliced mid-character.
        let cn = "中文".repeat(50);
        assert!(clipped(&cn, 11).chars().count() <= 11);
    }

    #[test]
    fn engine_names_round_trip_through_json() {
        for (json, engine) in [
            ("\"auto\"", Engine::Auto),
            ("\"public\"", Engine::Public),
            ("\"duckduckgo\"", Engine::Duckduckgo),
            ("\"mojeek\"", Engine::Mojeek),
        ] {
            let parsed: Engine = serde_json::from_str(json).unwrap();
            assert_eq!(parsed, engine);
        }
        let parsed: SearchArgs = serde_json::from_str(
            r#"{"intent":"看看","query":"rust","limit":5,"recency":"week","engine":"google"}"#,
        )
        .unwrap();
        assert_eq!(parsed.limit, Some(5));
        assert_eq!(parsed.recency, Some(Recency::Week));
        assert_eq!(parsed.engine, Some(Engine::Google));
        // Omitted fields keep the defaults.
        let parsed: SearchArgs =
            serde_json::from_str(r#"{"intent":"看看","query":"rust"}"#).unwrap();
        assert_eq!(parsed.engine, None);
        assert_eq!(parsed.recency, None);
        assert_eq!(parsed.limit, None);
    }

    #[test]
    fn the_chain_starts_with_the_cheapest_engine_and_never_repeats_one() {
        assert_eq!(CHAIN[0], Engine::Bing);
        let mut seen = std::collections::HashSet::new();
        for engine in CHAIN {
            assert!(seen.insert(engine), "{engine:?} 重复出现");
            assert!(matches!(
                engine,
                Engine::Bing
                    | Engine::Duckduckgo
                    | Engine::Google
                    | Engine::Ecosia
                    | Engine::Startpage
                    | Engine::Mojeek
            ));
        }
        assert_eq!(seen.len(), 6);
    }
}
