//! Local time: the zone table and the stamp helpers.

/// The local zone: a table of "from this moment on, the offset is that".
struct Zone {
    /// `(transition moment (UTC seconds), offset after it (seconds))`,
    /// ascending by moment.
    transitions: Vec<(i64, i32)>,
    /// The offset before the first transition.
    initial: i32,
}

impl Zone {
    fn fixed(secs: i32) -> Self {
        Self {
            transitions: Vec::new(),
            initial: secs,
        }
    }

    /// The offset at a moment.
    fn offset_at(&self, unix: i64) -> i32 {
        match self.transitions.partition_point(|(t, _)| *t <= unix) {
            0 => self.initial,
            n => self.transitions[n - 1].1,
        }
    }
}

/// The process's zone, written exactly once.
static ZONE: std::sync::OnceLock<Zone> = std::sync::OnceLock::new();

/// Take the zone from configuration. Called once, at startup, before anyone
/// asks for the time. `None` = whatever this machine says.
pub fn init(name: Option<&str>) {
    let zone = load_zone(name).unwrap_or_else(|| Zone::fixed(0));
    let _ = ZONE.set(zone);
}

/// The process's zone, loading the machine's own when nobody called [`init`].
fn local_zone() -> &'static Zone {
    ZONE.get_or_init(|| load_zone(None).unwrap_or_else(|| Zone::fixed(0)))
}

/// The named zone, else the machine's, else nothing.
fn load_zone(name: Option<&str>) -> Option<Zone> {
    if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
        // A fixed offset like `UTC+8` needs no file at all.
        if let Some(off) = fixed_offset(name) {
            return Some(Zone::fixed(off));
        }
        // A bare name is looked up in the system's zoneinfo; an absolute
        // path is read where it points.
        let path = if name.starts_with('/') {
            std::path::PathBuf::from(name)
        } else {
            std::path::PathBuf::from("/usr/share/zoneinfo").join(name)
        };
        if let Ok(bytes) = std::fs::read(&path)
            && let Some(zone) = parse_tzif(&bytes)
        {
            return Some(zone);
        }
    }
    parse_tzif(&std::fs::read("/etc/localtime").ok()?)
}

/// `UTC+8` / `GMT-5` / `UTC+8:30` → the seconds to add to UTC for local
/// time.
fn fixed_offset(name: &str) -> Option<i32> {
    let rest = name
        .strip_prefix("UTC")
        .or_else(|| name.strip_prefix("GMT"))
        .unwrap_or(name);
    let (sign, digits) = rest.split_at_checked(1)?;
    // POSIX sign: `UTC+8` means "eight hours ahead of UTC", i.e. local =
    // UTC - 8h.
    let mult = match sign {
        "+" => -1,
        "-" => 1,
        _ => return None,
    };
    let (h, m) = match digits.split_once(':') {
        Some((h, m)) => (h, m),
        None => (digits, "0"),
    };
    let (h, m) = (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?);
    (h <= 24 && m < 60).then(|| mult * (h * 3600 + m * 60))
}

