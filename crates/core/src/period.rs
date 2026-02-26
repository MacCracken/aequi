use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiscalYear(u16);

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

    pub fn end_date(self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.0 as i32 + 1, 1, 1).unwrap()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quarter {
    Q1,
    Q2,
    Q3,
    Q4,
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
    start: NaiveDate,
    end: NaiveDate,
}

impl DateRange {
    pub fn new(start: NaiveDate, end: NaiveDate) -> Self {
        DateRange { start, end }
    }

    pub fn start(self) -> NaiveDate {
        self.start
    }

    pub fn end(self) -> NaiveDate {
        self.end
    }

    pub fn contains(self, date: NaiveDate) -> bool {
        date >= self.start && date <= self.end
    }
}
