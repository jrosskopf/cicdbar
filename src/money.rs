//! Money as integer micro-dollars.
//!
//! The billing API quotes `pricePerUnit` values like 0.00033602 and amounts
//! like 79.977071728, and we sum thousands of them. f64 drifts; this does not.

use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Mul, Sub};

const MICROS: i64 = 1_000_000;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
pub struct Usd(i64);

impl Usd {
    pub const fn from_micros(micros: i64) -> Self {
        Usd(micros)
    }

    pub fn from_f64(v: f64) -> Self {
        Usd((v * MICROS as f64).round() as i64)
    }

    pub const fn zero() -> Self {
        Usd(0)
    }

    pub const fn micros(self) -> i64 {
        self.0
    }

    pub fn as_f64(self) -> f64 {
        self.0 as f64 / MICROS as f64
    }

    /// Cents, rounded half away from zero.
    fn cents(self) -> i64 {
        let (sign, m) = if self.0 < 0 { (-1, -self.0) } else { (1, self.0) };
        sign * ((m + 5_000) / 10_000)
    }

    /// Percentage of `budget`; `None` when the budget is zero.
    pub fn pct_of(self, budget: Usd) -> Option<f64> {
        if budget.0 == 0 {
            None
        } else {
            Some(self.0 as f64 / budget.0 as f64 * 100.0)
        }
    }

    /// Short form for the collapsed bar, where width is scarce.
    pub fn compact(self) -> String {
        let v = self.as_f64();
        let a = v.abs();
        if a >= 1000.0 {
            format!("${:.1}k", v / 1000.0)
        } else if a >= 100.0 {
            format!("${:.0}", v)
        } else {
            format!("${:.2}", v)
        }
    }
}

impl fmt::Display for Usd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = self.cents();
        let neg = c < 0;
        let c = c.abs();
        let (whole, frac) = (c / 100, c % 100);
        let digits = whole.to_string();
        let mut grouped = String::new();
        for (i, ch) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(ch);
        }
        write!(f, "{}${}.{:02}", if neg { "-" } else { "" }, grouped, frac)
    }
}

impl Add for Usd {
    type Output = Usd;
    fn add(self, rhs: Usd) -> Usd {
        Usd(self.0 + rhs.0)
    }
}

impl Sub for Usd {
    type Output = Usd;
    fn sub(self, rhs: Usd) -> Usd {
        Usd(self.0 - rhs.0)
    }
}

impl Mul<f64> for Usd {
    type Output = Usd;
    fn mul(self, rhs: f64) -> Usd {
        Usd((self.0 as f64 * rhs).round() as i64)
    }
}

impl AddAssign for Usd {
    fn add_assign(&mut self, rhs: Usd) {
        self.0 += rhs.0;
    }
}

impl Sum for Usd {
    fn sum<I: Iterator<Item = Usd>>(iter: I) -> Usd {
        iter.fold(Usd::zero(), |a, b| a + b)
    }
}
