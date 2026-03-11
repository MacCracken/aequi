use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::Money;

use super::contact::ContactId;
use super::lifecycle::InvoiceStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceId(pub i64);

impl std::fmt::Display for InvoiceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub id: Option<InvoiceId>,
    pub invoice_number: String,
    pub contact_id: ContactId,
    pub status: InvoiceStatus,
    pub issue_date: NaiveDate,
    pub due_date: NaiveDate,
    pub lines: Vec<InvoiceLine>,
    pub discount: Option<Discount>,
    pub tax_lines: Vec<TaxLine>,
    pub notes: Option<String>,
    pub terms: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceLine {
    pub description: String,
    pub quantity: Decimal,
    pub unit_rate: Money,
    pub taxable: bool,
}

impl InvoiceLine {
    pub fn amount(&self) -> Money {
        self.unit_rate * self.quantity
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Discount {
    Percentage(Decimal),
    Flat(Money),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxLine {
    pub label: String,
    pub rate: Decimal,
}

impl Invoice {
    pub fn subtotal(&self) -> Money {
        self.lines
            .iter()
            .map(|l| l.amount())
            .fold(Money::zero(), |a, b| a + b)
    }

    pub fn discount_amount(&self) -> Money {
        match &self.discount {
            Some(Discount::Percentage(pct)) => self.subtotal() * *pct,
            Some(Discount::Flat(amount)) => *amount,
            None => Money::zero(),
        }
    }

    pub fn taxable_subtotal(&self) -> Money {
        let taxable = self
            .lines
            .iter()
            .filter(|l| l.taxable)
            .map(|l| l.amount())
            .fold(Money::zero(), |a, b| a + b);
        // Apply discount proportionally to taxable amount
        let subtotal = self.subtotal();
        if subtotal.is_zero() {
            return Money::zero();
        }
        let discount_ratio = self.discount_amount().as_decimal() / subtotal.as_decimal();
        taxable - taxable * discount_ratio
    }

    pub fn tax_amount(&self) -> Money {
        let taxable = self.taxable_subtotal();
        self.tax_lines
            .iter()
            .map(|t| taxable * t.rate)
            .fold(Money::zero(), |a, b| a + b)
    }

    pub fn total(&self) -> Money {
        self.subtotal() - self.discount_amount() + self.tax_amount()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn sample_invoice() -> Invoice {
        Invoice {
            id: None,
            invoice_number: "INV-001".to_string(),
            contact_id: ContactId(1),
            status: InvoiceStatus::Draft,
            issue_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            due_date: NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
            lines: vec![
                InvoiceLine {
                    description: "Web Development".to_string(),
                    quantity: Decimal::from(10),
                    unit_rate: Money::from_cents(15000), // $150/hr
                    taxable: true,
                },
                InvoiceLine {
                    description: "Hosting".to_string(),
                    quantity: Decimal::from(1),
                    unit_rate: Money::from_cents(5000), // $50
                    taxable: false,
                },
            ],
            discount: None,
            tax_lines: vec![TaxLine {
                label: "Sales Tax 8.5%".to_string(),
                rate: Decimal::from_str("0.085").unwrap(),
            }],
            notes: None,
            terms: None,
        }
    }

    #[test]
    fn subtotal() {
        let inv = sample_invoice();
        // 10 * $150 + 1 * $50 = $1,550
        assert_eq!(inv.subtotal().to_cents(), 155000);
    }

    #[test]
    fn no_discount() {
        let inv = sample_invoice();
        assert_eq!(inv.discount_amount().to_cents(), 0);
    }

    #[test]
    fn flat_discount() {
        let mut inv = sample_invoice();
        inv.discount = Some(Discount::Flat(Money::from_cents(10000))); // $100 off
        assert_eq!(inv.discount_amount().to_cents(), 10000);
        // Total should be less than total_with_tax (no discount) = $1677.50
        let no_discount_total = 167750;
        assert!(inv.total().to_cents() < no_discount_total);
        // Subtotal $1550 - $100 discount = $1450 base
        // Tax reduced proportionally on taxable portion
        assert!(inv.total().to_cents() > 145000); // more than base due to tax
    }

    #[test]
    fn percentage_discount() {
        let mut inv = sample_invoice();
        inv.discount = Some(Discount::Percentage(Decimal::from_str("0.10").unwrap()));
        // 10% of $1550 = $155
        assert_eq!(inv.discount_amount().to_cents(), 15500);
    }

    #[test]
    fn tax_on_taxable_lines_only() {
        let inv = sample_invoice();
        // Only "Web Development" is taxable: $1500
        // Tax = $1500 * 0.085 = $127.50
        assert_eq!(inv.tax_amount().to_cents(), 12750);
    }

    #[test]
    fn total_with_tax() {
        let inv = sample_invoice();
        // $1550 + $127.50 = $1677.50
        assert_eq!(inv.total().to_cents(), 167750);
    }

    #[test]
    fn line_amount() {
        let line = InvoiceLine {
            description: "Work".to_string(),
            quantity: Decimal::from(5),
            unit_rate: Money::from_cents(10000),
            taxable: false,
        };
        assert_eq!(line.amount().to_cents(), 50000);
    }

    #[test]
    fn invoice_id_display() {
        assert_eq!(InvoiceId(42).to_string(), "42");
    }

    #[test]
    fn empty_invoice_total_is_zero() {
        let inv = Invoice {
            id: None,
            invoice_number: "INV-000".to_string(),
            contact_id: ContactId(1),
            status: InvoiceStatus::Draft,
            issue_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            due_date: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            lines: vec![],
            discount: None,
            tax_lines: vec![],
            notes: None,
            terms: None,
        };
        assert_eq!(inv.total().to_cents(), 0);
    }
}
