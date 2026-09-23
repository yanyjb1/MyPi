
use mypi::cdp::Browser;
fn main() -> anyhow::Result<()> {
    let profile = std::env::temp_dir().join(format!("mypi-cdp-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&profile);
    let b = Browser::launch(&profile)?;
    let t = b.page_target()?;
    println!("port={} target={}", b.port, t.id);
    let mut cdp = mypi::cdp::Cdp::connect(&t.ws_url)?;
    cdp.navigate("data:text/html,<title>hi-cdp</title><h1>works</h1>", std::time::Duration::from_secs(10))?;
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let html = cdp.dom_html(std::time::Duration::from_secs(10))?;
    println!("dom_has_title={} dom_has_h1={}", html.contains("hi-cdp"), html.contains("works"));
    std::fs::remove_dir_all(&profile).ok();
    Ok(())
}
