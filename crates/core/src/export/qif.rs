use crate::{AccountType, ValidatedTransaction};

/// Map account type to QIF account type header.
fn qif_type(account_type: AccountType) -> &'static str {
    match account_type {
        AccountType::Asset => "Bank",
        AccountType::Liability => "CCard",
        AccountType::Equity => "Oth A",
        AccountType::Income => "Oth A",
        AccountType::Expense => "Oth A",
    }
}

/// Export transactions to QIF format for a given account type.
/// QIF is a per-account format, so the caller specifies which account type to export.
pub fn export_qif(transactions: &[ValidatedTransaction], account_type: AccountType) -> String {
    let mut out = String::new();

    out.push_str(&format!("!Type:{}\n", qif_type(account_type)));

    for tx in transactions {
        // Date in MM/DD/YYYY format (QIF standard)
        out.push_str(&format!(
            "D{}/{}/{}\n",
            tx.date.format("%m"),
            tx.date.format("%d"),
            tx.date.format("%Y")
        ));

        // Total amount (from the perspective of this account type)
        let amount = tx.balanced_total.to_cents() as f64 / 100.0;
        out.push_str(&format!("T{amount:.2}\n"));

        // Payee / description
        out.push_str(&format!("P{}\n", tx.description));

        // Memo
        if let Some(memo) = &tx.memo {
            out.push_str(&format!("M{memo}\n"));
        }

        // End of record
        out.push_str("^\n");
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountId, Money, TransactionLine, UnvalidatedTransaction};
    use chrono::NaiveDate;

    #[test]
    fn export_empty() {
        let out = export_qif(&[], AccountType::Asset);
        assert_eq!(out, "!Type:Bank\n");
    }

    #[test]
    fn export_single_transaction() {
        let tx = UnvalidatedTransaction {
            date: NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
            description: "Coffee Shop".to_string(),
            lines: vec![
                TransactionLine::debit(AccountId(1), Money::from_cents(500), None),
                TransactionLine::credit(AccountId(2), Money::from_cents(500), None),
            ],
            memo: Some("Morning coffee".to_string()),
        };
        let validated = ValidatedTransaction::validate(tx).unwrap();
        let out = export_qif(&[validated], AccountType::Asset);

        assert!(out.contains("!Type:Bank"));
        assert!(out.contains("D03/15/2026"));
        assert!(out.contains("T5.00"));
        assert!(out.contains("PCoffee Shop"));
        assert!(out.contains("MMorning coffee"));
        assert!(out.contains("^"));
    }

    #[test]
    fn qif_type_mapping() {
        assert_eq!(qif_type(AccountType::Asset), "Bank");
        assert_eq!(qif_type(AccountType::Liability), "CCard");
    }
}
