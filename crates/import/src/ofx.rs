use chrono::NaiveDate;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct OfxTransaction {
    pub fit_id: String,
    pub date: NaiveDate,
    pub amount: i64,
    pub memo: Option<String>,
    pub name: Option<String>,
    pub check_number: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OfxAccount {
    pub account_id: String,
    pub bank_id: Option<String>,
    pub account_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OfxStatement {
    pub account: OfxAccount,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub transactions: Vec<OfxTransaction>,
    pub currency: Option<String>,
}

#[derive(Error, Debug)]
pub enum OfxError {
    #[error("Failed to parse OFX: {0}")]
    ParseError(String),
    #[error("Missing required field: {0}")]
    MissingField(String),
    #[error("Invalid date format: {0}")]
    InvalidDate(String),
}

pub struct OfxParser;

impl OfxParser {
    pub fn parse(data: &str) -> Result<OfxStatement, OfxError> {
        let data = data.trim();

        let mut account = OfxAccount {
            account_id: String::new(),
            bank_id: None,
            account_type: None,
        };

        let mut start_date = None;
        let mut end_date = None;
        let mut transactions = Vec::new();
        let mut currency = None;

        let mut in_stmttrn = false;
        let mut current_trx: Option<BuildingTrx> = None;

        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if let Some(tag) = line.strip_prefix('<') {
                let (tag_name, value) = if let Some((name, val)) = tag.split_once('>') {
                    (name.trim(), Some(val.trim().to_string()))
                } else {
                    (tag.trim_end_matches(&['>', '\r', '\n'][..]), None)
                };

                match tag_name.to_uppercase().as_str() {
                    "ACCTID" => {
                        if let Some(v) = value {
                            account.account_id = v;
                        }
                    }
                    "BANKID" => {
                        if let Some(v) = value {
                            account.bank_id = Some(v);
                        }
                    }
                    "ACCTTYPE" => {
                        if let Some(v) = value {
                            account.account_type = Some(v);
                        }
                    }
                    "DTSTART" => {
                        if let Some(v) = value {
                            start_date = parse_ofx_date(&v);
                        }
                    }
                    "DTEND" => {
                        if let Some(v) = value {
                            end_date = parse_ofx_date(&v);
                        }
                    }
                    "CURDEF" => {
                        if let Some(v) = value {
                            currency = Some(v);
                        }
                    }
                    "STMTTRN" => {
                        in_stmttrn = true;
                        current_trx = Some(BuildingTrx::default());
                    }
                    "/STMTTRN" => {
                        if let Some(trx) = current_trx.take() {
                            if let Some(date) = trx.date {
                                transactions.push(OfxTransaction {
                                    fit_id: trx.fit_id.unwrap_or_default(),
                                    date,
                                    amount: trx.amount.unwrap_or(0),
                                    memo: trx.memo,
                                    name: trx.name,
                                    check_number: trx.check_number,
                                });
                            }
                        }
                        in_stmttrn = false;
                    }
                    _ => {
                        if in_stmttrn {
                            if let Some(ref mut trx) = current_trx {
                                match tag_name.to_uppercase().as_str() {
                                    "FITID" => {
                                        if let Some(v) = value {
                                            trx.fit_id = Some(v);
                                        }
                                    }
                                    "DTPOSTED" => {
                                        if let Some(v) = value {
                                            trx.date = parse_ofx_date(&v);
                                        }
                                    }
                                    "TRNAMT" => {
                                        if let Some(v) = value {
                                            trx.amount = parse_ofx_amount(&v);
                                        }
                                    }
                                    "MEMO" => {
                                        if let Some(v) = value {
                                            trx.memo = Some(v);
                                        }
                                    }
                                    "NAME" => {
                                        if let Some(v) = value {
                                            trx.name = Some(v);
                                        }
                                    }
                                    "CHECKNUM" => {
                                        if let Some(v) = value {
                                            trx.check_number = Some(v);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        let start_date = start_date.ok_or(OfxError::MissingField("DTSTART".to_string()))?;
        let end_date = end_date.ok_or(OfxError::MissingField("DTEND".to_string()))?;

        if account.account_id.is_empty() {
            return Err(OfxError::MissingField("ACCTID".to_string()));
        }

        Ok(OfxStatement {
            account,
            start_date,
            end_date,
            transactions,
            currency,
        })
    }
}

#[derive(Default)]
struct BuildingTrx {
    fit_id: Option<String>,
    date: Option<NaiveDate>,
    amount: Option<i64>,
    memo: Option<String>,
    name: Option<String>,
    check_number: Option<String>,
}

fn parse_ofx_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    if s.len() >= 8 {
        let y: i32 = s[0..4].parse().ok()?;
        let m: u32 = s[4..6].parse().ok()?;
        let d: u32 = s[6..8].parse().ok()?;

        if let Some(date) = NaiveDate::from_ymd_opt(y, m, d) {
            return Some(date);
        }
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.date_naive());
    }

    None
}

fn parse_ofx_amount(s: &str) -> Option<i64> {
    let s = s.trim();
    let s = s.replace(',', "");
    let dec = Decimal::from_str(&s).ok()?;
    (dec * Decimal::from(100)).round().to_i64()
}

pub fn parse(data: &[u8]) -> Result<OfxStatement, OfxError> {
    let content = String::from_utf8_lossy(data);
    OfxParser::parse(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── unit helpers ──────────────────────────────────────────────────────────

    #[test]
    fn parse_ofx_date_8digit() {
        assert_eq!(
            parse_ofx_date("20240115"),
            Some(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap())
        );
        assert_eq!(
            parse_ofx_date("20240301"),
            Some(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap())
        );
    }

    #[test]
    fn parse_ofx_date_with_time_suffix_ignored() {
        // Banks sometimes emit e.g. "20240115120000[-5:EST]" — only first 8 chars used
        assert_eq!(
            parse_ofx_date("20240115120000[-5:EST]"),
            Some(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap())
        );
    }

    #[test]
    fn parse_ofx_date_invalid_returns_none() {
        assert_eq!(parse_ofx_date("not-a-date"), None);
        assert_eq!(parse_ofx_date(""), None);
    }

    #[test]
    fn parse_ofx_amount_positive() {
        assert_eq!(parse_ofx_amount("123.45"), Some(12345));
        assert_eq!(parse_ofx_amount("0.99"), Some(99));
        assert_eq!(parse_ofx_amount("0.01"), Some(1));
    }

    #[test]
    fn parse_ofx_amount_negative() {
        assert_eq!(parse_ofx_amount("-50.00"), Some(-5000));
        assert_eq!(parse_ofx_amount("-0.01"), Some(-1));
    }

    #[test]
    fn parse_ofx_amount_with_commas() {
        assert_eq!(parse_ofx_amount("1,234.56"), Some(123456));
    }

    #[test]
    fn parse_ofx_amount_invalid_returns_none() {
        assert_eq!(parse_ofx_amount("abc"), None);
        assert_eq!(parse_ofx_amount(""), None);
    }

    // ── full statement parse ──────────────────────────────────────────────────

    const SAMPLE_OFX: &str = r#"
OFXHEADER:100
DATA:OFXSGML
VERSION:102

<OFX>
<BANKMSGSRSV1>
<STMTTRNRS>
<STMTRS>
<CURDEF>USD
<BANKACCTFROM>
<BANKID>123456789
<ACCTID>000112345
<ACCTTYPE>CHECKING
</BANKACCTFROM>
<BANKTRANLIST>
<DTSTART>20240101
<DTEND>20240131
<STMTTRN>
<TRNTYPE>DEBIT
<DTPOSTED>20240115
<TRNAMT>-49.99
<FITID>TXN001
<NAME>AMAZON MARKETPLACE
<MEMO>Online purchase
</STMTTRN>
<STMTTRN>
<TRNTYPE>CREDIT
<DTPOSTED>20240120
<TRNAMT>1500.00
<FITID>TXN002
<NAME>DIRECT DEPOSIT
</STMTTRN>
</BANKTRANLIST>
</STMTRS>
</STMTTRNRS>
</BANKMSGSRSV1>
</OFX>
"#;

    #[test]
    fn parse_full_ofx_statement() {
        let stmt = parse(SAMPLE_OFX.as_bytes()).unwrap();

        assert_eq!(stmt.account.account_id, "000112345");
        assert_eq!(stmt.account.bank_id.as_deref(), Some("123456789"));
        assert_eq!(stmt.account.account_type.as_deref(), Some("CHECKING"));
        assert_eq!(stmt.start_date, NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        assert_eq!(stmt.end_date, NaiveDate::from_ymd_opt(2024, 1, 31).unwrap());
        assert_eq!(stmt.currency.as_deref(), Some("USD"));
        assert_eq!(stmt.transactions.len(), 2);
    }

    #[test]
    fn parse_ofx_transaction_fields() {
        let stmt = parse(SAMPLE_OFX.as_bytes()).unwrap();
        let t0 = &stmt.transactions[0];
        assert_eq!(t0.fit_id, "TXN001");
        assert_eq!(t0.date, NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());
        assert_eq!(t0.amount, -4999);
        assert_eq!(t0.name.as_deref(), Some("AMAZON MARKETPLACE"));
        assert_eq!(t0.memo.as_deref(), Some("Online purchase"));
    }

    #[test]
    fn parse_ofx_second_transaction() {
        let stmt = parse(SAMPLE_OFX.as_bytes()).unwrap();
        let t1 = &stmt.transactions[1];
        assert_eq!(t1.fit_id, "TXN002");
        assert_eq!(t1.amount, 150000);
        assert!(t1.memo.is_none());
    }

    #[test]
    fn parse_ofx_missing_account_id_errors() {
        let bad = r#"
<OFX>
<BANKMSGSRSV1><STMTTRNRS><STMTRS>
<CURDEF>USD
<BANKACCTFROM></BANKACCTFROM>
<BANKTRANLIST>
<DTSTART>20240101
<DTEND>20240131
</BANKTRANLIST>
</STMTRS></STMTTRNRS></BANKMSGSRSV1>
</OFX>
"#;
        assert!(parse(bad.as_bytes()).is_err());
    }

    #[test]
    fn parse_ofx_missing_dates_errors() {
        let bad = r#"
<OFX>
<BANKMSGSRSV1><STMTTRNRS><STMTRS>
<BANKACCTFROM>
<ACCTID>12345
</BANKACCTFROM>
<BANKTRANLIST>
</BANKTRANLIST>
</STMTRS></STMTTRNRS></BANKMSGSRSV1>
</OFX>
"#;
        assert!(parse(bad.as_bytes()).is_err());
    }
}
