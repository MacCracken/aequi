use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::Money;

use super::document::InvoiceId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payment {
    pub id: Option<i64>,
    pub invoice_id: InvoiceId,
    pub amount: Money,
    pub date: NaiveDate,
    pub method: Option<String>,
    pub transaction_id: Option<i64>,
}

impl Payment {
    pub fn new(invoice_id: InvoiceId, amount: Money, date: NaiveDate) -> Self {
        Payment {
            id: None,
            invoice_id,
            amount,
            date,
            method: None,
            transaction_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payment_new() {
        let p = Payment::new(
            InvoiceId(1),
            Money::from_cents(50000),
            NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
        );
        assert_eq!(p.invoice_id, InvoiceId(1));
        assert_eq!(p.amount.to_cents(), 50000);
        assert!(p.id.is_none());
        assert!(p.method.is_none());
        assert!(p.transaction_id.is_none());
    }
}
