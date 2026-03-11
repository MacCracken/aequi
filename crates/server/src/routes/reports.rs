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
    total_cents: i64,
}

async fn profit_loss(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<DateRange>,
) -> Result<Json<Vec<ProfitLossEntry>>, ApiError> {
    let now = chrono::Utc::now().date_naive();
    let start = q
        .start_date
        .unwrap_or_else(|| format!("{}-01-01", now.format("%Y")));
    let end = q.end_date.unwrap_or_else(|| now.to_string());

    let rows = sqlx::query(
        r#"
        SELECT a.code, a.name,
            COALESCE(SUM(tl.credit_cents - tl.debit_cents), 0) as total_cents
        FROM accounts a
        LEFT JOIN transaction_lines tl ON a.id = tl.account_id
        LEFT JOIN transactions t ON tl.transaction_id = t.id
            AND t.date >= ? AND t.date <= ?
        WHERE a.account_type IN ('Income', 'Expense')
        GROUP BY a.id, a.code, a.name
        ORDER BY a.account_type, a.code
        "#,
    )
    .bind(&start)
    .bind(&end)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| {
                let total_cents: i64 = r.get("total_cents");
                ProfitLossEntry {
                    account_code: r.get("code"),
                    account_name: r.get("name"),
                    total_cents,
                }
            })
            .collect(),
    ))
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new().route("/reports/profit-loss", get(profit_loss))
}
