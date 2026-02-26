use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiscalYear(pub u16);

impl fmt::Display for FiscalYear {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FY{}", self.0)
    }
}

impl FiscalYear {
    pub fn new(year: u16) -> Self {
        FiscalYear(year)
    }

    pub fn year(self) -> u16 {
        self.0
    }

    pub fn start_date(self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.0 as i32, 1, 1).unwrap()
    }

    /// Returns December 31 of this fiscal year (inclusive end, matching Quarter::end_date).
    pub fn end_date(self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.0 as i32, 12, 31).unwrap()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quarter {
    Q1,
    Q2,
    Q3,
    Q4,
}

impl fmt::Display for Quarter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Quarter::Q1 => write!(f, "Q1"),
            Quarter::Q2 => write!(f, "Q2"),
            Quarter::Q3 => write!(f, "Q3"),
            Quarter::Q4 => write!(f, "Q4"),
        }
    }
}

impl Quarter {
    pub fn new(n: u8) -> Option<Self> {
        match n {
            1 => Some(Quarter::Q1),
            2 => Some(Quarter::Q2),
            3 => Some(Quarter::Q3),
            4 => Some(Quarter::Q4),
            _ => None,
        }
    }

    pub fn start_date(self, year: FiscalYear) -> NaiveDate {
        match self {
            Quarter::Q1 => NaiveDate::from_ymd_opt(year.year() as i32, 1, 1).unwrap(),
            Quarter::Q2 => NaiveDate::from_ymd_opt(year.year() as i32, 4, 1).unwrap(),
            Quarter::Q3 => NaiveDate::from_ymd_opt(year.year() as i32, 7, 1).unwrap(),
            Quarter::Q4 => NaiveDate::from_ymd_opt(year.year() as i32, 10, 1).unwrap(),
        }
    }

    pub fn end_date(self, year: FiscalYear) -> NaiveDate {
        match self {
            Quarter::Q1 => NaiveDate::from_ymd_opt(year.year() as i32, 3, 31).unwrap(),
            Quarter::Q2 => NaiveDate::from_ymd_opt(year.year() as i32, 6, 30).unwrap(),
            Quarter::Q3 => NaiveDate::from_ymd_opt(year.year() as i32, 9, 30).unwrap(),
            Quarter::Q4 => NaiveDate::from_ymd_opt(year.year() as i32, 12, 31).unwrap(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateRange {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl fmt::Display for DateRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} to {}", self.start, self.end)
    }
}

impl DateRange {
    pub fn new(start: NaiveDate, end: NaiveDate) -> Self {
        DateRange { start, end }
    }

    pub fn contains(self, date: NaiveDate) -> bool {
        date >= self.start && date <= self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    #[test]
    fn fiscal_year_display() {
        assert_eq!(FiscalYear::new(2024).to_string(), "FY2024");
    }

    #[test]
    fn fiscal_year_start_date() {
        let fy = FiscalYear::new(2024);
        assert_eq!(fy.start_date(), NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
    }

    #[test]
    fn fiscal_year_end_date_is_dec_31() {
        let fy = FiscalYear::new(2024);
        assert_eq!(fy.end_date(), NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
    }

    #[test]
    fn fiscal_year_start_and_end_are_same_year() {
        let fy = FiscalYear::new(2024);
        assert_eq!(fy.start_date().year(), 2024);
        assert_eq!(fy.end_date().year(), 2024);
    }

    #[test]
    fn quarter_new_valid_and_invalid() {
        assert_eq!(Quarter::new(1), Some(Quarter::Q1));
        assert_eq!(Quarter::new(4), Some(Quarter::Q4));
        assert_eq!(Quarter::new(0), None);
        assert_eq!(Quarter::new(5), None);
    }

    #[test]
    fn quarter_display() {
        assert_eq!(Quarter::Q1.to_string(), "Q1");
        assert_eq!(Quarter::Q4.to_string(), "Q4");
    }

    #[test]
    fn quarter_start_dates() {
        let fy = FiscalYear::new(2024);
        assert_eq!(Quarter::Q1.start_date(fy), NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        assert_eq!(Quarter::Q2.start_date(fy), NaiveDate::from_ymd_opt(2024, 4, 1).unwrap());
        assert_eq!(Quarter::Q3.start_date(fy), NaiveDate::from_ymd_opt(2024, 7, 1).unwrap());
        assert_eq!(Quarter::Q4.start_date(fy), NaiveDate::from_ymd_opt(2024, 10, 1).unwrap());
    }

    #[test]
    fn quarter_end_dates() {
        let fy = FiscalYear::new(2024);
        assert_eq!(Quarter::Q1.end_date(fy), NaiveDate::from_ymd_opt(2024, 3, 31).unwrap());
        assert_eq!(Quarter::Q2.end_date(fy), NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());
        assert_eq!(Quarter::Q3.end_date(fy), NaiveDate::from_ymd_opt(2024, 9, 30).unwrap());
        assert_eq!(Quarter::Q4.end_date(fy), NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
    }

    #[test]
    fn quarter_covers_full_year() {
        let fy = FiscalYear::new(2024);
        assert_eq!(Quarter::Q1.start_date(fy), fy.start_date());
        assert_eq!(Quarter::Q4.end_date(fy), fy.end_date());
    }

    #[test]
    fn date_range_contains() {
        let range = DateRange::new(
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
        );
        assert!(range.contains(NaiveDate::from_ymd_opt(2024, 6, 15).unwrap()));
        assert!(range.contains(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())); // inclusive start
        assert!(range.contains(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap())); // inclusive end
        assert!(!range.contains(NaiveDate::from_ymd_opt(2023, 12, 31).unwrap()));
        assert!(!range.contains(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()));
    }

    #[test]
    fn date_range_display() {
        let range = DateRange::new(
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
        );
        assert_eq!(range.to_string(), "2024-01-01 to 2024-12-31");
    }
}
