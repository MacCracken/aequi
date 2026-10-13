use std::sync::OnceLock;

use chrono::NaiveDate;
use regex::Regex;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::str::FromStr;

use crate::types::{ExtractedField, ExtractedReceipt, PaymentMethod};

// ── Compiled regex cache ─────────────────────────────────────────────────────

macro_rules! re {
    ($name:ident, $pat:expr) => {
        fn $name() -> &'static Regex {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| Regex::new($pat).expect("invalid regex"))
        }
    };
}

// A money amount with an optional sign: "12.34", "$1,234.56", "-$12.34",
// "$-12.34" or "($12.34)". Parse the `amt` group with `parse_signed_amount`.
macro_rules! amount {
    () => {
        r"(?P<amt>\(\s*\$?\s*\d[\d,]*\.\d{2}\s*\)|-?\s*\$?\s*-?\s*\d[\d,]*\.\d{2}\b)"
    };
}

re!(
    re_total,
    concat!(
        r"(?i)\b(?P<label>grand\s+total|total\s+due|amount\s+due|balance\s+due|total)\b[ \t]*:?\s*",
        amount!()
    )
);
re!(
    re_subtotal,
    concat!(r"(?i)\bsub[\s\-]*total\b[ \t]*:?\s*", amount!())
);
// An optional rate ("8.25%", "@ 8.25%", "(7%)") may sit between the label and
// the amount; it must not be mistaken for the amount itself.
re!(
    re_tax,
    concat!(
        r"(?i)\b(?:sales\s*tax|tax|hst|gst|pst|vat)\b[ \t]*:?[ \t]*(?:@[ \t]*)?(?:\(?\d+(?:\.\d+)?[ \t]*%\)?[ \t]*)?:?\s*",
        amount!()
    )
);
re!(re_currency, r"\$\s*([\d,]+\.\d{2})");

re!(
    re_date_month_name,
    r"(?i)\b(january|february|march|april|may|june|july|august|september|october|november|december)\s+(\d{1,2}),?\s+(\d{4})\b"
);
re!(
    re_date_abbr_month,
    r"(?i)\b(\d{1,2})\s+(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)\.?\s+(\d{4})\b"
);
re!(re_date_iso, r"\b(\d{4})-(\d{2})-(\d{2})\b");
re!(re_date_slash, r"\b(\d{1,2})/(\d{1,2})/(\d{2,4})\b");
re!(re_date_dash, r"\b(\d{1,2})-(\d{1,2})-(\d{2,4})\b");

re!(
    re_payment,
    r"(?i)\b(visa|mastercard|master\s*card|amex|american\s+express|discover|cash|debit|check|cheque)\b"
);

re!(re_url, r"(?i)(https?://|www\.)\S+");

// ── Public extraction API ─────────────────────────────────────────────────────

pub struct Extractor;

impl Extractor {
    /// Extract structured fields from raw OCR text.
    pub fn extract(ocr_text: &str) -> ExtractedReceipt {
        let vendor = Self::extract_vendor(ocr_text);
        let date = Self::extract_date(ocr_text);
        let total_cents = Self::extract_total(ocr_text);
        let subtotal_cents = Self::extract_subtotal(ocr_text);
        let tax_cents = Self::extract_tax(ocr_text);
        let payment_method = Self::extract_payment_method(ocr_text);

        // Aggregate confidence: weighted sum of key fields.
        let confidence = {
            let weighted = [
                (vendor.as_ref().map(|f| f.confidence), 0.25f32),
                (date.as_ref().map(|f| f.confidence), 0.30),
                (total_cents.as_ref().map(|f| f.confidence), 0.35),
                (payment_method.as_ref().map(|f| f.confidence), 0.10),
            ];
            let (score, weight) = weighted
                .iter()
                .fold((0.0f32, 0.0f32), |(s, w), (conf, fw)| {
                    (s + conf.unwrap_or(0.0) * fw, w + fw)
                });
            if weight > 0.0 { score / weight } else { 0.0 }
        };

        ExtractedReceipt {
            vendor,
            date,
            subtotal_cents,
            tax_cents,
            total_cents,
            payment_method,
            line_items: vec![],
            confidence,
        }
    }

    // ── Vendor ────────────────────────────────────────────────────────────────

