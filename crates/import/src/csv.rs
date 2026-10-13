use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::io::Read;
use thiserror::Error;

use crate::util::parse_amount_cents;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvColumnMapping {
    pub date_column: Option<usize>,
    pub description_column: Option<usize>,
    /// Single signed amount column. Takes precedence over debit/credit columns.
    pub amount_column: Option<usize>,
    /// Money-out column (withdrawals). Imported as a negative amount.
    pub debit_column: Option<usize>,
    /// Money-in column (deposits). Imported as a positive amount.
    pub credit_column: Option<usize>,
    pub memo_column: Option<usize>,
    /// chrono format applied to every row; rows that do not match are errors.
    pub date_format: String,
    /// Flip the sign of `amount_column` for banks that export money out as
    /// positive. Ignored for debit/credit columns, whose direction is fixed.
    #[serde(default)]
    pub invert_sign: bool,
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
            invert_sign: false,
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
    /// Signed cents: negative = money out of the account, positive = money in.
    pub amount: i64,
    pub memo: Option<String>,
    /// Magnitude read from the debit column, when the profile uses one.
    pub debit: Option<i64>,
    /// Magnitude read from the credit column, when the profile uses one.
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
    #[error("Too many rows (max {0})")]
    TooManyRows(usize),
}

pub struct CsvImporter;

