//! Plain-HTTP transport for the search engines — the "direct tier" half of
//! omp's `browserFetch`: a browser-shaped request first, browser rendering
//! only when the site answers with a wall (see `providers::render`).
//!
//! Two deliberate differences from omp, both dependency-driven:
//!
//! - omp mints a fresh, internally consistent header fingerprint per request
//!   from a header-generator library. MyPi pins three coherent desktop
//!   profiles and rotates them, so a site that fingerprints scripts sees a
//!   small population of honest browsers instead of one repeated identity —
//!   without taking on a header-generation dependency.
//! - Bodies are byte-capped like omp's `readLimitedText`; a SERP is a few
//!   hundred KB at most, so a rogue response cannot eat memory.

use anyhow::{Context as _, anyhow};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use ureq::ResponseExt as _;

pub(super) const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BODY_BYTES: u64 = 2 * 1024 * 1024;

/// A fetched page: status, body, and the URL after redirects.
pub(super) struct HtmlPage {
    pub status: u16,
    pub body: String,
    pub url: String,
}

impl HtmlPage {
    pub(super) fn is_ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// One pinned desktop profile. Every field describes the *same* browser:
/// a profile that claims Chrome and sends a Firefox UA is worse than no
/// headers at all.
struct Profile {
    ua: &'static str,
    lang: &'static str,
    /// Chrome-only client hints; Firefox sends none.
    chrome_hints: Option<(&'static str, &'static str)>,
}

const PROFILES: [Profile; 3] = [
    Profile {
        ua: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36",
        lang: "en-US,en;q=0.9",
        chrome_hints: Some(("\"Linux\"", "\"153\"")),
    },
    Profile {
        ua: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36",
        lang: "en-US,en;q=0.8",
        chrome_hints: Some(("\"Windows\"", "\"153\"")),
    },
    Profile {
        ua: "Mozilla/5.0 (X11; Linux x86_64; rv:141.0) Gecko/20100101 Firefox/141.0",
        lang: "en-US,en;q=0.5",
        chrome_hints: None,
    },
];

static NEXT_PROFILE: AtomicUsize = AtomicUsize::new(0);

fn rotate() -> &'static Profile {
    &PROFILES[NEXT_PROFILE.fetch_add(1, Ordering::Relaxed) % PROFILES.len()]
}

fn agent() -> ureq::Agent {
    // `http_status_as_error(false)`: a 403/429 body is the diagnostic ("Ecosia
    // Firewall", an ALTCHA page) — turning it into an opaque transport error
    // would throw away the reason we escalate to the browser.
    ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into()
}

fn nav_headers(profile: &'static Profile, referer: Option<&str>) -> Vec<(&'static str, String)> {
    let mut h: Vec<(&'static str, String)> = vec![
        ("User-Agent", profile.ua.to_string()),
        (
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".to_string(),
        ),
        ("Accept-Language", profile.lang.to_string()),
        ("Upgrade-Insecure-Requests", "1".to_string()),
        ("Sec-Fetch-Dest", "document".to_string()),
        ("Sec-Fetch-Mode", "navigate".to_string()),
        (
            "Sec-Fetch-Site",
            if referer.is_some() {
                "same-origin"
            } else {
                "none"
            }
            .to_string(),
        ),
        ("Sec-Fetch-User", "?1".to_string()),
        ("Priority", "u=0, i".to_string()),
    ];
    if let Some((platform, major)) = profile.chrome_hints {
        h.push((
            "Sec-Ch-Ua",
            format!(
                "\"Chromium\";v={major}, \"Google Chrome\";v={major}, \"Not?A_Brand\";v=\"24\""
            ),
        ));
        h.push(("Sec-Ch-Ua-Mobile", "?0".into()));
        h.push(("Sec-Ch-Ua-Platform", platform.into()));
    }
    if let Some(r) = referer {
        h.push(("Referer", r.to_string()));
    }
    h
}