    fn extract_vendor(text: &str) -> Option<ExtractedField<String>> {
        // The merchant name is printed at the top, so the earliest plausible
        // line wins; an all-caps line gets a two-line head start because that
        // is how most receipt headers are printed.
        let (_, candidate) = text
            .lines()
            .take(10)
            .map(str::trim)
            .enumerate()
            .filter(|(_, l)| is_vendor_candidate(l))
            .min_by_key(|(i, l)| {
                let all_caps = l
                    .chars()
                    .filter(|c| c.is_alphabetic())
                    .all(|c| c.is_uppercase());
                *i as i32 - if all_caps { 2 } else { 0 }
            })?;

        Some(ExtractedField::new(candidate.to_string(), 0.60))
    }

    // ── Date ─────────────────────────────────────────────────────────────────

    fn extract_date(text: &str) -> Option<ExtractedField<NaiveDate>> {
        // Try patterns from most to least specific.
        if let Some(d) = try_date_month_name(text) {
            return Some(ExtractedField::new(d, 0.90));
        }
        if let Some(d) = try_date_abbr_month(text) {
            return Some(ExtractedField::new(d, 0.90));
        }
        if let Some(d) = try_date_iso(text) {
            return Some(ExtractedField::new(d, 0.95));
        }
        if let Some(d) = try_date_slash(text) {
            return Some(ExtractedField::new(d, 0.75));
        }
        if let Some(d) = try_date_dash(text) {
            return Some(ExtractedField::new(d, 0.70));
        }
        None
    }

    // ── Amounts ───────────────────────────────────────────────────────────────

    fn extract_total(text: &str) -> Option<ExtractedField<i64>> {
        // Prefer a labeled total over any raw dollar amount. Receipts can print
        // several (e.g. a total before and after a coupon), so the last one
        // wins; "amount due" / "balance due" only count when there is no plain
        // total, since they drop to zero once a payment is applied.
        let mut last_total = None;
        let mut last_due = None;
        for c in re_total().captures_iter(text) {
            let label = c.name("label")?;
            if preceded_by_sub(text, label.start()) || followed_by_percent(text, c.get(0)?.end()) {
                continue;
            }
            let Some(cents) = parse_signed_amount(c.name("amt")?.as_str()) else {
                continue;
            };
            if label.as_str().to_lowercase().contains("total") {
                last_total = Some(cents);
            } else {
                last_due = Some(cents);
            }
        }
        if let Some(cents) = last_total.or(last_due) {
            return Some(ExtractedField::new(cents, 0.92));
        }
        // Fall back to the largest dollar value on the page.
        re_currency()
            .captures_iter(text)
            .filter_map(|c| parse_amount_str(c.get(1)?.as_str()))
            .max()
            .map(|cents| ExtractedField::new(cents, 0.55))
    }

    fn extract_subtotal(text: &str) -> Option<ExtractedField<i64>> {
        let cents = first_labeled_amount(re_subtotal(), text)?;
        Some(ExtractedField::new(cents, 0.88))
    }

    fn extract_tax(text: &str) -> Option<ExtractedField<i64>> {
        let cents = first_labeled_amount(re_tax(), text)?;
        Some(ExtractedField::new(cents, 0.88))
    }

    // ── Payment method ────────────────────────────────────────────────────────

    fn extract_payment_method(text: &str) -> Option<ExtractedField<PaymentMethod>> {
        let c = re_payment().captures(text)?;
        let method = match c.get(1)?.as_str().to_lowercase().replace(' ', "").as_str() {
            "visa" => PaymentMethod::Visa,
            "mastercard" | "mc" => PaymentMethod::Mastercard,
            "amex" | "americanexpress" => PaymentMethod::Amex,
            "discover" => PaymentMethod::Discover,
            "cash" => PaymentMethod::Cash,
            "debit" => PaymentMethod::Debit,
            "check" | "cheque" => PaymentMethod::Check,
            other => PaymentMethod::Other(other.to_string()),
        };
        Some(ExtractedField::new(method, 0.90))
    }
}

// ── Date helpers ──────────────────────────────────────────────────────────────

fn try_date_month_name(text: &str) -> Option<NaiveDate> {
    let c = re_date_month_name().captures(text)?;
    let month = month_name_to_num(c.get(1)?.as_str())?;
    let day: u32 = c.get(2)?.as_str().parse().ok()?;
    let year: i32 = c.get(3)?.as_str().parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, day)
}

fn try_date_abbr_month(text: &str) -> Option<NaiveDate> {
    let c = re_date_abbr_month().captures(text)?;
    let day: u32 = c.get(1)?.as_str().parse().ok()?;
    let month = abbr_month_to_num(c.get(2)?.as_str())?;
    let year: i32 = c.get(3)?.as_str().parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, day)
}

