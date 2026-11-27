use std::sync::Arc;

use anyhow::Result;

use crate::platform::stop::Stop;
use crate::web::WebArgs;
use crate::web::browser::page::Page;
use crate::web::browser::tabs;
use crate::web::url;

pub(crate) fn run(args: &WebArgs, session: i64, stop: &Arc<Stop>) -> Result<String> {
    let target = match args.url.as_deref() {
        Some(url) => url::normalize(url)?,
        None => "about:blank".to_string(),
    };
    tabs::with_work(session, |page: &Page| {
        let _hook = page.end_on_stop(stop);
        page.navigate(&target)?;
        Ok(format!("opened: {target}"))
    })
}
