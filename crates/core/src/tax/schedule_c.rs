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
}
