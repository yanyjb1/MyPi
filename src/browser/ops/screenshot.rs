use std::sync::Arc;

use anyhow::Result;

use crate::browser::WebArgs;
use crate::browser::artifact;
use crate::browser::page::Page;
use crate::browser::tabs;
use crate::browser::url;
use crate::platform::stop::Stop;

pub(crate) fn run(args: &WebArgs, session: i64, stop: &Arc<Stop>) -> Result<String> {
    tabs::with_work(session, |page: &Page| {
        let _hook = page.end_on_stop(stop);
        let bytes = page.screenshot(args.full_page)?;
        let site = url::site(&page.url()?);
        let path = artifact::save(&site, "png", &bytes)?;
        Ok(format!(
            "screenshot: {} bytes -> {}",
            bytes.len(),
            path.display()
        ))
    })
}
