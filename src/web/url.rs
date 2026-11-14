use anyhow::{anyhow, bail};

pub(crate) fn encode_query(s: &str) -> String {
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

pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn base64_decode(s: &str) -> anyhow::Result<Vec<u8>> {
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

pub(crate) fn normalize(raw: &str) -> anyhow::Result<String> {
    let url = raw.trim();
    if url.is_empty() {
        bail!("url is empty");
    }
    let lower = url.to_ascii_lowercase();
    for scheme in ["file:", "data:", "ftp:", "chrome:", "javascript:"] {
        if lower.starts_with(scheme) {
            bail!("scheme not allowed: {scheme}");
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

/// What a page's files are called: the site it came from, without the `www.` it may or may not
/// print, and in a shape a file name can hold.
pub(crate) fn site(address: &str) -> String {
    let host = ::url::Url::parse(address)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_default();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let name: String = host
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '-' })
        .collect();
    if name.is_empty() {
        "page".to_string()
    } else {
        name
    }
}
