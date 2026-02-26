use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::account::{AccountId, LedgerError};
use super::money::Money;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionLine {
    pub account_id: AccountId,
    pub debit: Money,
    pub credit: Money,
    pub memo: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnvalidatedTransaction {
    pub date: NaiveDate,
    pub description: String,
    pub lines: Vec<TransactionLine>,
    pub memo: Option<String>,
}

impl UnvalidatedTransaction {
    pub fn total_debits(&self) -> Money {
        self.lines
            .iter()
            .map(|l| l.debit)
            .fold(Money::zero(), |a, b| a + b)
    }

    pub fn total_credits(&self) -> Money {
        self.lines
            .iter()
            .map(|l| l.credit)
            .fold(Money::zero(), |a, b| a + b)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedTransaction {
    pub id: Option<i64>,
    pub date: NaiveDate,
    pub description: String,
    pub lines: Vec<TransactionLine>,
    pub memo: Option<String>,
    pub balanced_total: Money,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl ValidatedTransaction {
    pub fn validate(tx: UnvalidatedTransaction) -> Result<ValidatedTransaction, LedgerError> {
        if tx.lines.len() < 2 {
            return Err(LedgerError::EmptyTransaction);
        }

        let total_debits = tx.total_debits();
        let total_credits = tx.total_credits();

        if total_debits != total_credits {
            return Err(LedgerError::Unbalanced(total_debits, total_credits));
        }

        Ok(ValidatedTransaction {
            id: None,
            date: tx.date,
            description: tx.description,
            lines: tx.lines,
            memo: tx.memo,
            balanced_total: total_debits,
            created_at: None,
        })
    }
}

impl TransactionLine {
    pub fn debit(account_id: AccountId, amount: Money, memo: Option<String>) -> Self {
        TransactionLine {
            account_id,
            debit: amount,
            credit: Money::zero(),
            memo,
        }
    }

    pub fn credit(account_id: AccountId, amount: Money, memo: Option<String>) -> Self {
        TransactionLine {
            account_id,
            debit: Money::zero(),
            credit: amount,
            memo,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn id(n: i64) -> AccountId {
        AccountId(n)
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn simple_tx(debit_id: AccountId, credit_id: AccountId, cents: i64) -> UnvalidatedTransaction {
        UnvalidatedTransaction {
            date: date(2024, 1, 15),
            description: "Test".to_string(),
            lines: vec![
                TransactionLine::debit(debit_id, Money::from_cents(cents), None),
                TransactionLine::credit(credit_id, Money::from_cents(cents), None),
            ],
            memo: None,
        }
    }

    #[test]
    fn validate_balanced_transaction() {
        let tx = simple_tx(id(1), id(2), 5000);
        let validated = ValidatedTransaction::validate(tx).unwrap();
        assert_eq!(validated.balanced_total.to_cents(), 5000);
    }

    #[test]
    fn validate_rejects_unbalanced() {
        let tx = UnvalidatedTransaction {
            date: date(2024, 1, 15),
            description: "Bad".to_string(),
            lines: vec![
                TransactionLine::debit(id(1), Money::from_cents(500), None),
                TransactionLine::credit(id(2), Money::from_cents(400), None),
            ],
            memo: None,
        };
        assert!(matches!(
            ValidatedTransaction::validate(tx),
            Err(LedgerError::Unbalanced(_, _))
        ));
    }

    #[test]
    fn validate_rejects_single_line() {
        let tx = UnvalidatedTransaction {
            date: date(2024, 1, 15),
            description: "Single".to_string(),
            lines: vec![TransactionLine::debit(id(1), Money::from_cents(500), None)],
            memo: None,
        };
        assert!(matches!(
            ValidatedTransaction::validate(tx),
            Err(LedgerError::EmptyTransaction)
        ));
    }

    #[test]
    fn validate_rejects_empty_lines() {
        let tx = UnvalidatedTransaction {
            date: date(2024, 1, 15),
            description: "Empty".to_string(),
            lines: vec![],
            memo: None,
        };
        assert!(matches!(
            ValidatedTransaction::validate(tx),
            Err(LedgerError::EmptyTransaction)
        ));
    }

    #[test]
    fn validate_multi_line_balanced() {
        // Split transaction: one credit, two debits
        let tx = UnvalidatedTransaction {
            date: date(2024, 1, 15),
            description: "Split".to_string(),
            lines: vec![
                TransactionLine::debit(id(1), Money::from_cents(300), None),
                TransactionLine::debit(id(2), Money::from_cents(200), None),
                TransactionLine::credit(id(3), Money::from_cents(500), None),
            ],
            memo: None,
        };
        let validated = ValidatedTransaction::validate(tx).unwrap();
        assert_eq!(validated.balanced_total.to_cents(), 500);
    }

    #[test]
    fn total_debits_and_credits() {
        let tx = simple_tx(id(1), id(2), 1234);
        assert_eq!(tx.total_debits().to_cents(), 1234);
        assert_eq!(tx.total_credits().to_cents(), 1234);
    }

    #[test]
    fn transaction_line_constructors() {
        let d = TransactionLine::debit(id(5), Money::from_cents(100), Some("note".to_string()));
        assert_eq!(d.debit.to_cents(), 100);
        assert_eq!(d.credit.to_cents(), 0);
        assert_eq!(d.memo.as_deref(), Some("note"));

        let c = TransactionLine::credit(id(5), Money::from_cents(100), None);
        assert_eq!(c.debit.to_cents(), 0);
        assert_eq!(c.credit.to_cents(), 100);
    }
}
