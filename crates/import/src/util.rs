use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use std::str::FromStr;

/// Levenshtein edit distance over Unicode characters (not bytes), using the
/// two-row O(min(m,n)) space algorithm.
pub fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let a: Vec<char> = s1.chars().collect();
    let b: Vec<char> = s2.chars().collect();
    let (m, n) = (a.len(), b.len());

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    // Keep the shorter string in the inner loop to minimise allocation.
    let (a, b, m, n) = if m <= n { (a, b, m, n) } else { (b, a, n, m) };

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

/// Edit-distance similarity in `[0.0, 1.0]`, normalised by the longer string's
/// character count. Two empty strings are identical.
pub fn similarity(s1: &str, s2: &str) -> f32 {
    let max_len = s1.chars().count().max(s2.chars().count());
    if max_len == 0 {
        return 1.0;
    }
    1.0 - (levenshtein_distance(s1, s2) as f32 / max_len as f32)
}

/// Shorten `s` to at most `max_bytes` bytes without splitting a UTF-8 character.
pub fn truncate_on_char_boundary(s: &mut String, max_bytes: usize) {
    if s.len() > max_bytes {
        let cut = s.floor_char_boundary(max_bytes);
        s.truncate(cut);
    }
}

/// Parse a human-formatted money amount into integer cents.
///
/// Accepts surrounding whitespace, a leading `+`/`-`, a leading `$`, accounting
/// parentheses for negatives (`(50.00)`) and thousands separators
/// (`1,234.56`). A comma is read as the decimal point only when there is no
/// `.` and exactly one `,` followed by one or two digits (`-49,99`). Any other
/// mix of separators, stray characters or an empty value is rejected instead
/// of guessed. Sub-cent digits are rounded half away from zero.
pub fn parse_amount_cents(input: &str) -> Option<i64> {
    let compact: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let mut s = compact.as_str();

    let mut negative = false;
    if let Some(inner) = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        negative = true;
        s = inner;
    }

    let mut signed = false;
    let mut take_sign = |s: &mut &str| -> Option<()> {
        if let Some(rest) = s.strip_prefix('-') {
            if signed {
                return None;
            }
            signed = true;
            negative = !negative;
            *s = rest;
        } else if let Some(rest) = s.strip_prefix('+') {
            if signed {
                return None;
            }
            signed = true;
            *s = rest;
        }
        Some(())
    };
    take_sign(&mut s)?;
    if let Some(rest) = s.strip_prefix('$') {
        s = rest;
        take_sign(&mut s)?;
    }
    // "(-5.00)" is contradictory rather than doubly negative.
    if signed && compact.starts_with('(') {
        return None;
    }

    if !s.bytes().any(|b| b.is_ascii_digit())
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.' || b == b',')
    {
        return None;
    }

    let normalized = normalize_separators(s)?;
    let value = Decimal::from_str(&normalized).ok()?;
    let cents = value
        .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
        .checked_mul(Decimal::ONE_HUNDRED)?
        .to_i64()?;
    Some(if negative { -cents } else { cents })
}

