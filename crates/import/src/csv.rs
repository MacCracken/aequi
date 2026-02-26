use chrono::NaiveDate;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvColumnMapping {
    pub date_column: Option<usize>,
    pub description_column: Option<usize>,
    pub amount_column: Option<usize>,
    pub debit_column: Option<usize>,
    pub credit_column: Option<usize>,
    pub memo_column: Option<usize>,
    pub date_format: String,
}

impl Default for CsvColumnMapping {
    fn default() -> Self {
        Self {
            date_column: None,
            description_column: None,
            amount_column: None,
            debit_column: None,
            credit_column: None,
            memo_column: None,
            date_format: "%Y-%m-%d".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvImportProfile {
    pub id: Option<i64>,
    pub name: String,
    pub mapping: CsvColumnMapping,
    pub has_header: bool,
    pub delimiter: String,
}

impl Default for CsvImportProfile {
    fn default() -> Self {
        Self {
            id: None,
            name: "Unnamed Profile".to_string(),
            mapping: CsvColumnMapping::default(),
            has_header: true,
            delimiter: ",".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CsvTransaction {
    pub date: NaiveDate,
    pub description: String,
    pub amount: i64,
    pub memo: Option<String>,
    pub debit: Option<i64>,
    pub credit: Option<i64>,
}

#[derive(Error, Debug)]
pub enum CsvError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("CSV error: {0}")]
    CsvError(#[from] csv::Error),
    #[error("Missing required column: {0}")]
    MissingColumn(String),
    #[error("Invalid date format: {0}")]
    InvalidDate(String),
    #[error("Invalid amount: {0}")]
    InvalidAmount(String),
    #[error("No data rows")]
    NoDataRows,
}

pub struct CsvImporter;

impl CsvImporter {
    pub fn parse_profile<R: Read>(
        reader: &mut csv::Reader<R>,
        profile: &CsvImportProfile,
    ) -> Result<Vec<CsvTransaction>, CsvError> {
        let mut transactions = Vec::new();
        let mapping = &profile.mapping;

        for result in reader.records() {
            let record = result?;

            if record.is_empty() {
                continue;
            }

            let date = if let Some(col) = mapping.date_column {
                let field = record
                    .get(col)
                    .ok_or_else(|| CsvError::MissingColumn(format!("date_column {}", col)))?;
                parse_date(field, &mapping.date_format)?
            } else {
                continue;
            };

            let description = if let Some(col) = mapping.description_column {
                record.get(col).unwrap_or_default().to_string()
            } else {
                String::new()
            };

            let (amount, debit, credit) = if let Some(col) = mapping.amount_column {
                let field = record.get(col).unwrap_or_default();
                let amt = parse_amount(field)?;
                (amt, None, None)
            } else if let (Some(d_col), Some(c_col)) = (mapping.debit_column, mapping.credit_column)
            {
                let d = record
                    .get(d_col)
                    .filter(|s| !s.trim().is_empty())
                    .map(parse_amount)
                    .transpose()?;
                let c = record
                    .get(c_col)
                    .filter(|s| !s.trim().is_empty())
                    .map(parse_amount)
                    .transpose()?;
                let amt = match (d, c) {
                    (Some(d), None) => d,
                    (None, Some(c)) => -c,
                    (None, None) => 0,
                    _ => 0,
                };
                (amt, d, c)
            } else {
                continue;
            };

            let memo = mapping
                .memo_column
                .and_then(|col| record.get(col))
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());

            transactions.push(CsvTransaction {
                date,
                description,
                amount,
                memo,
                debit,
                credit,
            });
        }

        if transactions.is_empty() {
            return Err(CsvError::NoDataRows);
        }

        Ok(transactions)
    }

    pub fn detect_columns<R: Read>(reader: &mut csv::Reader<R>) -> Result<Vec<String>, CsvError> {
        let mut headers = Vec::new();

        if let Some(result) = reader.records().next() {
            let record = result?;
            headers = record.iter().map(|s| s.to_string()).collect();
        }

        Ok(headers)
    }
}

fn parse_date(s: &str, format: &str) -> Result<NaiveDate, CsvError> {
    let s = s.trim();

    if let Ok(date) = NaiveDate::parse_from_str(s, format) {
        return Ok(date);
    }

    for fmt in &[
        "%m/%d/%Y", "%d/%m/%Y", "%Y/%m/%d", "%m-%d-%Y", "%d-%m-%Y", "%Y-%m-%d",
    ] {
        if let Ok(date) = NaiveDate::parse_from_str(s, fmt) {
            return Ok(date);
        }
    }

    Err(CsvError::InvalidDate(s.to_string()))
}

fn parse_amount(s: &str) -> Result<i64, CsvError> {
    let s = s.trim();
    let (negative, s) = if s.starts_with('(') && s.ends_with(')') {
        (true, &s[1..s.len() - 1])
    } else {
        (false, s)
    };
    let s = s.replace([',', '$', ' '], "");
    let mut dec = Decimal::from_str(&s)
        .map_err(|_| CsvError::InvalidAmount(s.to_string()))?;
    if negative {
        dec = -dec;
    }
    let cents = (dec * Decimal::from(100))
        .round()
        .to_i64()
        .ok_or_else(|| CsvError::InvalidAmount(s.to_string()))?;
    Ok(cents)
}

pub fn parse<R: Read>(
    reader: &mut csv::Reader<R>,
    profile: &CsvImportProfile,
) -> Result<Vec<CsvTransaction>, CsvError> {
    CsvImporter::parse_profile(reader, profile)
}

pub fn import_csv<R: Read>(
    data: R,
    profile: &CsvImportProfile,
) -> Result<Vec<CsvTransaction>, CsvError> {
    let delimiter = profile
        .delimiter
        .as_bytes()
        .first()
        .copied()
        .unwrap_or(b',');
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(profile.has_header)
        .delimiter(delimiter)
        .from_reader(data);

    parse(&mut reader, profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_amount ──────────────────────────────────────────────────────────

    #[test]
    fn parse_amount_plain() {
        assert_eq!(parse_amount("123.45").unwrap(), 12345);
    }

    #[test]
    fn parse_amount_with_dollar_sign() {
        assert_eq!(parse_amount("$99.99").unwrap(), 9999);
    }

    #[test]
    fn parse_amount_with_commas() {
        assert_eq!(parse_amount("1,234.56").unwrap(), 123456);
    }

    #[test]
    fn parse_amount_negative() {
        assert_eq!(parse_amount("-50.00").unwrap(), -5000);
    }

    #[test]
    fn parse_amount_accounting_parens() {
        assert_eq!(parse_amount("(75.25)").unwrap(), -7525);
    }

    #[test]
    fn parse_amount_zero() {
        assert_eq!(parse_amount("0.00").unwrap(), 0);
        assert_eq!(parse_amount("0").unwrap(), 0);
    }

    #[test]
    fn parse_amount_whole_number() {
        assert_eq!(parse_amount("100").unwrap(), 10000);
    }

    #[test]
    fn parse_amount_single_cent() {
        assert_eq!(parse_amount("0.01").unwrap(), 1);
    }

    #[test]
    fn parse_amount_invalid() {
        assert!(parse_amount("not_a_number").is_err());
        assert!(parse_amount("").is_err());
    }

    // ── parse_date ────────────────────────────────────────────────────────────

    #[test]
    fn parse_date_iso() {
        let d = parse_date("2024-01-15", "%Y-%m-%d").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());
    }

    #[test]
    fn parse_date_us_slash() {
        let d = parse_date("01/15/2024", "%Y-%m-%d").unwrap(); // fallback
        assert_eq!(d, NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());
    }

    #[test]
    fn parse_date_invalid() {
        assert!(parse_date("not-a-date", "%Y-%m-%d").is_err());
    }

    // ── full import round-trip ────────────────────────────────────────────────

    fn default_profile() -> CsvImportProfile {
        CsvImportProfile {
            id: None,
            name: "test".to_string(),
            has_header: true,
            delimiter: ",".to_string(),
            mapping: CsvColumnMapping {
                date_column: Some(0),
                description_column: Some(1),
                amount_column: Some(2),
                debit_column: None,
                credit_column: None,
                memo_column: None,
                date_format: "%Y-%m-%d".to_string(),
            },
        }
    }

    #[test]
    fn import_csv_basic() {
        let data = b"date,description,amount\n2024-01-15,AMAZON,49.99\n2024-01-16,STARBUCKS,-5.00\n";
        let txs = import_csv(data.as_ref(), &default_profile()).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].amount, 4999);
        assert_eq!(txs[0].description, "AMAZON");
        assert_eq!(txs[1].amount, -500);
    }

    #[test]
    fn import_csv_debit_credit_columns() {
        let data =
            b"date,description,debit,credit\n2024-01-15,PAYMENT,,100.00\n2024-01-16,CHARGE,50.00,\n";
        let profile = CsvImportProfile {
            mapping: CsvColumnMapping {
                date_column: Some(0),
                description_column: Some(1),
                amount_column: None,
                debit_column: Some(2),
                credit_column: Some(3),
                memo_column: None,
                date_format: "%Y-%m-%d".to_string(),
            },
            ..default_profile()
        };
        let txs = import_csv(data.as_ref(), &profile).unwrap();
        assert_eq!(txs.len(), 2);
        // Credit of 100 → amount = -100 (inflow from bank perspective)
        assert_eq!(txs[0].amount, -10000);
        // Debit of 50 → amount = 50
        assert_eq!(txs[1].amount, 5000);
    }

    #[test]
    fn import_csv_no_data_rows_errors() {
        let data = b"date,description,amount\n";
        let result = import_csv(data.as_ref(), &default_profile());
        assert!(matches!(result, Err(CsvError::NoDataRows)));
    }
}
