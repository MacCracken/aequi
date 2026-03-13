use chrono::NaiveDate;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::str::FromStr;
use thiserror::Error;

/// A transaction imported from Wave Accounting CSV export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveTransaction {
    pub id: String,
    pub date: NaiveDate,
    pub account: String,
    pub transaction_type: String,
    pub amount_cents: i64,
    pub description: String,
    pub category: Option<String>,
    pub notes: Option<String>,
}

#[derive(Error, Debug)]
pub enum WaveImportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    #[error("Invalid date: {0}")]
    InvalidDate(String),
    #[error("Invalid amount: {0}")]
    InvalidAmount(String),
    #[error("Missing column: {0}")]
    MissingColumn(String),
    #[error("No data rows")]
    NoDataRows,
}

/// Summary statistics for a Wave import.
#[derive(Debug, Clone, Serialize)]
pub struct WaveImportSummary {
    pub account_count: usize,
    pub transaction_count: usize,
    pub earliest_date: Option<NaiveDate>,
    pub latest_date: Option<NaiveDate>,
}

/// Parse a Wave Accounting CSV export from the given reader.
///
/// Wave CSV columns: Transaction ID, Date, Account, Transaction Type, Amount,
/// Description, Category, Notes
pub fn parse_wave_csv<R: Read>(reader: R) -> Result<Vec<WaveTransaction>, WaveImportError> {
    let mut csv_reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_reader(reader);

    let headers = csv_reader
        .headers()
        .map_err(WaveImportError::Csv)?
        .clone();

    let col = |name: &str| -> Result<usize, WaveImportError> {
        headers
            .iter()
            .position(|h| h.eq_ignore_ascii_case(name))
            .ok_or_else(|| WaveImportError::MissingColumn(name.to_string()))
    };

    let id_col = col("Transaction ID")?;
    let date_col = col("Date")?;
    let account_col = col("Account")?;
    let type_col = col("Transaction Type")?;
    let amount_col = col("Amount")?;
    let desc_col = col("Description")?;
    let cat_col = col("Category").ok();
    let notes_col = col("Notes").ok();

    let mut transactions = Vec::new();

    for result in csv_reader.records() {
        let record = result?;

        if record.is_empty() {
            continue;
        }

        let id = record
            .get(id_col)
            .ok_or_else(|| WaveImportError::MissingColumn("Transaction ID".into()))?
            .to_string();

        let date_str = record
            .get(date_col)
            .ok_or_else(|| WaveImportError::MissingColumn("Date".into()))?
            .trim();
        let date = parse_wave_date(date_str)?;

        let account = record
            .get(account_col)
            .ok_or_else(|| WaveImportError::MissingColumn("Account".into()))?
            .to_string();

        let transaction_type = record
            .get(type_col)
            .ok_or_else(|| WaveImportError::MissingColumn("Transaction Type".into()))?
            .trim()
            .to_lowercase();

        let amount_str = record
            .get(amount_col)
            .ok_or_else(|| WaveImportError::MissingColumn("Amount".into()))?
            .trim();
        let amount_cents = parse_wave_amount(amount_str)?;

        let description = record
            .get(desc_col)
            .ok_or_else(|| WaveImportError::MissingColumn("Description".into()))?
            .to_string();

        let category = cat_col
            .and_then(|c| record.get(c))
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        let notes = notes_col
            .and_then(|c| record.get(c))
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        transactions.push(WaveTransaction {
            id,
            date,
            account,
            transaction_type,
            amount_cents,
            description,
            category,
            notes,
        });
    }

    if transactions.is_empty() {
        return Err(WaveImportError::NoDataRows);
    }

    Ok(transactions)
}

/// Produce a summary of imported Wave transactions.
pub fn summarize(transactions: &[WaveTransaction]) -> WaveImportSummary {
    let mut accounts = std::collections::HashSet::new();
    let mut earliest: Option<NaiveDate> = None;
    let mut latest: Option<NaiveDate> = None;

    for tx in transactions {
        accounts.insert(&tx.account);
        earliest = Some(match earliest {
            Some(e) if e <= tx.date => e,
            _ => tx.date,
        });
        latest = Some(match latest {
            Some(l) if l >= tx.date => l,
            _ => tx.date,
        });
    }

    WaveImportSummary {
        account_count: accounts.len(),
        transaction_count: transactions.len(),
        earliest_date: earliest,
        latest_date: latest,
    }
}

fn parse_wave_date(s: &str) -> Result<NaiveDate, WaveImportError> {
    let s = s.trim();
    // Wave uses MM/DD/YYYY
    NaiveDate::parse_from_str(s, "%m/%d/%Y")
        .or_else(|_| NaiveDate::parse_from_str(s, "%Y-%m-%d"))
        .map_err(|_| WaveImportError::InvalidDate(s.to_string()))
}

