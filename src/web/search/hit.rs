//! The search-result model — one hit, the engine that produced it, and the
//! date/dedup helpers the pipeline shares.
//!
//! Ported from oh-my-pi's `SearchSource` (`web/search/types.ts`): `provider`
//! and the publication date are part of the model because a model reading
//! results needs to know how fresh they are and which engine to trust for
//! what. Unlike omp's `public` aggregate — which stamps every merged hit
//! `provider: "public"` — a merged hit here keeps the label of the engine
//! that ranked it best, so the name in the output is actionable.

use std::time::{SystemTime, UNIX_EPOCH};

/// One organic search result.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    /// Engine label shown to the model ("DuckDuckGo", "Google", …).
    pub provider: &'static str,
    pub title: String,
    pub url: String,
    pub snippet: String,
    /// ISO 8601 publish timestamp, when the engine emits one. Only DDG's HTML
    /// frontend does; the other engines leave this `None`.
    pub published_date: Option<String>,
    /// Age in seconds, derived from `published_date` at parse time.
    pub age_seconds: Option<u64>,
}

impl SearchHit {
    pub(crate) fn new(provider: &'static str, title: String, url: String, snippet: String) -> Self {
        Self {
            provider,
            title,
            url,
            snippet,
            published_date: None,
            age_seconds: None,
        }
    }

    /// Attach the engine's publication date (and the age derived from it).
    pub(crate) fn dated(mut self, date: Option<String>) -> Self {
        self.age_seconds = date.as_deref().and_then(age_seconds_of);
        self.published_date = date;
        self
    }

    /// Relative age for display: "2d ago", "3h ago", "just now". `None` when
    /// the engine gave no usable date.
    pub(crate) fn age_label(&self) -> Option<String> {
        let secs = self.age_seconds?;
        Some(match secs {
            0..=59 => "just now".to_string(),
            60..=3599 => format!("{}m ago", secs / 60),
            3600..=86_399 => format!("{}h ago", secs / 3600),
            86_400..=2_592_000 => format!("{}d ago", secs / 86_400),
            2_592_001..=31_536_000 => format!("{}mo ago", secs / 2_592_000),
            _ => format!("{}y ago", secs / 31_536_000),
        })
    }
}

/// Canonical dedup key for a result URL: host lowercased with a leading
/// `www.` dropped, path with a trailing slash removed, query preserved,
/// fragment dropped. Engines disagree on exactly these variations for the
/// same page, so the key is what makes cross-engine consolidation work.
pub(crate) fn dedup_key(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => {
            let host = u.host_str().unwrap_or_default().to_lowercase();
            let host = host.strip_prefix("www.").unwrap_or(host.as_str());
            let mut path = u.path().to_string();
            if path.len() > 1 && path.ends_with('/') {
                path.pop();
            }
            match u.query() {
                Some(q) => format!("{host}{path}?{q}"),
                None => format!("{host}{path}"),
            }
        }
        Err(_) => url.to_string(),
    }
}

/// Drop duplicates by [`dedup_key`], keeping first-seen order, capped at
/// `limit`. Applied per engine (omp does the same with a `seen` set) and
/// again across engines when results are merged.
pub(crate) fn take_unique(hits: Vec<SearchHit>, limit: usize) -> Vec<SearchHit> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(limit.min(hits.len()));
    for hit in hits {
        if out.len() >= limit {
            break;
        }
        if seen.insert(dedup_key(&hit.url)) {
            out.push(hit);
        }
    }
    out
}

pub(crate) fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Days from 1970-01-01 to a civil date — Howard Hinnant's `days_from_civil`,
/// which is exact for the whole proleptic Gregorian range.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (m + 9) % 12; // Mar=0 … Feb=11
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// Epoch seconds for an ISO 8601 `YYYY-MM-DD[Thh:mm[:ss]]` timestamp (UTC).
/// Anything else — relative text, junk, a date-shaped snippet — is `None`.
/// No date crate: the engines emit exactly this shape, and the conversion is
/// three lines of arithmetic.
pub(crate) fn iso_to_epoch(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = match s.find(['T', ' ']) {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    };
    let mut parts = date.split('-');
    let (y, m, d): (i64, i64, i64) = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    if parts.next().is_some() || !(1000..=9999).contains(&y) {
        return None;
    }
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut secs = days_from_civil(y, m, d) * 86_400;

    if !rest.is_empty() {
        let t = rest.trim_end_matches('Z');
        let mut tp = t.split([':', '.']);
        let h: i64 = tp.next().unwrap_or("0").parse().ok()?;
        let mi: i64 = tp.next().unwrap_or("0").parse().ok()?;
        let se: i64 = tp.next().unwrap_or("0").parse().ok()?;
        if !(0..24).contains(&h) || !(0..60).contains(&mi) || !(0..60).contains(&se) {
            return None;
        }
        secs += h * 3600 + mi * 60 + se;
    }
    Some(secs)
}

