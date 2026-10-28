use anyhow::{Result, bail};

use super::page::{Page, RENDER_WAIT};
use super::session;
use crate::server::config::BrowserConfig;
use crate::web::WebArgs;
use crate::web::html::{html_to_markdown, is_js_shell};
use crate::web::url;

pub(crate) fn run(args: &WebArgs, cfg: &BrowserConfig, session: i64) -> Result<String> {
    match args.url.as_deref() {
        Some(raw) => {
            let target = url::normalize(raw)?;
            session::with_search(session, cfg, |page: &Page| {
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
            session::with_work(session, cfg, args.reserve, |page: &Page| {
                let html = page.dom_html()?;
                html_to_markdown(&html)
            })
        }
    }
}