/// Turn a digits-and-separators string into `digits[.digits]`, or `None` when
/// the separators are ambiguous or malformed.
fn normalize_separators(s: &str) -> Option<String> {
    let dots = s.matches('.').count();
    let commas = s.matches(',').count();
    if dots > 1 {
        return None;
    }
    if commas == 0 {
        return Some(s.to_string());
    }

    // Decimal comma: "49,99" / "0,5".
    if dots == 0
        && commas == 1
        && let Some((int, frac)) = s.split_once(',')
        && (1..=2).contains(&frac.len())
    {
        return Some(format!("{int}.{frac}"));
    }

    // Otherwise every comma must be a thousands separator in the integer part.
    let (int, frac) = match s.split_once('.') {
        Some((int, frac)) => (int, Some(frac)),
        None => (s, None),
    };
    if frac.is_some_and(|f| f.contains(',')) {
        return None;
    }
    let mut groups = int.split(',');
    let first = groups.next()?;
    if !(1..=3).contains(&first.len()) {
        return None;
    }
    let mut out = first.to_string();
    for group in groups {
        if group.len() != 3 {
            return None;
        }
        out.push_str(group);
    }
    if let Some(frac) = frac {
        out.push('.');
        out.push_str(frac);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_strings_are_zero() {
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
        assert_eq!(levenshtein_distance("", ""), 0);
    }

    #[test]
    fn empty_string_is_length_of_other() {
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
    }

    #[test]
    fn single_substitution() {
        assert_eq!(levenshtein_distance("cat", "bat"), 1);
    }

    #[test]
    fn single_insertion() {
        assert_eq!(levenshtein_distance("abc", "abcd"), 1);
    }

    #[test]
    fn single_deletion() {
        assert_eq!(levenshtein_distance("abcd", "abc"), 1);
    }

    #[test]
    fn commutative() {
        assert_eq!(
            levenshtein_distance("amazon", "amzn"),
            levenshtein_distance("amzn", "amazon")
        );
    }

    #[test]
    fn non_ascii_counts_characters_not_bytes() {
        // 'é' is two bytes in UTF-8 but a single edit.
        assert_eq!(levenshtein_distance("café", "cafe"), 1);
        assert_eq!(levenshtein_distance("müller", "muller"), 1);
        assert!((similarity("café", "cafe") - 0.75).abs() < 1e-6);
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let mut s = "aé".repeat(300);
        truncate_on_char_boundary(&mut s, 500);
        assert!(s.len() <= 500);
        assert!(s.is_char_boundary(s.len()));

        let mut short = "héllo".to_string();
        truncate_on_char_boundary(&mut short, 500);
        assert_eq!(short, "héllo");

        // Cutting inside 'é' (bytes 1..3) backs off to byte 1.
        let mut cut = "héllo".to_string();
        truncate_on_char_boundary(&mut cut, 2);
        assert_eq!(cut, "h");
    }

    #[test]
    fn amount_plain_and_signed() {
        assert_eq!(parse_amount_cents("123.45"), Some(12345));
        assert_eq!(parse_amount_cents("-49.99"), Some(-4999));
        assert_eq!(parse_amount_cents("+49.99"), Some(4999));
        assert_eq!(parse_amount_cents("100"), Some(10000));
        assert_eq!(parse_amount_cents(".5"), Some(50));
        assert_eq!(parse_amount_cents("  7.10 "), Some(710));
    }

    #[test]
    fn amount_currency_and_parentheses() {
        assert_eq!(parse_amount_cents("$1,234.56"), Some(123456));
        assert_eq!(parse_amount_cents("-$5.00"), Some(-500));
        assert_eq!(parse_amount_cents("$-5.00"), Some(-500));
        assert_eq!(parse_amount_cents("(50.00)"), Some(-5000));
        assert_eq!(parse_amount_cents("($50.00)"), Some(-5000));
        assert_eq!(parse_amount_cents("(-50.00)"), None);
        assert_eq!(parse_amount_cents("--5"), None);
        assert_eq!(parse_amount_cents("-$-5"), None);
    }

    #[test]
    fn amount_decimal_comma() {
        assert_eq!(parse_amount_cents("-49,99"), Some(-4999));
        assert_eq!(parse_amount_cents("0,5"), Some(50));
        // Three digits after a lone comma is a thousands separator.
        assert_eq!(parse_amount_cents("1,234"), Some(123400));
        assert_eq!(parse_amount_cents("1,234,567.89"), Some(123456789));
    }

    #[test]
    fn amount_ambiguous_separators_rejected() {
        assert_eq!(parse_amount_cents("1.234,56"), None);
        assert_eq!(parse_amount_cents("12,34,567"), None);
        assert_eq!(parse_amount_cents("1,2345.00"), None);
        assert_eq!(parse_amount_cents("1.2.3"), None);
        assert_eq!(parse_amount_cents("1,23,4"), None);
    }

    #[test]
    fn amount_garbage_rejected() {
        assert_eq!(parse_amount_cents(""), None);
        assert_eq!(parse_amount_cents("abc"), None);
        assert_eq!(parse_amount_cents("12abc"), None);
        assert_eq!(parse_amount_cents("1_000"), None);
        assert_eq!(parse_amount_cents("-"), None);
        assert_eq!(parse_amount_cents("."), None);
        assert_eq!(parse_amount_cents("49.99</TRNAMT>"), None);
    }

    #[test]
    fn amount_rounds_half_away_from_zero() {
        assert_eq!(parse_amount_cents("0.005"), Some(1));
        assert_eq!(parse_amount_cents("-0.005"), Some(-1));
        assert_eq!(parse_amount_cents("2.345"), Some(235));
    }

    #[test]
    fn amount_out_of_range_rejected() {
        assert_eq!(parse_amount_cents("99999999999999999999999"), None);
    }
}
