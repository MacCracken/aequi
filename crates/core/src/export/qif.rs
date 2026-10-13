use std::collections::HashMap;

use rust_decimal::Decimal;

use crate::{Account, AccountId, AccountType, ValidatedTransaction};

/// QIF register type for a balance-sheet account, or `None` for accounts that
/// QIF represents as categories (income, expense, equity).
fn qif_type(account_type: AccountType) -> Option<&'static str> {
    match account_type {
        AccountType::Asset => Some("Bank"),
        AccountType::Liability => Some("CCard"),
        AccountType::Equity | AccountType::Income | AccountType::Expense => None,
    }
}

/// Strip characters that would break QIF's line-oriented records.
fn field(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

/// Export the ledger as a multi-account QIF file.
///
/// QIF is per-register, so each asset (Bank) and liability (CCard) account
/// gets its own `!Account` section listing the transactions that touch it,
/// with amounts signed from that account's point of view (money in positive,
/// money out negative). The other side of each transaction becomes the
/// category (`L`), or a split (`S`/`$`) when there are several; other
/// registers are written as `[Account]` transfers.
pub fn export_qif(accounts: &[Account], transactions: &[ValidatedTransaction]) -> String {
    let by_id: HashMap<AccountId, &Account> = accounts
        .iter()
        .filter_map(|a| a.id.map(|id| (id, a)))
        .collect();
    let category = |id: AccountId| match by_id.get(&id) {
        Some(a) if qif_type(a.account_type).is_some() => format!("[{}]", field(&a.name)),
        Some(a) => field(&a.name),
        None => format!("Account {}", id.0),
    };
    // Net effect of a line on its own account: debits increase assets and
    // decrease liabilities, which is the sign QIF uses for both registers.
    let net = |debit: Decimal, credit: Decimal| debit - credit;

    let mut out = String::from("!Option:AutoSwitch\n");
    for acct in accounts {
        let (Some(_), Some(kind)) = (acct.id, qif_type(acct.account_type)) else {
            continue;
        };
        out.push_str(&format!("!Account\nN{}\nT{kind}\n^\n", field(&acct.name)));
    }
    out.push_str("!Clear:AutoSwitch\n");

    for acct in accounts {
        let (Some(id), Some(kind)) = (acct.id, qif_type(acct.account_type)) else {
            continue;
        };
        let touching: Vec<&ValidatedTransaction> = transactions
            .iter()
            .filter(|tx| tx.lines.iter().any(|l| l.account_id == id))
            .collect();
        if touching.is_empty() {
            continue;
        }

        out.push_str(&format!("!Account\nN{}\nT{kind}\n^\n", field(&acct.name)));
        out.push_str(&format!("!Type:{kind}\n"));
        for tx in touching {
            let (mine, others): (Vec<_>, Vec<_>) =
                tx.lines.iter().partition(|l| l.account_id == id);
            let amount: Decimal = mine
                .iter()
                .map(|l| net(l.debit.as_decimal(), l.credit.as_decimal()))
                .sum();

            out.push_str(&format!("D{}\n", tx.date.format("%m/%d/%Y")));
            out.push_str(&format!("T{amount:.2}\n"));
            out.push_str(&format!("P{}\n", field(&tx.description)));
            if let Some(memo) = &tx.memo {
                out.push_str(&format!("M{}\n", field(memo)));
            }
            match others.as_slice() {
                [] => {}
                [only] => out.push_str(&format!("L{}\n", category(only.account_id))),
                many => {
                    // Split amounts are from this account's perspective too, so
                    // they sum to the transaction amount.
                    for l in many {
                        let split = -net(l.debit.as_decimal(), l.credit.as_decimal());
                        out.push_str(&format!("S{}\n${split:.2}\n", category(l.account_id)));
                    }
                }
            }
            out.push_str("^\n");
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Money, TransactionLine, UnvalidatedTransaction};
    use chrono::NaiveDate;

    fn account(id: i64, name: &str, account_type: AccountType) -> Account {
        Account {
            id: Some(AccountId(id)),
            code: format!("{id}"),
            name: name.to_string(),
            account_type,
            is_archetype: false,
            is_archived: false,
            schedule_c_line: None,
        }
    }

    fn accounts() -> Vec<Account> {
        vec![
            account(1, "Checking", AccountType::Asset),
            account(2, "Credit Card", AccountType::Liability),
            account(3, "Meals", AccountType::Expense),
            account(4, "Services Revenue", AccountType::Income),
        ]
    }

    fn tx(lines: Vec<TransactionLine>, memo: Option<&str>) -> ValidatedTransaction {
        ValidatedTransaction::validate(UnvalidatedTransaction {
            date: NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
            description: "Coffee Shop".to_string(),
            lines,
            memo: memo.map(str::to_string),
        })
        .unwrap()
    }

    #[test]
    fn export_empty() {
        let out = export_qif(&accounts(), &[]);
        assert!(out.starts_with("!Option:AutoSwitch\n"));
        assert!(!out.contains("!Type:"));
    }

    #[test]
    fn outflow_is_negative_with_category() {
        let t = tx(
            vec![
                TransactionLine::debit(AccountId(3), Money::from_cents(500), None),
                TransactionLine::credit(AccountId(1), Money::from_cents(500), None),
            ],
            Some("Morning coffee"),
        );
        let out = export_qif(&accounts(), &[t]);
        assert!(out.contains("!Account\nNChecking\nTBank\n^\n!Type:Bank\n"));
        assert!(out.contains("D03/15/2026\nT-5.00\nPCoffee Shop\nMMorning coffee\nLMeals\n^\n"));
    }

    #[test]
    fn inflow_is_positive() {
        let t = tx(
            vec![
                TransactionLine::debit(AccountId(1), Money::from_cents(10000), None),
                TransactionLine::credit(AccountId(4), Money::from_cents(10000), None),
            ],
            None,
        );
        let out = export_qif(&accounts(), &[t]);
        assert!(out.contains("T100.00\n"));
        assert!(out.contains("LServices Revenue\n"));
    }

    #[test]
    fn card_charge_is_negative_and_transfers_are_bracketed() {
        let charge = tx(
            vec![
                TransactionLine::debit(AccountId(3), Money::from_cents(2000), None),
                TransactionLine::credit(AccountId(2), Money::from_cents(2000), None),
            ],
            None,
        );
        let payment = tx(
            vec![
                TransactionLine::debit(AccountId(2), Money::from_cents(2000), None),
                TransactionLine::credit(AccountId(1), Money::from_cents(2000), None),
            ],
            None,
        );
        let out = export_qif(&accounts(), &[charge, payment]);
        let card = &out[out.find("!Type:CCard").unwrap()..];
        assert!(card.contains("T-20.00\nPCoffee Shop\nLMeals\n"));
        assert!(card.contains("T20.00\nPCoffee Shop\nL[Checking]\n"));
    }

    #[test]
    fn multi_line_transactions_become_splits() {
        let t = tx(
            vec![
                TransactionLine::debit(AccountId(3), Money::from_cents(300), None),
                TransactionLine::debit(AccountId(4), Money::from_cents(200), None),
                TransactionLine::credit(AccountId(1), Money::from_cents(500), None),
            ],
            None,
        );
        let out = export_qif(&accounts(), &[t]);
        assert!(out.contains("T-5.00\n"));
        assert!(out.contains("SMeals\n$-3.00\nSServices Revenue\n$-2.00\n"));
    }

    #[test]
    fn qif_type_mapping() {
        assert_eq!(qif_type(AccountType::Asset), Some("Bank"));
        assert_eq!(qif_type(AccountType::Liability), Some("CCard"));
        assert_eq!(qif_type(AccountType::Expense), None);
    }
}