/// Age of an ISO timestamp in seconds, or `None` when it is unparsable or in
/// the future. Relative strings ("2 days ago") are handled by the constraint
/// filter, not here — mirroring omp's split between `dateToAgeSeconds` and
/// `sourceTime`.
pub(crate) fn age_seconds_of(date: &str) -> Option<u64> {
    let then = iso_to_epoch(date)?;
    let now = now_epoch() as i64;
    (then <= now).then(|| (now - then) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(url: &str) -> SearchHit {
        SearchHit::new("Test", "t".into(), url.into(), String::new())
    }

    #[test]
    fn dedup_key_folds_the_variations_engines_disagree_on() {
        let want = dedup_key("https://example.com/a");
        assert_eq!(dedup_key("https://www.example.com/a"), want, "www. 折叠");
        assert_eq!(dedup_key("https://example.com/a/"), want, "尾斜杠折叠");
        assert_eq!(dedup_key("https://EXAMPLE.com/a"), want, "host 大小写折叠");
        // 片段丢掉，query 保留：带不同 query 的是不同页面，
        // 只有片段/尾斜杠差异的才是同一页。
        assert_eq!(
            dedup_key("https://example.com/a#top"),
            dedup_key("https://example.com/a")
        );
        assert_eq!(
            dedup_key("https://example.com/a?x=1#frag"),
            dedup_key("https://example.com/a?x=1")
        );
        assert_ne!(
            dedup_key("https://example.com/a?x=1"),
            want,
            "query 不同即不同页"
        );
        assert_ne!(
            dedup_key("https://example.com/a?x=2"),
            dedup_key("https://example.com/a?x=1")
        );
        // 解析不了的原样返回（不 panic）
        assert_eq!(dedup_key("not a url"), "not a url");
    }

    #[test]
    fn take_unique_keeps_first_seen_order_and_caps() {
        let hits = vec![
            hit("https://a.test/1"),
            hit("https://www.a.test/1"), // 同一结果
            hit("https://b.test/2"),
            hit("https://c.test/3"),
        ];
        let out = take_unique(hits, 10);
        assert_eq!(out.len(), 3, "重复项被折叠");
        assert_eq!(out[0].url, "https://a.test/1", "先到者留下");
        assert_eq!(out[1].url, "https://b.test/2");
        let capped = take_unique(vec![hit("https://a.test/1"), hit("https://b.test/2")], 1);
        assert_eq!(capped.len(), 1);
    }

    #[test]
    fn iso_dates_convert_to_epoch_exactly() {
        assert_eq!(iso_to_epoch("1970-01-01"), Some(0));
        assert_eq!(iso_to_epoch("1970-01-02"), Some(86_400));
        assert_eq!(iso_to_epoch("2026-07-30T20:19:00.0000000"), {
            // DDG emits seven fractional digits; they are dropped, not fatal.
            let mut expected = 0i64;
            let mut parts = "2026-07-30".split('-');
            let (y, m, d): (i64, i64, i64) = (
                parts.next().unwrap().parse().unwrap(),
                parts.next().unwrap().parse().unwrap(),
                parts.next().unwrap().parse().unwrap(),
            );
            expected += days_from_civil(y, m, d) * 86_400;
            Some(expected + 20 * 3600 + 19 * 60)
        });
        assert_eq!(
            iso_to_epoch("2026-07-30 20:19"),
            Some({
                let mut parts = "2026-07-30".split('-');
                let (y, m, d): (i64, i64, i64) = (
                    parts.next().unwrap().parse().unwrap(),
                    parts.next().unwrap().parse().unwrap(),
                    parts.next().unwrap().parse().unwrap(),
                );
                days_from_civil(y, m, d) * 86_400 + 20 * 3600 + 19 * 60
            })
        );
        // 相对文本与垃圾输入不是日期
        assert_eq!(iso_to_epoch("2 days ago"), None);
        assert_eq!(iso_to_epoch(""), None);
        assert_eq!(iso_to_epoch("2026-13-01"), None);
        assert_eq!(iso_to_epoch("09:30"), None);
    }

    #[test]
    fn age_label_speaks_in_units() {
        let mut h = hit("https://a.test/1");
        h.age_seconds = Some(30);
        assert_eq!(h.age_label().as_deref(), Some("just now"));
        h.age_seconds = Some(90);
        assert_eq!(h.age_label().as_deref(), Some("1m ago"));
        h.age_seconds = Some(3 * 86_400);
        assert_eq!(h.age_label().as_deref(), Some("3d ago"));
        h.age_seconds = None;
        assert_eq!(h.age_label(), None);
    }

    #[test]
    fn dated_rejects_a_future_timestamp() {
        let h = SearchHit::new("Test", "t".into(), "https://a.test/".into(), String::new())
            .dated(Some("2999-01-01".into()));
        assert_eq!(h.published_date.as_deref(), Some("2999-01-01"));
        assert_eq!(h.age_seconds, None, "未来时间没有年龄");
    }
}
