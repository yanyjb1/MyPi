use anyhow::Result;

use crate::web::WebArgs;
use crate::web::artifact;
use crate::web::browser::page::Page;
use crate::web::browser::tabs;

pub(crate) fn run(args: &WebArgs, session: i64) -> Result<String> {
    tabs::with_work(session, |page: &Page| {
        let bytes = page.screenshot(args.full_page)?;
        let path = artifact::save("png", &bytes)?;
        Ok(format!(
            "screenshot: {} bytes -> {}",
            bytes.len(),
            path.display()
        ))
    })
}
