use std::sync::Arc;

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use chrono::Datelike;

use aequi_core::{FiscalYear, Money, Quarter};

use crate::error::ApiError;
use crate::state::ServerState;

#[derive(Deserialize)]
struct TaxQuery {
    year: Option<u16>,
    quarter: Option<u8>,
}

#[derive(Serialize)]
struct EstimateOut {
    year: u16,
    quarter: String,
    /// Last date of ledger activity included in the estimate.
    through: String,
    ytd_gross_income_cents: i64,
    ytd_total_expenses_cents: i64,
    ytd_net_profit_cents: i64,
    annualization_factor: Decimal,
    annualized_net_profit_cents: i64,
    se_tax_cents: i64,
    additional_medicare_tax_cents: i64,
    se_tax_deduction_cents: i64,
    standard_deduction_cents: i64,
    taxable_income_cents: i64,
    income_tax_cents: i64,
    /// Projected full-year tax.
    total_tax_cents: i64,
    safe_harbor_cents: i64,
    quarterly_payment_cents: i64,
    payment_due_date: String,
}

fn load_rules(year: u16) -> Result<aequi_core::TaxRules, ApiError> {
    let toml_str = include_str!("../../../../rules/tax/us/2026.toml");
    let rules =
        aequi_core::TaxRules::from_toml(toml_str).map_err(|e| ApiError::Internal(e.to_string()))?;
    if rules.year.value != year {
        return Err(ApiError::BadRequest(format!(
            "Tax rules for year {year} not available"
        )));
    }
    Ok(rules)
}

fn current_year(today: chrono::NaiveDate) -> u16 {
    today.year_ce().1 as u16
}

fn parse_quarter(q: Option<u8>) -> Result<Quarter, ApiError> {
    match q {
        None => Ok(Quarter::Q1),
        Some(n) => Quarter::new(n)
            .ok_or_else(|| ApiError::BadRequest(format!("quarter must be 1-4, got {n}"))),
    }
}

async fn quarterly_estimate(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<TaxQuery>,
) -> Result<Json<EstimateOut>, ApiError> {
    let today = chrono::Local::now().date_naive();
    let yr = q.year.unwrap_or_else(|| current_year(today));
    let qtr = parse_quarter(q.quarter)?;

    let rules = load_rules(yr)?;
    let fy = FiscalYear::new(yr);

    let prior = aequi_storage::get_prior_year_total_tax(&state.db, yr)
        .await?
        .map(Money::from_cents);

    let through = aequi_core::estimate_through(fy, qtr, today);
    let snapshot = aequi_storage::build_ledger_snapshot(&state.db, fy, through, prior).await?;
    let est = aequi_core::compute_quarterly_estimate(&rules, &snapshot, qtr);

    Ok(Json(EstimateOut {
        year: est.year,
        quarter: est.quarter.to_string(),
        through: snapshot.through.to_string(),
        ytd_gross_income_cents: est.ytd_gross_income.to_cents(),
        ytd_total_expenses_cents: est.ytd_total_expenses.to_cents(),
        ytd_net_profit_cents: est.ytd_net_profit.to_cents(),
        annualization_factor: est.annualization_factor,
        annualized_net_profit_cents: est.annualized_net_profit.to_cents(),
        se_tax_cents: est.se_tax_amount.to_cents(),
        additional_medicare_tax_cents: est.additional_medicare_tax.to_cents(),
        se_tax_deduction_cents: est.se_tax_deduction.to_cents(),
        standard_deduction_cents: est.standard_deduction.to_cents(),
        taxable_income_cents: est.taxable_income.to_cents(),
        income_tax_cents: est.estimated_income_tax.to_cents(),
        total_tax_cents: est.total_tax_estimate.to_cents(),
        safe_harbor_cents: est.safe_harbor_amount.to_cents(),
        quarterly_payment_cents: est.quarterly_payment.to_cents(),
        payment_due_date: est.payment_due_date.to_string(),
    }))
}

#[derive(Deserialize)]
struct PaymentInput {
    year: u16,
    quarter: u8,
    amount_cents: i64,
    date: String,
}

fn validate_payment(input: &PaymentInput) -> Result<(), ApiError> {
    if Quarter::new(input.quarter).is_none() {
        return Err(ApiError::BadRequest(format!(
            "quarter must be 1-4, got {}",
            input.quarter
        )));
    }
    if input.amount_cents <= 0 {
        return Err(ApiError::BadRequest(
            "amount_cents must be positive".to_string(),
        ));
    }
    if !(2000..=2100).contains(&input.year) {
        return Err(ApiError::BadRequest(format!(
            "year {} is out of range",
            input.year
        )));
    }
    aequi_storage::validate_date("date", &input.date)?;
    Ok(())
}

async fn record_payment(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<PaymentInput>,
) -> Result<Json<()>, ApiError> {
    validate_payment(&input)?;
    aequi_storage::record_tax_payment(
        &state.db,
        input.year,
        input.quarter,
        input.amount_cents,
        &input.date,
    )
    .await?;
    Ok(Json(()))
}

async fn schedule_c(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<TaxQuery>,
) -> Result<Json<aequi_core::ScheduleCPreview>, ApiError> {
    let yr = q
        .year
        .unwrap_or_else(|| current_year(chrono::Local::now().date_naive()));
    let rules = load_rules(yr)?;
    let fy = FiscalYear::new(yr);
    let snapshot = aequi_storage::build_ledger_snapshot(&state.db, fy, fy.end_date(), None).await?;
    let preview = aequi_core::tax::engine::schedule_c_preview(&rules, &snapshot);
    Ok(Json(preview))
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/tax/quarterly-estimate", get(quarterly_estimate))
        .route("/tax/schedule-c", get(schedule_c))
        .route("/tax/payments", post(record_payment))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payment(quarter: u8, amount_cents: i64, date: &str) -> PaymentInput {
        PaymentInput {
            year: 2026,
            quarter,
            amount_cents,
            date: date.to_string(),
        }
    }

    #[test]
    fn payment_validation() {
        assert!(validate_payment(&payment(1, 100, "2026-04-15")).is_ok());
        for bad in [
            payment(0, 100, "2026-04-15"),
            payment(5, 100, "2026-04-15"),
            payment(1, 0, "2026-04-15"),
            payment(1, -5, "2026-04-15"),
            payment(1, 100, "4/15/2026"),
            payment(1, 100, "2026-02-30"),
        ] {
            assert!(matches!(
                validate_payment(&bad),
                Err(ApiError::BadRequest(_))
            ));
        }
    }

    #[test]
    fn invalid_quarter_is_rejected_not_defaulted() {
        assert!(matches!(
            parse_quarter(Some(7)),
            Err(ApiError::BadRequest(_))
        ));
        assert_eq!(parse_quarter(Some(3)).unwrap(), Quarter::Q3);
    }
}