fn read_page(response: ureq::http::Response<ureq::Body>) -> anyhow::Result<HtmlPage> {
    let status = response.status().as_u16();
    let url = response.get_uri().to_string();
    let mut body = response.into_body();
    let text = body
        .with_config()
        .limit(MAX_BODY_BYTES)
        .read_to_string()
        .context("reading response body")?;
    Ok(HtmlPage {
        status,
        body: text,
        url,
    })
}

/// GET with a browser-shaped identity.
pub(super) fn get(url: &str, referer: Option<&str>) -> anyhow::Result<HtmlPage> {
    let profile = rotate();
    let mut req = agent().get(url);
    for (k, v) in nav_headers(profile, referer) {
        req = req.header(k, v);
    }
    let response = req.call().map_err(|e| anyhow!("request failed: {e}"))?;
    read_page(response)
}

/// POST a URL-encoded form (DuckDuckGo's and Startpage's search forms are
/// POSTs in the flow a real browser performs).
pub(super) fn post_form(
    url: &str,
    form: &[(String, String)],
    referer: Option<&str>,
) -> anyhow::Result<HtmlPage> {
    let profile = rotate();
    let mut req = agent().post(url);
    for (k, v) in nav_headers(profile, referer) {
        req = req.header(k, v);
    }
    let response = req
        .send_form(form.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .map_err(|e| anyhow!("request failed: {e}"))?;
    read_page(response)
}

/// Percent-encoded query string for engine URLs.
pub(super) fn encode(value: &str) -> String {
    crate::web::utils::url::urlencoded(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_profile_is_internally_consistent() {
        for p in &PROFILES {
            match p.chrome_hints {
                Some((platform, major)) => {
                    assert!(
                        p.ua.contains("Chrome/"),
                        "Chrome 档位必须自称 Chrome: {}",
                        p.ua
                    );
                    assert!(
                        p.ua.contains(&format!("Chrome/{}.", major.trim_matches('"'))),
                        "Sec-Ch-Ua 的版本必须与 UA 一致: {} vs {major}",
                        p.ua
                    );
                    assert!(
                        (platform.contains("Linux")) == p.ua.contains("Linux"),
                        "平台提示必须与 UA 一致: {} vs {platform}",
                        p.ua
                    );
                }
                None => assert!(
                    p.ua.contains("Firefox/"),
                    "无 hints 的档位必须是 Firefox: {}",
                    p.ua
                ),
            }
        }
    }

    #[test]
    fn profiles_rotate_instead_of_repeating_one_identity() {
        let seen: Vec<*const Profile> = (0..PROFILES.len())
            .map(|_| rotate() as *const Profile)
            .collect();
        let unique: std::collections::HashSet<*const Profile> = seen.iter().copied().collect();
        assert_eq!(unique.len(), PROFILES.len(), "一轮之内必须走遍所有档位");
    }

    #[test]
    fn navigation_headers_describe_one_browser() {
        let headers = nav_headers(&PROFILES[0], Some("https://www.google.com/"));
        let map: std::collections::HashMap<&str, String> = headers.into_iter().collect();
        assert_eq!(map["Sec-Fetch-Site"], "same-origin");
        assert_eq!(map["Referer"], "https://www.google.com/");
        assert!(map["User-Agent"].contains("Chrome/"));
        // A first navigation (no referer) is a top-level one.
        let headers = nav_headers(&PROFILES[0], None);
        let map: std::collections::HashMap<&str, String> = headers.into_iter().collect();
        assert_eq!(map["Sec-Fetch-Site"], "none");
        assert!(!map.contains_key("Referer"));
    }

    #[test]
    fn encoding_matches_the_url_rules() {
        assert_eq!(encode("rust async"), "rust%20async");
        assert_eq!(
            encode("site:github.com \"a b\""),
            "site%3Agithub.com%20%22a%20b%22"
        );
    }
}
