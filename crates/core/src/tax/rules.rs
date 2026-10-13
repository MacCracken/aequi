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
    #[error("Invalid tax rules: {0}")]
    Invalid(String),
}

/// Tax rules loaded from a TOML file (e.g., `rules/tax/us/2026.toml`).
#[derive(Debug, Clone, Deserialize)]
pub struct TaxRules {
    pub year: YearField,
    pub se_tax: SeTaxRules,
    pub standard_deduction: StandardDeduction,
    /// Qualified business income deduction; omitted for years before it existed.
    #[serde(default)]
    pub qbi: Option<QbiRules>,
    #[serde(default)]
    pub safe_harbor: SafeHarborRules,
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
    /// No SE tax is owed when net earnings from self-employment are below this.
    #[serde(default = "default_min_net_earnings")]
    pub min_net_earnings: Decimal,
    /// Additional Medicare Tax rate on SE earnings above the threshold.
    #[serde(default = "default_additional_medicare_rate")]
    pub additional_medicare_rate: Decimal,
    #[serde(default = "default_additional_medicare_threshold")]
    pub additional_medicare_threshold: Decimal,
}

// Statutory values that are not inflation-indexed, so older rule files that
// predate these fields still compute correctly.
fn default_min_net_earnings() -> Decimal {
    Decimal::from(400)
}

fn default_additional_medicare_rate() -> Decimal {
    Decimal::new(9, 3)
}

fn default_additional_medicare_threshold() -> Decimal {
    Decimal::from(200_000)
}

#[derive(Debug, Clone, Deserialize)]
pub struct StandardDeduction {
    pub single: Decimal,
}

/// Section 199A qualified business income deduction (single filer).
#[derive(Debug, Clone, Deserialize)]
pub struct QbiRules {
    /// Deduction rate (20%).
    pub rate: Decimal,
    /// Taxable income (before the deduction) above which it phases out.
    pub threshold: Decimal,
    /// Width of the phase-out band above `threshold`.
    pub phase_in_range: Decimal,
    /// Minimum deduction for taxpayers with at least `minimum_qbi` of QBI.
    #[serde(default)]
    pub minimum_deduction: Decimal,
    #[serde(default)]
    pub minimum_qbi: Decimal,
}

/// Prior-year safe harbor for estimated tax.
#[derive(Debug, Clone, Deserialize)]
pub struct SafeHarborRules {
    /// Prior-year AGI above which the prior-year safe harbor rises to `high_income_percentage`.
    pub high_income_agi: Decimal,
    pub high_income_percentage: Decimal,
}

// Statutory (§6654(d)(1)(C)) and not inflation-indexed.
impl Default for SafeHarborRules {
    fn default() -> Self {
        Self {
            high_income_agi: Decimal::from(150_000),
            high_income_percentage: Decimal::new(110, 2),
        }
    }
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
    // Decimal: IRS rates can include half cents (e.g. 72.5¢ for 2026).
    pub business_cents_per_mile: Decimal,
    pub medical_cents_per_mile: Decimal,
    pub charity_cents_per_mile: Decimal,
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
        // Validated in `TaxRules::from_toml`.
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
        rules.validate()?;
        Ok(rules)
    }

    /// Structural checks that would otherwise surface later as a panic
    /// (unparseable due dates) or as silently wrong tax (gapped brackets).
    fn validate(&self) -> Result<(), TaxRulesError> {
        let invalid = |msg: String| Err(TaxRulesError::Invalid(msg));

        let d = &self.quarterly_due_dates;
        for (name, s) in [("q1", &d.q1), ("q2", &d.q2), ("q3", &d.q3), ("q4", &d.q4)] {
            if NaiveDate::from_str(s).is_err() {
                return invalid(format!(
                    "quarterly_due_dates.{name} is not a YYYY-MM-DD date: {s}"
                ));
            }
        }

        let mut expected_floor = Decimal::ZERO;
        for (i, b) in self.income_brackets.single.iter().enumerate() {
            if b.floor != expected_floor {
                return invalid(format!(
                    "income bracket {} starts at {} but the previous bracket ends at {expected_floor}",
                    i + 1,
                    b.floor
                ));
            }
            if b.ceiling <= b.floor {
                return invalid(format!("income bracket {} has ceiling <= floor", i + 1));
            }
            if b.rate < Decimal::ZERO || b.rate > Decimal::ONE {
                return invalid(format!(
                    "income bracket {} rate must be between 0 and 1",
                    i + 1
                ));
            }
            expected_floor = b.ceiling;
        }

        if self.standard_deduction.single < Decimal::ZERO {
            return invalid("standard_deduction.single must not be negative".into());
        }
        if let Some(q) = &self.qbi {
            if q.rate < Decimal::ZERO || q.rate > Decimal::ONE {
                return invalid("qbi.rate must be between 0 and 1".into());
            }
            if q.threshold < Decimal::ZERO || q.phase_in_range <= Decimal::ZERO {
                return invalid("qbi.threshold must be >= 0 and qbi.phase_in_range > 0".into());
            }
            if q.minimum_deduction < Decimal::ZERO || q.minimum_qbi < Decimal::ZERO {
                return invalid("qbi minimums must not be negative".into());
            }
        }
        if self.safe_harbor.high_income_percentage < Decimal::ONE {
            return invalid("safe_harbor.high_income_percentage must be at least 1".into());
        }
        Ok(())
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
        assert_eq!(rules.se_tax.ss_wage_base, 184500);
        assert_eq!(rules.standard_deduction.single, Decimal::from(16_100));
        assert_eq!(
            rules.mileage.business_cents_per_mile,
            Decimal::from_str("72.5").unwrap()
        );
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

        // $60,000 → spans the first three 2026 brackets
        // 12400 * 0.10 = 1240.00
        // (50400 - 12400) * 0.12 = 4560.00
        // (60000 - 50400) * 0.22 = 2112.00
        // Total: 7912.00
        let tax = rules.compute_income_tax(Decimal::from(60_000));
        assert_eq!(tax, Decimal::from_str("7912.00").unwrap());
    }

    #[test]
    fn invalid_due_date_rejected() {
        let toml = sample_toml().replace("q2 = \"2026-06-15\"", "q2 = \"June 15\"");
        assert!(matches!(
            TaxRules::from_toml(&toml),
            Err(TaxRulesError::Invalid(_))
        ));
    }

    #[test]
    fn gapped_brackets_rejected() {
        let toml = sample_toml().replace(
            "floor = 12400\nceiling = 50400",
            "floor = 13000\nceiling = 50400",
        );
        assert!(matches!(
            TaxRules::from_toml(&toml),
            Err(TaxRulesError::Invalid(_))
        ));
    }

    #[test]
    fn parses_qbi_and_safe_harbor() {
        let rules = TaxRules::from_toml(sample_toml()).unwrap();
        let qbi = rules.qbi.expect("2026 rules include QBI");
        assert_eq!(qbi.threshold, Decimal::from(201_750));
        assert_eq!(qbi.phase_in_range, Decimal::from(75_000));
        assert_eq!(qbi.minimum_deduction, Decimal::from(400));
        assert_eq!(rules.safe_harbor.high_income_agi, Decimal::from(150_000));
    }

    #[test]
    fn missing_standard_deduction_rejected() {
        let toml = sample_toml().replace("[standard_deduction]\nsingle = 16100", "");
        assert!(TaxRules::from_toml(&toml).is_err());
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
