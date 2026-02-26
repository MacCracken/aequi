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
    type Output = Option<Self>;
    fn add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Money)
    }
}

impl Sub for Money {
    type Output = Option<Self>;
    fn sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Money)
    }
}
