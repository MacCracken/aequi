use chrono::NaiveDate;
use thiserror::Error;

use crate::util::parse_amount_cents;

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
    #[error("Invalid amount: {0}")]
    InvalidAmount(String),
}

/// Leaf elements that may legally be empty in SGML files. They carry no closing
/// tag there, so they must not be mistaken for the start of an aggregate.
const KNOWN_LEAVES: &[&str] = &[
    "ACCTID",
    "ACCTKEY",
    "ACCTTYPE",
    "BANKID",
    "BRANCHID",
    "CHECKNUM",
    "CORRECTACTION",
    "CORRECTFITID",
    "CURDEF",
    "DTAVAIL",
    "DTEND",
    "DTPOSTED",
    "DTSTART",
    "DTUSER",
    "EXTDNAME",
    "FITID",
    "INV401KSOURCE",
    "MEMO",
    "NAME",
    "PAYEEID",
    "REFNUM",
    "SIC",
    "SRVRTID",
    "TRNAMT",
    "TRNTYPE",
];

pub struct OfxParser;

impl OfxParser {
    /// Parse an OFX/QFX document containing exactly one bank (`STMTRS`) or
    /// credit-card (`CCSTMTRS`) statement.
    ///
    /// Files with several statements are rejected rather than merged; use
    /// [`OfxParser::parse_all`] for those.
    pub fn parse(data: &str) -> Result<OfxStatement, OfxError> {
        let mut statements = Self::parse_all(data)?;
        if statements.len() != 1 {
            return Err(OfxError::ParseError(format!(
                "file contains {} statements; import them separately",
                statements.len()
            )));
        }
        Ok(statements.remove(0))
    }

    /// Parse every bank and credit-card statement in an OFX/QFX document.
    ///
    /// Handles both OFX 1.x SGML (unclosed leaf elements, one or many tags per
    /// line) and OFX 2.x XML. Every transaction must carry a valid `DTPOSTED`
    /// and `TRNAMT`; a missing or malformed value is an error rather than a
    /// silently dropped or zero-amount transaction.
    pub fn parse_all(data: &str) -> Result<Vec<OfxStatement>, OfxError> {
        let tokens = tokenize(data)?;
        let mut state = ParseState::default();

        for token in tokens {
            match token {
                Token::Open { name, text: "" } => {
                    if KNOWN_LEAVES.contains(&name.as_str()) {
                        // Empty SGML leaf (or XML `<MEMO></MEMO>`): no value to record.
                        continue;
                    }
                    state.open_aggregate(name)?;
                }
                Token::Open { name, text } => state.leaf(&name, unescape(text)),
                Token::Close { name } => {
                    if let Some(pos) = state.stack.iter().rposition(|n| *n == name) {
                        state.close_to(pos)?;
                    }
                    // A close tag with no matching aggregate is an XML leaf's
                    // end tag (`</TRNAMT>`); nothing to do.
                }
            }
        }
        state.close_to(0)?;

        if state.statements.is_empty() {
            return Err(OfxError::ParseError(
                "no bank or credit card statement (STMTRS/CCSTMTRS) found".to_string(),
            ));
        }
        Ok(state.statements)
    }
}

#[derive(Default)]
struct ParseState {
    stack: Vec<String>,
    statements: Vec<OfxStatement>,
    statement: Option<BuildingStatement>,
    trx: Option<BuildingTrx>,
}

impl ParseState {
    fn open_aggregate(&mut self, name: String) -> Result<(), OfxError> {
        match name.as_str() {
            "STMTRS" | "CCSTMTRS" => {
                if let Some(pos) = self
                    .stack
                    .iter()
                    .rposition(|n| n == "STMTRS" || n == "CCSTMTRS")
                {
                    // Unterminated previous statement: SGML implies its end.
                    self.close_to(pos)?;
                }
                self.statement = Some(BuildingStatement::default());
            }
            "STMTTRN" => {
                if let Some(pos) = self.stack.iter().rposition(|n| n == "STMTTRN") {
                    self.close_to(pos)?;
                }
                if self.statement.is_some() {
                    self.trx = Some(BuildingTrx::default());
                }
            }
            _ => {}
        }
        self.stack.push(name);
        Ok(())
    }

