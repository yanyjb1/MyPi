use anyhow::Result;

use crate::web::WebArgs;
use crate::web::artifact;
use crate::web::browser::page::{Page, RENDER_WAIT};
use crate::web::browser::tabs;
use crate::web::html::{expression, html_to_markdown, is_js_shell, scan};
use crate::web::ops::ACTION_TIMEOUT;
use crate::web::rules;
use crate::web::url;

/// The work tab only: the search tab belongs to `search`.
pub(crate) fn run(args: &WebArgs, session: i64) -> Result<String> {
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
                true => page.wait(RENDER_WAIT, |_| Some(Ok(())))?,
                false => page.dom_html()?,
            };
            return saved(&html);
        }
        let html = match target.is_some() {
            true => page.wait(RENDER_WAIT, |html| match html_to_markdown(html) {
                Ok(md) if is_js_shell(html.len(), &md) => None,
                Ok(_) => Some(Ok(())),
                Err(_) => None,
            })?,
            false => page.dom_html()?,
        };
        answered(page, target.as_deref(), &html)
    })
}

/// One page read as prose. A site rule narrows what is read to the regions it names; every
/// later step sees only that. A page whose regions hold no text is named for what it is, so an
/// empty answer is not read as an empty page.
fn answered(page: &Page, target: Option<&str>, dom: &str) -> Result<String> {
    let (html, scope) = narrowed(page, target, dom)?;
    let operable = scan(&page.evaluate(&expression(&scope), ACTION_TIMEOUT)?)?;
    if operable.text == 0 {
        return Ok("target url has an empty body".to_string());
    }
    let mut text = html_to_markdown(&html)?;
    text.push_str(&operable.section());
    Ok(text)
}

/// What is read, and where the controls are looked for inside it. The address the call was given
/// is the rule's subject; only a read of the tab as it stands has to ask the page which one it is.
fn narrowed(page: &Page, target: Option<&str>, dom: &str) -> Result<(String, Vec<String>)> {
    let address = match target {
        Some(target) => target.to_string(),
        None => match page.url() {
            Ok(address) => address,
            Err(_) => return Ok((dom.to_string(), Vec::new())),
        },
    };
    let Some((host, path)) = split(&address) else {
        return Ok((dom.to_string(), Vec::new()));
    };
    let rules = rules::load();
    let regions = rules.regions(&host, &path);
    if regions.is_empty() {
        return Ok((dom.to_string(), Vec::new()));
    }
    let kept = rules::kept_html(&page.evaluate(&rules::expression(regions), ACTION_TIMEOUT)?);
    if kept.trim().is_empty() {
        return Ok((dom.to_string(), Vec::new()));
    }
    let scope = regions
        .iter()
        .map(|part| part.selector().to_string())
        .collect();
    Ok((kept, scope))
}

/// The host and the path of an address; the query is not part of a rule's match.
fn split(address: &str) -> Option<(String, String)> {
    let parsed = ::url::Url::parse(address).ok()?;
    let host = parsed.host_str()?.to_string();
    Some((host, parsed.path().to_string()))
}

/// The DOM itself never goes into the context: it is written out and the path is the answer.
fn saved(html: &str) -> Result<String> {
    let path = artifact::save("html", html.as_bytes())?;
    Ok(format!("html: {}", path.display()))
}
