use std::time::Duration;
fn main() -> anyhow::Result<()> {
    let profile = std::path::Path::new("/tmp/nav-debug");
    let b = mypi::cdp::Browser::attach(profile, 9224)?;
    let t = b.page_target()?;
    println!("[1 target] {}", t.id);
    let mut cdp = mypi::cdp::Cdp::connect(&t.ws_url)?;
    println!("[2 connected]");
    // 页面已在 DDG 结果页（上轮导航过），直接取 DOM，不再导航：
    let html = cdp.dom_html(Duration::from_secs(8))?;
    println!("[3 dom] {} bytes, result__a={}", html.len(), html.contains("result__a"));
    Ok(())
}
