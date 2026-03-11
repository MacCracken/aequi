use crate::{Account, AccountType, ValidatedTransaction};

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

/// Sanitize an account name for Beancount (must be CamelCase, no spaces).
fn sanitize_name(name: &str) -> String {
    name.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => {
                    let mut s = c.to_uppercase().to_string();
                    s.extend(chars);
                    // Remove non-alphanumeric characters
                    s.retain(|c| c.is_alphanumeric());
                    s
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Export accounts and transactions to Beancount plain-text format.
pub fn export_beancount(accounts: &[Account], transactions: &[ValidatedTransaction]) -> String {
    let mut out = String::new();

    out.push_str("; Exported from Aequi\n");
    out.push_str("option \"operating_currency\" \"USD\"\n\n");

    // Account declarations
    for acct in accounts {
        let bc_type = beancount_type(acct.account_type);
        let name = sanitize_name(&acct.name);
        out.push_str(&format!("1970-01-01 open {bc_type}:{name}\n"));
    }

    if !accounts.is_empty() && !transactions.is_empty() {
        out.push('\n');
    }

    // Transactions
    for tx in transactions {
        out.push_str(&format!(
            "{} * \"{}\"\n",
            tx.date,
            tx.description.replace('"', "\\\"")
        ));
        if let Some(memo) = &tx.memo {
            out.push_str(&format!("  ; {memo}\n"));
        }
        for line in &tx.lines {
            // Find the account name
            let acct_name = accounts
                .iter()
                .find(|a| a.id == Some(line.account_id))
                .map(|a| {
                    let bc_type = beancount_type(a.account_type);
                    let name = sanitize_name(&a.name);
                    format!("{bc_type}:{name}")
                })
                .unwrap_or_else(|| format!("Unknown:Account{}", line.account_id.0));

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
}
