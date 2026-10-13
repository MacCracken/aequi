use std::sync::Arc;

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
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

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new().route("/reports/profit-loss", get(profit_loss))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn post(db: &aequi_storage::DbPool, date: &str, debit: &str, credit: &str, cents: i64) {
        let id = |code: &str| {
            let code = code.to_string();
            async move {
                aequi_storage::get_account_by_code(db, &code)
                    .await
                    .unwrap()
                    .unwrap()
                    .id
                    .unwrap()
                    .0
            }
        };
        let (debit_id, credit_id) = (id(debit).await, id(credit).await);
        let tx: (i64,) = sqlx::query_as(
            "INSERT INTO transactions (date, description, balanced_total_cents) VALUES (?, 'x', ?) RETURNING id",
        )
        .bind(date)
        .bind(cents)
        .fetch_one(db)
        .await
        .unwrap();
        for (acct, d, c) in [(debit_id, cents, 0), (credit_id, 0, cents)] {
            sqlx::query(
                "INSERT INTO transaction_lines (transaction_id, account_id, debit_cents, credit_cents) VALUES (?, ?, ?, ?)",
            )
            .bind(tx.0)
            .bind(acct)
            .bind(d)
            .bind(c)
            .execute(db)
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn profit_loss_only_sums_transactions_in_range() {
        let state = crate::routes::tests::test_state(None).await;
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
}
