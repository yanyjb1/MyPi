//! The engine contract: page addresses, hits, and the link shapes only the
//! engine itself can resolve.

use std::time::Duration;

use super::hit::Hit;
use super::providers::PROVIDERS;

pub(crate) trait SearchProvider: Sync {
    fn name(&self) -> &'static str;
    fn page_url(&self, query: &str, page: u32) -> String;
    fn parse(&self, html: &str) -> Vec<Hit>;
    /// Whether this is the engine's own results page. Anything else — a
    /// consent gate, a traffic challenge, a script check — is something a
    /// human has to clear, so the caller keeps polling.
    fn normal(&self, html: &str) -> bool;
}

pub(crate) fn get(name: &str) -> Option<&'static dyn SearchProvider> {
    PROVIDERS.iter().copied().find(|p| p.name() == name)
}

pub(crate) fn is_external(url: &str, engines: &[&str]) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.to_lowercase();
    !engines
        .iter()
        .any(|e| host == *e || host.ends_with(&format!(".{e}")))
}

pub(crate) fn absolute(href: &str, base: &str) -> Option<String> {
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    if href.starts_with("//") {
        return Some(format!("https:{href}"));
    }
    let base = url::Url::parse(base).ok()?;
    base.join(href).ok().map(|u| u.to_string())
}

/// Redirect endpoints the engines mint into their result pages. The query value
/// is an opaque token, not a destination; one request to the engine turns it
/// into the real URL.
const REDIRECTS: [(&str, &str); 4] = [
    ("www.google.com", "/goto"),
    ("www.google.com", "/url"),
    ("www.baidu.com", "/link"),
    ("baidu.com", "/link"),
];

/// Hosts that are never a destination: ad gateways, static assets, Bing's link
/// shim. An engine's own content hosts (`baike.baidu.com`, `developers.google.com`)
/// are not here: those are real results.
const NOT_A_RESULT: [&str; 6] = [
    "googleadservices.com",
    "googlesyndication.com",
    "googleusercontent.com",
    "bdstatic.com",
    "bdurl.net",
    "ubs.baidu.com",
];

/// The engines' own search paths are navigation, not results. Only the site host
/// is matched, so an engine page on a content host survives.
const ENGINE_PAGES: [&str; 4] = [
    "www.google.com",
    "www.baidu.com",
    "www.bing.com",
    "bing.com",
];
const ENGINE_PATHS: [&str; 6] = ["/search", "/s", "/url", "/goto", "/link", "/ck"];

/// How long one redirect lookup may take.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(6);
/// Enough of the redirect body to hold Google's meta-refresh fallback.
const LOOKUP_BODY: u64 = 4096;

fn is_redirect(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    REDIRECTS
        .iter()
        .any(|(h, path)| *h == host && parsed.path() == *path)
}

fn is_destination(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") || is_redirect(url) {
        return false;
    }
    let host = parsed.host_str().unwrap_or_default().to_lowercase();
    if NOT_A_RESULT
        .iter()
        .any(|blocked| host == *blocked || host.ends_with(&format!(".{blocked}")))
    {
        return false;
    }
    !(ENGINE_PAGES.contains(&host.as_str()) && ENGINE_PATHS.contains(&parsed.path()))
}

fn agent() -> &'static ureq::Agent {
    static AGENT: std::sync::LazyLock<ureq::Agent> = std::sync::LazyLock::new(|| {
        ureq::Agent::config_builder()
            .max_redirects(0)
            .timeout_global(Some(LOOKUP_TIMEOUT))
            .build()
            .new_agent()
    });
    &AGENT
}

/// The destination named in a redirect body: Google answers some tokens with a
/// 200 whose body is a meta refresh and one anchor instead of a `Location`.
fn body_target(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let open = lower.find("<a href=\"")? + "<a href=\"".len();
    let end = body[open..].find('"')? + open;
    Some(body[open..end].to_string())
}

/// Follow the engine's own redirect once, without keeping the body.
fn resolve(url: &str) -> Option<String> {
    let mut resp = agent().get(url).call().ok()?;
    let target = match resp
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
    {
        Some(location) => absolute(location, url)?,
        None => {
            let body = resp
                .body_mut()
                .with_config()
                .limit(LOOKUP_BODY)
                .read_to_string()
                .ok()?;
            body_target(&body)?
        }
    };
    is_destination(&target).then_some(target)
}

fn clean(hit: Hit) -> Option<Hit> {
    let url = if is_redirect(&hit.url) {
        resolve(&hit.url)?
    } else {
        hit.url
    };
    is_destination(&url).then(|| Hit::new(hit.title, url, hit.snippet))
}

/// Turn parsed hits into final ones: resolve each engine redirect, and drop
/// whatever the engine did not point at an external page. One lookup at a time, paced.
pub(crate) fn settle(hits: Vec<Hit>) -> Vec<Hit> {
    let mut settled = Vec::with_capacity(hits.len());
    for hit in hits {
        if is_redirect(&hit.url) {
            super::pace::between_lookups();
        }
        if let Some(hit) = clean(hit) {
            settled.push(hit);
        }
    }
    settled
}
