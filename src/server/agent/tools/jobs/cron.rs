//! Five-field cron (`minute hour day month weekday`) in the process's local zone.

use anyhow::{Result, bail};

/// A parsed cron expression. Bits are indexed by the field value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cron {
    minutes: u64,
    hours: u32,
    days: u32,
    months: u16,
    weekdays: u8,
    days_restricted: bool,
    weekdays_restricted: bool,
}

impl Cron {
    pub(crate) fn parse(text: &str) -> Result<Self> {
        let fields: Vec<&str> = text.split_whitespace().collect();
        anyhow::ensure!(fields.len() == 5, "cron needs five fields");
        let minutes = field(fields[0], 0, 59)?;
        let hours = field(fields[1], 0, 23)?;
        let days = field(fields[2], 1, 31)?;
        let months = field(fields[3], 1, 12)?;
        let weekdays = field(fields[4], 0, 7)?;
        let weekdays = if weekdays & (1 << 7) != 0 {
            weekdays | 1
        } else {
            weekdays
        };
        Ok(Self {
            minutes,
            hours: hours as u32,
            days: days as u32,
            months: months as u16,
            weekdays: (weekdays & 0x7f) as u8,
            days_restricted: fields[2] != "*",
            weekdays_restricted: fields[4] != "*",
        })
    }

    /// The first matching minute strictly after `after_unix`.
    pub(crate) fn next(&self, after_unix: i64) -> Option<i64> {
        let mut minute = after_unix.div_euclid(60).saturating_add(1);
        let limit = minute.saturating_add(5 * 366 * 24 * 60);
        while minute <= limit {
            let (y, month, day, hour, min, _) =
                crate::server::platform::time::local_parts(minute * 60);
            let weekday = weekday(y, month, day);
            let day_ok = bit(self.days as u64, day) != 0;
            let weekday_ok = bit(self.weekdays as u64, weekday) != 0;
            let date_ok = if self.days_restricted && self.weekdays_restricted {
                day_ok || weekday_ok
            } else {
                day_ok && weekday_ok
            };
            if bit(self.months as u64, month) != 0
                && date_ok
                && bit(self.hours as u64, hour) != 0
                && bit(self.minutes, min) != 0
            {
                return Some(minute * 60);
            }
            minute += 1;
        }
        None
    }
}

fn bit(bits: u64, value: u32) -> u64 {
    bits & (1u64 << value)
}

fn weekday(year: i64, month: u32, day: u32) -> u32 {
    let days = crate::server::platform::time::days_from_civil(year, month, day);
    (days + 4).rem_euclid(7) as u32
}

fn field(text: &str, min: u32, max: u32) -> Result<u64> {
    let mut bits = 0u64;
    for part in text.split(',') {
        anyhow::ensure!(!part.is_empty(), "empty cron field");
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (range, step.parse::<u32>()?),
            None => (part, 1),
        };
        anyhow::ensure!(step > 0, "cron step must be positive");
        let (start, end) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (a.parse()?, b.parse()?)
        } else {
            let value = range.parse::<u32>()?;
            (value, value)
        };
        anyhow::ensure!(
            start >= min && end <= max && start <= end,
            "cron value out of range"
        );
        let mut value = start;
        while value <= end {
            bits |= 1u64 << value;
            value = value.saturating_add(step);
            if value == u32::MAX {
                break;
            }
        }
    }
    if bits == 0 {
        bail!("empty cron field");
    }
    Ok(bits)
}
