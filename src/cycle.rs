//! The billing cycle is the calendar month in UTC, which is how GitHub's
//! usage endpoint is keyed (`?year=&month=`).

use crate::money::Usd;
use jiff::civil::date;
use jiff::Timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cycle {
    pub year: i16,
    pub month: i8,
}

impl Cycle {
    pub fn containing(now: Timestamp) -> Cycle {
        let d = now.to_zoned(jiff::tz::TimeZone::UTC).date();
        Cycle { year: d.year(), month: d.month() }
    }

    pub fn start(&self) -> Timestamp {
        date(self.year, self.month, 1)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .unwrap()
            .timestamp()
    }

    pub fn end(&self) -> Timestamp {
        let (y, m) = if self.month == 12 { (self.year + 1, 1) } else { (self.year, self.month + 1) };
        date(y, m, 1).to_zoned(jiff::tz::TimeZone::UTC).unwrap().timestamp()
    }

    pub fn days_in_month(&self) -> i64 {
        date(self.year, self.month, 1)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .unwrap()
            .date()
            .days_in_month() as i64
    }

    pub fn label(&self) -> String {
        const NAMES: [&str; 12] = [
            "January", "February", "March", "April", "May", "June", "July",
            "August", "September", "October", "November", "December",
        ];
        format!("{} {}", NAMES[(self.month - 1) as usize], self.year)
    }

    fn secs_total(&self) -> f64 {
        (self.end().as_second() - self.start().as_second()) as f64
    }

    pub fn elapsed_fraction(&self, now: Timestamp) -> f64 {
        let elapsed = (now.as_second() - self.start().as_second()) as f64;
        (elapsed / self.secs_total()).clamp(0.0, 1.0)
    }

    /// Extrapolate spend-so-far to month end. Never returns less than actual
    /// spend, and never divides by a near-zero elapsed fraction.
    pub fn project(&self, spent: Usd, now: Timestamp) -> Usd {
        let f = self.elapsed_fraction(now);
        // Below ~1% of the month there is no signal worth extrapolating from.
        if f < 0.01 {
            return spent;
        }
        let projected = spent * (1.0 / f);
        projected.max(spent)
    }

    pub fn resets_in_human(&self, now: Timestamp) -> String {
        let secs = (self.end().as_second() - now.as_second()).max(0);
        human_duration(secs)
    }
}

pub fn human_duration(secs: i64) -> String {
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3_600;
    let m = (secs % 3_600) / 60;
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// Elapsed wall-clock for an in-flight job, e.g. "6m12s".
pub fn elapsed_short(from: Timestamp, now: Timestamp) -> String {
    let secs = (now.as_second() - from.as_second()).max(0);
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h{m:02}m")
    } else {
        format!("{m}m{s:02}s")
    }
}
