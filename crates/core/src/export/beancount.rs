use std::collections::{HashMap, HashSet};

use crate::{Account, AccountId, AccountType, ValidatedTransaction};

/// Map aequi account type to Beancount account type prefix.
fn beancount_type(account_type: AccountType) -> &'static str {
    match account_type {
        AccountType::Asset => "Assets",
        AccountType::Liability => "Liabilities",
        AccountType::Equity => "Equity",
        AccountType::Income => "Income",
        AccountType::Expense => "Expenses",
    }
}

/// Sanitize an account name into a Beancount account component: CamelCase,
/// alphanumeric only, starting with an uppercase letter or a digit.
fn sanitize_name(name: &str) -> String {
    name.split_whitespace()
        .map(|word| {
            let cleaned: String = word.chars().filter(|c| c.is_alphanumeric()).collect();
            let mut chars = cleaned.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Escape text for a Beancount double-quoted string (single line).
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Assign every account a valid, unique Beancount name.
fn account_names(accounts: &[Account]) -> HashMap<AccountId, String> {
    let mut names = HashMap::new();
    let mut used = HashSet::new();
    for acct in accounts {
        let Some(id) = acct.id else { continue };
        let mut component = sanitize_name(&acct.name);
        let valid_start = component
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit());
        if !valid_start {
            component = format!("Account{}", sanitize_name(&acct.code));
        }
        let mut name = format!("{}:{component}", beancount_type(acct.account_type));
        if !used.insert(name.clone()) {
            // Two accounts sanitize to the same name; disambiguate by id.
            name = format!("{name}-{}", id.0);
            used.insert(name.clone());
        }
        names.insert(id, name);
    }
    names
}

/// Export accounts and transactions to Beancount plain-text format.
///
/// `accounts` should include archived accounts, since historical transactions
/// may post to them. Any account still missing is declared under `Equity:` so
/// the file always parses.
pub fn export_beancount(accounts: &[Account], transactions: &[ValidatedTransaction]) -> String {
    let mut out = String::new();

    out.push_str("; Exported from Aequi\n");
    out.push_str("option \"operating_currency\" \"USD\"\n\n");

    let mut names = account_names(accounts);
    let mut opens: Vec<String> = accounts
        .iter()
        .filter_map(|a| a.id.and_then(|id| names.get(&id).cloned()))
        .collect();
    for tx in transactions {
        for line in &tx.lines {
            names.entry(line.account_id).or_insert_with(|| {
                let name = format!("Equity:UnknownAccount{}", line.account_id.0);
                opens.push(name.clone());
                name
            });
        }
    }

    // Account declarations
    for name in &opens {
        out.push_str(&format!("1970-01-01 open {name}\n"));
    }

    if !opens.is_empty() && !transactions.is_empty() {
        out.push('\n');
    }

    // Transactions
    for tx in transactions {
        out.push_str(&format!("{} * {}\n", tx.date, quote(&tx.description)));
        if let Some(memo) = &tx.memo {
            out.push_str(&format!("  ; {}\n", memo.replace(['\n', '\r'], " ")));
        }
        for line in &tx.lines {
            let acct_name = &names[&line.account_id];
            if !line.debit.is_zero() {
                out.push_str(&format!(
                    "  {acct_name}  {:.2} USD\n",
                    line.debit.as_decimal()
                ));
            }
            if !line.credit.is_zero() {
                out.push_str(&format!(
                    "  {acct_name}  -{:.2} USD\n",
                    line.credit.as_decimal()
                ));
            }
        }
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountId, Money, TransactionLine, UnvalidatedTransaction};
    use chrono::NaiveDate;

    fn test_accounts() -> Vec<Account> {
        vec![
            Account {
                id: Some(AccountId(1)),
                code: "1000".to_string(),
                name: "Checking".to_string(),
                account_type: AccountType::Asset,
                is_archetype: false,
                is_archived: false,
                schedule_c_line: None,
            },
            Account {
                id: Some(AccountId(2)),
                code: "5000".to_string(),
                name: "Office Supplies".to_string(),
                account_type: AccountType::Expense,
                is_archetype: false,
                is_archived: false,
                schedule_c_line: None,
            },
        ]
    }

    #[test]
    fn export_empty() {
        let out = export_beancount(&[], &[]);
        assert!(out.contains("operating_currency"));
    }

    #[test]
    fn export_accounts_only() {
        let out = export_beancount(&test_accounts(), &[]);
        assert!(out.contains("Assets:Checking"));
        assert!(out.contains("Expenses:OfficeSupplies"));
    }

    #[test]
    fn export_with_transaction() {
        let accounts = test_accounts();
        let tx = UnvalidatedTransaction {
            date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            description: "Bought pens".to_string(),
            lines: vec![
                TransactionLine::debit(AccountId(2), Money::from_cents(1500), None),
                TransactionLine::credit(AccountId(1), Money::from_cents(1500), None),
            ],
            memo: None,
        };
        let validated = ValidatedTransaction::validate(tx).unwrap();
        let out = export_beancount(&accounts, &[validated]);
        assert!(out.contains("2026-03-01 * \"Bought pens\""));
        assert!(out.contains("Expenses:OfficeSupplies  15.00 USD"));
        assert!(out.contains("Assets:Checking  -15.00 USD"));
    }

    #[test]
    fn sanitize_name_handles_spaces_and_special_chars() {
        assert_eq!(sanitize_name("Office Supplies"), "OfficeSupplies");
        assert_eq!(
            sanitize_name("Advertising & Marketing"),
            "AdvertisingMarketing"
        );
        assert_eq!(
            sanitize_name("Business Meals (50% deductible)"),
            "BusinessMeals50Deductible"
        );
    }

    #[test]
    fn descriptions_and_memos_are_escaped() {
        let accounts = test_accounts();
        let tx = ValidatedTransaction::validate(crate::UnvalidatedTransaction {
            date: chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            description: "C:\\temp \"quoted\"\nsecond line".to_string(),
            lines: vec![
                crate::TransactionLine::debit(
                    crate::AccountId(2),
                    crate::Money::from_cents(100),
                    None,
                ),
                crate::TransactionLine::credit(
                    crate::AccountId(1),
                    crate::Money::from_cents(100),
                    None,
                ),
            ],
            memo: Some("multi\nline".to_string()),
        })
        .unwrap();
        let out = export_beancount(&accounts, &[tx]);
        assert!(out.contains("2026-03-01 * \"C:\\\\temp \\\"quoted\\\" second line\"\n"));
        assert!(out.contains("  ; multi line\n"));
    }

    #[test]
    fn missing_accounts_are_declared() {
        let tx = ValidatedTransaction::validate(crate::UnvalidatedTransaction {
            date: chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            description: "x".to_string(),
            lines: vec![
                crate::TransactionLine::debit(
                    crate::AccountId(77),
                    crate::Money::from_cents(100),
                    None,
                ),
                crate::TransactionLine::credit(
                    crate::AccountId(1),
                    crate::Money::from_cents(100),
                    None,
                ),
            ],
            memo: None,
        })
        .unwrap();
        let out = export_beancount(&test_accounts(), &[tx]);
        assert!(out.contains("1970-01-01 open Equity:UnknownAccount77\n"));
        assert!(out.contains("  Equity:UnknownAccount77  1.00 USD\n"));
    }

    #[test]
    fn account_names_are_valid_and_unique() {
        let mk = |id, name: &str| Account {
            id: Some(crate::AccountId(id)),
            code: format!("{id}"),
            name: name.to_string(),
            account_type: AccountType::Asset,
            is_archetype: false,
            is_archived: false,
            schedule_c_line: None,
        };
        let names = account_names(&[mk(1, "(old) checking"), mk(2, "Old Checking"), mk(3, "!!!")]);
        assert_eq!(names[&crate::AccountId(1)], "Assets:OldChecking");
        assert_eq!(names[&crate::AccountId(2)], "Assets:OldChecking-2");
        assert_eq!(names[&crate::AccountId(3)], "Assets:Account3");
    }
}
