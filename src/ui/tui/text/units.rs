//! Human units for counts and durations.

/// A byte count: one decimal above 1 KiB, bare below it.
pub fn bytes(n: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let n = n as f64;
    if n >= MIB {
        format!("{:.1} MB", n / MIB)
    } else if n >= KIB {
        format!("{:.1} KB", n / KIB)
    } else {
        format!("{} B", n as u64)
    }
}

/// A duration in milliseconds, at whatever scale it reads best.
pub fn duration(ms: u64) -> String {
    const SEC: u64 = 1_000;
    const MIN: u64 = 60 * SEC;
    const HOUR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HOUR;
    if ms < SEC {
        return format!("{ms}ms");
    }
    if ms < MIN {
        return format!("{:.1}s", ms as f64 / SEC as f64);
    }
    if ms < HOUR {
        let (m, s) = (ms / MIN, (ms % MIN) / SEC);
        return if s > 0 {
            format!("{m}m{s}s")
        } else {
            format!("{m}m")
        };
    }
    if ms < DAY {
        let (h, m) = (ms / HOUR, (ms % HOUR) / MIN);
        return if m > 0 {
            format!("{h}h{m}m")
        } else {
            format!("{h}h")
        };
    }
    let (d, h) = (ms / DAY, (ms % DAY) / HOUR);
    if h > 0 {
        format!("{d}d{h}h")
    } else {
        format!("{d}d")
    }
}
