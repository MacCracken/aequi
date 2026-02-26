use aequi_core::{Account, Money, TransactionLine, UnvalidatedTransaction, ValidatedTransaction};
use aequi_ocr::{ReceiptPipeline, MockRecognizer};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

use crate::AppState;

#[derive(Debug, Serialize)]
pub struct CommandError {
    pub message: String,
}

impl From<sqlx::Error> for CommandError {
    fn from(e: sqlx::Error) -> Self {
        CommandError {
            message: e.to_string(),
        }
    }
}

impl From<aequi_core::LedgerError> for CommandError {
    fn from(e: aequi_core::LedgerError) -> Self {
        CommandError { message: e.to_string() }
    }
}

impl From<aequi_ocr::PipelineError> for CommandError {
    fn from(e: aequi_ocr::PipelineError) -> Self {
        CommandError { message: e.to_string() }
    }
}

#[derive(Debug, Deserialize)]
pub struct TransactionInput {
    pub date: String,
    pub description: String,
    pub lines: Vec<TransactionLineInput>,
    pub memo: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TransactionLineInput {
    pub account_code: String,
    pub debit_cents: i64,
    pub credit_cents: i64,
    pub memo: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TransactionOutput {
    pub id: i64,
    pub date: String,
    pub description: String,
    pub balanced_total: String,
    pub memo: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ProfitLossEntry {
    pub account_code: String,
    pub account_name: String,
    pub total: String,
}

#[tauri::command]
pub async fn get_accounts(state: State<'_, Arc<Mutex<AppState>>>) -> Result<Vec<Account>, CommandError> {
    let state = state.lock().await;
    let accounts = aequi_storage::get_all_accounts(&state.db).await?;
    Ok(accounts)
}

#[tauri::command]
pub async fn create_transaction(
    state: State<'_, Arc<Mutex<AppState>>>,
    input: TransactionInput,
) -> Result<TransactionOutput, CommandError> {
    let state = state.lock().await;
    let db = &state.db;

    let date = NaiveDate::parse_from_str(&input.date, "%Y-%m-%d")
        .map_err(|e| CommandError { message: e.to_string() })?;

    let mut lines = Vec::new();
    for line in input.lines {
        let account = aequi_storage::get_account_by_code(db, &line.account_code)
            .await?
            .ok_or_else(|| CommandError {
                message: format!("Account not found: {}", line.account_code),
            })?;

        let debit = Money::from_cents(line.debit_cents);
        let credit = Money::from_cents(line.credit_cents);

        lines.push(TransactionLine {
            account_id: account.id.unwrap(),
            debit,
            credit,
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

    let result = sqlx::query(
        "INSERT INTO transactions (date, description, memo, balanced_total_cents) VALUES (?, ?, ?, ?) RETURNING id, date, description, memo, balanced_total_cents, created_at"
    )
    .bind(validated.date.to_string())
    .bind(&validated.description)
    .bind(&validated.memo)
    .bind(validated.balanced_total.to_cents())
    .fetch_one(db)
    .await?;

    let id: i64 = result.get("id");
    let balanced_cents: i64 = result.get("balanced_total_cents");

    for line in validated.lines {
        sqlx::query(
            "INSERT INTO transaction_lines (transaction_id, account_id, debit_cents, credit_cents, memo) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(id)
        .bind(line.account_id.0)
        .bind(line.debit.to_cents())
        .bind(line.credit.to_cents())
        .bind(&line.memo)
        .execute(db)
        .await?;
    }

    let created_at: String = result.get("created_at");

    Ok(TransactionOutput {
        id,
        date: validated.date.to_string(),
        description: validated.description,
        balanced_total: Money::from_cents(balanced_cents).to_string(),
        memo: validated.memo,
        created_at,
    })
}

#[tauri::command]
pub async fn get_transactions(
    state: State<'_, Arc<Mutex<AppState>>>,
    start_date: Option<String>,
    end_date: Option<String>,
) -> Result<Vec<TransactionOutput>, CommandError> {
    let state = state.lock().await;
    let db = &state.db;

    let query = match (start_date, end_date) {
        (Some(start), Some(end)) => {
            sqlx::query_as::<_, (i64, String, String, Option<String>, i64, String)>(
                "SELECT id, date, description, memo, balanced_total_cents, created_at FROM transactions WHERE date >= ? AND date <= ? ORDER BY date DESC, id DESC"
            )
            .bind(start)
            .bind(end)
            .fetch_all(db)
            .await?
        },
        _ => {
            sqlx::query_as::<_, (i64, String, String, Option<String>, i64, String)>(
                "SELECT id, date, description, memo, balanced_total_cents, created_at FROM transactions ORDER BY date DESC, id DESC"
            )
            .fetch_all(db)
            .await?
        }
    };

    Ok(query.into_iter().map(|r| {
        TransactionOutput {
            id: r.0,
            date: r.1,
            description: r.2,
            memo: r.3,
            balanced_total: Money::from_cents(r.4).to_string(),
            created_at: r.5,
        }
    }).collect())
}

#[tauri::command]
pub async fn get_profit_loss(
    state: State<'_, Arc<Mutex<AppState>>>,
    start_date: Option<String>,
    end_date: Option<String>,
) -> Result<Vec<ProfitLossEntry>, CommandError> {
    let state = state.lock().await;
    let db = &state.db;

    let (start, end) = match (start_date, end_date) {
        (Some(s), Some(e)) => (s, e),
        _ => {
            let now = chrono::Utc::now().date_naive();
            let start = NaiveDate::from_ymd_opt(now.year(), 1, 1).unwrap();
            let end = now;
            (start.to_string(), end.to_string())
        }
    };

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
        "#
    )
    .bind(&start)
    .bind(&end)
    .fetch_all(db)
    .await?;

    Ok(rows.into_iter().map(|r| {
        let total_cents: i64 = r.get("total_cents");
        ProfitLossEntry {
            account_code: r.get("code"),
            account_name: r.get("name"),
            total: Money::from_cents(total_cents).to_string(),
        }
    }).collect())
}

// ── Receipt commands ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ReceiptOutput {
    pub id: i64,
    pub file_hash: String,
    pub vendor: Option<String>,
    pub receipt_date: Option<String>,
    pub total_cents: Option<i64>,
    pub subtotal_cents: Option<i64>,
    pub tax_cents: Option<i64>,
    pub payment_method: Option<String>,
    pub confidence: f64,
    pub status: String,
    pub transaction_id: Option<i64>,
    pub attachment_path: String,
    pub needs_review: bool,
    pub created_at: String,
}

impl From<aequi_storage::ReceiptRecord> for ReceiptOutput {
    fn from(r: aequi_storage::ReceiptRecord) -> Self {
        let needs_review = r.confidence < 0.7;
        ReceiptOutput {
            id: r.id,
            file_hash: r.file_hash,
            vendor: r.vendor,
            receipt_date: r.receipt_date,
            total_cents: r.total_cents,
            subtotal_cents: r.subtotal_cents,
            tax_cents: r.tax_cents,
            payment_method: r.payment_method,
            confidence: r.confidence,
            status: r.status,
            transaction_id: r.transaction_id,
            attachment_path: r.attachment_path,
            needs_review,
            created_at: r.created_at,
        }
    }
}

/// Ingest a receipt from a file path on disk.
/// Processes the image through the OCR pipeline and stores the result.
#[tauri::command]
pub async fn ingest_receipt(
    state: State<'_, Arc<Mutex<AppState>>>,
    file_path: String,
) -> Result<ReceiptOutput, CommandError> {
    let path = PathBuf::from(&file_path);
    let (db, attachments_dir) = {
        let s = state.lock().await;
        (s.db.clone(), s.attachments_dir.clone())
    };

    // Use MockRecognizer by default; swap for TesseractRecognizer when the
    // `tesseract` feature is enabled and Tesseract data is available.
    let pipeline = ReceiptPipeline::new(MockRecognizer::new(""), attachments_dir);
    let result = pipeline.process_file(&path).await?;

    let e = &result.extracted;
    let id = aequi_storage::insert_receipt(
        &db,
        &result.hash_hex,
        path.extension().and_then(|x| x.to_str()).unwrap_or("bin"),
        result.attachment_path.to_str().unwrap_or(""),
        Some(&result.ocr_text),
        e.vendor.as_ref().map(|f| f.value.as_str()),
        e.date.as_ref().map(|f| f.value.to_string()).as_deref(),
        e.total_cents.as_ref().map(|f| f.value),
        e.subtotal_cents.as_ref().map(|f| f.value),
        e.tax_cents.as_ref().map(|f| f.value),
        e.payment_method.as_ref().map(|f| f.value.to_string()).as_deref(),
        e.confidence as f64,
    )
    .await
    .map_err(|e| CommandError { message: e.to_string() })?;

    let record = aequi_storage::get_receipt_by_id(&db, id)
        .await
        .map_err(|e| CommandError { message: e.to_string() })?
        .ok_or(CommandError { message: "Receipt not found after insert".into() })?;

    Ok(record.into())
}

/// Return all receipts currently awaiting review.
#[tauri::command]
pub async fn get_pending_receipts(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<ReceiptOutput>, CommandError> {
    let state = state.lock().await;
    let records = aequi_storage::get_receipts_pending_review(&state.db)
        .await
        .map_err(|e| CommandError { message: e.to_string() })?;
    Ok(records.into_iter().map(ReceiptOutput::from).collect())
}

/// Approve a receipt, optionally linking it to an existing transaction.
#[tauri::command]
pub async fn approve_receipt(
    state: State<'_, Arc<Mutex<AppState>>>,
    receipt_id: i64,
    transaction_id: Option<i64>,
) -> Result<(), CommandError> {
    let state = state.lock().await;
    if let Some(tx_id) = transaction_id {
        aequi_storage::link_receipt_to_transaction(&state.db, receipt_id, tx_id)
            .await
            .map_err(|e| CommandError { message: e.to_string() })?;
    } else {
        aequi_storage::update_receipt_status(&state.db, receipt_id, "approved")
            .await
            .map_err(|e| CommandError { message: e.to_string() })?;
    }
    Ok(())
}

/// Reject a receipt (marks it as not usable / duplicate).
#[tauri::command]
pub async fn reject_receipt(
    state: State<'_, Arc<Mutex<AppState>>>,
    receipt_id: i64,
) -> Result<(), CommandError> {
    let state = state.lock().await;
    aequi_storage::update_receipt_status(&state.db, receipt_id, "rejected")
        .await
        .map_err(|e| CommandError { message: e.to_string() })?;
    Ok(())
}