/// Parse one TZif (zoneinfo) file into a transition table: the 64-bit block when the version has
/// one, both DST entries, and no POSIX footer (that describes "after the last transition").
fn parse_tzif(data: &[u8]) -> Option<Zone> {
    if data.len() < 44 || &data[0..4] != b"TZif" {
        return None;
    }
    let be = |o: usize| -> Option<u32> {
        let b = data.get(o..o + 4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let counts = |h: usize| -> Option<(usize, usize, usize, usize, usize, usize)> {
        Some((
            be(h + 20)? as usize, // isutcnt
            be(h + 24)? as usize, // isstdcnt
            be(h + 28)? as usize, // leapcnt
            be(h + 32)? as usize, // timecnt
            be(h + 36)? as usize, // typecnt
            be(h + 40)? as usize, // charcnt
        ))
    };
    let (isutcnt, isstdcnt, leapcnt, mut timecnt, mut typecnt, charcnt) = counts(0)?;
    let mut block = 44usize;
    let mut wide = false;
    if matches!(data[4], b'2' | b'3' | b'4') {
        let first = timecnt
            .checked_mul(4)?
            .checked_add(timecnt)?
            .checked_add(typecnt.checked_mul(6)?)?
            .checked_add(charcnt)?
            .checked_add(leapcnt.checked_mul(8)?)?
            .checked_add(isstdcnt)?
            .checked_add(isutcnt)?;
        block = 44usize.checked_add(first)?;
        let (_, _, _, t, y, _) = counts(block)?;
        block = block.checked_add(44)?;
        timecnt = t;
        typecnt = y;
        wide = true;
    }
    let width = if wide { 8 } else { 4 };
    let times_at = block;
    let types_at = times_at.checked_add(timecnt.checked_mul(width)?)?;
    let tt_at = types_at.checked_add(timecnt)?;
    let mut offsets = Vec::with_capacity(typecnt);
    let mut standard = None;
    for i in 0..typecnt {
        let at = tt_at.checked_add(i * 6)?;
        let b = data.get(at..at + 4)?;
        let off = i32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        // ttinfo = 4-byte offset + 1-byte DST flag + 1-byte abbreviation
        // index.
        let isdst = *data.get(at + 4)? != 0;
        if !isdst && standard.is_none() {
            standard = Some(off);
        }
        offsets.push(off);
    }
    let mut transitions = Vec::with_capacity(timecnt);
    for i in 0..timecnt {
        let at = times_at + i * width;
        let t = if wide {
            let b = data.get(at..at + 8)?;
            i64::from_be_bytes(b.try_into().ok()?)
        } else {
            let b = data.get(at..at + 4)?;
            i32::from_be_bytes(b.try_into().ok()?) as i64
        };
        let idx = *data.get(types_at + i)? as usize;
        transitions.push((t, *offsets.get(idx)?));
    }
    // Before the first transition use the first non-DST type (the TZif convention).
    let initial = standard.or_else(|| offsets.first().copied()).unwrap_or(0);
    Some(Zone {
        transitions,
        initial,
    })
}

pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn offset_at(unix: i64) -> i32 {
    local_zone().offset_at(unix)
}

pub(crate) fn local_parts(unix: i64) -> (i64, u32, u32, u32, u32, u32) {
    let local = unix + local_zone().offset_at(unix) as i64;
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    (
        y,
        m,
        d,
        (secs / 3600) as u32,
        ((secs % 3600) / 60) as u32,
        (secs % 60) as u32,
    )
}

/// Now (local zone) as `YYYY-MM-DD HH:MM:SS`.
pub fn now_stamp() -> String {
    let now = unix_now();
    stamp_from_unix(now + local_zone().offset_at(now) as i64)
}

/// `YYYY-MM-DD HH:MM:SS` for a Unix timestamp **already shifted into local time**: the civil-date
/// arithmetic never touches the clock.
fn stamp_from_unix(local: i64) -> String {
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 → (y, m, d).
/// Floored arithmetic, so it is correct for negative (pre-1970) inputs too.
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Local `YYYY-MM-DD HH:MM:SS` → seconds (treated as UTC; the inverse of
/// [`stamp_from_unix`]). `None` when unreadable.
fn parse_stamp(stamp: &str) -> Option<i64> {
    let b = stamp.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> {
        let s = std::str::from_utf8(b.get(a..z)?).ok()?;
        s.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| s.parse().ok())?
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, s) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if b[4] != b'-' || b[7] != b'-' || b[10] != b' ' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let days = days_from_civil(y, mo as u32, d as u32);
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
}

/// Inverse of [`civil_from_days`]: (y, m, d) → days since 1970-01-01.
pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// A local stamp → days since 1970-01-01 (weekday math runs on days).
pub fn days_from_stamp(stamp: &str) -> Option<i64> {
    parse_stamp(stamp).map(|s| s.div_euclid(86_400))
}

/// A local stamp's age in seconds. `None`: the stamp is unreadable; a future stamp saturates to 0.
pub fn age_seconds(stamp: &str) -> Option<u64> {
    let local = parse_stamp(stamp)?;
    let now = unix_now();
    let now_local = now + local_zone().offset_at(now) as i64;
    Some(now_local.saturating_sub(local).max(0) as u64)
}
