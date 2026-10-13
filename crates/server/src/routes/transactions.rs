use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use aequi_core::{Money, TransactionLine, UnvalidatedTransaction};

use crate::error::ApiError;
use crate::state::ServerState;

#[derive(Deserialize)]
struct CreateTransaction {
    date: String,
    description: String,
    lines: Vec<LineInput>,
    memo: Option<String>,
    /// Payee or payer. Money paid from a bank account to a contractor counts
    /// toward their 1099-NEC total.
    contact_id: Option<i64>,
}

#[derive(Deserialize)]
struct LineInput {
    account_code: String,
    debit_cents: i64,
    credit_cents: i64,
    memo: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
struct TransactionOut {
    id: i64,
    date: String,
    description: String,
    balanced_total_cents: i64,
    memo: Option<String>,
    contact_id: Option<i64>,
}

/// `SELECT … FROM transactions` with the columns of [`TransactionOut`],
/// followed by `$rest` (sqlx only accepts static SQL).
macro_rules! select_transactions {
    ($rest:literal) => {
        concat!(
            "SELECT id, date, description, balanced_total_cents, memo, contact_id FROM transactions ",
            $rest
        )
    };
}

async fn list_transactions(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<TransactionOut>>, ApiError> {
    let rows = sqlx::query_as::<_, TransactionOut>(select_transactions!(
        "ORDER BY date DESC, id DESC LIMIT 1000"
    ))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

/// Post a journal entry. Line rules (at least two lines, each a positive debit
/// or credit, balanced) are enforced by core; storage rejects archived
/// accounts and unknown contacts.
async fn create_transaction(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<CreateTransaction>,
) -> Result<Json<TransactionOut>, ApiError> {
    if input.description.trim().is_empty() {
        return Err(ApiError::BadRequest("Description is required".to_string()));
    }
    aequi_storage::validate_date("date", &input.date)?;
    let date = chrono::NaiveDate::parse_from_str(&input.date, "%Y-%m-%d")
        .map_err(|e| ApiError::BadRequest(format!("Invalid date: {e}")))?;

    let mut lines = Vec::with_capacity(input.lines.len());
    for line in input.lines {
        let account = aequi_storage::get_account_by_code(&state.db, &line.account_code)
            .await?
            .ok_or_else(|| {
                ApiError::BadRequest(format!("Account {} does not exist", line.account_code))
            })?;
        if account.is_archived {
            return Err(ApiError::BadRequest(format!(
                "Account {} is archived",
                line.account_code
            )));
        }
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
    let id = aequi_storage::create_ledger_transaction(&state.db, tx, input.contact_id).await?;

    let out = sqlx::query_as::<_, TransactionOut>(select_transactions!("WHERE id = ?"))
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(out))
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new().route(
        "/transactions",
        get(list_transactions).post(create_transaction),
    )
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::{Value, json};

    use crate::routes::tests::{call, contact, test_state};

    fn payment(contact_id: Option<i64>) -> Value {
        json!({
            "date": "2026-04-15",
            "description": "Logo design",
            "memo": "Check #1001",
            "contact_id": contact_id,
            "lines": [
                {"account_code": "5080", "debit_cents": 50_000, "credit_cents": 0},
                {"account_code": "1000", "debit_cents": 0, "credit_cents": 50_000, "memo": "check"}
            ]
        })
    }

    #[tokio::test]
    async fn create_transaction_records_payee() {
        let state = test_state(None).await;
        let designer = contact(&state, "Pat Designer", true).await;

        let (status, tx) = call(
            &state,
            Method::POST,
            "/api/v1/transactions",
            Some(payment(Some(designer))),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{tx}");
        assert_eq!(tx["contact_id"], designer);
        assert_eq!(tx["balanced_total_cents"], 50_000);
        assert_eq!(tx["date"], "2026-04-15");
        assert_eq!(tx["memo"], "Check #1001");
        let id = tx["id"].as_i64().unwrap();

        let (n,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM transaction_lines WHERE transaction_id = ?")
                .bind(id)
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(n, 2);

        // Without a payee.
        let (status, tx) = call(
            &state,
            Method::POST,
            "/api/v1/transactions",
            Some(payment(None)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tx["contact_id"], Value::Null);

        let (status, list) = call(&state, Method::GET, "/api/v1/transactions", None).await;
        assert_eq!(status, StatusCode::OK);
        let list = list.as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert!(
            list.iter()
                .any(|t| t["id"] == id && t["contact_id"] == designer)
        );
    }

    #[tokio::test]
    async fn create_transaction_validation_errors_are_400() {
        let state = test_state(None).await;
        let post = |body: Value| {
            let state = state.clone();
            async move { call(&state, Method::POST, "/api/v1/transactions", Some(body)).await }
        };

        // Unknown payee.
        let (status, body) = post(payment(Some(999))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("contact 999"));

        // Unknown account code.
        let mut tx = payment(None);
        tx["lines"][0]["account_code"] = json!("9999");
        let (status, body) = post(tx).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("9999"));

        // Archived account.
        sqlx::query("UPDATE accounts SET is_archived = 1 WHERE code = '5080'")
            .execute(&state.db)
            .await
            .unwrap();
        let (status, body) = post(payment(None)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("archived"));
        sqlx::query("UPDATE accounts SET is_archived = 0 WHERE code = '5080'")
            .execute(&state.db)
            .await
            .unwrap();

        // Unbalanced, negative, single-line, blank description, bad date.
        let mut tx = payment(None);
        tx["lines"][1]["credit_cents"] = json!(40_000);
        assert_eq!(post(tx).await.0, StatusCode::BAD_REQUEST);
        let mut tx = payment(None);
        tx["lines"][0]["debit_cents"] = json!(-50_000);
        tx["lines"][1]["credit_cents"] = json!(-50_000);
        assert_eq!(post(tx).await.0, StatusCode::BAD_REQUEST);
        let mut tx = payment(None);
        tx["lines"] = json!([tx["lines"][0].clone()]);
        assert_eq!(post(tx).await.0, StatusCode::BAD_REQUEST);
        let mut tx = payment(None);
        tx["description"] = json!("  ");
        assert_eq!(post(tx).await.0, StatusCode::BAD_REQUEST);
        let mut tx = payment(None);
        tx["date"] = json!("2026-4-15");
        assert_eq!(post(tx).await.0, StatusCode::BAD_REQUEST);

        // Nothing was written.
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transactions")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }
}
