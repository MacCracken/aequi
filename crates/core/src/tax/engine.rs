use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
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
    /// Last date (inclusive) covered by `line_totals`; the snapshot starts on
    /// Jan 1 of `year`. Used to annualize year-to-date figures.
    pub through: NaiveDate,
    /// Income and expense totals keyed by Schedule C line.
    /// Income lines have positive values; expense lines have positive values
    /// (representing the amount spent).
    pub line_totals: BTreeMap<ScheduleCLine, Money>,
    /// Prior year total tax liability (for safe harbor calculation).
    /// None if this is the first year using the app.
    pub prior_year_tax: Option<Money>,
    /// Prior year adjusted gross income, which decides whether the prior-year
    /// safe harbor is 100% or 110%. None if unknown (100% is assumed).
    pub prior_year_agi: Option<Money>,
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
    /// Factor that scales year-to-date profit to a full year (1 for a full year).
    pub annualization_factor: Decimal,
    /// Projected full-year net profit: ytd_net_profit * annualization_factor.
    /// All tax figures below are full-year projections based on this.
    pub annualized_net_profit: Money,
    /// SE tax base: annualized_net_profit * net_earnings_factor (0.9235)
    pub se_tax_base: Money,
    /// SE tax amount: Social Security (capped at the wage base) + Medicare
    pub se_tax_amount: Money,
    /// Additional Medicare Tax on SE earnings above the threshold (not deductible)
    pub additional_medicare_tax: Money,
    /// SE tax deduction: se_tax_amount * deductible_fraction (0.50)
    pub se_tax_deduction: Money,
    /// Net income after SE tax deduction
    pub adjusted_net_income: Money,
    /// Standard deduction subtracted before applying brackets
    pub standard_deduction: Money,
    /// Qualified business income deduction (§199A)
    pub qbi_deduction: Money,
    /// adjusted_net_income - standard_deduction - qbi_deduction (floored at zero)
    pub taxable_income: Money,
    /// Estimated federal income tax from brackets
    pub estimated_income_tax: Money,
    /// Projected full-year tax (SE tax + Additional Medicare + income tax)
    pub total_tax_estimate: Money,
    /// Safe harbor: min(100% or 110% of prior year tax, 90% of current year estimate)
    pub safe_harbor_amount: Money,
    /// Share of prior-year tax the prior-year safe harbor requires (1.00 or 1.10)
    pub safe_harbor_prior_year_percentage: Decimal,
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

/// Last day of the estimated-tax installment period for `quarter` (the periods
/// used by the IRS annualized income installment method, Form 2210 Schedule AI):
/// Q1 → Mar 31, Q2 → May 31, Q3 → Aug 31, Q4 → Dec 31.
pub fn installment_period_end(year: FiscalYear, quarter: Quarter) -> NaiveDate {
    let y = year.year() as i32;
    let (m, d) = match quarter {
        Quarter::Q1 => (3, 31),
        Quarter::Q2 => (5, 31),
        Quarter::Q3 => (8, 31),
        Quarter::Q4 => (12, 31),
    };
    NaiveDate::from_ymd_opt(y, m, d).expect("valid installment period end")
}

/// The last date a snapshot should cover when estimating `quarter` on `today`:
/// the end of the installment period, or today if the period is still open.
pub fn estimate_through(year: FiscalYear, quarter: Quarter, today: NaiveDate) -> NaiveDate {
    installment_period_end(year, quarter).min(today)
}

/// Factor that scales profit for Jan 1..=`through` up to a full year.
///
/// A completed installment period uses the IRS factors (4, 2.4, 1.5, 1). A
/// period still in progress is prorated by days elapsed, so an estimate made
/// mid-period isn't understated by applying a full period's factor to a
/// partial period's income.
pub fn annualization_factor(year: FiscalYear, quarter: Quarter, through: NaiveDate) -> Decimal {
    let period_end = installment_period_end(year, quarter);
    if through >= period_end {
        return match quarter {
            Quarter::Q1 => Decimal::from(4),
            Quarter::Q2 => Decimal::new(24, 1),
            Quarter::Q3 => Decimal::new(15, 1),
            Quarter::Q4 => Decimal::ONE,
        };
    }
    let start = year.start_date();
    if through < start {
        // Nothing booked yet for this year; the factor is irrelevant.
        return Decimal::ONE;
    }
    let elapsed = (through - start).num_days() + 1;
    let days_in_year = if year.end_date().ordinal() == 366 {
        366
    } else {
        365
    };
    Decimal::from(days_in_year) / Decimal::from(elapsed)
}

