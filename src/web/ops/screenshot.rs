use anyhow::Result;

use crate::web::WebArgs;
use crate::web::browser::page::Page;
use crate::web::browser::tabs;

pub(crate) fn run(args: &WebArgs, session: i64) -> Result<String> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("mypi-web-{session}-{millis}.png"));
    tabs::with_work(session, |page: &Page| {
        let bytes = page.screenshot(args.full_page)?;
        std::fs::write(&path, &bytes)
            .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
        Ok(format!(
            "screenshot: {} bytes -> {}",
            bytes.len(),
            path.display()
        ))
    })
}