fn parse_wave_amount(s: &str) -> Result<i64, WaveImportError> {
    // Strip all non-numeric characters except '.', '-', and digits
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    if cleaned.is_empty() {
        return Err(WaveImportError::InvalidAmount(s.trim().to_string()));
    }
    let dec = Decimal::from_str(&cleaned)
        .map_err(|_| WaveImportError::InvalidAmount(s.trim().to_string()))?;
    let cents = (dec * Decimal::from(100))
        .round()
        .to_i64()
        .ok_or_else(|| WaveImportError::InvalidAmount(s.trim().to_string()))?;
    Ok(cents)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave_csv(rows: &str) -> String {
        format!(
            "Transaction ID,Date,Account,Transaction Type,Amount,Description,Category,Notes\n{}",
            rows
        )
    }

    #[test]
    fn parse_basic() {
        let data = wave_csv(
            "1,01/15/2024,Checking,debit,50.00,Office Supplies,Expenses,Staples run\n\
             2,01/16/2024,Checking,credit,1200.00,Client Payment,Income,Invoice #42\n",
        );
        let txs = parse_wave_csv(data.as_bytes()).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].id, "1");
        assert_eq!(txs[0].account, "Checking");
        assert_eq!(txs[0].description, "Office Supplies");
        assert_eq!(txs[1].id, "2");
        assert_eq!(txs[1].transaction_type, "credit");
    }

    #[test]
    fn amount_conversion() {
        let data = wave_csv("1,01/01/2024,Acct,debit,1234.56,Test,Cat,\n");
        let txs = parse_wave_csv(data.as_bytes()).unwrap();
        assert_eq!(txs[0].amount_cents, 123456);
    }

    #[test]
    fn amount_with_dollar_sign_and_commas() {
        let data = wave_csv("1,01/01/2024,Acct,debit,\"$1,234.56\",Test,Cat,\n");
        let txs = parse_wave_csv(data.as_bytes()).unwrap();
        assert_eq!(txs[0].amount_cents, 123456);
    }

    #[test]
    fn date_parsing_mm_dd_yyyy() {
        let data = wave_csv("1,12/31/2024,Acct,debit,10.00,Test,,\n");
        let txs = parse_wave_csv(data.as_bytes()).unwrap();
        assert_eq!(txs[0].date, NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
    }

    #[test]
    fn empty_input_errors() {
        let data = "Transaction ID,Date,Account,Transaction Type,Amount,Description,Category,Notes\n";
        let result = parse_wave_csv(data.as_bytes());
        assert!(matches!(result, Err(WaveImportError::NoDataRows)));
    }

    #[test]
    fn debit_and_credit_types_preserved() {
        let data = wave_csv(
            "1,01/01/2024,Checking,Debit,100.00,Purchase,,\n\
             2,01/02/2024,Checking,Credit,200.00,Refund,,\n",
        );
        let txs = parse_wave_csv(data.as_bytes()).unwrap();
        assert_eq!(txs[0].transaction_type, "debit");
        assert_eq!(txs[1].transaction_type, "credit");
    }

    #[test]
    fn summary_statistics() {
        let data = wave_csv(
            "1,01/10/2024,Checking,debit,50.00,A,,\n\
             2,01/05/2024,Savings,credit,100.00,B,,\n\
             3,01/20/2024,Checking,debit,75.00,C,,\n",
        );
        let txs = parse_wave_csv(data.as_bytes()).unwrap();
        let summary = summarize(&txs);
        assert_eq!(summary.transaction_count, 3);
        assert_eq!(summary.account_count, 2);
        assert_eq!(
            summary.earliest_date,
            Some(NaiveDate::from_ymd_opt(2024, 1, 5).unwrap())
        );
        assert_eq!(
            summary.latest_date,
            Some(NaiveDate::from_ymd_opt(2024, 1, 20).unwrap())
        );
    }

    #[test]
    fn optional_fields_none_when_empty() {
        let data = wave_csv("1,01/01/2024,Acct,debit,10.00,Desc,,\n");
        let txs = parse_wave_csv(data.as_bytes()).unwrap();
        assert!(txs[0].category.is_none());
        assert!(txs[0].notes.is_none());
    }

    #[test]
    fn negative_amount() {
        let data = wave_csv("1,01/01/2024,Acct,credit,-99.99,Refund,,\n");
        let txs = parse_wave_csv(data.as_bytes()).unwrap();
        assert_eq!(txs[0].amount_cents, -9999);
    }
}
