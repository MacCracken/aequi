use chrono::Datelike;

use crate::Money;

use super::payment::Payment;

/// The IRS 1099-NEC reporting threshold: $600.
const NEC_THRESHOLD_CENTS: i64 = 60000;

/// Sum all payments within a calendar year.
pub fn compute_ytd_payments(payments: &[Payment], year: u16) -> Money {
    payments
        .iter()
        .filter(|p| p.date.year_ce().1 as u16 == year)
        .map(|p| p.amount)
        .fold(Money::zero(), |a, b| a + b)
}

/// Returns true if the YTD amount meets or exceeds the $600 1099-NEC threshold.
pub fn check_1099_threshold(ytd_amount: Money) -> bool {
    ytd_amount.to_cents() >= NEC_THRESHOLD_CENTS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invoice::InvoiceId;
    use chrono::NaiveDate;

    fn payment(cents: i64, year: i32, month: u32, day: u32) -> Payment {
        Payment {
            id: None,
            invoice_id: InvoiceId(1),
            amount: Money::from_cents(cents),
            date: NaiveDate::from_ymd_opt(year, month, day).unwrap(),
            method: None,
            transaction_id: None,
        }
    }

    #[test]
    fn ytd_payments_filters_by_year() {
        let payments = vec![
            payment(10000, 2025, 6, 1),
            payment(20000, 2026, 1, 15),
            payment(30000, 2026, 3, 10),
            payment(5000, 2027, 1, 1),
        ];
        let ytd = compute_ytd_payments(&payments, 2026);
        assert_eq!(ytd.to_cents(), 50000); // $200 + $300
    }

    #[test]
    fn ytd_payments_empty() {
        let ytd = compute_ytd_payments(&[], 2026);
        assert_eq!(ytd.to_cents(), 0);
    }

    #[test]
    fn threshold_at_600() {
        assert!(check_1099_threshold(Money::from_cents(60000)));
    }

    #[test]
    fn threshold_below_600() {
        assert!(!check_1099_threshold(Money::from_cents(59999)));
    }

    #[test]
    fn threshold_above_600() {
        assert!(check_1099_threshold(Money::from_cents(100000)));
    }

    #[test]
    fn threshold_zero() {
        assert!(!check_1099_threshold(Money::zero()));
    }
}
