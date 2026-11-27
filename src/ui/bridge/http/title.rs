// The name a cited page gives itself, read by the door because a page cannot read another site's head.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Map, Value, json};

// How long one page gets, and how much of it is read: a title stands near the top of any document.
const LIMIT: Duration = Duration::from_secs(6);
const BODY: u64 = 192 * 1024;
// How many addresses one frame may ask about: a round's prose cites a few, never a crawl.
const MOST: usize = 32;
// A site that sees a strange client answers a challenge instead of a page, so the door says it is a browser.
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
// The words a challenge page says where a title should stand.
const GATES: [&str; 5] = [
    "just a moment",
    "attention required",
    "please wait",
    "checking your browser",
    "enable javascript",
];

// What each address was once found to call itself; `None` is the answer of a page that would not say.
static SEEN: LazyLock<Mutex<HashMap<String, Option<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// `local/title`: the title of every address that gives one, by address.
pub fn lookup(payload: &Value) -> Value {
    let mut asked: Vec<String> = Vec::new();
    let list = payload.get("urls").and_then(Value::as_array);
    for url in list.into_iter().flatten().filter_map(Value::as_str) {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            continue;
        }
        if !asked.iter().any(|one| one == url) {
            asked.push(url.to_string());
        }
        if asked.len() == MOST {
            break;
        }
    }
    // One frame, one wait: every address is read beside the others, so a slow site costs the drawer one timeout.
    let ways: Vec<_> = asked
        .into_iter()
        .map(|url| {
            std::thread::spawn(move || {
                let said = known(&url);
                (url, said)
            })
        })
        .collect();
    let mut titles = Map::new();
    for way in ways {
        if let Ok((url, Some(said))) = way.join() {
            titles.insert(url, Value::String(said));
        }
    }
    json!({ "titles": titles })
}

// What one address is known to call itself, read now when it was never read before.
fn known(url: &str) -> Option<String> {
    if let Some(said) = SEEN.lock().get(url) {
        return said.clone();
    }
    let said = read(url);
    SEEN.lock().insert(url.to_string(), said.clone());
    said
}

// One page as it names itself, or nothing when it answered with a status or a gate instead.
fn read(url: &str) -> Option<String> {
    let mut resp = agent().get(url).header("User-Agent", UA).call().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp
        .body_mut()
        .with_config()
        .limit(BODY)
        .read_to_string()
        .ok()?;
    let said = tag(&body)?;
    let lower = said.to_lowercase();
    if GATES.iter().any(|gate| lower.contains(gate)) {
        return None;
    }
    Some(said)
}

// The document's own `<title>`, as its words: the marks inside it are not its words.
fn tag(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let after = lower[open..].find('>')? + open + 1;
    let end = lower[after..].find("</title")? + after;
    let said = unescape(&html[after..end]);
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    (!said.is_empty()).then_some(said)
}

// The five entities an ordinary title keeps; anything rarer stands as it was written.
fn unescape(text: &str) -> String {
    let mut said = text.to_string();
    for (from, to) in [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&apos;", "'"),
        ("&nbsp;", " "),
    ] {
        said = said.replace(from, to);
    }
    said
}

fn agent() -> &'static ureq::Agent {
    static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(LIMIT))
            .build()
            .new_agent()
    });
    &AGENT
}