/// Compute a quarterly tax estimate. This is a pure function — no I/O.
///
/// The snapshot's year-to-date profit is annualized (see [`annualization_factor`])
/// and every tax figure is a projection for the full year; `quarterly_payment`
/// is one quarter of that projection.
pub fn compute_quarterly_estimate(
    rules: &TaxRules,
    snapshot: &LedgerSnapshot,
    quarter: Quarter,
) -> QuarterlyEstimate {
    let ytd_gross_income = snapshot.gross_income();
    let ytd_total_expenses = snapshot.total_expenses(rules);
    let ytd_net_profit = ytd_gross_income - ytd_total_expenses;

    let annualization_factor = annualization_factor(snapshot.year, quarter, snapshot.through);
    let annualized_net_profit = ytd_net_profit * annualization_factor;

    // SE tax calculation
    let se_tax_base = if annualized_net_profit > Money::zero() {
        annualized_net_profit * rules.se_tax.net_earnings_factor
    } else {
        Money::zero()
    };

    let se_tax_amount = compute_se_tax(rules, se_tax_base);
    let additional_medicare_tax = compute_additional_medicare_tax(rules, se_tax_base);
    let se_tax_deduction = se_tax_amount * rules.se_tax.deductible_fraction;

    // Income tax on adjusted income
    let adjusted_net_income = if annualized_net_profit > Money::zero() {
        annualized_net_profit - se_tax_deduction
    } else {
        annualized_net_profit
    };

    let standard_deduction = Money::from_decimal(rules.standard_deduction.single);
    let income_before_qbi = if adjusted_net_income > standard_deduction {
        adjusted_net_income - standard_deduction
    } else {
        Money::zero()
    };
    // QBI for a sole proprietor: net profit less the deductible half of SE tax.
    let qbi = if annualized_net_profit > Money::zero() {
        annualized_net_profit - se_tax_deduction
    } else {
        Money::zero()
    };
    let qbi_deduction = compute_qbi_deduction(rules, qbi, income_before_qbi);
    let taxable_income = if income_before_qbi > qbi_deduction {
        income_before_qbi - qbi_deduction
    } else {
        Money::zero()
    };

    let estimated_income_tax =
        Money::from_decimal(rules.compute_income_tax(taxable_income.as_decimal()));

    let total_tax_estimate =
        (se_tax_amount + additional_medicare_tax + estimated_income_tax).round_to_dollar();

    // Safe harbor: the smaller of 90% of this year's tax and 100% of last
    // year's (110% when last year's AGI was above the high-income threshold).
    let ninety_pct_current = total_tax_estimate * Decimal::from_str_exact("0.90").unwrap();
    let safe_harbor_prior_year_percentage = match snapshot.prior_year_agi {
        Some(agi) if agi.as_decimal() > rules.safe_harbor.high_income_agi => {
            rules.safe_harbor.high_income_percentage
        }
        _ => Decimal::ONE,
    };
    let safe_harbor_amount = match snapshot.prior_year_tax {
        Some(prior) => {
            let prior_requirement = prior * safe_harbor_prior_year_percentage;
            if prior_requirement < ninety_pct_current {
                prior_requirement
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
        annualization_factor,
        annualized_net_profit,
        se_tax_base,
        se_tax_amount,
        additional_medicare_tax,
        se_tax_deduction,
        adjusted_net_income,
        standard_deduction,
        qbi_deduction,
        taxable_income,
        estimated_income_tax,
        total_tax_estimate,
        safe_harbor_amount,
        safe_harbor_prior_year_percentage,
        quarterly_payment,
        payment_due_date,
        schedule_c_lines,
    }
}

/// Compute SE tax, respecting the Social Security wage base cap.
fn compute_se_tax(rules: &TaxRules, se_tax_base: Money) -> Money {
    if se_tax_base <= Money::zero() || se_tax_base.as_decimal() < rules.se_tax.min_net_earnings {
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

/// Qualified business income deduction (§199A) for a single filer.
///
/// `income_before_qbi` is taxable income before this deduction. Below the
/// threshold it is the full rate × QBI. Above it, the business is treated as a
/// specified service trade or business with no W-2 wages or qualified
/// property (the usual freelancer case, and the conservative assumption), so
/// the deduction phases out as rate × QBI × (1 − f)², where f is how far
/// through the phase-in range taxable income is, and is zero beyond it. It
/// never exceeds rate × taxable income, and is at least the minimum deduction
/// when QBI reaches the minimum (except past the phase-out, where an SSTB has
/// no qualified business income at all).
fn compute_qbi_deduction(rules: &TaxRules, qbi: Money, income_before_qbi: Money) -> Money {
    let Some(q) = &rules.qbi else {
        return Money::zero();
    };
    let qbi = qbi.as_decimal();
    let income = income_before_qbi.as_decimal();
    if qbi <= Decimal::ZERO {
        return Money::zero();
    }

    let excess = income - q.threshold;
    let remaining = if excess <= Decimal::ZERO {
        Decimal::ONE
    } else if excess >= q.phase_in_range {
        return Money::zero();
    } else {
        let left = Decimal::ONE - excess / q.phase_in_range;
        left * left
    };

    let mut deduction = (q.rate * qbi * remaining).min(q.rate * income);
    if qbi >= q.minimum_qbi && deduction < q.minimum_deduction {
        deduction = q.minimum_deduction;
    }
    Money::from_decimal(deduction.max(Decimal::ZERO))
}

/// Additional Medicare Tax (0.9%) on SE earnings above the filing threshold.
fn compute_additional_medicare_tax(rules: &TaxRules, se_tax_base: Money) -> Money {
    let excess = se_tax_base.as_decimal() - rules.se_tax.additional_medicare_threshold;
    if excess <= Decimal::ZERO || se_tax_base.as_decimal() < rules.se_tax.min_net_earnings {
        return Money::zero();
    }
    Money::from_decimal(excess * rules.se_tax.additional_medicare_rate)
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

        full_year(line_totals)
    }

    /// A snapshot covering all of 2026 (annualization factor 1 for Q4).
    fn full_year(line_totals: BTreeMap<ScheduleCLine, Money>) -> LedgerSnapshot {
        LedgerSnapshot {
            year: FiscalYear::new(2026),
            through: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            line_totals,
            prior_year_tax: None,
            prior_year_agi: None,
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
        let est = compute_quarterly_estimate(&rules, &snap, Quarter::Q4);

        assert_eq!(est.year, 2026);
        assert_eq!(est.quarter, Quarter::Q4);
        assert_eq!(est.ytd_gross_income.to_cents(), 10_000_000);
        assert_eq!(est.ytd_net_profit.to_cents(), 9_050_000);
        assert_eq!(est.annualization_factor, Decimal::ONE);
        assert_eq!(est.annualized_net_profit.to_cents(), 9_050_000);

        // SE tax base: $90,500 * 0.9235 = $83,576.75
        assert_eq!(est.se_tax_base.to_cents(), 8_357_675);

        // SE tax: $83,576.75 * 0.153 = $12,787.24
        assert_eq!(est.se_tax_amount.to_cents(), 1_278_724);

        // SE deduction: $12,787.24 * 0.50 = $6,393.62
        assert_eq!(est.se_tax_deduction.to_cents(), 639_362);

        // Adjusted income: $90,500 - $6,393.62 = $84,106.38
        assert_eq!(est.adjusted_net_income.to_cents(), 8_410_638);

        // After the $16,100 standard deduction: $68,006.38. QBI is $84,106.38,
        // but the deduction is capped at 20% of taxable income: $13,601.28.
        assert_eq!(est.standard_deduction.to_cents(), 1_610_000);
        assert_eq!(est.qbi_deduction.to_cents(), 1_360_128);
        assert_eq!(est.taxable_income.to_cents(), 5_440_510);

        // Income tax: 1,240 + 4,560 + 22% × 4,005.10 = $6,681.12
        assert_eq!(est.estimated_income_tax.to_cents(), 668_112);

        // Total: $12,787.24 + $6,681.12 = $19,468.36 → $19,468
        assert_eq!(est.additional_medicare_tax.to_cents(), 0);
        assert_eq!(est.total_tax_estimate.to_cents(), 1_946_800);
        assert_eq!(est.quarterly_payment.to_cents(), 486_700);

        assert_eq!(
            est.payment_due_date,
            NaiveDate::from_ymd_opt(2027, 1, 15).unwrap()
        );
    }

    #[test]
    fn q1_estimate_annualizes_first_quarter_profit() {
        // $22,625 of Q1 profit annualizes (×4) to the same $90,500 as the
        // full-year sample, so the projected tax must match it exactly.
        let rules = load_rules();
        let mut line_totals = BTreeMap::new();
        line_totals.insert(ScheduleCLine::Line1, Money::from_cents(2_500_000));
        line_totals.insert(ScheduleCLine::Line8, Money::from_cents(237_500));
        let snap = LedgerSnapshot {
            year: FiscalYear::new(2026),
            through: NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
            line_totals,
            prior_year_tax: None,
            prior_year_agi: None,
        };
        let est = compute_quarterly_estimate(&rules, &snap, Quarter::Q1);

        assert_eq!(est.ytd_net_profit.to_cents(), 2_262_500);
        assert_eq!(est.annualization_factor, Decimal::from(4));
        assert_eq!(est.annualized_net_profit.to_cents(), 9_050_000);
        assert_eq!(est.total_tax_estimate.to_cents(), 1_946_800);
        assert_eq!(est.quarterly_payment.to_cents(), 486_700);
        assert_eq!(
            est.payment_due_date,
            NaiveDate::from_ymd_opt(2026, 4, 15).unwrap()
        );
    }

    #[test]
    fn annualization_factors() {
        let y = FiscalYear::new(2026);
        let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        // Completed installment periods use the IRS factors.
        assert_eq!(
            annualization_factor(y, Quarter::Q1, d(3, 31)),
            Decimal::from(4)
        );
        assert_eq!(
            annualization_factor(y, Quarter::Q2, d(5, 31)),
            Decimal::new(24, 1)
        );
        assert_eq!(
            annualization_factor(y, Quarter::Q3, d(8, 31)),
            Decimal::new(15, 1)
        );
        assert_eq!(
            annualization_factor(y, Quarter::Q4, d(12, 31)),
            Decimal::ONE
        );
        // A period in progress is prorated by days elapsed (Jan 1..=Feb 14 = 45 days).
        assert_eq!(
            annualization_factor(y, Quarter::Q1, d(2, 14)),
            Decimal::from(365) / Decimal::from(45)
        );
        // The window is capped at the installment period end, or today.
        assert_eq!(estimate_through(y, Quarter::Q2, d(10, 1)), d(5, 31));
        assert_eq!(estimate_through(y, Quarter::Q4, d(10, 1)), d(10, 1));
    }

    #[test]
    fn no_se_tax_below_400_of_net_earnings() {
        let rules = load_rules();
        let mut line_totals = BTreeMap::new();
        line_totals.insert(ScheduleCLine::Line1, Money::from_cents(30_000)); // $300
        let est = compute_quarterly_estimate(&rules, &full_year(line_totals), Quarter::Q4);
        assert_eq!(est.se_tax_amount.to_cents(), 0);
        assert_eq!(est.total_tax_estimate.to_cents(), 0);
    }

    #[test]
    fn quarterly_estimate_zero_income() {
        let rules = load_rules();
        let snap = full_year(BTreeMap::new());
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

        let snap = full_year(line_totals);
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
        let snap = full_year(line_totals);
        let est = compute_quarterly_estimate(&rules, &snap, Quarter::Q4);

        // SE base = $500,000 * 0.9235 = $461,750
        // SS portion: capped at $184,500 * 0.124 = $22,878.00
        // Medicare: $461,750 * 0.029 = $13,390.75
        // Total SE: $36,268.75
        let se_base_cents = est.se_tax_base.to_cents();
        assert_eq!(se_base_cents, 46_175_000);
        assert_eq!(est.se_tax_amount.to_cents(), 3_626_875);

        // Additional Medicare: ($461,750 - $200,000) * 0.009 = $2,355.75
        assert_eq!(est.additional_medicare_tax.to_cents(), 235_575);

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

    #[test]
    fn qbi_deduction_phases_out_for_service_businesses() {
        let rules = load_rules();
        let m = |d: i64| Money::from_cents(d * 100);
        // Below the $201,750 threshold: 20% of QBI, capped at 20% of taxable income.
        assert_eq!(
            compute_qbi_deduction(&rules, m(50_000), m(100_000)),
            m(10_000)
        );
        assert_eq!(
            compute_qbi_deduction(&rules, m(80_000), m(40_000)),
            m(8_000)
        );
        // Halfway through the $75,000 phase-in: 20% × QBI × (1 − 0.5)².
        assert_eq!(
            compute_qbi_deduction(&rules, m(250_000), m(239_250)),
            m(12_500)
        );
        // Past the phase-in range ($276,750): nothing.
        assert_eq!(
            compute_qbi_deduction(&rules, m(300_000), m(276_750)),
            Money::zero()
        );
        // Minimum $400 once QBI reaches $1,000; not below that.
        assert_eq!(compute_qbi_deduction(&rules, m(1_500), m(1_000)), m(400));
        assert_eq!(compute_qbi_deduction(&rules, m(900), m(1_000)), m(180));
        assert_eq!(
            compute_qbi_deduction(&rules, Money::zero(), m(1_000)),
            Money::zero()
        );
    }

    #[test]
    fn rules_without_qbi_give_no_deduction() {
        let toml = include_str!("../../test_data/tax_rules_2026.toml");
        let start = toml.find("[qbi]").unwrap();
        let end = toml.find("[safe_harbor]").unwrap();
        let rules = TaxRules::from_toml(&format!("{}{}", &toml[..start], &toml[end..])).unwrap();
        let qbi = Money::from_cents(5_000_000);
        assert_eq!(compute_qbi_deduction(&rules, qbi, qbi), Money::zero());
    }

    #[test]
    fn safe_harbor_is_110_percent_for_high_prior_year_agi() {
        let rules = load_rules();
        let mut snap = sample_snapshot();
        snap.prior_year_tax = Some(Money::from_cents(1_500_000)); // $15,000

        // Prior-year AGI above $150,000: 110% of $15,000 = $16,500 < 90% of $19,468.
        snap.prior_year_agi = Some(Money::from_cents(20_000_000));
        let est = compute_quarterly_estimate(&rules, &snap, Quarter::Q4);
        assert_eq!(est.safe_harbor_prior_year_percentage, Decimal::new(110, 2));
        assert_eq!(est.safe_harbor_amount.to_cents(), 1_650_000);

        // At or below $150,000: 100%.
        snap.prior_year_agi = Some(Money::from_cents(15_000_000));
        let est = compute_quarterly_estimate(&rules, &snap, Quarter::Q4);
        assert_eq!(est.safe_harbor_prior_year_percentage, Decimal::ONE);
        assert_eq!(est.safe_harbor_amount.to_cents(), 1_500_000);

        // A large prior-year tax: 90% of this year's ($17,521.20) is smaller.
        snap.prior_year_tax = Some(Money::from_cents(2_000_000));
        let est = compute_quarterly_estimate(&rules, &snap, Quarter::Q4);
        assert_eq!(est.safe_harbor_amount.to_cents(), 1_752_120);
    }
}
