use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{FiscalYear, Money, Quarter};

use super::rules::TaxRules;
use super::schedule_c::ScheduleCLine;

/// A point-in-time snapshot of the ledger aggregated by Schedule C line.
/// Built from a storage query — no business logic here, just data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerSnapshot {
    pub year: FiscalYear,
    /// Income and expense totals keyed by Schedule C line.
    /// Income lines have positive values; expense lines have positive values
    /// (representing the amount spent).
    pub line_totals: BTreeMap<ScheduleCLine, Money>,
    /// Prior year total tax liability (for safe harbor calculation).
    /// None if this is the first year using the app.
    pub prior_year_tax: Option<Money>,
}

impl LedgerSnapshot {
    /// Total gross income (Schedule C lines 1, 2, 6).
    pub fn gross_income(&self) -> Money {
        self.line_totals
            .iter()
            .filter(|(line, _)| line.is_income())
            .map(|(_, amount)| *amount)
            .fold(Money::zero(), |a, b| a + b)
    }

    /// Total expenses (all non-income Schedule C lines), with deduction caps applied.
    pub fn total_expenses(&self, rules: &TaxRules) -> Money {
        self.line_totals
            .iter()
            .filter(|(line, _)| !line.is_income())
            .map(|(line, amount)| {
                if *line == ScheduleCLine::Line24b {
                    // Meals are capped at the configured fraction (typically 50%)
                    *amount * rules.meals_deduction_cap.fraction
                } else {
                    *amount
                }
            })
            .fold(Money::zero(), |a, b| a + b)
    }

    /// Net profit = gross income - total expenses.
    pub fn net_profit(&self, rules: &TaxRules) -> Money {
        self.gross_income() - self.total_expenses(rules)
    }
}

/// Result of computing a quarterly tax estimate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarterlyEstimate {
    pub year: u16,
    pub quarter: Quarter,
    pub ytd_gross_income: Money,
    pub ytd_total_expenses: Money,
    pub ytd_net_profit: Money,
    /// SE tax base: net_profit * net_earnings_factor (0.9235)
    pub se_tax_base: Money,
    /// SE tax amount: se_tax_base * combined_rate (0.153), capped at SS wage base
    pub se_tax_amount: Money,
    /// SE tax deduction: se_tax_amount * deductible_fraction (0.50)
    pub se_tax_deduction: Money,
    /// Net income after SE tax deduction
    pub adjusted_net_income: Money,
    /// Estimated federal income tax from brackets
    pub estimated_income_tax: Money,
    /// Total estimated tax (SE tax + income tax)
    pub total_tax_estimate: Money,
    /// Safe harbor: min(100% prior year, 90% current year estimate)
    pub safe_harbor_amount: Money,
    /// Quarterly payment = total_tax_estimate / 4
    pub quarterly_payment: Money,
    /// When this quarter's estimated payment is due
    pub payment_due_date: NaiveDate,
    /// Schedule C line totals (for preview)
    pub schedule_c_lines: BTreeMap<ScheduleCLine, Money>,
}

/// Full Schedule C preview with deduction-adjusted totals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleCPreview {
    pub year: u16,
    pub gross_income: Money,
    pub total_expenses: Money,
    pub net_profit: Money,
    /// Line-by-line breakdown with deduction caps applied.
    pub lines: BTreeMap<ScheduleCLine, Money>,
}

/// Compute a quarterly tax estimate. This is a pure function — no I/O.
pub fn compute_quarterly_estimate(
    rules: &TaxRules,
    snapshot: &LedgerSnapshot,
    quarter: Quarter,
) -> QuarterlyEstimate {
    let ytd_gross_income = snapshot.gross_income();
    let ytd_total_expenses = snapshot.total_expenses(rules);
    let ytd_net_profit = ytd_gross_income - ytd_total_expenses;

    // SE tax calculation
    let se_tax_base = if ytd_net_profit > Money::zero() {
        ytd_net_profit * rules.se_tax.net_earnings_factor
    } else {
        Money::zero()
    };

    let se_tax_amount = compute_se_tax(rules, se_tax_base);
    let se_tax_deduction = se_tax_amount * rules.se_tax.deductible_fraction;

    // Income tax on adjusted income
    let adjusted_net_income = if ytd_net_profit > Money::zero() {
        ytd_net_profit - se_tax_deduction
    } else {
        ytd_net_profit
    };

    let estimated_income_tax =
        Money::from_decimal(rules.compute_income_tax(adjusted_net_income.as_decimal()));

    let total_tax_estimate = (se_tax_amount + estimated_income_tax).round_to_dollar();

    // Safe harbor: 100% of prior year tax or 90% of current year estimate
    let ninety_pct_current = total_tax_estimate * Decimal::from_str_exact("0.90").unwrap();
    let safe_harbor_amount = match snapshot.prior_year_tax {
        Some(prior) => {
            if prior < ninety_pct_current {
                prior
            } else {
                ninety_pct_current
            }
        }
        None => ninety_pct_current,
    };

    let quarterly_payment =
        Money::from_decimal(total_tax_estimate.as_decimal() / Decimal::from(4)).round_to_dollar();

    let payment_due_date = rules.quarterly_due_dates.due_date(quarter);

    // Build schedule C lines with deduction caps applied
    let mut schedule_c_lines = BTreeMap::new();
    for (line, amount) in &snapshot.line_totals {
        let adjusted = if *line == ScheduleCLine::Line24b {
            *amount * rules.meals_deduction_cap.fraction
        } else {
            *amount
        };
        schedule_c_lines.insert(*line, adjusted);
    }

    QuarterlyEstimate {
        year: rules.year.value,
        quarter,
        ytd_gross_income,
        ytd_total_expenses,
        ytd_net_profit,
        se_tax_base,
        se_tax_amount,
        se_tax_deduction,
        adjusted_net_income,
        estimated_income_tax,
        total_tax_estimate,
        safe_harbor_amount,
        quarterly_payment,
        payment_due_date,
        schedule_c_lines,
    }
}

