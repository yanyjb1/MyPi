use std::time::Duration;

use anyhow::Result;

use crate::server::config::rules::{Part, Rules};
use crate::web::WebArgs;
use crate::web::artifact;
use crate::web::browser::page::{Page, RENDER_WAIT};
use crate::web::browser::tabs;
use crate::web::html::{expression, html_to_markdown, is_js_shell, scan};
use crate::web::ops::ACTION_TIMEOUT;
use crate::web::seen;
use crate::web::url;

// How long a body that painted nothing gets before it is called empty.
const EMPTY_WAIT: Duration = Duration::from_secs(3);

// The work tab only: the search tab belongs to `search`.
pub(crate) fn run(args: &WebArgs, session: i64, rules: &Rules) -> Result<String> {
    let target = match args.url.as_deref() {
        Some(raw) => Some(url::normalize(raw)?),
        None => None,
    };
    tabs::with_work(session, |page: &Page| {
        if let Some(target) = target.as_deref() {
            page.navigate(target)?;
        }
        if args.raw {
            let html = match target.is_some() {
                true => page.wait(RENDER_WAIT, |html| has_body(html).then_some(Ok(())))?,
                false => page.dom_html()?,
            };
            let address = match target.as_deref() {
                Some(target) => target.to_string(),
                None => page.url()?,
            };
            return saved(&url::site(&address), &html);
        }
        if target.is_some() {
            page.wait(RENDER_WAIT, |html| match html_to_markdown(html) {
                Ok(md) if is_js_shell(html.len(), &md) => None,
                Ok(_) if has_body(html) => Some(Ok(())),
                Ok(_) => None,
                Err(_) => None,
            })?;
        }
        answered(page, target.as_deref(), rules, session)
    })
}

// One page read as prose: a site rule narrows it once, and every later step sees only that.
fn answered(page: &Page, target: Option<&str>, rules: &Rules, session: i64) -> Result<String> {
    let address = match target {
        Some(target) => target.to_string(),
        None => page.url().unwrap_or_default(),
    };
    let regions = regions(&address, rules);
    let expression = expression(regions);
    let mut found = scan(&page.evaluate(&expression, ACTION_TIMEOUT)?)?;
    if found.text == 0 {
        // A body that paints late is worth one more look before it is called empty.
        std::thread::sleep(EMPTY_WAIT);
        found = scan(&page.evaluate(&expression, ACTION_TIMEOUT)?)?;
    }
    if found.text == 0 {
        return Ok("target url has an empty body".to_string());
    }
    let mut text = seen::answer(session, &address, &html_to_markdown(&found.html)?);
    text.push_str(&found.section(&url::site(&address))?);
    Ok(text)
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
