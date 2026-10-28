use super::hit::Hit;
use super::providers::PROVIDERS;

pub(crate) trait SearchProvider: Sync {
    fn name(&self) -> &'static str;
    fn page_url(&self, query: &str, page: u32) -> String;
    fn parse(&self, html: &str) -> Vec<Hit>;
    fn challenge(&self, url: &str, html: &str) -> Option<&'static str>;
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