impl CsvImporter {
    pub fn parse_profile<R: Read>(
        reader: &mut csv::Reader<R>,
        profile: &CsvImportProfile,
    ) -> Result<Vec<CsvTransaction>, CsvError> {
        let mut transactions = Vec::new();
        const MAX_CSV_RECORDS: usize = 100_000;
        let mapping = &profile.mapping;

        let date_col = mapping
            .date_column
            .ok_or_else(|| CsvError::MissingColumn("date_column".to_string()))?;
        let amount_source = match (
            mapping.amount_column,
            mapping.debit_column,
            mapping.credit_column,
        ) {
            (Some(col), _, _) => AmountSource::Single(col),
            (None, Some(debit), Some(credit)) => AmountSource::DebitCredit { debit, credit },
            _ => {
                return Err(CsvError::MissingColumn(
                    "amount_column (or both debit_column and credit_column)".to_string(),
                ));
            }
        };

        for (index, result) in reader.records().enumerate() {
            let record = result?;

            if record.iter().all(|field| field.trim().is_empty()) {
                continue;
            }
            if transactions.len() >= MAX_CSV_RECORDS {
                return Err(CsvError::TooManyRows(MAX_CSV_RECORDS));
            }
            let row = record.position().map_or(index as u64 + 1, |p| p.line());

            let date_field = record.get(date_col).ok_or_else(|| {
                CsvError::MissingColumn(format!("row {row}: date_column {date_col}"))
            })?;
            let date = parse_date(date_field, &mapping.date_format)
                .map_err(|reason| CsvError::InvalidDate(format!("row {row}: {reason}")))?;

            let description = if let Some(col) = mapping.description_column {
                record.get(col).unwrap_or_default().to_string()
            } else {
                String::new()
            };

            let invalid_amount =
                |field: &str| CsvError::InvalidAmount(format!("row {row}: '{}'", field.trim()));

            let (amount, debit, credit) = match amount_source {
                AmountSource::Single(col) => {
                    let field = record.get(col).ok_or_else(|| {
                        CsvError::MissingColumn(format!("row {row}: amount_column {col}"))
                    })?;
                    let amt = parse_amount(field).map_err(|_| invalid_amount(field))?;
                    let amt = if mapping.invert_sign {
                        amt.checked_neg().ok_or_else(|| invalid_amount(field))?
                    } else {
                        amt
                    };
                    (amt, None, None)
                }
                AmountSource::DebitCredit { debit, credit } => {
                    // The column, not the sign, says which way the money moved:
                    // some banks write withdrawals as "-50.00" in the debit column.
                    let magnitude = |col: usize| -> Result<Option<i64>, CsvError> {
                        match record.get(col).filter(|s| !s.trim().is_empty()) {
                            Some(field) => parse_amount(field)
                                .ok()
                                .and_then(i64::checked_abs)
                                .map(Some)
                                .ok_or_else(|| invalid_amount(field)),
                            None => Ok(None),
                        }
                    };
                    let d = magnitude(debit)?;
                    let c = magnitude(credit)?;
                    if d.is_none() && c.is_none() {
                        return Err(CsvError::InvalidAmount(format!(
                            "row {row}: both debit and credit columns are empty"
                        )));
                    }
                    // Banks that fill the unused column with "0.00" still net correctly.
                    (c.unwrap_or(0) - d.unwrap_or(0), d, c)
                }
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

#[derive(Clone, Copy)]
enum AmountSource {
    Single(usize),
    DebitCredit { debit: usize, credit: usize },
}

/// Parse with the profile's format only. Falling back to other formats per
/// row would silently mix DD/MM and MM/DD within one file, so a mismatch is
/// reported instead. Years before 1900 are rejected because chrono's `%Y`
/// accepts one to four digits ("01/15/24" → year 24).
fn parse_date(s: &str, format: &str) -> Result<NaiveDate, String> {
    let s = s.trim();
    let date = NaiveDate::parse_from_str(s, format)
        .map_err(|_| format!("'{s}' does not match date format '{format}'"))?;
    if !(1900..=9999).contains(&date.year()) {
        return Err(format!(
            "'{s}' parsed as year {} with format '{format}' (use %y for two-digit years)",
            date.year()
        ));
    }
    Ok(date)
}

fn parse_amount(s: &str) -> Result<i64, CsvError> {
    parse_amount_cents(s).ok_or_else(|| CsvError::InvalidAmount(s.trim().to_string()))
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
        assert!(parse_amount("12abc").is_err());
    }

    #[test]
    fn parse_amount_decimal_comma() {
        assert_eq!(parse_amount("-49,99").unwrap(), -4999);
    }

    // ── parse_date ────────────────────────────────────────────────────────────

    #[test]
    fn parse_date_iso() {
        let d = parse_date("2024-01-15", "%Y-%m-%d").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());
    }

    #[test]
    fn parse_date_us_slash() {
        let d = parse_date("01/15/2024", "%m/%d/%Y").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());
    }

    #[test]
    fn parse_date_no_silent_fallback_to_other_formats() {
        // Used to fall back to %m/%d/%Y; a mismatch must now be reported.
        assert!(parse_date("01/15/2024", "%Y-%m-%d").is_err());
        // ...and must not reinterpret a DD/MM date under an MM/DD profile.
        assert!(parse_date("15/01/2024", "%m/%d/%Y").is_err());
    }

    #[test]
    fn parse_date_rejects_two_digit_year_under_four_digit_format() {
        // chrono's %Y accepts "24" as year 0024.
        let err = parse_date("01/15/24", "%m/%d/%Y").unwrap_err();
        assert!(err.contains("year 24"), "{err}");
        let d = parse_date("01/15/24", "%m/%d/%y").unwrap();
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
                invert_sign: false,
            },
        }
    }

    #[test]
    fn import_csv_basic() {
        let data =
            b"date,description,amount\n2024-01-15,AMAZON,49.99\n2024-01-16,STARBUCKS,-5.00\n";
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
                invert_sign: false,
            },
            ..default_profile()
        };
        let txs = import_csv(data.as_ref(), &profile).unwrap();
        assert_eq!(txs.len(), 2);
        // Credit (deposit) of 100 → money in → positive
        assert_eq!(txs[0].amount, 10000);
        assert_eq!(txs[0].credit, Some(10000));
        assert_eq!(txs[0].debit, None);
        // Debit (withdrawal) of 50 → money out → negative
        assert_eq!(txs[1].amount, -5000);
        assert_eq!(txs[1].debit, Some(5000));
    }

    fn debit_credit_profile() -> CsvImportProfile {
        CsvImportProfile {
            mapping: CsvColumnMapping {
                date_column: Some(0),
                description_column: Some(1),
                debit_column: Some(2),
                credit_column: Some(3),
                ..CsvColumnMapping::default()
            },
            ..default_profile()
        }
    }

    #[test]
    fn import_csv_debit_credit_zero_in_unused_column() {
        // Banks that write "0.00" in the unused column used to import as 0.
        let data = b"date,description,debit,credit\n\
            2024-01-15,PAYMENT,0.00,100.00\n\
            2024-01-16,CHARGE,50.00,0.00\n";
        let txs = import_csv(data.as_ref(), &debit_credit_profile()).unwrap();
        assert_eq!(txs[0].amount, 10000);
        assert_eq!(txs[1].amount, -5000);
    }

    #[test]
    fn import_csv_debit_column_with_negative_sign() {
        // The column decides direction, whatever sign the bank wrote.
        let data = b"date,description,debit,credit\n2024-01-16,CHARGE,-50.00,\n";
        let txs = import_csv(data.as_ref(), &debit_credit_profile()).unwrap();
        assert_eq!(txs[0].amount, -5000);
        assert_eq!(txs[0].debit, Some(5000));
    }

    #[test]
    fn import_csv_debit_credit_both_empty_errors() {
        let data = b"date,description,debit,credit\n2024-01-16,NOTHING,,\n";
        let err = import_csv(data.as_ref(), &debit_credit_profile()).unwrap_err();
        assert!(matches!(err, CsvError::InvalidAmount(ref m) if m.contains("row 2")));
    }

    #[test]
    fn import_csv_single_and_split_columns_agree_on_sign() {
        let single =
            b"date,description,amount\n2024-01-16,CHARGE,-50.00\n2024-01-17,REFUND,20.00\n";
        let split =
            b"date,description,debit,credit\n2024-01-16,CHARGE,50.00,\n2024-01-17,REFUND,,20.00\n";
        let a = import_csv(single.as_ref(), &default_profile()).unwrap();
        let b = import_csv(split.as_ref(), &debit_credit_profile()).unwrap();
        assert_eq!(
            a.iter().map(|t| t.amount).collect::<Vec<_>>(),
            b.iter().map(|t| t.amount).collect::<Vec<_>>()
        );
    }

    #[test]
    fn import_csv_invert_sign() {
        let data = b"date,description,amount\n2024-01-16,CHARGE,50.00\n2024-01-17,REFUND,-20.00\n";
        let mut profile = default_profile();
        profile.mapping.invert_sign = true;
        let txs = import_csv(data.as_ref(), &profile).unwrap();
        assert_eq!(txs[0].amount, -5000);
        assert_eq!(txs[1].amount, 2000);
    }

    #[test]
    fn invert_sign_defaults_to_false_when_absent() {
        let json = r#"{"date_column":0,"description_column":1,"amount_column":2,
            "debit_column":null,"credit_column":null,"memo_column":null,"date_format":"%Y-%m-%d"}"#;
        let mapping: CsvColumnMapping = serde_json::from_str(json).unwrap();
        assert!(!mapping.invert_sign);
    }

    #[test]
    fn import_csv_bad_date_reports_row() {
        let data = b"date,description,amount\n2024-01-15,A,1.00\n01/16/2024,B,2.00\n";
        let err = import_csv(data.as_ref(), &default_profile()).unwrap_err();
        assert!(
            matches!(err, CsvError::InvalidDate(ref m) if m.contains("row 3")),
            "{err}"
        );
    }

    #[test]
    fn import_csv_missing_mapping_columns_error() {
        let data = b"date,description,amount\n2024-01-15,A,1.00\n";
        let mut profile = default_profile();
        profile.mapping.amount_column = None;
        profile.mapping.debit_column = Some(2);
        assert!(matches!(
            import_csv(data.as_ref(), &profile),
            Err(CsvError::MissingColumn(_))
        ));

        let mut profile = default_profile();
        profile.mapping.date_column = None;
        assert!(matches!(
            import_csv(data.as_ref(), &profile),
            Err(CsvError::MissingColumn(_))
        ));
    }

    #[test]
    fn import_csv_skips_blank_rows() {
        let data = b"date,description,amount\n2024-01-15,A,1.00\n,,\n";
        let txs = import_csv(data.as_ref(), &default_profile()).unwrap();
        assert_eq!(txs.len(), 1);
    }

    #[test]
    fn import_csv_no_data_rows_errors() {
        let data = b"date,description,amount\n";
        let result = import_csv(data.as_ref(), &default_profile());
        assert!(matches!(result, Err(CsvError::NoDataRows)));
    }
}
