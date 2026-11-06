use anyhow::{Result, bail};

use crate::web::WebArgs;
use crate::web::browser::page::{Page, RENDER_WAIT};
use crate::web::browser::tabs;
use crate::web::html::{html_to_markdown, is_js_shell};
use crate::web::url;

pub(crate) fn run(args: &WebArgs, session: i64) -> Result<String> {
    match args.url.as_deref() {
        Some(raw) => {
            let target = url::normalize(raw)?;
            tabs::with_search(session, |page: &Page| {
                page.navigate(&target)?;
                if args.raw {
                    return page.wait(RENDER_WAIT, |_| Some(Ok(())));
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
            if args.raw {
                bail!("raw requires url");
            }
            tabs::with_work(session, |page: &Page| {
                let html = page.dom_html()?;
                html_to_markdown(&html)
            })
        }
    }
}