/// Compute SE tax, respecting the Social Security wage base cap.
fn compute_se_tax(rules: &TaxRules, se_tax_base: Money) -> Money {
    if se_tax_base <= Money::zero() {
        return Money::zero();
    }

    let base = se_tax_base.as_decimal();
    let wage_base = Decimal::from(rules.se_tax.ss_wage_base);

    // Social Security portion: capped at wage base
    let ss_taxable = base.min(wage_base);
    let ss_tax = ss_taxable * rules.se_tax.ss_rate;

    // Medicare portion: no cap
    let medicare_tax = base * rules.se_tax.medicare_rate;

    Money::from_decimal(ss_tax + medicare_tax)
}

/// Build a Schedule C preview from a ledger snapshot.
pub fn schedule_c_preview(rules: &TaxRules, snapshot: &LedgerSnapshot) -> ScheduleCPreview {
    let gross_income = snapshot.gross_income();
    let total_expenses = snapshot.total_expenses(rules);
    let net_profit = gross_income - total_expenses;

    let mut lines = BTreeMap::new();
    for (line, amount) in &snapshot.line_totals {
        let adjusted = if *line == ScheduleCLine::Line24b {
            *amount * rules.meals_deduction_cap.fraction
        } else {
            *amount
        };
        lines.insert(*line, adjusted);
    }

    ScheduleCPreview {
        year: rules.year.value,
        gross_income,
        total_expenses,
        net_profit,
        lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn load_rules() -> TaxRules {
        TaxRules::from_toml(include_str!("../../test_data/tax_rules_2026.toml")).unwrap()
    }

    fn sample_snapshot() -> LedgerSnapshot {
        let mut line_totals = BTreeMap::new();
        // $100,000 services revenue
        line_totals.insert(ScheduleCLine::Line1, Money::from_cents(10_000_000));
        // $5,000 advertising
        line_totals.insert(ScheduleCLine::Line8, Money::from_cents(500_000));
        // $2,000 office expense
        line_totals.insert(ScheduleCLine::Line18, Money::from_cents(200_000));
        // $3,000 meals (50% deductible → $1,500 effective)
        line_totals.insert(ScheduleCLine::Line24b, Money::from_cents(300_000));
        // $1,000 travel
        line_totals.insert(ScheduleCLine::Line24a, Money::from_cents(100_000));

        LedgerSnapshot {
            year: FiscalYear::new(2026),
            line_totals,
            prior_year_tax: None,
        }
    }

    #[test]
    fn gross_income() {
        let snap = sample_snapshot();
        assert_eq!(snap.gross_income().to_cents(), 10_000_000); // $100,000
    }

    #[test]
    fn total_expenses_with_meals_cap() {
        let rules = load_rules();
        let snap = sample_snapshot();
        // $5,000 + $2,000 + ($3,000 * 0.50) + $1,000 = $9,500
        assert_eq!(snap.total_expenses(&rules).to_cents(), 950_000);
    }

    #[test]
    fn net_profit() {
        let rules = load_rules();
        let snap = sample_snapshot();
        // $100,000 - $9,500 = $90,500
        assert_eq!(snap.net_profit(&rules).to_cents(), 9_050_000);
    }

    #[test]
    fn quarterly_estimate_basic() {
        let rules = load_rules();
        let snap = sample_snapshot();
        let est = compute_quarterly_estimate(&rules, &snap, Quarter::Q1);

        assert_eq!(est.year, 2026);
        assert_eq!(est.quarter, Quarter::Q1);
        assert_eq!(est.ytd_gross_income.to_cents(), 10_000_000);
        assert_eq!(est.ytd_net_profit.to_cents(), 9_050_000);

        // SE tax base: $90,500 * 0.9235 = $83,576.75
        assert_eq!(est.se_tax_base.to_cents(), 8_357_675);

        // SE tax: $83,576.75 * 0.153 = $12,787.24
        assert_eq!(est.se_tax_amount.to_cents(), 1_278_724);

        // SE deduction: $12,787.24 * 0.50 = $6,393.62
        assert_eq!(est.se_tax_deduction.to_cents(), 639_362);

        // Adjusted income: $90,500 - $6,393.62 = $84,106.38
        assert_eq!(est.adjusted_net_income.to_cents(), 8_410_638);

        // Income tax on $84,106.38 (spans 10%, 12%, 22% brackets)
        assert!(est.estimated_income_tax.to_cents() > 0);

        // Total tax should be positive
        assert!(est.total_tax_estimate.to_cents() > 0);

        // Due date for Q1
        assert_eq!(
            est.payment_due_date,
            NaiveDate::from_ymd_opt(2026, 4, 15).unwrap()
        );
    }

    #[test]
    fn quarterly_estimate_zero_income() {
        let rules = load_rules();
        let snap = LedgerSnapshot {
            year: FiscalYear::new(2026),
            line_totals: BTreeMap::new(),
            prior_year_tax: None,
        };
        let est = compute_quarterly_estimate(&rules, &snap, Quarter::Q2);

        assert_eq!(est.ytd_gross_income.to_cents(), 0);
        assert_eq!(est.se_tax_amount.to_cents(), 0);
        assert_eq!(est.total_tax_estimate.to_cents(), 0);
    }

    #[test]
    fn quarterly_estimate_loss() {
        let rules = load_rules();
        let mut line_totals = BTreeMap::new();
        line_totals.insert(ScheduleCLine::Line1, Money::from_cents(500_000)); // $5,000 income
        line_totals.insert(ScheduleCLine::Line8, Money::from_cents(1_000_000)); // $10,000 expenses

        let snap = LedgerSnapshot {
            year: FiscalYear::new(2026),
            line_totals,
            prior_year_tax: None,
        };
        let est = compute_quarterly_estimate(&rules, &snap, Quarter::Q3);

        // Net loss → no SE tax, no income tax
        assert_eq!(est.se_tax_amount.to_cents(), 0);
        assert_eq!(est.estimated_income_tax.to_cents(), 0);
        assert_eq!(est.total_tax_estimate.to_cents(), 0);
    }

    #[test]
    fn safe_harbor_with_prior_year() {
        let rules = load_rules();
        let mut snap = sample_snapshot();
        snap.prior_year_tax = Some(Money::from_cents(1_500_000)); // $15,000 prior year

        let est = compute_quarterly_estimate(&rules, &snap, Quarter::Q1);

        // Safe harbor = min($15,000, 90% of current year)
        let ninety_pct = Money::from_decimal(
            est.total_tax_estimate.as_decimal() * Decimal::from_str("0.90").unwrap(),
        );
        let expected = if Money::from_cents(1_500_000) < ninety_pct {
            Money::from_cents(1_500_000)
        } else {
            ninety_pct
        };
        assert_eq!(est.safe_harbor_amount, expected);
    }

    #[test]
    fn se_tax_respects_wage_base() {
        let rules = load_rules();
        // Very high income: $500,000
        let mut line_totals = BTreeMap::new();
        line_totals.insert(ScheduleCLine::Line1, Money::from_cents(50_000_000));
        let snap = LedgerSnapshot {
            year: FiscalYear::new(2026),
            line_totals,
            prior_year_tax: None,
        };
        let est = compute_quarterly_estimate(&rules, &snap, Quarter::Q1);

        // SE base = $500,000 * 0.9235 = $461,750
        // SS portion: capped at $176,100 * 0.124 = $21,836.40
        // Medicare: $461,750 * 0.029 = $13,390.75
        // Total SE: $35,227.15
        let se_base_cents = est.se_tax_base.to_cents();
        assert_eq!(se_base_cents, 46_175_000);

        // Verify SE tax is less than uncapped amount
        let uncapped =
            Money::from_decimal(est.se_tax_base.as_decimal() * rules.se_tax.combined_rate());
        assert!(est.se_tax_amount < uncapped);
    }

    #[test]
    fn schedule_c_preview_basic() {
        let rules = load_rules();
        let snap = sample_snapshot();
        let preview = schedule_c_preview(&rules, &snap);

        assert_eq!(preview.year, 2026);
        assert_eq!(preview.gross_income.to_cents(), 10_000_000);
        assert_eq!(preview.net_profit.to_cents(), 9_050_000);

        // Meals should be 50% of original
        assert_eq!(
            preview
                .lines
                .get(&ScheduleCLine::Line24b)
                .unwrap()
                .to_cents(),
            150_000
        );
    }

    #[test]
    fn quarterly_payment_is_quarter_of_total() {
        let rules = load_rules();
        let snap = sample_snapshot();
        let est = compute_quarterly_estimate(&rules, &snap, Quarter::Q1);

        // Quarterly payment should be approximately total / 4 (rounded to dollar)
        let expected = Money::from_decimal(est.total_tax_estimate.as_decimal() / Decimal::from(4))
            .round_to_dollar();
        assert_eq!(est.quarterly_payment, expected);
    }

    #[test]
    fn all_quarters_have_due_dates() {
        let rules = load_rules();
        let snap = sample_snapshot();
        for q in [Quarter::Q1, Quarter::Q2, Quarter::Q3, Quarter::Q4] {
            let est = compute_quarterly_estimate(&rules, &snap, q);
            // Just verify it doesn't panic
            let _ = est.payment_due_date;
        }
    }
}
