use anyhow::Result;

use crate::web::WebArgs;
use crate::web::artifact;
use crate::web::browser::page::Page;
use crate::web::browser::tabs;
use crate::web::url;

pub(crate) fn run(args: &WebArgs, session: i64) -> Result<String> {
    tabs::with_work(session, |page: &Page| {
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
