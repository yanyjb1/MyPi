//! One provider per call: pages 1..=N through the search tab, stopped as soon as a page adds
//! nothing new.

use anyhow::{Result, anyhow, bail};
use std::collections::HashSet;
use std::time::Duration;

use self::hit::{Hit, dedup_key};
use self::provider::SearchProvider;
use super::browser::page::{Page, RENDER_WAIT};
use super::browser::{tabs, verify};

mod hit;
mod pace;
mod provider;
mod providers;

/// The engines, one list for the schema, the error text and the table.
pub(crate) const ENGINES: &[&str] = providers::NAMES;

/// How long the tidy pass may take.
const TIDY_TIMEOUT: Duration = Duration::from_secs(5);

/// A result the site or an ad blocker hides is still in the serialized DOM, so serialization
/// alone cannot tell an ad card from a result. Ask the page which containers have no box, and
/// delete them: what is left is what a human would see.
///
/// The container classes are the providers' own: rename one there and this list must follow.
const TIDY: &str = r#"(() => {
  let gone = 0;
  for (const el of document.querySelectorAll('div.c-container, div.MjjYud, li.b_algo')) {
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden' || el.getClientRects().length === 0) {
      el.remove();
      gone += 1;
    }
  }
  return gone;
})()"#;

pub(crate) fn run(session: i64, query: &str, provider_name: &str, pages: u32) -> Result<String> {
    let provider = provider::get(provider_name)
        .ok_or_else(|| anyhow!("unknown provider {provider_name:?}; {}", ENGINES.join("|")))?;
    let name = provider.name();

    let mut hits: Vec<Hit> = Vec::new();
    let mut seen = HashSet::new();
    let mut current = 1u32;
    let mut fetched = 0u32;
    while current <= pages {
        if current > 1 {
            pace::between_pages();
        }
        let url = provider.page_url(query, current);
        let mut html = fetch(session, &url, provider)?;

        // A gate — verification, consent, a script check — is a page whose result body is not
        // there. Hand it to the human and poll until the engine's own page is back.
        if !provider.normal(&html) {
            verify::await_human(session, |_url, html| provider.normal(html))
                .map_err(|e| anyhow!("{name}: {e}"))?;
            html = fetch(session, &url, provider)?;
            if !provider.normal(&html) {
                bail!(
                    "{name}: the page is still a gate. Finish it in the browser window, then \
                     retry."
                );
            }
        }

        let parsed = provider::settle(provider.parse(&html));
        if parsed.is_empty() {
            break;
        }
        let mut fresh = 0usize;
        for hit in parsed {
            if seen.insert(dedup_key(&hit.url)) {
                hits.push(hit);
                fresh += 1;
            }
        }
        if fresh == 0 {
            break;
        }
        fetched += 1;
        current += 1;
    }

    Ok(render(name, fetched, &hits))
}

fn fetch(session: i64, url: &str, provider: &'static dyn SearchProvider) -> Result<String> {
    tabs::with_search(session, |page: &Page| {
        page.navigate(url)?;
        let html = page.wait(RENDER_WAIT, |html| provider.normal(html).then_some(Ok(())))?;
        if !provider.normal(&html) {
            return Ok(html);
        }
        let _ = page.evaluate(TIDY, TIDY_TIMEOUT);
        page.dom_html()
    })
}

fn render(name: &str, pages: u32, hits: &[Hit]) -> String {
    let mut out = match pages {
        0 => format!("{name}: no results\n"),
        1 => format!("{name}: page 1, {} results\n", hits.len()),
        n => format!("{name}: pages 1-{n}, {} results\n", hits.len()),
    };
    for (i, hit) in hits.iter().enumerate() {
        out.push_str(&format!("\n{}. {}\n   {}\n", i + 1, hit.title, hit.url));
        if !hit.snippet.is_empty() {
            out.push_str(&format!("   {}\n", hit.snippet));
        }
    }
    out
}