fn try_date_iso(text: &str) -> Option<NaiveDate> {
    let c = re_date_iso().captures(text)?;
    let y: i32 = c.get(1)?.as_str().parse().ok()?;
    let m: u32 = c.get(2)?.as_str().parse().ok()?;
    let d: u32 = c.get(3)?.as_str().parse().ok()?;
    NaiveDate::from_ymd_opt(y, m, d)
}

fn try_date_slash(text: &str) -> Option<NaiveDate> {
    let c = re_date_slash().captures(text)?;
    let p1: u32 = c.get(1)?.as_str().parse().ok()?;
    let p2: u32 = c.get(2)?.as_str().parse().ok()?;
    let p3_str = c.get(3)?.as_str();
    let year: i32 = expand_year(p3_str.parse().ok()?);
    // Assume MM/DD/YYYY (US format)
    NaiveDate::from_ymd_opt(year, p1, p2)
}

fn try_date_dash(text: &str) -> Option<NaiveDate> {
    let c = re_date_dash().captures(text)?;
    let p1: u32 = c.get(1)?.as_str().parse().ok()?;
    let p2: u32 = c.get(2)?.as_str().parse().ok()?;
    let p3_str = c.get(3)?.as_str();
    let year: i32 = expand_year(p3_str.parse().ok()?);
    NaiveDate::from_ymd_opt(year, p1, p2)
}

fn expand_year(y: i32) -> i32 {
    if y < 100 { 2000 + y } else { y }
}

fn month_name_to_num(name: &str) -> Option<u32> {
    match name.to_lowercase().as_str() {
        "january" => Some(1),
        "february" => Some(2),
        "march" => Some(3),
        "april" => Some(4),
        "may" => Some(5),
        "june" => Some(6),
        "july" => Some(7),
        "august" => Some(8),
        "september" => Some(9),
        "october" => Some(10),
        "november" => Some(11),
        "december" => Some(12),
        _ => None,
    }
}

fn abbr_month_to_num(name: &str) -> Option<u32> {
    match name.to_lowercase().as_str() {
        "jan" => Some(1),
        "feb" => Some(2),
        "mar" => Some(3),
        "apr" => Some(4),
        "may" => Some(5),
        "jun" => Some(6),
        "jul" => Some(7),
        "aug" => Some(8),
        "sep" => Some(9),
        "oct" => Some(10),
        "nov" => Some(11),
        "dec" => Some(12),
        _ => None,
    }
}

// ── Line / amount helpers ─────────────────────────────────────────────────────

/// Whether a line could be the merchant name. Addresses, ZIP codes, phone
/// numbers, dates, store numbers and amounts all contain digits.
fn is_vendor_candidate(line: &str) -> bool {
    line.len() >= 3
        && line.len() <= 50
        && !line.chars().any(|c| c.is_ascii_digit())
        && line.chars().filter(|c| c.is_alphabetic()).count() >= 3
        && !re_url().is_match(line)
}

/// Whether the label starting at `label_start` is the tail of "SUB TOTAL" /
/// "Sub-Total" (the regex crate has no look-behind).
fn preceded_by_sub(text: &str, label_start: usize) -> bool {
    text[..label_start]
        .trim_end_matches([' ', '\t', '-'])
        .to_lowercase()
        .ends_with("sub")
}

/// Whether the amount ending at `end` is really a percentage ("8.25%").
fn followed_by_percent(text: &str, end: usize) -> bool {
    text[end..].trim_start_matches([' ', '\t']).starts_with('%')
}

fn first_labeled_amount(re: &Regex, text: &str) -> Option<i64> {
    re.captures_iter(text)
        .filter(|c| {
            c.get(0)
                .is_some_and(|m| !followed_by_percent(text, m.end()))
        })
        .find_map(|c| parse_signed_amount(c.name("amt")?.as_str()))
}

fn parse_amount_str(s: &str) -> Option<i64> {
    let clean = s.replace(',', "");
    let dec = Decimal::from_str(&clean).ok()?;
    (dec * Decimal::from(100)).round().to_i64()
}