    /// Pop the stack down to (and including) `pos`, finishing any transaction
    /// or statement aggregate that ends along the way.
    fn close_to(&mut self, pos: usize) -> Result<(), OfxError> {
        while self.stack.len() > pos {
            let Some(name) = self.stack.pop() else { break };
            match name.as_str() {
                "STMTTRN" => {
                    if let (Some(trx), Some(stmt)) = (self.trx.take(), self.statement.as_mut()) {
                        let index = stmt.transactions.len() + 1;
                        stmt.transactions.push(trx.finish(index)?);
                    }
                }
                "STMTRS" | "CCSTMTRS" => {
                    if let Some(stmt) = self.statement.take() {
                        self.statements.push(stmt.finish()?);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn leaf(&mut self, name: &str, value: String) {
        let parent = self.stack.last().map(String::as_str);
        match parent {
            Some("STMTTRN") => {
                if let Some(trx) = self.trx.as_mut() {
                    match name {
                        "FITID" => trx.fit_id = Some(value),
                        "DTPOSTED" => trx.date = Some(value),
                        "TRNAMT" => trx.amount = Some(value),
                        "MEMO" => trx.memo = Some(value),
                        "NAME" => trx.name = Some(value),
                        "CHECKNUM" => trx.check_number = Some(value),
                        _ => {}
                    }
                }
            }
            // OFX allows a PAYEE aggregate in place of NAME.
            Some("PAYEE") => {
                if name == "NAME"
                    && let Some(trx) = self.trx.as_mut()
                    && trx.name.is_none()
                {
                    trx.name = Some(value);
                }
            }
            // Only the statement's own account; BANKACCTTO / CCACCTTO inside a
            // transaction describe the counterparty and are ignored.
            Some("BANKACCTFROM" | "CCACCTFROM") => {
                if let Some(stmt) = self.statement.as_mut() {
                    match name {
                        "ACCTID" => stmt.account_id = Some(value),
                        "BANKID" => stmt.bank_id = Some(value),
                        "ACCTTYPE" => stmt.account_type = Some(value),
                        _ => {}
                    }
                }
            }
            Some("BANKTRANLIST") => {
                if let Some(stmt) = self.statement.as_mut() {
                    match name {
                        "DTSTART" => stmt.start_date = Some(value),
                        "DTEND" => stmt.end_date = Some(value),
                        _ => {}
                    }
                }
            }
            Some("STMTRS" | "CCSTMTRS") => {
                if name == "CURDEF"
                    && let Some(stmt) = self.statement.as_mut()
                {
                    stmt.currency = Some(value);
                }
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct BuildingStatement {
    account_id: Option<String>,
    bank_id: Option<String>,
    account_type: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    currency: Option<String>,
    transactions: Vec<OfxTransaction>,
}

impl BuildingStatement {
    fn finish(self) -> Result<OfxStatement, OfxError> {
        let account_id = self
            .account_id
            .filter(|id| !id.is_empty())
            .ok_or_else(|| OfxError::MissingField("ACCTID".to_string()))?;
        let start_date = required_date(self.start_date, "DTSTART", "statement")?;
        let end_date = required_date(self.end_date, "DTEND", "statement")?;
        Ok(OfxStatement {
            account: OfxAccount {
                account_id,
                bank_id: self.bank_id,
                account_type: self.account_type,
            },
            start_date,
            end_date,
            transactions: self.transactions,
            currency: self.currency,
        })
    }
}

#[derive(Default)]
struct BuildingTrx {
    fit_id: Option<String>,
    date: Option<String>,
    amount: Option<String>,
    memo: Option<String>,
    name: Option<String>,
    check_number: Option<String>,
}

impl BuildingTrx {
    fn finish(self, index: usize) -> Result<OfxTransaction, OfxError> {
        let label = match &self.fit_id {
            Some(id) => format!("transaction {id}"),
            None => format!("transaction #{index}"),
        };
        let date = required_date(self.date, "DTPOSTED", &label)?;
        let raw_amount = self
            .amount
            .ok_or_else(|| OfxError::MissingField(format!("TRNAMT in {label}")))?;
        let amount = parse_ofx_amount(&raw_amount)
            .ok_or_else(|| OfxError::InvalidAmount(format!("TRNAMT '{raw_amount}' in {label}")))?;
        Ok(OfxTransaction {
            fit_id: self.fit_id.unwrap_or_default(),
            date,
            amount,
            memo: self.memo,
            name: self.name,
            check_number: self.check_number,
        })
    }
}

fn required_date(raw: Option<String>, field: &str, context: &str) -> Result<NaiveDate, OfxError> {
    let raw = raw.ok_or_else(|| OfxError::MissingField(format!("{field} in {context}")))?;
    parse_ofx_date(&raw)
        .ok_or_else(|| OfxError::InvalidDate(format!("{field} '{raw}' in {context}")))
}

enum Token<'a> {
    Open { name: String, text: &'a str },
    Close { name: String },
}

/// Split a document into tags on `<` rather than on lines, so SGML (one tag
/// per line, no leaf end tags), XML and single-line files all tokenize the
/// same way. Header lines, processing instructions and comments are skipped.
fn tokenize(data: &str) -> Result<Vec<Token<'_>>, OfxError> {
    let mut tokens = Vec::new();
    let mut rest = match data.find('<') {
        Some(start) => &data[start..],
        None => return Ok(tokens),
    };

    while let Some(after_lt) = rest.strip_prefix('<') {
        if let Some(comment) = after_lt.strip_prefix("!--") {
            let end = comment
                .find("-->")
                .ok_or_else(|| OfxError::ParseError("unterminated comment".to_string()))?;
            rest = skip_to_next_tag(&comment[end + 3..]);
            continue;
        }

        let close = after_lt.find('>').ok_or_else(|| {
            let snippet: String = after_lt.chars().take(20).collect();
            OfxError::ParseError(format!("unterminated tag '<{snippet}'"))
        })?;
        let tag = after_lt[..close].trim();
        let after_gt = &after_lt[close + 1..];
        let text_end = after_gt.find('<').unwrap_or(after_gt.len());
        let text = after_gt[..text_end].trim();
        rest = &after_gt[text_end..];

        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            tokens.push(Token::Close {
                name: tag_name(name),
            });
        } else if let Some(name) = tag.strip_suffix('/') {
            // XML empty element: `<MEMO/>`.
            let name = tag_name(name);
            tokens.push(Token::Open {
                name: name.clone(),
                text: "",
            });
            tokens.push(Token::Close { name });
        } else {
            tokens.push(Token::Open {
                name: tag_name(tag),
                text,
            });
        }
    }

    Ok(tokens)
}

fn skip_to_next_tag(s: &str) -> &str {
    s.find('<').map_or("", |i| &s[i..])
}

fn tag_name(tag: &str) -> String {
    tag.split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase()
}

/// Decode XML/SGML character entities. Unknown or malformed entities are kept
/// verbatim because SGML files frequently contain bare `&` ("AT&T").
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let candidate = &rest[amp..];
        let decoded = candidate[1..]
            .char_indices()
            .take(10)
            .find(|&(_, c)| c == ';')
            .and_then(|(semi, _)| decode_entity(&candidate[1..1 + semi]).map(|c| (c, semi + 2)));
        match decoded {
            Some((c, consumed)) => {
                out.push(c);
                rest = &candidate[consumed..];
            }
            None => {
                out.push('&');
                rest = &candidate[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let num = entity.strip_prefix('#')?;
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Parse an OFX date: `YYYYMMDD[HHMMSS[.XXX]][[offset:TZ]]`, e.g.
/// `20240115120000.000[-5:EST]`. The calendar date is taken as written (the
/// institution's local date). RFC 3339 is accepted as a fallback.
fn parse_ofx_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    if let Some(date_part) = s.get(..8)
        && date_part.bytes().all(|b| b.is_ascii_digit())
    {
        if !is_ofx_time_suffix(&s[8..]) {
            return None;
        }
        let y: i32 = date_part[0..4].parse().ok()?;
        let m: u32 = date_part[4..6].parse().ok()?;
        let d: u32 = date_part[6..8].parse().ok()?;
        return NaiveDate::from_ymd_opt(y, m, d);
    }

    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.date_naive())
}

/// `[HHMMSS][.XXX][ ][[offset[:TZ]]]` — everything after the 8 date digits.
fn is_ofx_time_suffix(rest: &str) -> bool {
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let rest = match rest.strip_prefix('.') {
        Some(frac) => {
            let after = frac.trim_start_matches(|c: char| c.is_ascii_digit());
            if after.len() == frac.len() {
                return false;
            }
            after
        }
        None => rest,
    };
    let rest = rest.trim_start();
    if rest.is_empty() {
        return true;
    }
    rest.strip_prefix('[')
        .and_then(|tz| tz.strip_suffix(']'))
        .is_some_and(|tz| !tz.contains(['[', ']']))
}

fn parse_ofx_amount(s: &str) -> Option<i64> {
    parse_amount_cents(s)
}

/// Parse an OFX/QFX file containing a single statement.
pub fn parse(data: &[u8]) -> Result<OfxStatement, OfxError> {
    OfxParser::parse(&decode(data)?)
}

/// Parse every statement in an OFX/QFX file.
pub fn parse_all(data: &[u8]) -> Result<Vec<OfxStatement>, OfxError> {
    OfxParser::parse_all(&decode(data)?)
}

fn decode(data: &[u8]) -> Result<std::borrow::Cow<'_, str>, OfxError> {
    // Guard against pathologically large input
    const MAX_OFX_SIZE: usize = 50 * 1024 * 1024; // 50MB
    if data.len() > MAX_OFX_SIZE {
        return Err(OfxError::ParseError(format!(
            "OFX file too large ({} bytes, max {})",
            data.len(),
            MAX_OFX_SIZE
        )));
    }

    Ok(String::from_utf8_lossy(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    // ── unit helpers ──────────────────────────────────────────────────────────

    #[test]
    fn parse_ofx_date_8digit() {
        assert_eq!(parse_ofx_date("20240115"), Some(ymd(2024, 1, 15)));
        assert_eq!(parse_ofx_date("20240301"), Some(ymd(2024, 3, 1)));
    }

    #[test]
    fn parse_ofx_date_with_time_suffix_ignored() {
        // Banks sometimes emit e.g. "20240115120000[-5:EST]" — only first 8 chars used
        assert_eq!(
            parse_ofx_date("20240115120000[-5:EST]"),
            Some(ymd(2024, 1, 15))
        );
    }

    #[test]
    fn parse_ofx_date_full_timezone_forms() {
        for raw in [
            "20240115120000.000[-5:EST]",
            "20240115120000.000",
            "20240115120000",
            "202401151200",
            "20240115120000[0:GMT]",
            "20240115120000.000 [-5:EST]",
            "20240115[+5.30:IST]",
        ] {
            assert_eq!(parse_ofx_date(raw), Some(ymd(2024, 1, 15)), "{raw}");
        }
    }

    #[test]
    fn parse_ofx_date_rfc3339_fallback() {
        assert_eq!(
            parse_ofx_date("2024-01-15T12:00:00-05:00"),
            Some(ymd(2024, 1, 15))
        );
    }

    #[test]
    fn parse_ofx_date_invalid_returns_none() {
        assert_eq!(parse_ofx_date("not-a-date"), None);
        assert_eq!(parse_ofx_date(""), None);
        assert_eq!(parse_ofx_date("20241315"), None);
        assert_eq!(parse_ofx_date("20240115junk"), None);
        assert_eq!(parse_ofx_date("20240115</DTPOSTED>"), None);
    }

    #[test]
    fn parse_ofx_date_non_ascii_does_not_panic() {
        // U+FFFD from lossy decoding is 3 bytes; byte slicing used to panic here.
        assert_eq!(parse_ofx_date("2024\u{FFFD}0115"), None);
        assert_eq!(parse_ofx_date("\u{FFFD}\u{FFFD}\u{FFFD}"), None);
        assert_eq!(parse_ofx_date("20240115\u{FFFD}"), None);
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
    fn parse_ofx_amount_decimal_comma() {
        // Used to strip the comma and import -4999 dollars.
        assert_eq!(parse_ofx_amount("-49,99"), Some(-4999));
        assert_eq!(parse_ofx_amount("12,5"), Some(1250));
        assert_eq!(parse_ofx_amount("1.234,56"), None);
    }

    #[test]
    fn parse_ofx_amount_invalid_returns_none() {
        assert_eq!(parse_ofx_amount("abc"), None);
        assert_eq!(parse_ofx_amount(""), None);
    }

    #[test]
    fn unescape_entities() {
        assert_eq!(unescape("AT&amp;T"), "AT&T");
        assert_eq!(unescape("a &lt;b&gt; &quot;c&quot;"), "a <b> \"c\"");
        assert_eq!(unescape("&#65;&#x42;"), "AB");
        // Bare ampersands (common in SGML) survive.
        assert_eq!(unescape("AT&T"), "AT&T");
        assert_eq!(unescape("Tom & Jerry;"), "Tom & Jerry;");
        assert_eq!(unescape("trailing &"), "trailing &");
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
        assert_eq!(stmt.start_date, ymd(2024, 1, 1));
        assert_eq!(stmt.end_date, ymd(2024, 1, 31));
        assert_eq!(stmt.currency.as_deref(), Some("USD"));
        assert_eq!(stmt.transactions.len(), 2);
    }

    #[test]
    fn parse_ofx_transaction_fields() {
        let stmt = parse(SAMPLE_OFX.as_bytes()).unwrap();
        let t0 = &stmt.transactions[0];
        assert_eq!(t0.fit_id, "TXN001");
        assert_eq!(t0.date, ymd(2024, 1, 15));
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

    const SAMPLE_OFX_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<?OFX OFXHEADER="200" VERSION="220" SECURITY="NONE" OLDFILEUID="NONE" NEWFILEUID="NONE"?>
<OFX>
  <BANKMSGSRSV1>
    <STMTTRNRS>
      <STMTRS>
        <CURDEF>USD</CURDEF>
        <BANKACCTFROM>
          <BANKID>123456789</BANKID>
          <ACCTID>000112345</ACCTID>
          <ACCTTYPE>CHECKING</ACCTTYPE>
        </BANKACCTFROM>
        <BANKTRANLIST>
          <DTSTART>20240101000000.000[-5:EST]</DTSTART>
          <DTEND>20240131235959.000[-5:EST]</DTEND>
          <STMTTRN>
            <TRNTYPE>DEBIT</TRNTYPE>
            <DTPOSTED>20240115120000.000[-5:EST]</DTPOSTED>
            <TRNAMT>-49.99</TRNAMT>
            <FITID>TXN001</FITID>
            <NAME>AT&amp;T WIRELESS</NAME>
            <MEMO>Bill &lt;Jan&gt;</MEMO>
          </STMTTRN>
          <STMTTRN>
            <TRNTYPE>CREDIT</TRNTYPE>
            <DTPOSTED>20240120</DTPOSTED>
            <TRNAMT>1500.00</TRNAMT>
            <FITID>TXN002</FITID>
            <NAME>DIRECT DEPOSIT</NAME>
            <MEMO></MEMO>
          </STMTTRN>
        </BANKTRANLIST>
      </STMTRS>
    </STMTTRNRS>
  </BANKMSGSRSV1>
</OFX>
"#;

    #[test]
    fn parse_ofx2_xml_with_closing_tags() {
        let stmt = parse(SAMPLE_OFX_XML.as_bytes()).unwrap();
        assert_eq!(stmt.account.account_id, "000112345");
        assert_eq!(stmt.account.bank_id.as_deref(), Some("123456789"));
        assert_eq!(stmt.currency.as_deref(), Some("USD"));
        assert_eq!(stmt.start_date, ymd(2024, 1, 1));
        assert_eq!(stmt.end_date, ymd(2024, 1, 31));
        assert_eq!(stmt.transactions.len(), 2);

        let t0 = &stmt.transactions[0];
        assert_eq!(t0.amount, -4999);
        assert_eq!(t0.fit_id, "TXN001");
        assert_eq!(t0.date, ymd(2024, 1, 15));
        assert_eq!(t0.name.as_deref(), Some("AT&T WIRELESS"));
        assert_eq!(t0.memo.as_deref(), Some("Bill <Jan>"));

        let t1 = &stmt.transactions[1];
        assert_eq!(t1.amount, 150000);
        assert!(t1.memo.is_none());
    }

    #[test]
    fn parse_single_line_sgml() {
        let data = "OFXHEADER:100\nDATA:OFXSGML\n\n<OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS><CURDEF>USD\
            <BANKACCTFROM><BANKID>1<ACCTID>42<ACCTTYPE>SAVINGS</BANKACCTFROM>\
            <BANKTRANLIST><DTSTART>20240101<DTEND>20240131\
            <STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20240105<TRNAMT>-12.34<FITID>A1<NAME>COFFEE</STMTTRN>\
            </BANKTRANLIST></STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>";
        let stmt = parse(data.as_bytes()).unwrap();
        assert_eq!(stmt.account.account_id, "42");
        assert_eq!(stmt.account.account_type.as_deref(), Some("SAVINGS"));
        assert_eq!(stmt.transactions.len(), 1);
        let t = &stmt.transactions[0];
        assert_eq!(t.amount, -1234);
        assert_eq!(t.fit_id, "A1");
        assert_eq!(t.name.as_deref(), Some("COFFEE"));
        assert_eq!(t.date, ymd(2024, 1, 5));
    }

    fn statement_with_trx(trx_body: &str) -> String {
        format!(
            "<OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS><CURDEF>USD\n\
             <BANKACCTFROM><BANKID>1<ACCTID>42<ACCTTYPE>CHECKING</BANKACCTFROM>\n\
             <BANKTRANLIST><DTSTART>20240101<DTEND>20240131\n\
             <STMTTRN>\n{trx_body}\n</STMTTRN>\n\
             </BANKTRANLIST></STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>"
        )
    }

    #[test]
    fn missing_trnamt_is_an_error_not_zero() {
        let data = statement_with_trx("<DTPOSTED>20240105\n<FITID>A1");
        let err = parse(data.as_bytes()).unwrap_err();
        assert!(
            matches!(err, OfxError::MissingField(ref f) if f.contains("TRNAMT") && f.contains("A1"))
        );
    }

    #[test]
    fn unparseable_trnamt_is_an_error() {
        let data = statement_with_trx("<DTPOSTED>20240105\n<TRNAMT>12.3.4\n<FITID>A1");
        assert!(matches!(
            parse(data.as_bytes()),
            Err(OfxError::InvalidAmount(_))
        ));
    }

    #[test]
    fn missing_dtposted_is_an_error_not_dropped() {
        let data = statement_with_trx("<TRNAMT>-5.00\n<FITID>A1");
        let err = parse(data.as_bytes()).unwrap_err();
        assert!(matches!(err, OfxError::MissingField(ref f) if f.contains("DTPOSTED")));
    }

    #[test]
    fn invalid_dtposted_is_an_error() {
        let data = statement_with_trx("<DTPOSTED>2024\u{FFFD}0105\n<TRNAMT>-5.00\n<FITID>A1");
        assert!(matches!(
            parse(data.as_bytes()),
            Err(OfxError::InvalidDate(_))
        ));
    }

    #[test]
    fn decimal_comma_amount_in_statement() {
        let data = statement_with_trx("<DTPOSTED>20240105\n<TRNAMT>-49,99\n<FITID>A1");
        let stmt = parse(data.as_bytes()).unwrap();
        assert_eq!(stmt.transactions[0].amount, -4999);
    }

    #[test]
    fn nested_account_to_does_not_clobber_statement_account() {
        let data = statement_with_trx(
            "<TRNTYPE>XFER\n<DTPOSTED>20240105\n<TRNAMT>-100.00\n<FITID>X1\n\
             <BANKACCTTO><BANKID>999<ACCTID>OTHER<ACCTTYPE>SAVINGS</BANKACCTTO>\n\
             <CCACCTTO><ACCTID>CARD</CCACCTTO>",
        );
        let stmt = parse(data.as_bytes()).unwrap();
        assert_eq!(stmt.account.account_id, "42");
        assert_eq!(stmt.account.bank_id.as_deref(), Some("1"));
        assert_eq!(stmt.account.account_type.as_deref(), Some("CHECKING"));
        assert_eq!(stmt.transactions[0].amount, -10000);
    }

    #[test]
    fn payee_aggregate_supplies_name() {
        let data = statement_with_trx(
            "<DTPOSTED>20240105\n<TRNAMT>-5.00\n<FITID>A1\n\
             <PAYEE><NAME>LOCAL HARDWARE<ADDR1>1 MAIN ST<CITY>X</PAYEE>",
        );
        let stmt = parse(data.as_bytes()).unwrap();
        assert_eq!(stmt.transactions[0].name.as_deref(), Some("LOCAL HARDWARE"));
    }

    #[test]
    fn ledger_balance_dates_do_not_override_statement_range() {
        let data = "<OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS><CURDEF>USD\
            <BANKACCTFROM><ACCTID>42</BANKACCTFROM>\
            <BANKTRANLIST><DTSTART>20240101<DTEND>20240131</BANKTRANLIST>\
            <LEDGERBAL><BALAMT>100.00<DTASOF>20240215</LEDGERBAL>\
            </STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>";
        let stmt = parse(data.as_bytes()).unwrap();
        assert_eq!(stmt.end_date, ymd(2024, 1, 31));
        assert!(stmt.transactions.is_empty());
    }

    const TWO_STATEMENTS: &str = "<OFX>\
        <BANKMSGSRSV1><STMTTRNRS><STMTRS><CURDEF>USD\
        <BANKACCTFROM><BANKID>1<ACCTID>CHK<ACCTTYPE>CHECKING</BANKACCTFROM>\
        <BANKTRANLIST><DTSTART>20240101<DTEND>20240131\
        <STMTTRN><DTPOSTED>20240105<TRNAMT>-1.00<FITID>C1</STMTTRN>\
        </BANKTRANLIST></STMTRS></STMTTRNRS></BANKMSGSRSV1>\
        <CREDITCARDMSGSRSV1><CCSTMTTRNRS><CCSTMTRS><CURDEF>CAD\
        <CCACCTFROM><ACCTID>CARD</CCACCTFROM>\
        <BANKTRANLIST><DTSTART>20240201<DTEND>20240229\
        <STMTTRN><DTPOSTED>20240210<TRNAMT>-2.00<FITID>V1</STMTTRN>\
        <STMTTRN><DTPOSTED>20240211<TRNAMT>3.00<FITID>V2</STMTTRN>\
        </BANKTRANLIST></CCSTMTRS></CCSTMTTRNRS></CREDITCARDMSGSRSV1></OFX>";

    #[test]
    fn multiple_statements_are_kept_separate() {
        let stmts = parse_all(TWO_STATEMENTS.as_bytes()).unwrap();
        assert_eq!(stmts.len(), 2);

        assert_eq!(stmts[0].account.account_id, "CHK");
        assert_eq!(stmts[0].currency.as_deref(), Some("USD"));
        assert_eq!(stmts[0].start_date, ymd(2024, 1, 1));
        assert_eq!(stmts[0].end_date, ymd(2024, 1, 31));
        assert_eq!(stmts[0].transactions.len(), 1);

        assert_eq!(stmts[1].account.account_id, "CARD");
        assert!(stmts[1].account.bank_id.is_none());
        assert_eq!(stmts[1].currency.as_deref(), Some("CAD"));
        assert_eq!(stmts[1].start_date, ymd(2024, 2, 1));
        assert_eq!(stmts[1].end_date, ymd(2024, 2, 29));
        assert_eq!(stmts[1].transactions.len(), 2);
    }

    #[test]
    fn single_statement_parse_rejects_multi_statement_file() {
        assert!(matches!(
            parse(TWO_STATEMENTS.as_bytes()),
            Err(OfxError::ParseError(_))
        ));
    }

    #[test]
    fn unclosed_stmttrn_is_closed_by_the_next_one() {
        let data = "<OFX><STMTRS><BANKACCTFROM><ACCTID>42</BANKACCTFROM>\
            <BANKTRANLIST><DTSTART>20240101<DTEND>20240131\
            <STMTTRN><DTPOSTED>20240105<TRNAMT>-1.00<FITID>A\
            <STMTTRN><DTPOSTED>20240106<TRNAMT>-2.00<FITID>B\
            </BANKTRANLIST></STMTRS></OFX>";
        let stmt = parse(data.as_bytes()).unwrap();
        let ids: Vec<_> = stmt
            .transactions
            .iter()
            .map(|t| t.fit_id.as_str())
            .collect();
        assert_eq!(ids, ["A", "B"]);
    }

    #[test]
    fn comments_and_processing_instructions_skipped() {
        let data = "<?xml version=\"1.0\"?><!-- exported <by> bank --><OFX><STMTRS>\
            <BANKACCTFROM><ACCTID>42</ACCTID></BANKACCTFROM>\
            <BANKTRANLIST><DTSTART>20240101</DTSTART><DTEND>20240131</DTEND></BANKTRANLIST>\
            </STMTRS></OFX>";
        let stmt = parse(data.as_bytes()).unwrap();
        assert_eq!(stmt.account.account_id, "42");
    }

    #[test]
    fn unterminated_tag_errors() {
        assert!(matches!(
            parse(b"<OFX><STMTRS><ACCTID"),
            Err(OfxError::ParseError(_))
        ));
    }

    #[test]
    fn no_statement_errors() {
        assert!(parse(b"<OFX><SIGNONMSGSRSV1></SIGNONMSGSRSV1></OFX>").is_err());
        assert!(parse(b"").is_err());
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
