//! Timestamp injection: one user message's prefix and suffix, rendered from
//! the templates in the configuration at the moment that message is written.

/// The pieces of one local stamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub y: i64,
    pub mo: u32,
    pub d: u32,
    pub h: u32,
    pub mi: u32,
    pub s: u32,
    /// Days since 1970-01-01. The weekday math runs on this.
    pub days: i64,
}

/// Split a `YYYY-MM-DD HH:MM:SS` stamp. `None` = not a stamp.
pub fn parse(stamp: &str) -> Option<Parts> {
    let b = stamp.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> {
        let s = std::str::from_utf8(b.get(a..z)?).ok()?;
        s.bytes().all(|c| c.is_ascii_digit()).then(|| s.parse().ok())?
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)? as u32, num(8, 10)? as u32);
    let (h, mi, s) = (num(11, 13)? as u32, num(14, 16)? as u32, num(17, 19)? as u32);
    if b[4] != b'-' || b[7] != b'-' || b[10] != b' ' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    Some(Parts {
        y,
        mo,
        d,
        h,
        mi,
        s,
        days: super::time::days_from_stamp(stamp)?,
    })
}

const WEEKDAYS: [&str; 7] = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

/// The weekday name. 1970-01-01 was a Thursday.
pub fn weekday(days: i64) -> &'static str {
    WEEKDAYS[((days + 4).rem_euclid(7)) as usize]
}

/// What one `{{…}}` turned into.
enum Marker {
    /// Rendered text.
    Text(String),
    /// Not one of ours: leave the `{{…}}` exactly as written.
    Unknown,
    /// It asks for a moment we cannot produce (no previous reply, an
    /// unreadable stamp). Nothing gets injected at all.
    Missing,
}

/// Render one template. `None` = nothing to inject: the template asks for a
/// moment that is not available.
pub fn render(template: &str, now: &str, last: Option<&str>) -> Option<String> {
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(j) = tail.find("}}") else {
            // Unclosed: nothing to match it, so the rest is literal text.
            out.push_str(tail);
            return Some(out);
        };
        let body = &tail[2..j];
        match marker(body, now, last) {
            Marker::Text(text) => out.push_str(&text),
            Marker::Unknown => out.push_str(&tail[..j + 2]),
            Marker::Missing => return None,
        }
        rest = &tail[j + 2..];
    }
    out.push_str(rest);
    Some(out)
}

/// Expand one `{{name}}` or `{{name:format}}`.
fn marker(body: &str, now: &str, last: Option<&str>) -> Marker {
    let (name, format) = match body.split_once(':') {
        Some((name, format)) => (name.trim(), Some(format)),
        None => (body.trim(), None),
    };
    let value = match name {
        "now" => now,
        "last" => match last {
            Some(last) => last,
            None => return Marker::Missing,
        },
        _ => return Marker::Unknown,
    };
    match format {
        // No format: the stamp itself, exactly as the rest of the program
        // spells it.
        None => Marker::Text(value.to_string()),
        Some(format) => match parse(value) {
            Some(parts) => Marker::Text(expand(format, &parts)),
            None => Marker::Missing,
        },
    }
}

/// Expand a format string. Both vocabularies are scanned left to right;
/// `%` wins a tie because it can never start a letter name.
fn expand(format: &str, p: &Parts) -> String {
    let mut out = String::with_capacity(format.len() + 8);
    let mut i = 0;
    while i < format.len() {
        let rest = &format[i..];
        if let Some(tail) = rest.strip_prefix('%') {
            let mut chars = tail.chars();
            if let Some(c) = chars.next()
                && let Some(text) = strftime(c, p)
            {
                out.push_str(&text);
                i += 1 + c.len_utf8();
                continue;
            }
            // A lone `%` or an unknown directive is a typo; keep it where the writer can see it.
            out.push('%');
            i += 1;
            continue;
        }
        if let Some((width, text)) = letters(rest, p) {
            out.push_str(&text);
            i += width;
            continue;
        }
        let c = rest.chars().next().expect("i is on a char boundary");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// A letter name at the head of `rest`, with its byte width. Longest first:
/// `yyyy` has to win over `yy`, and `day` over `dd`.
fn letters(rest: &str, p: &Parts) -> Option<(usize, String)> {
    const NAMES: [&str; 8] = ["yyyy", "day", "yy", "mm", "dd", "hh", "mi", "ss"];
    let name = NAMES.into_iter().find(|n| rest.starts_with(n))?;
    Some((name.len(), letter_value(name, p)))
}

fn letter_value(name: &str, p: &Parts) -> String {
    match name {
        "yyyy" => format!("{:04}", p.y),
        "yy" => format!("{:02}", p.y.rem_euclid(100)),
        "mm" => format!("{:02}", p.mo),
        "dd" => format!("{:02}", p.d),
        "hh" => format!("{:02}", p.h),
        "mi" => format!("{:02}", p.mi),
        "ss" => format!("{:02}", p.s),
        "day" => weekday(p.days).to_string(),
        _ => unreachable!("letter_value is called with a name from NAMES"),
    }
}

/// The strftime vocabulary, deliberately small: `None` = not ours, the caller copies it through.
fn strftime(c: char, p: &Parts) -> Option<String> {
    Some(match c {
        'Y' => format!("{:04}", p.y),
        'y' => format!("{:02}", p.y.rem_euclid(100)),
        'm' => format!("{:02}", p.mo),
        'd' => format!("{:02}", p.d),
        'H' => format!("{:02}", p.h),
        'M' => format!("{:02}", p.mi),
        'S' => format!("{:02}", p.s),
        'F' => format!("{:04}-{:02}-{:02}", p.y, p.mo, p.d),
        'T' => format!("{:02}:{:02}:{:02}", p.h, p.mi, p.s),
        '%' => "%".to_string(),
        _ => return None,
    })
}

/// One user message's injection: prefix and suffix, each `None` when its template renders to
/// nothing. Arguments: the switch, both templates, now, and the previous reply.
pub fn inject(
    enabled: bool,
    prefix: &str,
    suffix: &str,
    now: &str,
    last: Option<&str>,
) -> (Option<String>, Option<String>) {
    if !enabled {
        return (None, None);
    }
    (render(prefix, now, last), render(suffix, now, last))
}
