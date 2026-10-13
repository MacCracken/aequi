use std::sync::Arc;

use axum::extract::{Path, Query, State};
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
    qbi_deduction_cents: i64,
    /// After the standard and QBI deductions.
    taxable_income_cents: i64,
    income_tax_cents: i64,
    /// Projected full-year tax.
    total_tax_cents: i64,
    safe_harbor_cents: i64,
    /// Share of prior-year tax the prior-year safe harbor uses: "1.00", or
    /// "1.10" when prior-year AGI was over the threshold.
    safe_harbor_prior_year_percentage: Decimal,
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
        qbi_deduction_cents: est.qbi_deduction.to_cents(),
        taxable_income_cents: est.taxable_income.to_cents(),
        income_tax_cents: est.estimated_income_tax.to_cents(),
        total_tax_cents: est.total_tax_estimate.to_cents(),
        safe_harbor_cents: est.safe_harbor_amount.to_cents(),
        safe_harbor_prior_year_percentage: {
            // Always two places ("1.00", "1.10"), whatever scale the rules use.
            let mut pct = est.safe_harbor_prior_year_percentage;
            pct.rescale(2);
            pct
        },
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

fn validate_year(year: u16) -> Result<(), ApiError> {
    if !(2000..=2100).contains(&year) {
        return Err(ApiError::BadRequest(format!("year {year} is out of range")));
    }
    Ok(())
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
    validate_year(input.year)?;
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

/// Figures from the filed return for a year. A field left out (or null)
/// clears it; unknown fields are rejected so a typo can't clear a value.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaxReturnInput {
    #[serde(default)]
    total_tax_cents: Option<i64>,
    #[serde(default)]
    agi_cents: Option<i64>,
}

/// GET /tax/returns/{year} — figures entered from that year's filed return.
async fn get_tax_return(
    State(state): State<Arc<ServerState>>,
    Path(year): Path<u16>,
) -> Result<Json<aequi_storage::TaxReturnFigures>, ApiError> {
    validate_year(year)?;
    Ok(Json(
        aequi_storage::get_tax_return_figures(&state.db, year).await?,
    ))
}

/// PUT /tax/returns/{year} — replace them. The next year's prior-year safe
/// harbor uses the total tax, and the AGI picks 100% or 110%.
async fn put_tax_return(
    State(state): State<Arc<ServerState>>,
    Path(year): Path<u16>,
    Json(input): Json<TaxReturnInput>,
) -> Result<Json<aequi_storage::TaxReturnFigures>, ApiError> {
    validate_year(year)?;
    let figures = aequi_storage::TaxReturnFigures {
        total_tax_cents: input.total_tax_cents,
        agi_cents: input.agi_cents,
    };
    aequi_storage::set_tax_return_figures(&state.db, year, figures).await?;
    Ok(Json(
        aequi_storage::get_tax_return_figures(&state.db, year).await?,
    ))
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
        .route(
            "/tax/returns/{year}",
            get(get_tax_return).put(put_tax_return),
        )
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
    fn payment_year_must_be_in_range() {
        let mut p = payment(1, 100, "2026-04-15");
        p.year = 1999;
        assert!(matches!(validate_payment(&p), Err(ApiError::BadRequest(_))));
    }

    #[test]
    fn invalid_quarter_is_rejected_not_defaulted() {
        assert!(matches!(
            parse_quarter(Some(7)),
            Err(ApiError::BadRequest(_))
        ));
        assert_eq!(parse_quarter(Some(3)).unwrap(), Quarter::Q3);
    }

    #[tokio::test]
    async fn tax_return_figures_round_trip_and_feed_the_safe_harbor() {
        use crate::routes::tests::{call, test_state};
        use axum::http::{Method, StatusCode};
        use serde_json::json;

        let state = test_state(None).await;
        let (status, body) = call(&state, Method::GET, "/api/v1/tax/returns/2025", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"total_tax_cents": null, "agi_cents": null}));

        // Over $150k AGI: the prior-year safe harbor is 110% of last year's tax.
        let filed = json!({"total_tax_cents": 1_200_000, "agi_cents": 20_000_000});
        let (status, body) = call(
            &state,
            Method::PUT,
            "/api/v1/tax/returns/2025",
            Some(filed.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, filed);
        let (_, body) = call(&state, Method::GET, "/api/v1/tax/returns/2025", None).await;
        assert_eq!(body, filed);

        let (status, est) = call(
            &state,
            Method::GET,
            "/api/v1/tax/quarterly-estimate?year=2026&quarter=1",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{est}");
        assert_eq!(est["safe_harbor_prior_year_percentage"], "1.10");
        assert!(est["qbi_deduction_cents"].is_i64(), "{est}");

        // Omitted fields are cleared.
        let (status, body) = call(
            &state,
            Method::PUT,
            "/api/v1/tax/returns/2025",
            Some(json!({"total_tax_cents": 900_000})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"total_tax_cents": 900_000, "agi_cents": null}));
        let (_, est) = call(
            &state,
            Method::GET,
            "/api/v1/tax/quarterly-estimate?year=2026&quarter=1",
            None,
        )
        .await;
        assert_eq!(est["safe_harbor_prior_year_percentage"], "1.00");
    }

    #[tokio::test]
    async fn tax_return_input_is_validated() {
        use crate::routes::tests::{call, test_state};
        use axum::http::{Method, StatusCode};
        use serde_json::json;

        let state = test_state(None).await;
        for uri in ["/api/v1/tax/returns/1999", "/api/v1/tax/returns/2101"] {
            let (status, _) = call(&state, Method::GET, uri, None).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            let (status, _) = call(&state, Method::PUT, uri, Some(json!({}))).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        }
        let (status, _) = call(&state, Method::GET, "/api/v1/tax/returns/abc", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, body) = call(
            &state,
            Method::PUT,
            "/api/v1/tax/returns/2025",
            Some(json!({"total_tax_cents": -1})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("negative"));

        // A misspelled field is rejected rather than clearing the figures.
        let (status, _) = call(
            &state,
            Method::PUT,
            "/api/v1/tax/returns/2025",
            Some(json!({"total_tax": 100})),
        )
        .await;
        assert!(status.is_client_error());
        let (_, body) = call(&state, Method::GET, "/api/v1/tax/returns/2025", None).await;
        assert_eq!(body, json!({"total_tax_cents": null, "agi_cents": null}));
    }
}
