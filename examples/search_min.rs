// 最小 DDG 链路：attach → connect → navigate → dom_html 一次
use std::time::Duration;
fn main() -> anyhow::Result<()> {
    let profile = std::path::Path::new("/tmp/mypi-search-profile");
    let b = mypi::cdp::Browser::attach(profile, 9223)?;
    let t = b.page_target()?;
    println!("[attach ok, target={}]", t.id);
    let mut cdp = mypi::cdp::Cdp::connect(&t.ws_url)?;
    println!("[connect ok]");
    cdp.navigate("https://html.duckduckgo.com/html/?q=site%3Agithub.com+rust+async", Duration::from_secs(20))?;
    println!("[navigate ok]");
    let html = cdp.dom_html(Duration::from_secs(8))?;
    println!("[dom ok, {} bytes, has result__a={}]", html.len(), html.contains("result__a"));
    Ok(())
}
