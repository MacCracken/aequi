use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;
use thiserror::Error;

/// Errors loading or validating tax rules.
#[derive(Debug, Error)]
pub enum TaxRulesError {
    #[error("Failed to parse TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("Invalid tax year: {0}")]
    InvalidYear(u16),
    #[error("Missing required field: {0}")]
    MissingField(String),
}

/// Tax rules loaded from a TOML file (e.g., `rules/tax/us/2026.toml`).
#[derive(Debug, Clone, Deserialize)]
pub struct TaxRules {
    pub year: YearField,
    pub se_tax: SeTaxRules,
    pub income_brackets: IncomeBrackets,
    pub mileage: MileageRules,
    pub meals_deduction_cap: MealsDeductionCap,
    pub home_office_simplified: HomeOfficeSimplified,
    pub quarterly_due_dates: QuarterlyDueDates,
}

#[derive(Debug, Clone, Deserialize)]
pub struct YearField {
    pub value: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeTaxRules {
    pub ss_rate: Decimal,
    pub medicare_rate: Decimal,
    pub ss_wage_base: i64,
    pub net_earnings_factor: Decimal,
    pub deductible_fraction: Decimal,
}

impl SeTaxRules {
    /// Combined SE tax rate (SS + Medicare).
    pub fn combined_rate(&self) -> Decimal {
        self.ss_rate + self.medicare_rate
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct IncomeBrackets {
    pub single: Vec<Bracket>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Bracket {
    pub floor: Decimal,
    pub ceiling: Decimal,
    pub rate: Decimal,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MileageRules {
    pub business_cents_per_mile: i64,
    pub medical_cents_per_mile: i64,
    pub charity_cents_per_mile: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MealsDeductionCap {
    pub fraction: Decimal,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HomeOfficeSimplified {
    pub rate_per_sqft: Decimal,
    pub max_sqft: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuarterlyDueDates {
    pub q1: String,
    pub q2: String,
    pub q3: String,
    pub q4: String,
}

impl QuarterlyDueDates {
    pub fn due_date(&self, quarter: crate::Quarter) -> NaiveDate {
        let s = match quarter {
            crate::Quarter::Q1 => &self.q1,
            crate::Quarter::Q2 => &self.q2,
            crate::Quarter::Q3 => &self.q3,
            crate::Quarter::Q4 => &self.q4,
        };
        NaiveDate::from_str(s).unwrap_or_else(|_| panic!("Invalid due date in tax rules: {s}"))
    }
}

impl TaxRules {
    /// Load tax rules from a TOML string.
    pub fn from_toml(toml_str: &str) -> Result<Self, TaxRulesError> {
        let rules: TaxRules = toml::from_str(toml_str)?;
        if rules.year.value < 2020 || rules.year.value > 2100 {
            return Err(TaxRulesError::InvalidYear(rules.year.value));
        }
        if rules.income_brackets.single.is_empty() {
            return Err(TaxRulesError::MissingField(
                "income_brackets.single".to_string(),
            ));
        }
        Ok(rules)
    }

    /// Compute income tax for a given taxable income using progressive brackets.
    pub fn compute_income_tax(&self, taxable_income: Decimal) -> Decimal {
        if taxable_income <= Decimal::ZERO {
            return Decimal::ZERO;
        }

        let mut tax = Decimal::ZERO;
        for bracket in &self.income_brackets.single {
            if taxable_income <= bracket.floor {
                break;
            }
            let taxable_in_bracket = taxable_income.min(bracket.ceiling) - bracket.floor;
            if taxable_in_bracket > Decimal::ZERO {
                tax += taxable_in_bracket * bracket.rate;
            }
        }
        tax
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_toml() -> &'static str {
        include_str!("../../test_data/tax_rules_2026.toml")
    }

    #[test]
    fn parse_tax_rules() {
        let rules = TaxRules::from_toml(sample_toml()).unwrap();
        assert_eq!(rules.year.value, 2026);
        assert_eq!(rules.se_tax.ss_wage_base, 176100);
        assert!(!rules.income_brackets.single.is_empty());
    }

    #[test]
    fn invalid_year_rejected() {
        let toml = sample_toml().replace("value = 2026", "value = 1900");
        assert!(TaxRules::from_toml(&toml).is_err());
    }

    #[test]
    fn se_tax_combined_rate() {
        let rules = TaxRules::from_toml(sample_toml()).unwrap();
        let combined = rules.se_tax.combined_rate();
        assert_eq!(combined, Decimal::from_str("0.153").unwrap());
    }

    #[test]
    fn quarterly_due_dates() {
        let rules = TaxRules::from_toml(sample_toml()).unwrap();
        let q1 = rules.quarterly_due_dates.due_date(crate::Quarter::Q1);
        assert_eq!(q1, NaiveDate::from_ymd_opt(2026, 4, 15).unwrap());
    }

    #[test]
    fn income_tax_brackets() {
        let rules = TaxRules::from_toml(sample_toml()).unwrap();

        // Zero income → zero tax
        assert_eq!(rules.compute_income_tax(Decimal::ZERO), Decimal::ZERO);

        // $10,000 → 10% bracket: $1,000
        let tax = rules.compute_income_tax(Decimal::from(10_000));
        assert_eq!(tax, Decimal::from(1_000));

        // $50,000 → spans first two brackets
        // 11925 * 0.10 = 1192.50
        // (48475 - 11925) * 0.12 = 4386.00
        // (50000 - 48475) * 0.22 = 335.50
        // Total: 5914.00
        let tax = rules.compute_income_tax(Decimal::from(50_000));
        assert_eq!(tax, Decimal::from_str("5914.00").unwrap());
    }

    #[test]
    fn negative_income_zero_tax() {
        let rules = TaxRules::from_toml(sample_toml()).unwrap();
        assert_eq!(
            rules.compute_income_tax(Decimal::from(-5000)),
            Decimal::ZERO
        );
    }
}