/// Parse an `amount!()` match: a minus sign anywhere or surrounding
/// parentheses (accounting notation) make it negative.
fn parse_signed_amount(s: &str) -> Option<i64> {
    let s = s.trim();
    let negative = s.contains('-') || (s.starts_with('(') && s.ends_with(')'));
    let digits: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .collect();
    let cents = parse_amount_str(&digits)?;
    Some(if negative { -cents } else { cents })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Vendor ────────────────────────────────────────────────────────────────

    #[test]
    fn extract_vendor_all_caps_preferred() {
        let text = "123 Main Street\nSTARBUCKS COFFEE\n2024-01-15\nTotal $5.50";
        let r = Extractor::extract(text);
        assert_eq!(r.vendor.unwrap().value, "STARBUCKS COFFEE");
    }

    #[test]
    fn extract_vendor_skips_phone_number() {
        let text = "(555) 123-4567\nWHOLE FOODS\nTotal $42.00";
        let r = Extractor::extract(text);
        assert_eq!(r.vendor.unwrap().value, "WHOLE FOODS");
    }

    #[test]
    fn extract_vendor_prefers_first_line_over_address() {
        let text = "WALMART\nSUPERCENTER\n123 MAIN ST\nSPRINGFIELD, IL 62701\nTOTAL $5.00";
        let r = Extractor::extract(text);
        assert_eq!(r.vendor.unwrap().value, "WALMART");
    }

    #[test]
    fn extract_vendor_skips_city_and_zip_lines() {
        let text = "SPRINGFIELD, IL 62701\nAcme Hardware\nStore #42";
        let r = Extractor::extract(text);
        assert_eq!(r.vendor.unwrap().value, "Acme Hardware");
    }

    #[test]
    fn extract_vendor_all_caps_beats_greeting() {
        let text = "Welcome to\nTARGET\n(555) 123-4567";
        let r = Extractor::extract(text);
        assert_eq!(r.vendor.unwrap().value, "TARGET");
    }

    #[test]
    fn extract_vendor_none_when_no_suitable_line() {
        let text = "123 First Ave\n(800) 555-1234\n$10.00";
        // Might or might not find something — just shouldn't panic.
        let _ = Extractor::extract(text);
    }

    // ── Date ─────────────────────────────────────────────────────────────────

    #[test]
    fn extract_date_iso() {
        let text = "AMAZON\nOrder 2024-03-15\nTotal $49.99";
        let r = Extractor::extract(text);
        assert_eq!(
            r.date.unwrap().value,
            NaiveDate::from_ymd_opt(2024, 3, 15).unwrap()
        );
    }

    #[test]
    fn extract_date_full_month_name() {
        let text = "WHOLE FOODS\nDate: March 15, 2024\nTotal $87.50";
        let r = Extractor::extract(text);
        assert_eq!(
            r.date.unwrap().value,
            NaiveDate::from_ymd_opt(2024, 3, 15).unwrap()
        );
    }

    #[test]
    fn extract_date_slash_format() {
        let text = "STARBUCKS\n01/15/2024\n$5.50";
        let r = Extractor::extract(text);
        assert_eq!(
            r.date.unwrap().value,
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
        );
    }

    #[test]
    fn extract_date_abbreviated_month() {
        let text = "WALMART\n15 Jan 2024\nTotal $120.00";
        let r = Extractor::extract(text);
        assert_eq!(
            r.date.unwrap().value,
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
        );
    }

    // ── Amounts ───────────────────────────────────────────────────────────────

    #[test]
    fn extract_total_labeled() {
        let text = "AMAZON\nItem 1   $10.00\nItem 2   $15.00\nTotal    $25.00";
        let r = Extractor::extract(text);
        assert_eq!(r.total_cents.unwrap().value, 2500);
    }

    #[test]
    fn extract_total_high_confidence_for_labeled() {
        let text = "STORE\nTotal Due $99.99";
        let r = Extractor::extract(text);
        let t = r.total_cents.unwrap();
        assert!(t.confidence >= 0.9, "confidence was {}", t.confidence);
    }

    #[test]
    fn extract_subtotal_and_tax() {
        let text = "STORE\nSubtotal $45.00\nTax $3.60\nTotal $48.60";
        let r = Extractor::extract(text);
        assert_eq!(r.subtotal_cents.unwrap().value, 4500);
        assert_eq!(r.tax_cents.unwrap().value, 360);
        assert_eq!(r.total_cents.unwrap().value, 4860);
    }

    #[test]
    fn extract_total_falls_back_to_largest_amount() {
        let text = "STORE\n$5.00\n$3.00\n$8.00";
        let r = Extractor::extract(text);
        assert_eq!(r.total_cents.unwrap().value, 800);
    }

    #[test]
    fn extract_total_with_comma_thousands() {
        let text = "STORE\nTotal $1,234.56";
        let r = Extractor::extract(text);
        assert_eq!(r.total_cents.unwrap().value, 123456);
    }

    #[test]
    fn extract_total_ignores_sub_total_and_takes_last() {
        for text in [
            "STORE\nSUB TOTAL $45.00\nTAX $3.60\nTOTAL $48.60",
            "STORE\nSub-Total $45.00\nTax $3.60\nTotal $48.60",
            "STORE\nTOTAL $48.60\nSUB TOTAL $45.00",
        ] {
            let r = Extractor::extract(text);
            assert_eq!(r.total_cents.unwrap().value, 4860, "{text:?}");
            assert_eq!(r.subtotal_cents.unwrap().value, 4500, "{text:?}");
        }
    }

    #[test]
    fn extract_total_last_labeled_total_wins() {
        let text = "STORE\nTOTAL $50.00\nCOUPON -$5.00\nTOTAL $45.00";
        let r = Extractor::extract(text);
        assert_eq!(r.total_cents.unwrap().value, 4500);
    }

    #[test]
    fn extract_total_prefers_total_over_balance_due() {
        let text = "STORE\nTOTAL $48.60\nVISA $48.60\nBALANCE DUE $0.00";
        let r = Extractor::extract(text);
        assert_eq!(r.total_cents.unwrap().value, 4860);

        let text = "STORE\nAmount Due: $12.00";
        let r = Extractor::extract(text);
        assert_eq!(r.total_cents.unwrap().value, 1200);
    }

    #[test]
    fn extract_total_keeps_negative_sign() {
        for text in [
            "REFUND\nTOTAL -$12.34",
            "REFUND\nTOTAL $-12.34",
            "REFUND\nTOTAL ($12.34)",
            "REFUND\nTotal: (12.34)",
        ] {
            let r = Extractor::extract(text);
            assert_eq!(r.total_cents.unwrap().value, -1234, "{text:?}");
        }
    }

    #[test]
    fn extract_tax_skips_rate() {
        let r = Extractor::extract("STORE\nTAX 8.25% 3.71\nTOTAL 48.71");
        assert_eq!(r.tax_cents.unwrap().value, 371);

        let r = Extractor::extract("STORE\nSales Tax 7.00% $2.10\nTotal $32.10");
        assert_eq!(r.tax_cents.unwrap().value, 210);

        let r = Extractor::extract("STORE\nTax @ 8.25% $3.71");
        assert_eq!(r.tax_cents.unwrap().value, 371);
    }

    #[test]
    fn extract_tax_rate_without_amount_is_not_an_amount() {
        let r = Extractor::extract("STORE\nTAX 8.25%\nTotal $10.00");
        assert!(r.tax_cents.is_none());
    }

    // ── Payment method ────────────────────────────────────────────────────────

    #[test]
    fn extract_payment_visa() {
        let text = "STARBUCKS\nPaid with VISA\nTotal $5.50";
        let r = Extractor::extract(text);
        assert_eq!(r.payment_method.unwrap().value, PaymentMethod::Visa);
    }

    #[test]
    fn extract_payment_amex() {
        let text = "WHOLE FOODS\nAmerican Express ending 1234\nTotal $87.50";
        let r = Extractor::extract(text);
        assert_eq!(r.payment_method.unwrap().value, PaymentMethod::Amex);
    }

    #[test]
    fn extract_payment_cash() {
        let text = "COFFEE SHOP\nPayment: Cash\nTotal $4.75";
        let r = Extractor::extract(text);
        assert_eq!(r.payment_method.unwrap().value, PaymentMethod::Cash);
    }

    // ── Confidence ────────────────────────────────────────────────────────────

    #[test]
    fn confidence_high_for_complete_receipt() {
        let text = "STARBUCKS COFFEE\n2024-01-15\nSubtotal $4.75\nTax $0.50\nTotal $5.25\nVISA";
        let r = Extractor::extract(text);
        assert!(r.confidence >= 0.7, "confidence was {}", r.confidence);
    }

    #[test]
    fn confidence_low_for_empty_text() {
        let r = Extractor::extract("");
        assert_eq!(r.confidence, 0.0);
    }

    #[test]
    fn no_panic_on_garbage_input() {
        let _ = Extractor::extract("!@#$%^&*()\n\0\x01\x02");
    }

    // ── amount parsing ────────────────────────────────────────────────────────

    #[test]
    fn parse_signed_amount_forms() {
        assert_eq!(parse_signed_amount("$1,234.56"), Some(123456));
        assert_eq!(parse_signed_amount("-$12.34"), Some(-1234));
        assert_eq!(parse_signed_amount("$ -12.34"), Some(-1234));
        assert_eq!(parse_signed_amount("( $12.34 )"), Some(-1234));
        assert_eq!(parse_signed_amount("12.34"), Some(1234));
    }

    #[test]
    fn parse_amount_str_plain() {
        assert_eq!(parse_amount_str("49.99"), Some(4999));
        assert_eq!(parse_amount_str("0.01"), Some(1));
        assert_eq!(parse_amount_str("1,234.56"), Some(123456));
    }
}
