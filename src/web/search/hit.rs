use std::collections::HashSet;

#[derive(Debug, Clone)]
pub(crate) struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

impl Hit {
    pub(crate) fn new(title: String, url: String, snippet: String) -> Self {
        Self {
            title,
            url,
            snippet,
        }
    }
}

pub(crate) fn dedup_key(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => {
            let host = u.host_str().unwrap_or_default().to_lowercase();
            let host = host.strip_prefix("www.").unwrap_or(host.as_str());
            let mut path = u.path().to_string();
            if path.len() > 1 && path.ends_with('/') {
                path.pop();
            }
            match u.query() {
                Some(q) => format!("{host}{path}?{q}"),
                None => format!("{host}{path}"),
            }
        }
        Err(_) => url.to_string(),
    }
}

pub(crate) fn take_unique(hits: Vec<Hit>) -> Vec<Hit> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(hits.len());
    for hit in hits {
        if seen.insert(dedup_key(&hit.url)) {
            out.push(hit);
        }
    }
    out
}
