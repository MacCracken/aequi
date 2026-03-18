use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use aequi_core::{Money, TransactionLine, UnvalidatedTransaction, ValidatedTransaction};

use crate::error::ApiError;
use crate::state::ServerState;

#[derive(Deserialize)]
struct CreateTransaction {
    date: String,
    description: String,
    lines: Vec<LineInput>,
    memo: Option<String>,
}

#[derive(Deserialize)]
struct LineInput {
    account_code: String,
    debit_cents: i64,
    credit_cents: i64,
    memo: Option<String>,
}

#[derive(Serialize)]
struct TransactionOut {
    id: i64,
    date: String,
    description: String,
    balanced_total_cents: i64,
    memo: Option<String>,
}

async fn list_transactions(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<TransactionOut>>, ApiError> {
    let rows = sqlx::query_as::<_, (i64, String, String, Option<String>, i64)>(
        "SELECT id, date, description, memo, balanced_total_cents FROM transactions ORDER BY date DESC, id DESC LIMIT 1000"
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| TransactionOut {
                id: r.0,
                date: r.1,
                description: r.2,
                memo: r.3,
                balanced_total_cents: r.4,
            })
            .collect(),
    ))
}

async fn create_transaction(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<CreateTransaction>,
) -> Result<Json<TransactionOut>, ApiError> {
    if input.description.trim().is_empty() {
        return Err(ApiError::BadRequest("Description is required".to_string()));
    }
    if input.lines.is_empty() {
        return Err(ApiError::BadRequest(
            "At least one line is required".to_string(),
        ));
    }
    for line in &input.lines {
        if line.debit_cents < 0 || line.credit_cents < 0 {
            return Err(ApiError::BadRequest(
                "Debit and credit must be non-negative".to_string(),
            ));
        }
    }

    let date = chrono::NaiveDate::parse_from_str(&input.date, "%Y-%m-%d")
        .map_err(|e| ApiError::BadRequest(format!("Invalid date: {e}")))?;

    let mut lines = Vec::new();
    for line in input.lines {
        let account = aequi_storage::get_account_by_code(&state.db, &line.account_code)
            .await?
            .ok_or_else(|| {
                ApiError::NotFound(format!("Account {} not found", line.account_code))
            })?;

        lines.push(TransactionLine {
            account_id: account
                .id
                .ok_or_else(|| ApiError::Internal("Account missing ID".to_string()))?,
            debit: Money::from_cents(line.debit_cents),
            credit: Money::from_cents(line.credit_cents),
            memo: line.memo,
        });
    }

    let tx = UnvalidatedTransaction {
        date,
        description: input.description,
        lines,
        memo: input.memo,
    };

    let validated = ValidatedTransaction::validate(tx)?;

    let mut db_tx = state.db.begin().await?;

    let row = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO transactions (date, description, memo, balanced_total_cents) VALUES (?, ?, ?, ?) RETURNING id"
    )
    .bind(validated.date.to_string())
    .bind(&validated.description)
    .bind(&validated.memo)
    .bind(validated.balanced_total.to_cents())
    .fetch_one(&mut *db_tx)
    .await?;

    let id = row.0;

    for line in &validated.lines {
        sqlx::query(
            "INSERT INTO transaction_lines (transaction_id, account_id, debit_cents, credit_cents, memo) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(id)
        .bind(line.account_id.0)
        .bind(line.debit.to_cents())
        .bind(line.credit.to_cents())
        .bind(&line.memo)
        .execute(&mut *db_tx)
        .await?;
    }

    db_tx.commit().await?;

    Ok(Json(TransactionOut {
        id,
        date: validated.date.to_string(),
        description: validated.description,
        balanced_total_cents: validated.balanced_total.to_cents(),
        memo: validated.memo,
    }))
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new().route(
        "/transactions",
        get(list_transactions).post(create_transaction),
    )
}
