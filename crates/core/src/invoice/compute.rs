use chrono::Datelike;

use crate::Money;

use super::payment::Payment;

/// The IRS 1099-NEC reporting threshold for payments made in `year`: $600
/// through 2025, $2,000 from 2026 (P.L. 119-21). The 2026 figure is indexed
/// for inflation from 2027; update this when the IRS publishes those amounts.
pub fn nec_threshold(year: u16) -> Money {
    if year >= 2026 {
        Money::from_cents(200_000)
    } else {
        Money::from_cents(60_000)
    }
}

/// Sum all payments within a calendar year.
pub fn compute_ytd_payments(payments: &[Payment], year: u16) -> Money {
    payments
        .iter()
        .filter(|p| p.date.year_ce().1 as u16 == year)
        .map(|p| p.amount)
        .fold(Money::zero(), |a, b| a + b)
}

/// Returns true if the YTD amount for `year` meets or exceeds the 1099-NEC threshold.
pub fn check_1099_threshold(ytd_amount: Money, year: u16) -> bool {
    ytd_amount >= nec_threshold(year)
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
    fn threshold_is_600_through_2025() {
        assert!(check_1099_threshold(Money::from_cents(60000), 2025));
        assert!(!check_1099_threshold(Money::from_cents(59999), 2025));
        assert!(check_1099_threshold(Money::from_cents(100000), 2025));
        assert!(!check_1099_threshold(Money::zero(), 2025));
    }

    #[test]
    fn threshold_is_2000_from_2026() {
        assert!(!check_1099_threshold(Money::from_cents(150000), 2026));
        assert!(!check_1099_threshold(Money::from_cents(199999), 2026));
        assert!(check_1099_threshold(Money::from_cents(200000), 2026));
        assert!(check_1099_threshold(Money::from_cents(200000), 2027));
    }
}
