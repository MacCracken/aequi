use std::sync::Arc;

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::error::ApiError;
use crate::state::ServerState;

#[derive(Deserialize)]
struct DateRange {
    start_date: Option<String>,
    end_date: Option<String>,
}

#[derive(Serialize)]
struct ProfitLossEntry {
    account_code: String,
    account_name: String,
    account_type: String,
    /// Net credits minus debits in the range (income positive, expenses negative).
    total_cents: i64,
}

/// Income and expense totals for transactions dated `start..=end`. Every
/// income/expense account is listed; accounts without activity show zero.
async fn profit_loss_entries(
    db: &aequi_storage::DbPool,
    start: &str,
    end: &str,
) -> Result<Vec<ProfitLossEntry>, sqlx::Error> {
    // The date filter must restrict which lines are summed, so it lives in the
    // aggregate subquery rather than in an outer join's ON clause.
    let rows = sqlx::query(
        r#"
        SELECT a.code, a.name, a.account_type,
            COALESCE(activity.total_cents, 0) AS total_cents
        FROM accounts a
        LEFT JOIN (
            SELECT tl.account_id,
                SUM(tl.credit_cents - tl.debit_cents) AS total_cents
            FROM transaction_lines tl
            JOIN transactions t ON t.id = tl.transaction_id
            WHERE t.date >= ? AND t.date <= ?
            GROUP BY tl.account_id
        ) activity ON activity.account_id = a.id
        WHERE a.account_type IN ('Income', 'Expense')
        ORDER BY a.account_type, a.code
        "#,
    )
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ProfitLossEntry {
            account_code: r.get("code"),
            account_name: r.get("name"),
            account_type: r.get("account_type"),
            total_cents: r.get("total_cents"),
        })
        .collect())
}

async fn profit_loss(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<DateRange>,
) -> Result<Json<Vec<ProfitLossEntry>>, ApiError> {
    // Defaults: year to date (local calendar).
    let today = chrono::Local::now().date_naive();
    let start = q
        .start_date
        .unwrap_or_else(|| format!("{}-01-01", today.format("%Y")));
    let end = q.end_date.unwrap_or_else(|| today.to_string());

    aequi_storage::validate_date("start_date", &start)?;
    aequi_storage::validate_date("end_date", &end)?;
    if start > end {
        return Err(ApiError::BadRequest(
            "start_date must not be after end_date".to_string(),
        ));
    }

    Ok(Json(profit_loss_entries(&state.db, &start, &end).await?))
}

#[derive(Deserialize)]
struct YearQuery {
    year: Option<u16>,
}

#[derive(Serialize)]
struct Nec1099Entry {
    contact_id: i64,
    contact_name: String,
    /// Paid to the contractor from bank/cash accounts during the year.
    ytd_cents: i64,
    threshold_cents: i64,
    /// Meets or exceeds the threshold: a 1099-NEC is required.
    over_threshold: bool,
}

/// 1099-NEC summary: what you paid each contractor in `year` (default: the
/// current local year). Card and payment-app payments are excluded; the
/// processor reports those on 1099-K.
async fn nec_1099(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<YearQuery>,
) -> Result<Json<Vec<Nec1099Entry>>, ApiError> {
    let year = match q.year {
        Some(y) => y,
        None => u16::try_from(chrono::Local::now().year())
            .map_err(|_| ApiError::Internal("Current year out of range".to_string()))?,
    };
    if !(1000..=9999).contains(&year) {
        return Err(ApiError::BadRequest(format!(
            "year must be a four-digit year, got {year}"
        )));
    }

    let threshold_cents = aequi_core::nec_threshold(year).to_cents();
    let rows = aequi_storage::get_contractor_ytd_payments(&state.db, year).await?;
    Ok(Json(
        rows.into_iter()
            .map(|(contact_id, contact_name, ytd_cents)| Nec1099Entry {
                contact_id,
                contact_name,
                ytd_cents,
                threshold_cents,
                over_threshold: aequi_core::check_1099_threshold(
                    aequi_core::Money::from_cents(ytd_cents),
                    year,
                ),
            })
            .collect(),
    ))
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/reports/profit-loss", get(profit_loss))
        .route("/reports/1099-nec", get(nec_1099))
}

#[cfg(test)]
mod tests {
    use super::*;

    use aequi_core::{Money, TransactionLine, UnvalidatedTransaction};
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use crate::routes::tests::{call, contact, test_state};

