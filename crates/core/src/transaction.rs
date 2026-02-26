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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineType {
    Debit,
    Credit,
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
