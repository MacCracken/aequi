use std::sync::Arc;

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
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
    ytd_gross_income_cents: i64,
    ytd_net_profit_cents: i64,
    se_tax_cents: i64,
    income_tax_cents: i64,
    total_tax_cents: i64,
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

async fn quarterly_estimate(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<TaxQuery>,
) -> Result<Json<EstimateOut>, ApiError> {
    let now = chrono::Utc::now().date_naive();
    let yr = q.year.unwrap_or(now.year_ce().1 as u16);
    let qtr = q.quarter.and_then(Quarter::new).unwrap_or(Quarter::Q1);

    let rules = load_rules(yr)?;
    let fy = FiscalYear::new(yr);

    let prior = aequi_storage::get_prior_year_total_tax(&state.db, yr)
        .await?
        .map(Money::from_cents);

    let snapshot = aequi_storage::build_ledger_snapshot(&state.db, fy, prior).await?;
    let est = aequi_core::compute_quarterly_estimate(&rules, &snapshot, qtr);

    Ok(Json(EstimateOut {
        year: est.year,
        quarter: est.quarter.to_string(),
        ytd_gross_income_cents: est.ytd_gross_income.to_cents(),
        ytd_net_profit_cents: est.ytd_net_profit.to_cents(),
        se_tax_cents: est.se_tax_amount.to_cents(),
        income_tax_cents: est.estimated_income_tax.to_cents(),
        total_tax_cents: est.total_tax_estimate.to_cents(),
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

async fn record_payment(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<PaymentInput>,
) -> Result<Json<()>, ApiError> {
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
    let yr = q.year.unwrap_or(2026);
    let rules = load_rules(yr)?;
    let fy = FiscalYear::new(yr);
    let snapshot = aequi_storage::build_ledger_snapshot(&state.db, fy, None).await?;
    let preview = aequi_core::tax::engine::schedule_c_preview(&rules, &snapshot);
    Ok(Json(preview))
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/tax/quarterly-estimate", get(quarterly_estimate))
        .route("/tax/schedule-c", get(schedule_c))
        .route("/tax/payments", post(record_payment))
}
