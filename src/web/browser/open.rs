use anyhow::Result;

use super::page::Page;
use super::session;
use crate::server::config::BrowserConfig;
use crate::web::WebArgs;
use crate::web::url;

pub(crate) fn run(args: &WebArgs, cfg: &BrowserConfig, session: i64) -> Result<String> {
    let target = match args.url.as_deref() {
        Some(url) => url::normalize(url)?,
        None => "about:blank".to_string(),
    };
    session::with_work(session, cfg, args.reserve, |page: &Page| {
        page.navigate(&target)?;
        Ok(format!("opened: {target}"))
    })
}
