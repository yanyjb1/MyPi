// The `read` operation: navigate, wait for a body, hand back prose or the DOM as a file.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use crate::browser::WebArgs;
use crate::browser::artifact;
use crate::browser::halt;
use crate::browser::html::{expression, html_to_markdown, is_js_shell, scan};
use crate::browser::ops::ACTION_TIMEOUT;
use crate::browser::page::{Page, RENDER_WAIT};
use crate::browser::seen;
use crate::browser::tabs;
use crate::browser::url;
use crate::platform::stop::Stop;
use crate::server::config::rules::{Part, Rules};

// How long a body that painted nothing gets before it is called empty.
const EMPTY_WAIT: Duration = Duration::from_secs(3);

// The work tab only: the search tab belongs to `search`.
pub(crate) fn run(args: &WebArgs, session: i64, rules: &Rules, stop: &Arc<Stop>) -> Result<String> {
    let target = match args.url.as_deref() {
        Some(raw) => Some(url::normalize(raw)?),
        None => None,
    };
    tabs::with_work(session, |page: &Page| {
        let _hook = page.end_on_stop(stop);
        if let Some(target) = target.as_deref() {
            if let Err(e) = page.navigate(target) {
                return match halt::is(&e) {
                    true => Ok(empty()),
                    false => Err(e),
                };
            }
        }
        let waited = match target.is_some() {
            true => page.wait(RENDER_WAIT, stop, |html| ready(html, args.raw)),
            false => page.dom_html(),
        };
        // A stopped read answers with the DOM already on the tab, and an empty one says so.
        let html = match waited {
            Ok(html) => html,
            Err(e) if halt::is(&e) => page.dom_html().unwrap_or_default(),
            Err(e) => return Err(e),
        };
        if html.is_empty() {
            return Ok(empty());
        }
        let address = match target.as_deref() {
            Some(target) => target.to_string(),
            None => page.url().unwrap_or_default(),
        };
        if args.raw {
            return saved(&url::site(&address), &html);
        }
        answered(page, &address, rules, session, stop, &html)
    })
}

// One page read as prose: a site rule narrows it once, and every later step sees only that. A stopped read answers the text already in hand.
fn answered(
    page: &Page,
    address: &str,
    rules: &Rules,
    session: i64,
    stop: &Arc<Stop>,
    held: &str,
) -> Result<String> {
    let expression = expression(regions(address, rules));
    let mut found = match page.evaluate(&expression, ACTION_TIMEOUT) {
        Ok(raw) => match scan(&raw) {
            Ok(found) => found,
            Err(e) if halt::is(&e) => return prose(session, address, held),
            Err(e) => return Err(e),
        },
        Err(e) if halt::is(&e) => return prose(session, address, held),
        Err(e) => return Err(e),
    };
    if found.text == 0 && stop.pause(EMPTY_WAIT) {
        found = scan(&page.evaluate(&expression, ACTION_TIMEOUT)?)?;
    }
    if found.text == 0 {
        return prose(session, address, held);
    }
    let mut text = seen::answer(session, address, &html_to_markdown(&found.html)?);
    text.push_str(&found.section(&url::site(address))?);
    Ok(text)
}

// A document as this conversation reads pages: the prose, and the one card of what it now holds.
fn prose(session: i64, address: &str, html: &str) -> Result<String> {
    let md = match html_to_markdown(html) {
        Ok(md) if !md.trim().is_empty() => md,
        _ => return Ok(empty()),
    };
    Ok(seen::answer(session, address, &md))
}

// Whether a page is worth stopping at: a body is there, and for prose it is not still a script shell.
fn ready(html: &str, raw: bool) -> Option<Result<()>> {
    if raw {
        return has_body(html).then_some(Ok(()));
    }
    match html_to_markdown(html) {
        Ok(md) if is_js_shell(html.len(), &md) => None,
        Ok(_) if has_body(html) => Some(Ok(())),
        Ok(_) => None,
        Err(_) => None,
    }
}

// What a read says when it holds no page at all.
fn empty() -> String {
    "target url has an empty body".to_string()
}

// The rule that applies to an address; none reads the whole page, drops and all.
fn regions<'r>(address: &str, rules: &'r Rules) -> &'r [Part] {
    match split(address) {
        Some((host, path)) => rules.regions(&host, &path),
        None => &[],
    }
}

// The host and the path of an address; the query is not part of a rule's match.
fn split(address: &str) -> Option<(String, String)> {
    let parsed = ::url::Url::parse(address).ok()?;
    let host = parsed.host_str()?.to_string();
    Some((host, parsed.path().to_string()))
}

// A document without a body yet is still arriving; it is not an answer.
fn has_body(html: &str) -> bool {
    html.contains("<body")
}

// The DOM never goes into the context: it is written out and the path is the answer.
fn saved(site: &str, html: &str) -> Result<String> {
    let path = artifact::save(site, "html", html.as_bytes())?;
    Ok(format!("html: {}", path.display()))
}
