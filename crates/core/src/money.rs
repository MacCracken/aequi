use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Money(Decimal);

impl Money {
    pub fn from_cents(cents: i64) -> Self {
        Money(Decimal::from(cents) / Decimal::from(100))
    }

    pub fn to_cents(self) -> i64 {
        (self.0 * Decimal::from(100)).to_i64().unwrap()
    }

    pub fn from_decimal(decimal: Decimal) -> Self {
        Money(decimal.round_dp(2))
    }

    pub fn zero() -> Self {
        Money(Decimal::ZERO)
    }

    pub fn is_zero(self) -> bool {
        self.0.is_zero()
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${:.2}", self.0)
    }
}

impl Add for Money {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Money(self.0 + rhs.0)
    }
}

impl Sub for Money {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Money(self.0 - rhs.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_cents_roundtrip() {
        assert_eq!(Money::from_cents(100).to_cents(), 100);
        assert_eq!(Money::from_cents(-5000).to_cents(), -5000);
        assert_eq!(Money::from_cents(0).to_cents(), 0);
        assert_eq!(Money::from_cents(1).to_cents(), 1);
        assert_eq!(Money::from_cents(i64::MAX / 100).to_cents(), i64::MAX / 100);
    }

    #[test]
    fn display_formats_correctly() {
        assert_eq!(Money::from_cents(1000).to_string(), "$10.00");
        assert_eq!(Money::from_cents(1).to_string(), "$0.01");
        assert_eq!(Money::from_cents(0).to_string(), "$0.00");
        assert_eq!(Money::from_cents(-500).to_string(), "$-5.00");
        assert_eq!(Money::from_cents(100000).to_string(), "$1000.00");
    }

    #[test]
    fn add() {
        assert_eq!((Money::from_cents(1000) + Money::from_cents(250)).to_cents(), 1250);
        assert_eq!((Money::from_cents(0) + Money::from_cents(0)).to_cents(), 0);
        assert_eq!((Money::from_cents(-500) + Money::from_cents(1000)).to_cents(), 500);
    }

    #[test]
    fn sub() {
        assert_eq!((Money::from_cents(1000) - Money::from_cents(250)).to_cents(), 750);
        assert_eq!((Money::from_cents(500) - Money::from_cents(500)).to_cents(), 0);
        assert_eq!((Money::from_cents(100) - Money::from_cents(200)).to_cents(), -100);
    }

    #[test]
    fn zero_and_is_zero() {
        assert!(Money::zero().is_zero());
        assert!(!Money::from_cents(1).is_zero());
        assert!(!Money::from_cents(-1).is_zero());
        assert_eq!(Money::zero().to_cents(), 0);
    }

    #[test]
    fn from_decimal_rounds_to_two_dp() {
        use rust_decimal::Decimal;
        use std::str::FromStr;
        let m = Money::from_decimal(Decimal::from_str("10.125").unwrap());
        // rust_decimal default rounding is MidpointNearestEven (banker's rounding)
        assert_eq!(m.to_cents(), 1012);
        let m = Money::from_decimal(Decimal::from_str("10.135").unwrap());
        assert_eq!(m.to_cents(), 1014);
    }

    #[test]
    fn ordering() {
        assert!(Money::from_cents(100) > Money::from_cents(50));
        assert!(Money::from_cents(-10) < Money::from_cents(0));
        assert_eq!(Money::from_cents(100), Money::from_cents(100));
    }
}
