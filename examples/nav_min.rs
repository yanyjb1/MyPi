// 定位 navigate 卡点：分步打印每一步
use std::time::Duration;
fn main() -> anyhow::Result<()> {
    let profile = std::path::Path::new("/tmp/nav-debug");
    let b = mypi::cdp::Browser::attach(profile, 9224)?;
    let t = b.page_target()?;
    println!("[1 attach+target ok]");
    let mut cdp = mypi::cdp::Cdp::connect(&t.ws_url)?;
    println!("[2 connect ok]");
    let r = cdp.call("Page.enable", serde_json::json!({}), Duration::from_secs(5));
    println!("[3 Page.enable: {:?}]", r.is_ok());
    let r = cdp.call("Page.navigate", serde_json::json!({"url": "https://html.duckduckgo.com/html/?q=test"}), Duration::from_secs(15));
    println!("[4 navigate ack: {:?}]", r.is_ok());
    // 读 5 秒事件，看 loadEventFired 是否到达
    for _ in 0..20 {
        // 用一个假请求试探：evaluate 1+1 若通，说明泵活着
        match cdp.call("Runtime.evaluate", serde_json::json!({"expression":"1+1","returnByValue":true}), Duration::from_secs(3)) {
            Ok(_) => { println!("[5 pump alive]"); return Ok(()); }
            Err(e) => { println!("[5 waiting… {e}]"); }
        }
    }
    println!("[5 pump DEAD]");
    Ok(())
}
