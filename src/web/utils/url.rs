//! URL / encoding utilities shared across the web domain.

use anyhow::anyhow;

/// Percent-encode for query values (RFC 3986 unreserved set kept literal).
pub fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Percent-decode (case-insensitive hex); invalid escapes pass through.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Standard base64 (screenshot payloads arrive b64-encoded).
pub fn base64_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a') as u32 + 26),
            b'0'..=b'9' => Some((c - b'0') as u32 + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let clean: Vec<u8> = s.bytes().filter(|b| !b" \n\r\t".contains(b)).collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &c in &clean {
        if c == b'=' {
            break;
        }
        let v = val(c).ok_or_else(|| anyhow!("bad base64 byte {c:#x}"))?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

/// Normalize a user/model-supplied URL: trim, reject non-web schemes before
/// the `https://` prefix could mint a live-looking URL, default the scheme.
pub fn normalize(raw: &str) -> anyhow::Result<String> {
    let url = raw.trim();
    let lower = url.to_ascii_lowercase();
    for scheme in ["file:", "data:", "ftp:", "chrome:", "javascript:"] {
        if lower.starts_with(scheme) {
            return Err(anyhow!("scheme not allowed: {scheme}"));
        }
    }
    Ok(
        if lower.starts_with("http://") || lower.starts_with("https://") {
            url.to_string()
        } else {
            format!("https://{url}")
        },
    )
}

/// Percent-encode everything except RFC 3986 unreserved (for URL query
/// embedding, e.g. /json/new?url=…).
pub fn urlencode_component(s: &str) -> String {
    urlencoded(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencoded_keeps_unreserved() {
        assert_eq!(urlencoded("aB1-_.~"), "aB1-_.~");
        assert_eq!(urlencoded("a b"), "a%20b");
        assert_eq!(urlencoded("中"), "%E4%B8%AD");
    }

    #[test]
    fn percent_decode_case_insensitive() {
        assert_eq!(percent_decode("http%3a%2F%2Fx.y"), "http://x.y");
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn base64_roundtrip() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert!(base64_decode("!!!!").is_err());
    }

    #[test]
    fn normalize_blocks_local_schemes_and_defaults_https() {
        assert!(normalize("file:///etc/passwd").is_err());
        assert!(normalize("javascript:alert(1)").is_err());
        assert_eq!(normalize("example.com").unwrap(), "https://example.com");
        assert_eq!(normalize("http://x.y/").unwrap(), "http://x.y/");
    }
}
