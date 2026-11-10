use anyhow::Result;

use crate::web::WebArgs;
use crate::web::artifact;
use crate::web::browser::page::{Page, RENDER_WAIT};
use crate::web::browser::tabs;
use crate::web::html::{html_to_markdown, is_js_shell};
use crate::web::url;

/// The work tab only: the search tab belongs to `search`.
pub(crate) fn run(args: &WebArgs, session: i64) -> Result<String> {
    match args.url.as_deref() {
        Some(raw) => {
            let target = url::normalize(raw)?;
            tabs::with_work(session, |page: &Page| {
                page.navigate(&target)?;
                if args.raw {
                    let html = page.wait(RENDER_WAIT, |_| Some(Ok(())))?;
                    return saved(&html);
                }
                let html = page.wait(RENDER_WAIT, |html| match html_to_markdown(html) {
                    Ok(md) if is_js_shell(html.len(), &md) => None,
                    Ok(_) => Some(Ok(())),
                    Err(_) => None,
                })?;
                html_to_markdown(&html)
            })
        }
        None => {
            let html = tabs::with_work(session, |page: &Page| page.dom_html())?;
            if args.raw {
                return saved(&html);
            }
            html_to_markdown(&html)
        }
    }
}

/// The DOM itself never goes into the context: it is written out and the path is the answer.
fn saved(html: &str) -> Result<String> {
    let path = artifact::save("html", html.as_bytes())?;
    Ok(format!("html: {}", path.display()))
}
