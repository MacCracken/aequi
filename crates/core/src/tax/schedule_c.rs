use serde::{Deserialize, Serialize};
use std::fmt;

/// Schedule C line items for IRS Form 1040 Schedule C (Profit or Loss From Business).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ScheduleCLine {
    /// Line 1: Gross receipts or sales
    Line1,
    /// Line 2: Returns and allowances
    Line2,
    /// Line 6: Other income
    Line6,
    /// Line 8: Advertising
    Line8,
    /// Line 14: Employee benefit programs
    Line14,
    /// Line 15: Insurance (other than health)
    Line15,
    /// Line 17: Legal and professional services
    Line17,
    /// Line 18: Office expense
    Line18,
    /// Line 24a: Travel
    Line24a,
    /// Line 24b: Deductible meals
    Line24b,
    /// Line 27: Other expenses
    Line27,
    /// Line 30: Business use of home
    Line30,
}

impl fmt::Display for ScheduleCLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

impl ScheduleCLine {
    /// Parse from the string format used in DEFAULT_ACCOUNTS (e.g., "line_1", "line_24b").
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "line_1" => Some(Self::Line1),
            "line_2" => Some(Self::Line2),
            "line_6" => Some(Self::Line6),
            "line_8" => Some(Self::Line8),
            "line_14" => Some(Self::Line14),
            "line_15" => Some(Self::Line15),
            "line_17" => Some(Self::Line17),
            "line_18" => Some(Self::Line18),
            "line_24a" => Some(Self::Line24a),
            "line_24b" => Some(Self::Line24b),
            "line_27" => Some(Self::Line27),
            "line_30" => Some(Self::Line30),
            _ => None,
        }
    }

    /// Human-readable label for display.
    pub fn label(self) -> &'static str {
        match self {
            Self::Line1 => "Line 1 — Gross receipts or sales",
            Self::Line2 => "Line 2 — Returns and allowances",
            Self::Line6 => "Line 6 — Other income",
            Self::Line8 => "Line 8 — Advertising",
            Self::Line14 => "Line 14 — Employee benefit programs",
            Self::Line15 => "Line 15 — Insurance",
            Self::Line17 => "Line 17 — Legal and professional services",
            Self::Line18 => "Line 18 — Office expense",
            Self::Line24a => "Line 24a — Travel",
            Self::Line24b => "Line 24b — Deductible meals",
            Self::Line27 => "Line 27 — Other expenses",
            Self::Line30 => "Line 30 — Business use of home",
        }
    }

    /// Whether this line is an income line (vs expense).
    pub fn is_income(self) -> bool {
        matches!(self, Self::Line1 | Self::Line2 | Self::Line6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_tag_roundtrip() {
        let tags = [
            "line_1", "line_2", "line_6", "line_8", "line_14", "line_15", "line_17", "line_18",
            "line_24a", "line_24b", "line_27", "line_30",
        ];
        for tag in tags {
            assert!(
                ScheduleCLine::from_tag(tag).is_some(),
                "Failed to parse: {tag}"
            );
        }
    }

    #[test]
    fn from_tag_invalid() {
        assert!(ScheduleCLine::from_tag("line_99").is_none());
        assert!(ScheduleCLine::from_tag("").is_none());
        assert!(ScheduleCLine::from_tag("bogus").is_none());
    }

    #[test]
    fn income_vs_expense() {
        assert!(ScheduleCLine::Line1.is_income());
        assert!(ScheduleCLine::Line2.is_income());
        assert!(ScheduleCLine::Line6.is_income());
        assert!(!ScheduleCLine::Line8.is_income());
        assert!(!ScheduleCLine::Line24b.is_income());
    }

    #[test]
    fn display() {
        assert!(ScheduleCLine::Line1.to_string().contains("Gross receipts"));
    }

    #[test]
    fn default_accounts_all_parse() {
        use crate::DEFAULT_ACCOUNTS;
        for (code, _, _, tag) in DEFAULT_ACCOUNTS {
            if !tag.is_empty() {
                assert!(
                    ScheduleCLine::from_tag(tag).is_some(),
                    "Account {code} has unparseable schedule_c_line: {tag}"
                );
            }
        }
    }

    #[test]
    fn all_expense_lines_are_not_income() {
        let expense_lines = [
            ScheduleCLine::Line8,
            ScheduleCLine::Line14,
            ScheduleCLine::Line15,
            ScheduleCLine::Line17,
            ScheduleCLine::Line18,
            ScheduleCLine::Line24a,
            ScheduleCLine::Line24b,
            ScheduleCLine::Line27,
            ScheduleCLine::Line30,
        ];
        for line in expense_lines {
            assert!(
                !line.is_income(),
                "{:?} should not be income",
                line
            );
        }
    }

    #[test]
    fn all_income_lines_are_income() {
        let income_lines = [
            ScheduleCLine::Line1,
            ScheduleCLine::Line2,
            ScheduleCLine::Line6,
        ];
        for line in income_lines {
            assert!(line.is_income(), "{:?} should be income", line);
        }
    }

    #[test]
    fn display_all_variants() {
        let all = [
            ScheduleCLine::Line1,
            ScheduleCLine::Line2,
            ScheduleCLine::Line6,
            ScheduleCLine::Line8,
            ScheduleCLine::Line14,
            ScheduleCLine::Line15,
            ScheduleCLine::Line17,
            ScheduleCLine::Line18,
            ScheduleCLine::Line24a,
            ScheduleCLine::Line24b,
            ScheduleCLine::Line27,
            ScheduleCLine::Line30,
        ];
        for line in all {
            let s = line.to_string();
            assert!(!s.is_empty(), "{:?} has empty display", line);
            assert!(s.contains("Line"), "{:?} display should contain 'Line': {s}", line);
        }
    }

    #[test]
    fn label_contains_description() {
        assert!(ScheduleCLine::Line8.label().contains("Advertising"));
        assert!(ScheduleCLine::Line14.label().contains("Employee benefit"));
        assert!(ScheduleCLine::Line15.label().contains("Insurance"));
        assert!(ScheduleCLine::Line17.label().contains("Legal"));
        assert!(ScheduleCLine::Line18.label().contains("Office"));
        assert!(ScheduleCLine::Line24a.label().contains("Travel"));
        assert!(ScheduleCLine::Line24b.label().contains("meals"));
        assert!(ScheduleCLine::Line27.label().contains("Other"));
        assert!(ScheduleCLine::Line30.label().contains("home"));
    }

    #[test]
    fn from_tag_case_sensitive() {
        // Tags are case-sensitive — uppercase should not match
        assert!(ScheduleCLine::from_tag("LINE_1").is_none());
        assert!(ScheduleCLine::from_tag("Line_1").is_none());
    }

    #[test]
    fn from_tag_partial_match_rejected() {
        assert!(ScheduleCLine::from_tag("line_").is_none());
        assert!(ScheduleCLine::from_tag("line_24").is_none());
        assert!(ScheduleCLine::from_tag("line_24c").is_none());
    }

    #[test]
    fn ordering() {
        // ScheduleCLine derives Ord — verify ordering makes sense
        assert!(ScheduleCLine::Line1 < ScheduleCLine::Line2);
        assert!(ScheduleCLine::Line2 < ScheduleCLine::Line6);
        assert!(ScheduleCLine::Line24a < ScheduleCLine::Line24b);
        assert!(ScheduleCLine::Line27 < ScheduleCLine::Line30);
    }

    #[test]
    fn serialize_deserialize_roundtrip() {
        let line = ScheduleCLine::Line24a;
        let json = serde_json::to_string(&line).unwrap();
        let deserialized: ScheduleCLine = serde_json::from_str(&json).unwrap();
        assert_eq!(line, deserialized);
    }

    #[test]
    fn serialize_all_variants() {
        let all = [
            ScheduleCLine::Line1,
            ScheduleCLine::Line2,
            ScheduleCLine::Line6,
            ScheduleCLine::Line8,
            ScheduleCLine::Line14,
            ScheduleCLine::Line15,
            ScheduleCLine::Line17,
            ScheduleCLine::Line18,
            ScheduleCLine::Line24a,
            ScheduleCLine::Line24b,
            ScheduleCLine::Line27,
            ScheduleCLine::Line30,
        ];
        for line in all {
            let json = serde_json::to_string(&line).unwrap();
            let back: ScheduleCLine = serde_json::from_str(&json).unwrap();
            assert_eq!(line, back, "Roundtrip failed for {:?}", line);
        }
    }

    #[test]
    fn clone_and_copy() {
        let line = ScheduleCLine::Line18;
        let cloned = line.clone();
        let copied = line;
        assert_eq!(line, cloned);
        assert_eq!(line, copied);
    }

    #[test]
    fn hash_works_in_hashset() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(ScheduleCLine::Line1);
        set.insert(ScheduleCLine::Line1);
        set.insert(ScheduleCLine::Line8);
        assert_eq!(set.len(), 2);
    }
}