    /// Post `cents` from `credit` to `debit` on `date`, optionally to a payee.
    async fn post_to(
        db: &aequi_storage::DbPool,
        date: &str,
        debit: &str,
        credit: &str,
        cents: i64,
        contact_id: Option<i64>,
    ) {
        let mut lines = Vec::new();
        for (code, d, c) in [(debit, cents, 0), (credit, 0, cents)] {
            let account_id = aequi_storage::get_account_by_code(db, code)
                .await
                .unwrap()
                .unwrap()
                .id
                .unwrap();
            lines.push(TransactionLine {
                account_id,
                debit: Money::from_cents(d),
                credit: Money::from_cents(c),
                memo: None,
            });
        }
        let tx = UnvalidatedTransaction {
            date: chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            description: "x".to_string(),
            lines,
            memo: None,
        };
        aequi_storage::create_ledger_transaction(db, tx, contact_id)
            .await
            .unwrap();
    }

    async fn post(db: &aequi_storage::DbPool, date: &str, debit: &str, credit: &str, cents: i64) {
        post_to(db, date, debit, credit, cents, None).await;
    }

    #[tokio::test]
    async fn profit_loss_only_sums_transactions_in_range() {
        let state = test_state(None).await;
        let db = &state.db;
        post(db, "2025-12-31", "1000", "4000", 70_000).await; // before range
        post(db, "2026-02-10", "1000", "4000", 10_000).await;
        post(db, "2026-03-05", "5000", "1000", 2_500).await;
        post(db, "2026-04-01", "1000", "4000", 99_000).await; // after range

        let entries = profit_loss_entries(db, "2026-01-01", "2026-03-31")
            .await
            .unwrap();
        let total = |code: &str| {
            entries
                .iter()
                .find(|e| e.account_code == code)
                .unwrap()
                .total_cents
        };
        assert_eq!(total("4000"), 10_000);
        assert_eq!(total("5000"), -2_500);
        // Accounts without activity in the range are still listed.
        assert!(entries.iter().any(|e| e.total_cents == 0));
        assert!(
            entries
                .iter()
                .all(|e| e.account_type == "Income" || e.account_type == "Expense")
        );
    }

    #[tokio::test]
    async fn nec_1099_summarizes_payments_to_contractors() {
        let state = test_state(None).await;
        let db = &state.db;
        let big = contact(&state, "Big Contractor", true).await;
        let small = contact(&state, "Small Contractor", true).await;
        let client = contact(&state, "Client Co", false).await;

        // Paid from Checking: counts. Over two payments, $2,100 >= $2,000 (2026).
        post_to(db, "2026-02-01", "5080", "1000", 150_000, Some(big)).await;
        post_to(db, "2026-06-01", "5080", "1000", 60_000, Some(big)).await;
        // Paid by credit card: reported on 1099-K, not counted.
        post_to(db, "2026-07-01", "5080", "2000", 500_000, Some(small)).await;
        post_to(db, "2026-07-02", "5080", "1000", 70_000, Some(small)).await;
        // Other years and non-contractors don't count.
        post_to(db, "2025-12-31", "5080", "1000", 900_000, Some(small)).await;
        post_to(db, "2026-03-01", "5080", "1000", 900_000, Some(client)).await;

        let (status, body) = call(
            &state,
            Method::GET,
            "/api/v1/reports/1099-nec?year=2026",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body,
            json!([
                {"contact_id": big, "contact_name": "Big Contractor", "ytd_cents": 210_000,
                 "threshold_cents": 200_000, "over_threshold": true},
                {"contact_id": small, "contact_name": "Small Contractor", "ytd_cents": 70_000,
                 "threshold_cents": 200_000, "over_threshold": false},
            ])
        );

        // The threshold was $600 before 2026.
        let (status, body) = call(
            &state,
            Method::GET,
            "/api/v1/reports/1099-nec?year=2025",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let small_2025 = body
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["contact_id"] == small)
            .unwrap();
        assert_eq!(small_2025["ytd_cents"], 900_000);
        assert_eq!(small_2025["threshold_cents"], 60_000);
        assert_eq!(small_2025["over_threshold"], true);

        // Defaults to the current year.
        let (status, default_year) =
            call(&state, Method::GET, "/api/v1/reports/1099-nec", None).await;
        assert_eq!(status, StatusCode::OK);
        let this_year = chrono::Local::now().year();
        let (_, explicit) = call(
            &state,
            Method::GET,
            &format!("/api/v1/reports/1099-nec?year={this_year}"),
            None,
        )
        .await;
        assert_eq!(default_year, explicit);
    }

    #[tokio::test]
    async fn nec_1099_rejects_bad_year() {
        let state = test_state(None).await;
        for uri in [
            "/api/v1/reports/1099-nec?year=99",
            "/api/v1/reports/1099-nec?year=abc",
            "/api/v1/reports/1099-nec?year=70000",
        ] {
            let (status, _) = call(&state, Method::GET, uri, None).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        }
    }
}
