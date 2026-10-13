use aequi_core::{
    Account, ContactId, FiscalYear, Money, Quarter, TransactionLine, UnvalidatedTransaction,
    ValidatedTransaction,
};
use aequi_ocr::{PipelineError, ReceiptPipeline};
use aequi_storage::{DbPool, InvoiceDetail, InvoiceSummary, NewInvoice};
use chrono::{Datelike, NaiveDate};
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

use crate::{AppState, Recognizer};

#[derive(Debug, Serialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    pub fn validation(msg: impl Into<String>) -> Self {
        CommandError {
            code: "VALIDATION".into(),
            message: msg.into(),
        }
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        CommandError {
            code: "NOT_FOUND".into(),
            message: msg.into(),
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        CommandError {
            code: "INTERNAL".into(),
            message: msg.into(),
        }
    }

    pub fn config(msg: impl Into<String>) -> Self {
        CommandError {
            code: "CONFIG".into(),
            message: msg.into(),
        }
    }
}

impl From<sqlx::Error> for CommandError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            // Storage reports bad input this way.
            sqlx::Error::InvalidArgument(msg) => CommandError::validation(msg),
            sqlx::Error::RowNotFound => CommandError::not_found("Record not found"),
            sqlx::Error::Database(db) if db.is_unique_violation() => CommandError::validation(
                format!("A record with this value already exists ({})", db.message()),
            ),
            sqlx::Error::Database(db) if db.is_foreign_key_violation() => {
                CommandError::validation("Referenced record does not exist")
            }
            other => {
                tracing::error!("Database error: {other}");
                CommandError {
                    code: "DATABASE".into(),
                    message: "A database error occurred".into(),
                }
            }
        }
    }
}

impl From<aequi_core::LedgerError> for CommandError {
    fn from(e: aequi_core::LedgerError) -> Self {
        CommandError::validation(e.to_string())
    }
}

impl From<PipelineError> for CommandError {
    fn from(e: PipelineError) -> Self {
        match e {
            PipelineError::NotAFile(_) | PipelineError::TooLarge { .. } => {
                CommandError::validation(e.to_string())
            }
            PipelineError::Preprocess(e) => {
                CommandError::validation(format!("Not a supported image file: {e}"))
            }
            PipelineError::Io(ref io)
                if matches!(
                    io.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                CommandError::validation(format!("Cannot read file: {io}"))
            }
            PipelineError::Ocr(_) => CommandError {
                code: "OCR".into(),
                message: e.to_string(),
            },
            PipelineError::Io(_) | PipelineError::Task(_) => CommandError::internal(e.to_string()),
        }
    }
}

impl From<aequi_storage::backup::BackupError> for CommandError {
    fn from(e: aequi_storage::backup::BackupError) -> Self {
        match e {
            aequi_storage::backup::BackupError::InvalidArchive(msg) => {
                CommandError::validation(format!("Invalid backup archive: {msg}"))
            }
            other => CommandError::internal(other.to_string()),
        }
    }
}

/// Convert an amount for output. Every figure derived from validated ledger
/// data fits, but a corrupt database must not panic the command.
fn cents(m: Money) -> Result<i64, CommandError> {
    m.checked_to_cents()
        .ok_or_else(|| CommandError::internal("Amount out of range"))
}

/// The user's calendar date ("today" is local, not UTC).
fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

#[derive(Debug, Deserialize)]
pub struct TransactionInput {
    pub date: String,
    pub description: String,
    pub lines: Vec<TransactionLineInput>,
    pub memo: Option<String>,
    /// The payee or payer, e.g. the contractor a payment went to (counts
    /// toward their 1099-NEC total when paid from a bank or cash account).
    #[serde(default)]
    pub contact_id: Option<i64>,
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
    pub contact_id: Option<i64>,
    pub contact_name: Option<String>,
}

/// A `transactions` row joined with its contact's name.
#[derive(sqlx::FromRow)]
struct TransactionRow {
    id: i64,
    date: String,
    description: String,
    memo: Option<String>,
    balanced_total_cents: i64,
    created_at: String,
    contact_id: Option<i64>,
    contact_name: Option<String>,
}

impl From<TransactionRow> for TransactionOutput {
    fn from(r: TransactionRow) -> Self {
        TransactionOutput {
            id: r.id,
            date: r.date,
            description: r.description,
            balanced_total: Money::from_cents(r.balanced_total_cents).to_string(),
            memo: r.memo,
            created_at: r.created_at,
            contact_id: r.contact_id,
            contact_name: r.contact_name,
        }
    }
}

/// The transaction query that fills a [`TransactionRow`], followed by `$tail`
/// (filter / order / limit; columns are qualified with `t.`).
macro_rules! transaction_query {
    ($tail:literal) => {
        concat!(
            "SELECT t.id, t.date, t.description, t.memo, t.balanced_total_cents, t.created_at, \
             t.contact_id, c.name AS contact_name \
             FROM transactions t LEFT JOIN contacts c ON c.id = t.contact_id ",
            $tail
        )
    };
}

#[derive(Debug, Serialize)]
pub struct ProfitLossEntry {
    pub account_code: String,
    pub account_name: String,
    /// Net credit (credits minus debits) for the period.
    pub total_cents: i64,
}

#[tauri::command]
pub async fn get_accounts(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<Account>, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let accounts = aequi_storage::get_all_accounts(&db).await?;
    Ok(accounts)
}

#[tauri::command]
pub async fn create_transaction(
    state: State<'_, Arc<Mutex<AppState>>>,
    input: TransactionInput,
) -> Result<TransactionOutput, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    insert_transaction(&db, input).await
}

/// Resolve account codes and post the transaction. Storage validates the
/// entry (balanced, non-negative, two or more lines), rejects archived
/// accounts and an unknown contact, and inserts it atomically.
async fn insert_transaction(
    db: &DbPool,
    input: TransactionInput,
) -> Result<TransactionOutput, CommandError> {
    let description = input.description.trim().to_string();
    if description.is_empty() {
        return Err(CommandError::validation(
            "Transaction description is required",
        ));
    }
    if input.lines.is_empty() {
        return Err(CommandError::validation(
            "Transaction must have at least one line",
        ));
    }

    let date = NaiveDate::parse_from_str(&input.date, "%Y-%m-%d")
        .map_err(|_| CommandError::validation("Invalid date format (expected YYYY-MM-DD)"))?;

    let mut lines = Vec::with_capacity(input.lines.len());
    for line in input.lines {
        let account = aequi_storage::get_account_by_code(db, &line.account_code)
            .await?
            .ok_or_else(|| {
                CommandError::not_found(format!("Account not found: {}", line.account_code))
            })?;

        lines.push(TransactionLine {
            account_id: account
                .id
                .ok_or_else(|| CommandError::internal("Account missing ID"))?,
            debit: Money::from_cents(line.debit_cents),
            credit: Money::from_cents(line.credit_cents),
            memo: line.memo,
        });
    }

    let tx = UnvalidatedTransaction {
        date,
        description,
        lines,
        memo: input.memo,
    };
    let id = aequi_storage::create_ledger_transaction(db, tx, input.contact_id).await?;

    sqlx::query_as::<_, TransactionRow>(transaction_query!("WHERE t.id = ?"))
        .bind(id)
        .fetch_optional(db)
        .await?
        .map(TransactionOutput::from)
        .ok_or_else(|| CommandError::internal("Transaction not found after insert"))
}

/// Sentinels for an open-ended date range. Dates are stored as YYYY-MM-DD
/// text, so they compare correctly as strings.
const MIN_DATE: &str = "0000-01-01";
const MAX_DATE: &str = "9999-12-31";

/// Validate optional inclusive date bounds. Each bound is independent; an
/// empty string counts as unset.
fn date_bounds(
    start_date: Option<String>,
    end_date: Option<String>,
) -> Result<(Option<String>, Option<String>), CommandError> {
    let check = |field: &str, value: Option<String>| -> Result<Option<String>, CommandError> {
        match value
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            Some(v) => {
                aequi_storage::validate_date(field, &v)?;
                Ok(Some(v))
            }
            None => Ok(None),
        }
    };
    let start = check("start_date", start_date)?;
    let end = check("end_date", end_date)?;
    if let (Some(s), Some(e)) = (&start, &end)
        && s > e
    {
        return Err(CommandError::validation(
            "start_date must be on or before end_date",
        ));
    }
    Ok((start, end))
}

#[tauri::command]
pub async fn get_transactions(
    state: State<'_, Arc<Mutex<AppState>>>,
    start_date: Option<String>,
    end_date: Option<String>,
) -> Result<Vec<TransactionOutput>, CommandError> {
    let (start, end) = date_bounds(start_date, end_date)?;
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    query_transactions(
        &db,
        start.as_deref().unwrap_or(MIN_DATE),
        end.as_deref().unwrap_or(MAX_DATE),
    )
    .await
}

async fn query_transactions(
    db: &DbPool,
    start: &str,
    end: &str,
) -> Result<Vec<TransactionOutput>, CommandError> {
    let rows = sqlx::query_as::<_, TransactionRow>(transaction_query!(
        "WHERE t.date >= ? AND t.date <= ? ORDER BY t.date DESC, t.id DESC"
    ))
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;

    Ok(rows.into_iter().map(TransactionOutput::from).collect())
}

/// Income and expense totals for the period. With no bounds at all this is
/// year-to-date; otherwise each given bound applies and a missing one is open.
#[tauri::command]
pub async fn get_profit_loss(
    state: State<'_, Arc<Mutex<AppState>>>,
    start_date: Option<String>,
    end_date: Option<String>,
) -> Result<Vec<ProfitLossEntry>, CommandError> {
    let (start, end) = match date_bounds(start_date, end_date)? {
        (None, None) => {
            let now = today();
            let year_start = NaiveDate::from_ymd_opt(now.year(), 1, 1).unwrap_or(now);
            (year_start.to_string(), now.to_string())
        }
        (start, end) => (
            start.unwrap_or_else(|| MIN_DATE.into()),
            end.unwrap_or_else(|| MAX_DATE.into()),
        ),
    };
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    query_profit_loss(&db, &start, &end).await
}

async fn query_profit_loss(
    db: &DbPool,
    start: &str,
    end: &str,
) -> Result<Vec<ProfitLossEntry>, CommandError> {
    // Filter lines by their transaction's date before the outer join, so only
    // in-range lines are summed while accounts without activity still appear.
    let rows = sqlx::query(
        r#"
        SELECT a.code, a.name, COALESCE(SUM(p.net_cents), 0) AS total_cents
        FROM accounts a
        LEFT JOIN (
            SELECT tl.account_id, tl.credit_cents - tl.debit_cents AS net_cents
            FROM transaction_lines tl
            JOIN transactions t ON t.id = tl.transaction_id
            WHERE t.date >= ? AND t.date <= ?
        ) p ON p.account_id = a.id
        WHERE a.account_type IN ('Income', 'Expense')
        GROUP BY a.id, a.code, a.name
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
            total_cents: r.get("total_cents"),
        })
        .collect())
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
    /// The fields were filled in by an AI agent (over MCP) rather than read
    /// by OCR: they have a confidence but no OCR text backs them. They still
    /// need the user's approval.
    pub filled_by_agent: bool,
    pub created_at: String,
}

impl From<aequi_storage::ReceiptRecord> for ReceiptOutput {
    fn from(r: aequi_storage::ReceiptRecord) -> Self {
        let needs_review = r.confidence < 0.7;
        let filled_by_agent =
            r.confidence > 0.0 && r.ocr_text.as_deref().is_none_or(|t| t.trim().is_empty());
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
            filled_by_agent,
            created_at: r.created_at,
        }
    }
}

/// Run a receipt file through the OCR pipeline and store it. The pipeline
/// rejects anything that isn't a regular image file of at most 50 MB before
/// copying it into attachments. Returns the receipt id — the existing one if
/// the same file (by content hash) was stored before.
pub(crate) async fn store_receipt(
    db: &DbPool,
    pipeline: &ReceiptPipeline<Recognizer>,
    path: &Path,
) -> Result<i64, CommandError> {
    let result = pipeline.process_file(path).await?;
    let attachment_path = result
        .attachment_path
        .to_str()
        .ok_or_else(|| CommandError::internal("Attachment path is not valid UTF-8"))?;

    let e = &result.extracted;
    let id = aequi_storage::insert_receipt(
        db,
        &result.hash_hex,
        &result.file_ext,
        attachment_path,
        Some(&result.ocr_text),
        e.vendor.as_ref().map(|f| f.value.as_str()),
        e.date.as_ref().map(|f| f.value.to_string()).as_deref(),
        e.total_cents.as_ref().map(|f| f.value),
        e.subtotal_cents.as_ref().map(|f| f.value),
        e.tax_cents.as_ref().map(|f| f.value),
        e.payment_method
            .as_ref()
            .map(|f| f.value.to_string())
            .as_deref(),
        e.confidence as f64,
    )
    .await?;
    Ok(id)
}

/// Whether `path` resolves to an entry inside `dir` (symlinks resolved).
async fn is_inside_dir(path: &Path, dir: &Path) -> bool {
    match (
        tokio::fs::canonicalize(path).await,
        tokio::fs::canonicalize(dir).await,
    ) {
        (Ok(path), Ok(dir)) => path != dir && path.starts_with(dir),
        _ => false,
    }
}

/// Ingest a receipt from a file path on disk (picked in a file dialog, or a
/// camera capture the frontend wrote to `$APPDATA/captures`).
/// Processes the image through the OCR pipeline and stores the result; a
/// capture file is deleted once it has been stored.
#[tauri::command]
pub async fn ingest_receipt(
    state: State<'_, Arc<Mutex<AppState>>>,
    file_path: String,
) -> Result<ReceiptOutput, CommandError> {
    let path = PathBuf::from(&file_path);

    let (db, pipeline, captures_dir) = {
        let s = state.lock().await;
        (
            s.db.clone(),
            Arc::clone(&s.pipeline),
            s.captures_dir.clone(),
        )
    };

    let is_capture = is_inside_dir(&path, &captures_dir).await;
    let id = store_receipt(&db, &pipeline, &path).await?;
    if is_capture && let Err(e) = tokio::fs::remove_file(&path).await {
        tracing::warn!("Failed to remove capture file {}: {e}", path.display());
    }

    let record = aequi_storage::get_receipt_by_id(&db, id)
        .await?
        .ok_or_else(|| CommandError::internal("Receipt not found after insert"))?;

    Ok(record.into())
}

/// Return all receipts currently awaiting review.
#[tauri::command]
pub async fn get_pending_receipts(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<ReceiptOutput>, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let records = aequi_storage::get_receipts_pending_review(&db).await?;
    Ok(records.into_iter().map(ReceiptOutput::from).collect())
}

/// Approve a receipt, optionally linking it to an existing transaction.
#[tauri::command]
pub async fn approve_receipt(
    state: State<'_, Arc<Mutex<AppState>>>,
    receipt_id: i64,
    transaction_id: Option<i64>,
) -> Result<(), CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };

    // Validate receipt exists and is still pending
    let receipt = aequi_storage::get_receipt_by_id(&db, receipt_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Receipt not found"))?;
    if receipt.status != "pending_review" {
        return Err(CommandError::validation(format!(
            "Receipt is already {} — cannot approve",
            receipt.status
        )));
    }

    if let Some(tx_id) = transaction_id {
        // Validate transaction exists
        let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM transactions WHERE id = ?")
            .bind(tx_id)
            .fetch_optional(&db)
            .await?;
        if exists.is_none() {
            return Err(CommandError::not_found("Transaction not found"));
        }

        aequi_storage::link_receipt_to_transaction(&db, receipt_id, tx_id).await?;
    } else {
        aequi_storage::update_receipt_status(&db, receipt_id, "approved").await?;
    }
    Ok(())
}

/// Reject a receipt (marks it as not usable / duplicate).
#[tauri::command]
pub async fn reject_receipt(
    state: State<'_, Arc<Mutex<AppState>>>,
    receipt_id: i64,
) -> Result<(), CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    aequi_storage::update_receipt_status(&db, receipt_id, "rejected").await?;
    Ok(())
}

// ── Tax commands ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct QuarterlyEstimateOutput {
    pub year: u16,
    pub quarter: String,
    pub ytd_gross_income_cents: i64,
    pub ytd_total_expenses_cents: i64,
    pub ytd_net_profit_cents: i64,
    /// Factor scaling year-to-date profit to a full year (1 for a full year).
    pub annualization_factor: f64,
    /// Projected full-year net profit; the tax figures below project from it.
    pub annualized_net_profit_cents: i64,
    pub se_tax_cents: i64,
    pub se_tax_deduction_cents: i64,
    pub additional_medicare_tax_cents: i64,
    pub standard_deduction_cents: i64,
    /// Qualified business income deduction (§199A), taken after the standard
    /// deduction.
    pub qbi_deduction_cents: i64,
    /// Income taxed by the brackets (after the standard and QBI deductions).
    pub taxable_income_cents: i64,
    pub income_tax_cents: i64,
    /// Projected full-year tax.
    pub total_tax_cents: i64,
    /// The smaller of 90% of the projected tax and the prior-year requirement
    /// (`prior_year_tax_cents` × `safe_harbor_prior_year_percentage`).
    pub safe_harbor_cents: i64,
    /// Share of last year's tax the prior-year safe harbor requires, as a
    /// decimal string: "1.00", or "1.10" when last year's AGI was above the
    /// high-income threshold.
    pub safe_harbor_prior_year_percentage: String,
    /// Last year's total tax the safe harbor is based on: the filed return's
    /// figure when entered, else the app's own projection; `None` when neither
    /// exists (the safe harbor is then 90% of this year's projection).
    pub prior_year_tax_cents: Option<i64>,
    /// This quarter's installment (a quarter of the projected annual tax).
    pub quarterly_payment_cents: i64,
    pub payment_due_date: String,
    pub schedule_c_lines: Vec<ScheduleCLineOutput>,
}

#[derive(Debug, Serialize)]
pub struct ScheduleCLineOutput {
    pub line: String,
    pub label: String,
    pub amount_cents: i64,
    pub is_income: bool,
}

/// The calendar quarter containing `date`.
fn quarter_of(date: NaiveDate) -> Quarter {
    Quarter::new((date.month0() / 3 + 1) as u8).unwrap_or(Quarter::Q1)
}

/// Compute a quarterly tax estimate for the given year and quarter.
#[tauri::command]
pub async fn estimate_quarterly_tax(
    state: State<'_, Arc<Mutex<AppState>>>,
    year: Option<u16>,
    quarter: Option<u8>,
) -> Result<QuarterlyEstimateOutput, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    quarterly_estimate(&db, year, quarter, today()).await
}

/// Compute (and persist) the estimate for `year`/`quarter` — the current ones
/// when omitted — as of `today`.
async fn quarterly_estimate(
    db: &DbPool,
    year: Option<u16>,
    quarter: Option<u8>,
    today: NaiveDate,
) -> Result<QuarterlyEstimateOutput, CommandError> {
    let yr = year.unwrap_or(today.year() as u16);
    let qtr = match quarter {
        Some(q) => Quarter::new(q)
            .ok_or_else(|| CommandError::validation(format!("Quarter must be 1-4, got {q}")))?,
        None => quarter_of(today),
    };

    let rules = load_tax_rules(yr)?;
    let fy = FiscalYear::new(yr);

    let prior_year_tax_cents = aequi_storage::get_prior_year_total_tax(db, yr).await?;
    let prior_year_tax = prior_year_tax_cents.map(Money::from_cents);

    // Annualize activity through the end of the installment period, or
    // through today while the period is still open.
    let through = aequi_core::estimate_through(fy, qtr, today);
    let snapshot = aequi_storage::build_ledger_snapshot(db, fy, through, prior_year_tax).await?;

    let est = aequi_core::compute_quarterly_estimate(&rules, &snapshot, qtr);

    // Persist the estimate
    aequi_storage::upsert_tax_period(
        db,
        yr,
        quarter_to_u8(qtr),
        cents(est.total_tax_estimate)?,
        cents(est.quarterly_payment)?,
        cents(est.se_tax_amount)?,
        cents(est.estimated_income_tax)?,
        cents(est.ytd_net_profit)?,
        &est.payment_due_date.to_string(),
        rules.year.value,
    )
    .await?;

    let schedule_c_lines = schedule_c_line_outputs(&est.schedule_c_lines)?;

    Ok(QuarterlyEstimateOutput {
        year: est.year,
        quarter: est.quarter.to_string(),
        ytd_gross_income_cents: cents(est.ytd_gross_income)?,
        ytd_total_expenses_cents: cents(est.ytd_total_expenses)?,
        ytd_net_profit_cents: cents(est.ytd_net_profit)?,
        annualization_factor: est.annualization_factor.to_f64().unwrap_or(1.0),
        annualized_net_profit_cents: cents(est.annualized_net_profit)?,
        se_tax_cents: cents(est.se_tax_amount)?,
        se_tax_deduction_cents: cents(est.se_tax_deduction)?,
        additional_medicare_tax_cents: cents(est.additional_medicare_tax)?,
        standard_deduction_cents: cents(est.standard_deduction)?,
        qbi_deduction_cents: cents(est.qbi_deduction)?,
        taxable_income_cents: cents(est.taxable_income)?,
        income_tax_cents: cents(est.estimated_income_tax)?,
        total_tax_cents: cents(est.total_tax_estimate)?,
        safe_harbor_cents: cents(est.safe_harbor_amount)?,
        safe_harbor_prior_year_percentage: format!("{:.2}", est.safe_harbor_prior_year_percentage),
        prior_year_tax_cents,
        quarterly_payment_cents: cents(est.quarterly_payment)?,
        payment_due_date: est.payment_due_date.to_string(),
        schedule_c_lines,
    })
}

fn schedule_c_line_outputs(
    lines: &std::collections::BTreeMap<aequi_core::ScheduleCLine, Money>,
) -> Result<Vec<ScheduleCLineOutput>, CommandError> {
    lines
        .iter()
        .map(|(line, amount)| {
            Ok(ScheduleCLineOutput {
                line: format!("{line:?}"),
                label: line.label().to_string(),
                amount_cents: cents(*amount)?,
                is_income: line.is_income(),
            })
        })
        .collect()
}

/// Get the Schedule C preview for a given year.
#[tauri::command]
pub async fn get_schedule_c_preview(
    state: State<'_, Arc<Mutex<AppState>>>,
    year: Option<u16>,
) -> Result<ScheduleCPreviewOutput, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let db = &db;

    let yr = year.unwrap_or(today().year() as u16);
    let rules = load_tax_rules(yr)?;
    let fy = FiscalYear::new(yr);

    let snapshot = aequi_storage::build_ledger_snapshot(db, fy, fy.end_date(), None).await?;

    let preview = aequi_core::tax::engine::schedule_c_preview(&rules, &snapshot);

    Ok(ScheduleCPreviewOutput {
        year: preview.year,
        gross_income_cents: cents(preview.gross_income)?,
        total_expenses_cents: cents(preview.total_expenses)?,
        net_profit_cents: cents(preview.net_profit)?,
        lines: schedule_c_line_outputs(&preview.lines)?,
    })
}

#[derive(Debug, Serialize)]
pub struct ScheduleCPreviewOutput {
    pub year: u16,
    pub gross_income_cents: i64,
    pub total_expenses_cents: i64,
    pub net_profit_cents: i64,
    pub lines: Vec<ScheduleCLineOutput>,
}

/// Load tax rules for a given year from the bundled rules directory.
fn load_tax_rules(year: u16) -> Result<aequi_core::TaxRules, CommandError> {
    // Use the bundled rules file. In production this would resolve from
    // the app's resource directory; for now we embed the 2026 rules.
    let toml_str = include_str!("../../../rules/tax/us/2026.toml");

    let rules = aequi_core::TaxRules::from_toml(toml_str)
        .map_err(|e| CommandError::internal(e.to_string()))?;

    if rules.year.value != year {
        return Err(CommandError::not_found(format!(
            "Tax rules for year {year} not available (have {})",
            rules.year.value
        )));
    }

    Ok(rules)
}

fn quarter_to_u8(q: Quarter) -> u8 {
    match q {
        Quarter::Q1 => 1,
        Quarter::Q2 => 2,
        Quarter::Q3 => 3,
        Quarter::Q4 => 4,
    }
}

// ── Filed tax return figures ────────────────────────────────────────────────

/// Years a filed return can be entered for.
const TAX_RETURN_YEARS: std::ops::RangeInclusive<u16> = 2000..=2100;

fn validate_tax_return_year(year: u16) -> Result<(), CommandError> {
    if TAX_RETURN_YEARS.contains(&year) {
        Ok(())
    } else {
        Err(CommandError::validation(format!(
            "Year must be between {} and {}, got {year}",
            TAX_RETURN_YEARS.start(),
            TAX_RETURN_YEARS.end()
        )))
    }
}

async fn load_tax_return(
    db: &DbPool,
    year: u16,
) -> Result<aequi_storage::TaxReturnFigures, CommandError> {
    validate_tax_return_year(year)?;
    Ok(aequi_storage::get_tax_return_figures(db, year).await?)
}

async fn save_tax_return(
    db: &DbPool,
    year: u16,
    figures: aequi_storage::TaxReturnFigures,
) -> Result<aequi_storage::TaxReturnFigures, CommandError> {
    validate_tax_return_year(year)?;
    aequi_storage::set_tax_return_figures(db, year, figures).await?;
    Ok(aequi_storage::get_tax_return_figures(db, year).await?)
}

/// Figures entered from the user's filed return for `year` (each `None` when
/// unknown). Last year's make the prior-year safe harbor exact.
#[tauri::command]
pub async fn get_tax_return(
    state: State<'_, Arc<Mutex<AppState>>>,
    year: u16,
) -> Result<aequi_storage::TaxReturnFigures, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    load_tax_return(&db, year).await
}

/// Store the figures from the filed return for `year` (`None` clears a
/// figure) and return what is now stored.
#[tauri::command]
pub async fn set_tax_return(
    state: State<'_, Arc<Mutex<AppState>>>,
    year: u16,
    figures: aequi_storage::TaxReturnFigures,
) -> Result<aequi_storage::TaxReturnFigures, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    save_tax_return(&db, year, figures).await
}

// ── Contact commands ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ContactInput {
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub contact_type: String,
    pub is_contractor: bool,
    pub tax_id: Option<String>,
    pub notes: Option<String>,
}

#[tauri::command]
pub async fn get_contacts(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<aequi_storage::ContactRecord>, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let contacts = aequi_storage::get_all_contacts(&db).await?;
    Ok(contacts)
}

#[tauri::command]
pub async fn create_contact(
    state: State<'_, Arc<Mutex<AppState>>>,
    input: ContactInput,
) -> Result<aequi_storage::ContactRecord, CommandError> {
    // Input validation
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(CommandError::validation("Contact name is required"));
    }
    if let Some(ref email) = input.email {
        let email = email.trim();
        if !email.is_empty() && (!email.contains('@') || !email.contains('.')) {
            return Err(CommandError::validation("Invalid email address format"));
        }
    }
    if !["Client", "Vendor", "Contractor"].contains(&input.contact_type.as_str()) {
        return Err(CommandError::validation(
            "Contact type must be Client, Vendor, or Contractor",
        ));
    }

    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let id = aequi_storage::insert_contact(
        &db,
        &name,
        input.email.as_deref(),
        input.phone.as_deref(),
        input.address.as_deref(),
        &input.contact_type,
        input.is_contractor,
        input.tax_id.as_deref(),
        input.notes.as_deref(),
    )
    .await?;

    aequi_storage::get_contact_by_id(&db, id)
        .await?
        .ok_or_else(|| CommandError::not_found("Contact not found after insert"))
}

// ── Invoice commands ─────────────────────────────────────────────────────────

/// A trimmed optional text input, with blank meaning "not given".
fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// Report a missing invoice as such rather than as a generic missing record.
fn invoice_error(e: sqlx::Error) -> CommandError {
    match e {
        sqlx::Error::RowNotFound => CommandError::not_found("Invoice not found"),
        other => other.into(),
    }
}

async fn invoice_detail(db: &DbPool, id: i64) -> Result<InvoiceDetail, CommandError> {
    aequi_storage::get_invoice_detail(db, id)
        .await?
        .ok_or_else(|| CommandError::not_found("Invoice not found"))
}

/// All invoices, newest first, with their totals, amount paid and balance.
#[tauri::command]
pub async fn get_invoices(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<InvoiceSummary>, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    Ok(aequi_storage::list_invoice_summaries(&db).await?)
}

/// One invoice with its line items, tax lines, payments and totals.
#[tauri::command]
pub async fn get_invoice(
    state: State<'_, Arc<Mutex<AppState>>>,
    id: i64,
) -> Result<InvoiceDetail, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    invoice_detail(&db, id).await
}

/// Create a draft invoice. Storage validates the number (unique), dates,
/// contact, line items, tax rates and discount.
#[tauri::command]
pub async fn create_invoice(
    state: State<'_, Arc<Mutex<AppState>>>,
    input: NewInvoice,
) -> Result<InvoiceDetail, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let id = aequi_storage::create_invoice(&db, &input).await?;
    invoice_detail(&db, id).await
}

/// Replace the contents of a draft invoice (sent invoices can't be edited).
#[tauri::command]
pub async fn update_invoice(
    state: State<'_, Arc<Mutex<AppState>>>,
    id: i64,
    input: NewInvoice,
) -> Result<InvoiceDetail, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    aequi_storage::update_draft_invoice(&db, id, &input)
        .await
        .map_err(invoice_error)?;
    invoice_detail(&db, id).await
}

/// Void an invoice that has no payments (Draft, Sent or Viewed).
#[tauri::command]
pub async fn void_invoice(
    state: State<'_, Arc<Mutex<AppState>>>,
    id: i64,
    reason: String,
) -> Result<(), CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    aequi_storage::void_invoice(&db, id, reason.trim())
        .await
        .map_err(invoice_error)
}

#[tauri::command]
pub async fn get_invoice_aging(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<aequi_storage::InvoiceRecord>, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    Ok(aequi_storage::get_invoice_aging(&db).await?)
}

#[derive(Debug, Deserialize)]
pub struct PaymentInput {
    pub invoice_id: i64,
    pub amount_cents: i64,
    pub date: String,
    pub method: Option<String>,
    /// Asset account the money went into; storage defaults to 1000 Checking.
    #[serde(default)]
    pub deposit_account_code: Option<String>,
}

/// Record a payment against an invoice. Storage validates the amount, date,
/// deposit account and invoice state (no void/paid invoices, no overpayment),
/// posts the cash-basis ledger entry and advances the invoice to
/// PartiallyPaid / Paid, all atomically.
#[tauri::command]
pub async fn record_invoice_payment(
    state: State<'_, Arc<Mutex<AppState>>>,
    input: PaymentInput,
) -> Result<i64, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    record_payment(&db, &input).await
}

async fn record_payment(db: &DbPool, input: &PaymentInput) -> Result<i64, CommandError> {
    aequi_storage::insert_payment(
        db,
        input.invoice_id,
        input.amount_cents,
        &input.date,
        non_blank(input.method.as_deref()),
        non_blank(input.deposit_account_code.as_deref()),
    )
    .await
    .map_err(invoice_error)
}

/// Invoice statuses that are awaiting payment.
const OPEN_INVOICE_STATUSES: [&str; 3] = ["Sent", "Viewed", "PartiallyPaid"];

/// Whether the client still owes money on this invoice: it has been sent (not
/// a draft, paid or void) and has a balance left.
fn is_receivable(inv: &InvoiceSummary) -> bool {
    OPEN_INVOICE_STATUSES.contains(&inv.invoice.status_type.as_str())
        && inv.totals.balance_cents > 0
}

/// Whether a receivable invoice is past due on `today` (the user's local date;
/// an invoice due today is not overdue yet).
fn is_overdue(inv: &InvoiceSummary, today: NaiveDate) -> bool {
    is_receivable(inv)
        && NaiveDate::parse_from_str(&inv.invoice.due_date, "%Y-%m-%d").is_ok_and(|d| d < today)
}

/// Money owed by clients on sent invoices.
#[derive(Debug, Default, PartialEq, Eq)]
struct Receivables {
    outstanding: u32,
    overdue: u32,
    balance_cents: i64,
}

fn receivables(invoices: &[InvoiceSummary], today: NaiveDate) -> Receivables {
    invoices
        .iter()
        .filter(|inv| is_receivable(inv))
        .fold(Receivables::default(), |mut r, inv| {
            r.outstanding += 1;
            r.overdue += u32::from(is_overdue(inv, today));
            r.balance_cents = r.balance_cents.saturating_add(inv.totals.balance_cents);
            r
        })
}

#[derive(Debug, Serialize)]
pub struct NecSummaryEntry {
    pub contact_id: i64,
    pub contact_name: String,
    /// Paid to the contractor from bank or cash accounts during the year.
    pub ytd_cents: i64,
    /// Whether `ytd_cents` reaches the year's 1099-NEC filing threshold.
    pub over_threshold: bool,
    pub threshold_cents: i64,
}

/// 1099-NEC totals: what each contractor was paid during `year`.
async fn nec_summary(db: &DbPool, year: u16) -> Result<Vec<NecSummaryEntry>, CommandError> {
    let threshold_cents = cents(aequi_core::nec_threshold(year))?;
    let rows = aequi_storage::get_contractor_ytd_payments(db, year).await?;
    Ok(rows
        .into_iter()
        .map(|(contact_id, contact_name, ytd_cents)| NecSummaryEntry {
            contact_id,
            contact_name,
            ytd_cents,
            over_threshold: aequi_core::check_1099_threshold(Money::from_cents(ytd_cents), year),
            threshold_cents,
        })
        .collect())
}

/// Payments made to each contractor in `year` (default: the current year).
#[tauri::command]
pub async fn get_1099_summary(
    state: State<'_, Arc<Mutex<AppState>>>,
    year: Option<u16>,
) -> Result<Vec<NecSummaryEntry>, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    nec_summary(&db, year.unwrap_or(today().year() as u16)).await
}

// ── Email delivery ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SendInvoiceInput {
    pub invoice_id: i64,
    pub subject: Option<String>,
}

fn record_to_contact(rec: &aequi_storage::ContactRecord) -> aequi_core::Contact {
    let contact_type = match rec.contact_type.as_str() {
        "Vendor" => aequi_core::ContactType::Vendor,
        "Contractor" => aequi_core::ContactType::Contractor,
        _ => aequi_core::ContactType::Client,
    };

    aequi_core::Contact {
        id: Some(ContactId(rec.id)),
        name: rec.name.clone(),
        email: rec.email.clone(),
        phone: rec.phone.clone(),
        address: rec.address.clone(),
        contact_type,
        is_contractor: rec.is_contractor,
        tax_id: rec.tax_id.clone(),
        notes: rec.notes.clone(),
    }
}

#[tauri::command]
pub async fn send_invoice(
    state: State<'_, Arc<Mutex<AppState>>>,
    input: SendInvoiceInput,
) -> Result<aequi_email::DeliveryResult, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };

    // Load email config from settings
    let config_json = aequi_storage::get_setting(&db, "email_config")
        .await?
        .ok_or_else(|| {
            CommandError::config("Email not configured — set email_config in Settings")
        })?;

    let config: aequi_email::EmailConfig = serde_json::from_str(&config_json)
        .map_err(|e| CommandError::config(format!("Invalid email config: {e}")))?;

    let rec = aequi_storage::get_invoice_by_id(&db, input.invoice_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Invoice not found"))?;
    // Check before emailing: storage would refuse to mark it sent afterwards.
    if rec.status_type == "Void" {
        return Err(CommandError::validation("Cannot send a void invoice"));
    }

    let lines = aequi_storage::get_invoice_lines(&db, input.invoice_id).await?;
    // Invoices created before line items existed have none (and total $0).
    if lines.is_empty() {
        return Err(CommandError::validation(
            "Add at least one line item before sending",
        ));
    }
    let tax_lines = aequi_storage::get_invoice_tax_lines(&db, input.invoice_id).await?;
    let invoice = aequi_storage::invoice_from_records(&rec, &lines, &tax_lines)?;

    let contact_rec = aequi_storage::get_contact_by_id(&db, rec.contact_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Contact not found"))?;
    let contact = record_to_contact(&contact_rec);

    let result = aequi_email::send_invoice(&config, &invoice, &contact, input.subject.as_deref())
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;

    // Draft → Sent; a reminder for an already sent or paid invoice keeps its status.
    aequi_storage::mark_invoice_sent(&db, input.invoice_id, &chrono::Utc::now().to_rfc3339())
        .await
        .map_err(|e| {
            let mut err = CommandError::from(e);
            err.message = format!(
                "The invoice was emailed, but updating its status failed: {}",
                err.message
            );
            err
        })?;

    Ok(result)
}

// ── Export commands ──────────────────────────────────────────────────────────

/// Load everything an export needs: all accounts (archived ones may still
/// carry history) and every ledger transaction.
async fn load_export_data(
    state: &State<'_, Arc<Mutex<AppState>>>,
) -> Result<(Vec<Account>, Vec<ValidatedTransaction>), CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let accounts = aequi_storage::get_all_accounts_including_archived(&db).await?;
    let transactions = aequi_storage::load_ledger_transactions(&db).await?;
    Ok((accounts, transactions))
}

#[tauri::command]
pub async fn export_beancount(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<String, CommandError> {
    let (accounts, transactions) = load_export_data(&state).await?;
    Ok(aequi_core::export::beancount::export_beancount(
        &accounts,
        &transactions,
    ))
}

#[tauri::command]
pub async fn export_qif(state: State<'_, Arc<Mutex<AppState>>>) -> Result<String, CommandError> {
    let (accounts, transactions) = load_export_data(&state).await?;
    Ok(aequi_core::export::qif::export_qif(
        &accounts,
        &transactions,
    ))
}

// ── Settings commands ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_setting(
    state: State<'_, Arc<Mutex<AppState>>>,
    key: String,
) -> Result<Option<String>, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    Ok(aequi_storage::get_setting(&db, &key).await?)
}

#[tauri::command]
pub async fn set_setting(
    state: State<'_, Arc<Mutex<AppState>>>,
    key: String,
    value: String,
) -> Result<(), CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    Ok(aequi_storage::set_setting(&db, &key, &value).await?)
}

// ── Audit log command ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_audit_log(
    state: State<'_, Arc<Mutex<AppState>>>,
    limit: Option<i64>,
) -> Result<Vec<aequi_storage::AuditLogRecord>, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    Ok(aequi_storage::get_audit_log(&db, limit.unwrap_or(100)).await?)
}

// ── Backup commands ─────────────────────────────────────────────────────────
//
// Not registered with the webview (no UI uses them yet). Before exposing them,
// pass paths the user picked in a native dialog.

/// Resolve symlinks in the longest existing prefix of an absolute path
/// without `..` components, so it can be compared with a canonical directory
/// even when its last components don't exist yet.
fn resolve_for_comparison(path: &Path) -> Result<PathBuf, CommandError> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(CommandError::validation(
            "Path must be absolute and must not contain '..'",
        ));
    }
    let mut missing = Vec::new();
    let mut existing = path;
    loop {
        if let Ok(canonical) = std::fs::canonicalize(existing) {
            return Ok(missing
                .iter()
                .rev()
                .fold(canonical, |acc: PathBuf, name| acc.join(name)));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_os_string());
                existing = parent;
            }
            _ => return Ok(path.to_path_buf()),
        }
    }
}

/// Whether `path` is `dir` or inside it, after resolving symlinks.
fn is_within(path: &Path, dir: &Path) -> Result<bool, CommandError> {
    let path = resolve_for_comparison(path)?;
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    Ok(path.starts_with(dir))
}

/// Write a backup archive to `output_path`, which must be outside the app
/// data folder (it holds the live ledger.db). An existing file is replaced
/// only when `overwrite` is true.
#[tauri::command]
pub async fn create_backup(
    state: State<'_, Arc<Mutex<AppState>>>,
    output_path: String,
    overwrite: Option<bool>,
) -> Result<aequi_storage::backup::BackupManifest, CommandError> {
    let (db, db_path, data_dir, attachments_dir) = {
        let s = state.lock().await;
        (
            s.db.clone(),
            s.db_path.clone(),
            s.data_dir.clone(),
            s.attachments_dir.clone(),
        )
    };
    let output = PathBuf::from(&output_path);
    if output.file_name().is_none() {
        return Err(CommandError::validation("Backup path must name a file"));
    }
    if is_within(&output, &data_dir)? {
        return Err(CommandError::validation(
            "Choose a backup location outside the app data folder",
        ));
    }
    match tokio::fs::symlink_metadata(&output).await {
        Ok(meta) if meta.is_dir() => {
            return Err(CommandError::validation("Backup path is a directory"));
        }
        Ok(_) if !overwrite.unwrap_or(false) => {
            return Err(CommandError::validation(
                "A file already exists at the backup path",
            ));
        }
        _ => {}
    }

    // Archiving and compressing are blocking file I/O; run the whole backup
    // on a blocking thread.
    let rt = tokio::runtime::Handle::current();
    let manifest = tokio::task::spawn_blocking(move || {
        rt.block_on(aequi_storage::backup::create_backup(
            &db,
            &db_path,
            &attachments_dir,
            &output,
            env!("CARGO_PKG_VERSION"),
        ))
    })
    .await
    .map_err(|e| CommandError::internal(format!("Backup task failed: {e}")))??;
    Ok(manifest)
}

/// Restore a backup archive into `target_dir`. Restoring over the running
/// app's own data folder is refused: its ledger.db is open.
#[tauri::command]
pub async fn restore_backup(
    state: State<'_, Arc<Mutex<AppState>>>,
    archive_path: String,
    target_dir: String,
) -> Result<String, CommandError> {
    let data_dir = {
        let s = state.lock().await;
        s.data_dir.clone()
    };
    let target = PathBuf::from(&target_dir);
    if is_within(&target, &data_dir)? {
        return Err(CommandError::validation(
            "Cannot restore into the app's live data folder; choose another folder",
        ));
    }

    let archive = PathBuf::from(&archive_path);
    let result = tokio::task::spawn_blocking(move || {
        aequi_storage::backup::restore_backup(&archive, &target)
    })
    .await
    .map_err(|e| CommandError::internal(format!("Restore task failed: {e}")))??;
    Ok(result.db_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_schema_versions(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<aequi_storage::migrate::SchemaVersion>, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    Ok(aequi_storage::migrate::get_schema_versions(&db).await?)
}

// ── Update commands ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct UpdateStatus {
    pub update_available: bool,
    pub current_version: String,
    pub latest_version: Option<String>,
}

fn update_error(e: tauri_plugin_updater::Error) -> CommandError {
    tracing::warn!("Update check failed: {e}");
    CommandError {
        code: "UPDATE".into(),
        message: format!("Update check failed: {e}"),
    }
}

#[tauri::command]
pub async fn check_for_updates(app: tauri::AppHandle) -> Result<UpdateStatus, CommandError> {
    let current = app.package_info().version.to_string();

    let update = app
        .updater()
        .map_err(update_error)?
        .check()
        .await
        .map_err(update_error)?;

    Ok(UpdateStatus {
        update_available: update.is_some(),
        current_version: current,
        latest_version: update.map(|u| u.version),
    })
}

// ── Notification commands ───────────────────────────────────────────────────

/// Notification text for the overdue invoices, if there are any.
fn overdue_notice(overdue: &[&InvoiceSummary]) -> Option<String> {
    match overdue {
        [] => None,
        [inv] => Some(format!("Invoice {} is overdue", inv.invoice.invoice_number)),
        many => Some(format!("{} invoices are past due", many.len())),
    }
}

/// Notify about invoices that are past due (by the local date) and still have
/// a balance owed. Returns how many there are.
#[tauri::command]
pub async fn check_overdue_invoices(
    app: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<u32, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let invoices = aequi_storage::list_invoice_summaries(&db).await?;
    let overdue: Vec<_> = invoices
        .iter()
        .filter(|inv| is_overdue(inv, today()))
        .collect();
    let count = overdue.len() as u32;

    if let Some(body) = overdue_notice(&overdue)
        && let Err(e) = app
            .notification()
            .builder()
            .title("Overdue Invoices")
            .body(&body)
            .show()
    {
        tracing::warn!("Failed to show overdue-invoice notification: {e}");
    }

    Ok(count)
}

// ── Dashboard commands ──────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DashboardSummary {
    pub ytd_income_cents: i64,
    pub ytd_expenses_cents: i64,
    pub ytd_net_profit_cents: i64,
    /// Sent invoices with a balance still owed.
    pub outstanding_invoices: u32,
    /// Outstanding invoices past their due date.
    pub overdue_invoices: u32,
    /// Total balance owed on outstanding invoices.
    pub outstanding_receivables_cents: i64,
    pub pending_receipts: u32,
    pub total_accounts: u32,
    pub total_transactions: u32,
    pub recent_transactions: Vec<TransactionOutput>,
    /// The current quarter's installment minus payments already recorded for it.
    pub quarterly_tax_due_cents: Option<i64>,
    pub next_tax_due_date: Option<String>,
}

/// What is still owed for a quarter: its installment less recorded payments.
fn remaining_quarterly_tax(period: &aequi_storage::TaxPeriodRecord) -> i64 {
    (period.quarterly_payment_cents - period.payment_recorded_cents).max(0)
}

#[tauri::command]
pub async fn get_dashboard_summary(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<DashboardSummary, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let db = &db;

    let now = today();
    let year_start = NaiveDate::from_ymd_opt(now.year(), 1, 1)
        .unwrap_or(now)
        .to_string();
    let today = now.to_string();

    // YTD P&L
    let pl_row = sqlx::query_as::<_, (i64, i64)>(
        r#"SELECT
            COALESCE(SUM(CASE WHEN a.account_type = 'Income' THEN tl.credit_cents - tl.debit_cents ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN a.account_type = 'Expense' THEN tl.debit_cents - tl.credit_cents ELSE 0 END), 0)
        FROM transaction_lines tl
        JOIN accounts a ON a.id = tl.account_id
        JOIN transactions t ON t.id = tl.transaction_id
        WHERE t.date >= ? AND t.date <= ?"#,
    )
    .bind(&year_start)
    .bind(&today)
    .fetch_one(db)
    .await?;

    let ytd_income = pl_row.0;
    let ytd_expenses = pl_row.1;

    // Outstanding + overdue invoices (paid, void and $0 invoices never count)
    let invoices = aequi_storage::list_invoice_summaries(db).await?;
    let owed = receivables(&invoices, now);

    // Pending receipts
    let receipts = aequi_storage::get_receipts_pending_review(db).await?;
    let pending_receipts = receipts.len() as u32;

    // Counts
    let accounts: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM accounts WHERE is_archived = 0")
        .fetch_one(db)
        .await?;
    let tx_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transactions")
        .fetch_one(db)
        .await?;

    // Recent transactions (last 5)
    let recent_transactions = sqlx::query_as::<_, TransactionRow>(transaction_query!(
        "ORDER BY t.date DESC, t.id DESC LIMIT 5"
    ))
    .fetch_all(db)
    .await?
    .into_iter()
    .map(TransactionOutput::from)
    .collect();

    // Current quarter's estimated tax still to pay
    let current_quarter = quarter_to_u8(quarter_of(now));
    let tax_periods = aequi_storage::get_tax_periods(db, now.year() as u16).await?;
    let current_period = tax_periods
        .iter()
        .find(|p| p.quarter == current_quarter as i64);

    Ok(DashboardSummary {
        ytd_income_cents: ytd_income,
        ytd_expenses_cents: ytd_expenses,
        ytd_net_profit_cents: ytd_income - ytd_expenses,
        outstanding_invoices: owed.outstanding,
        overdue_invoices: owed.overdue,
        outstanding_receivables_cents: owed.balance_cents,
        pending_receipts,
        total_accounts: accounts.0 as u32,
        total_transactions: tx_count.0 as u32,
        recent_transactions,
        quarterly_tax_due_cents: current_period.map(remaining_quarterly_tax),
        next_tax_due_date: current_period.map(|p| p.due_date.clone()),
    })
}

// ── Update contact command ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpdateContactInput {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub contact_type: String,
    pub is_contractor: bool,
    pub tax_id: Option<String>,
    pub notes: Option<String>,
}

#[tauri::command]
pub async fn update_contact(
    state: State<'_, Arc<Mutex<AppState>>>,
    input: UpdateContactInput,
) -> Result<aequi_storage::ContactRecord, CommandError> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(CommandError::validation("Contact name is required"));
    }
    if let Some(ref email) = input.email {
        let email = email.trim();
        if !email.is_empty() && (!email.contains('@') || !email.contains('.')) {
            return Err(CommandError::validation("Invalid email address format"));
        }
    }

    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    aequi_storage::update_contact(
        &db,
        input.id,
        &name,
        input.email.as_deref(),
        input.phone.as_deref(),
        input.address.as_deref(),
        &input.contact_type,
        input.is_contractor,
        input.tax_id.as_deref(),
        input.notes.as_deref(),
    )
    .await?;

    aequi_storage::get_contact_by_id(&db, input.id)
        .await?
        .ok_or_else(|| CommandError::not_found("Contact not found"))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_db() -> DbPool {
        let db = aequi_storage::create_db(Path::new(":memory:"))
            .await
            .unwrap();
        aequi_storage::seed_default_accounts(&db).await.unwrap();
        db
    }

    /// Post a transaction: `lines` are (account code, debit cents, credit cents).
    async fn post(db: &DbPool, date: &str, lines: &[(&str, i64, i64)]) {
        let total: i64 = lines.iter().map(|l| l.1).sum();
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO transactions (date, description, balanced_total_cents) VALUES (?, 'test', ?) RETURNING id",
        )
        .bind(date)
        .bind(total)
        .fetch_one(db)
        .await
        .unwrap();
        for (code, debit, credit) in lines {
            sqlx::query(
                "INSERT INTO transaction_lines (transaction_id, account_id, debit_cents, credit_cents) \
                 SELECT ?, id, ?, ? FROM accounts WHERE code = ?",
            )
            .bind(id)
            .bind(debit)
            .bind(credit)
            .bind(code)
            .execute(db)
            .await
            .unwrap();
        }
    }

    fn total_for(entries: &[ProfitLossEntry], code: &str) -> i64 {
        entries
            .iter()
            .find(|e| e.account_code == code)
            .map(|e| e.total_cents)
            .unwrap()
    }

    #[test]
    fn storage_errors_map_to_command_codes() {
        let e = CommandError::from(sqlx::Error::InvalidArgument("bad date".into()));
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("VALIDATION", "bad date")
        );

        let e = CommandError::from(sqlx::Error::RowNotFound);
        assert_eq!(e.code, "NOT_FOUND");

        let e = CommandError::from(sqlx::Error::PoolClosed);
        assert_eq!(e.code, "DATABASE");
        assert_eq!(e.message, "A database error occurred");

        let e = CommandError::from(aequi_core::LedgerError::NegativeAmount(1));
        assert_eq!(e.code, "VALIDATION");
    }

    #[tokio::test]
    async fn unique_violation_is_a_validation_error() {
        let db = test_db().await;
        let err = sqlx::query(
            "INSERT INTO accounts (code, name, account_type) VALUES ('4000', 'Dup', 'Income')",
        )
        .execute(&db)
        .await
        .unwrap_err();
        assert_eq!(CommandError::from(err).code, "VALIDATION");
    }

    #[test]
    fn pipeline_input_errors_are_validation_errors() {
        let e = CommandError::from(PipelineError::NotAFile("/dev/zero".into()));
        assert_eq!(e.code, "VALIDATION");
        let e = CommandError::from(PipelineError::TooLarge { size: 60 << 20 });
        assert_eq!(e.code, "VALIDATION");
        let e = CommandError::from(PipelineError::Io(std::io::Error::from(
            std::io::ErrorKind::NotFound,
        )));
        assert_eq!(e.code, "VALIDATION");
    }

    #[test]
    fn date_bounds_are_independent_and_validated() {
        assert_eq!(
            date_bounds(Some("2026-01-01".into()), None).unwrap(),
            (Some("2026-01-01".into()), None)
        );
        assert_eq!(
            date_bounds(None, Some("2026-03-31".into())).unwrap(),
            (None, Some("2026-03-31".into()))
        );
        assert_eq!(
            date_bounds(Some(String::new()), Some(" ".into())).unwrap(),
            (None, None)
        );
        for (start, end) in [
            (Some("2026-1-5"), None),
            (None, Some("03/31/2026")),
            (Some("2026-04-01"), Some("2026-03-31")),
        ] {
            let err = date_bounds(start.map(Into::into), end.map(Into::into)).unwrap_err();
            assert_eq!(err.code, "VALIDATION", "{start:?}..{end:?}");
        }
    }

    #[tokio::test]
    async fn transactions_honor_each_bound() {
        let db = test_db().await;
        for date in ["2026-01-15", "2026-02-15", "2026-03-15"] {
            post(&db, date, &[("1000", 100, 0), ("4000", 0, 100)]).await;
        }
        let dates =
            |txs: Vec<TransactionOutput>| txs.into_iter().map(|t| t.date).collect::<Vec<_>>();

        let from = query_transactions(&db, "2026-02-01", MAX_DATE)
            .await
            .unwrap();
        assert_eq!(dates(from), ["2026-03-15", "2026-02-15"]);

        let until = query_transactions(&db, MIN_DATE, "2026-02-15")
            .await
            .unwrap();
        assert_eq!(dates(until), ["2026-02-15", "2026-01-15"]);
    }

    #[tokio::test]
    async fn profit_loss_sums_only_in_range_transactions() {
        let db = test_db().await;
        post(
            &db,
            "2025-12-31",
            &[("1000", 70_000, 0), ("4000", 0, 70_000)],
        )
        .await;
        post(
            &db,
            "2026-02-01",
            &[("1000", 10_000, 0), ("4000", 0, 10_000)],
        )
        .await;
        post(&db, "2026-02-02", &[("5010", 500, 0), ("1000", 0, 500)]).await;
        post(&db, "2026-04-01", &[("1000", 3_000, 0), ("4000", 0, 3_000)]).await;

        let q1 = query_profit_loss(&db, "2026-01-01", "2026-03-31")
            .await
            .unwrap();
        assert_eq!(total_for(&q1, "4000"), 10_000);
        assert_eq!(total_for(&q1, "5010"), -500);
        // Accounts without activity in range are still listed.
        assert_eq!(total_for(&q1, "4010"), 0);

        let since = query_profit_loss(&db, "2026-01-01", MAX_DATE)
            .await
            .unwrap();
        assert_eq!(total_for(&since, "4000"), 13_000);

        let until = query_profit_loss(&db, MIN_DATE, "2026-02-01")
            .await
            .unwrap();
        assert_eq!(total_for(&until, "4000"), 80_000);
    }

    #[tokio::test]
    async fn dashboard_tax_due_is_installment_minus_payments() {
        let db = test_db().await;
        aequi_storage::upsert_tax_period(
            &db,
            2026,
            2,
            800_000,
            200_000,
            0,
            0,
            0,
            "2026-06-15",
            2026,
        )
        .await
        .unwrap();
        let period = |db: DbPool| async move {
            aequi_storage::get_tax_periods(&db, 2026)
                .await
                .unwrap()
                .remove(0)
        };

        assert_eq!(remaining_quarterly_tax(&period(db.clone()).await), 200_000);

        aequi_storage::record_tax_payment(&db, 2026, 2, 150_000, "2026-06-01")
            .await
            .unwrap();
        assert_eq!(remaining_quarterly_tax(&period(db.clone()).await), 50_000);

        aequi_storage::record_tax_payment(&db, 2026, 2, 100_000, "2026-06-10")
            .await
            .unwrap();
        assert_eq!(remaining_quarterly_tax(&period(db.clone()).await), 0);
    }

    #[test]
    fn backup_paths_inside_data_dir_are_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(data.join("attachments")).unwrap();

        assert!(is_within(&data, &data).unwrap());
        assert!(is_within(&data.join("ledger.db"), &data).unwrap());
        assert!(is_within(&data.join("new").join("backup.tar.gz"), &data).unwrap());
        assert!(!is_within(&tmp.path().join("backup.tar.gz"), &data).unwrap());

        #[cfg(unix)]
        {
            let link = tmp.path().join("link");
            std::os::unix::fs::symlink(&data, &link).unwrap();
            assert!(is_within(&link.join("ledger.db"), &data).unwrap());
        }

        let rel = is_within(Path::new("backup.tar.gz"), &data).unwrap_err();
        assert_eq!(rel.code, "VALIDATION");
        let dotdot = is_within(&tmp.path().join("x").join("..").join("data"), &data).unwrap_err();
        assert_eq!(dotdot.code, "VALIDATION");
    }

    async fn contact(db: &DbPool, name: &str, is_contractor: bool) -> i64 {
        let kind = if is_contractor {
            "Contractor"
        } else {
            "Client"
        };
        aequi_storage::insert_contact(db, name, None, None, None, kind, is_contractor, None, None)
            .await
            .unwrap()
    }

    fn tx_input(
        date: &str,
        lines: &[(&str, i64, i64)],
        contact_id: Option<i64>,
    ) -> TransactionInput {
        TransactionInput {
            date: date.into(),
            description: "Test entry".into(),
            lines: lines
                .iter()
                .map(|&(code, debit_cents, credit_cents)| TransactionLineInput {
                    account_code: code.into(),
                    debit_cents,
                    credit_cents,
                    memo: None,
                })
                .collect(),
            memo: None,
            contact_id,
        }
    }

    #[test]
    fn transaction_contact_id_is_optional() {
        let json = serde_json::json!({
            "date": "2026-03-01",
            "description": "Coffee",
            "lines": [],
            "memo": null,
        });
        let input: TransactionInput = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(input.contact_id, None);

        let mut with_contact = json;
        with_contact["contact_id"] = 7.into();
        let input: TransactionInput = serde_json::from_value(with_contact).unwrap();
        assert_eq!(input.contact_id, Some(7));
    }

    #[tokio::test]
    async fn created_transactions_carry_their_payee() {
        let db = test_db().await;
        let ana = contact(&db, "Ana Designer", true).await;

        let out = insert_transaction(
            &db,
            tx_input(
                "2026-03-01",
                &[("5080", 25_000, 0), ("1000", 0, 25_000)],
                Some(ana),
            ),
        )
        .await
        .unwrap();
        assert_eq!(out.contact_id, Some(ana));
        assert_eq!(out.contact_name.as_deref(), Some("Ana Designer"));
        assert_eq!(out.balanced_total, Money::from_cents(25_000).to_string());

        let plain = insert_transaction(
            &db,
            tx_input("2026-03-02", &[("5010", 500, 0), ("1000", 0, 500)], None),
        )
        .await
        .unwrap();
        assert_eq!((plain.contact_id, plain.contact_name), (None, None));

        let listed = query_transactions(&db, MIN_DATE, MAX_DATE).await.unwrap();
        let names: Vec<_> = listed
            .iter()
            .map(|t| (t.id, t.contact_name.as_deref()))
            .collect();
        assert_eq!(names, [(plain.id, None), (out.id, Some("Ana Designer"))]);
    }

    #[tokio::test]
    async fn invalid_transactions_are_rejected() {
        let db = test_db().await;
        let ok = [("5010", 500, 0), ("1000", 0, 500)];
        let cases = [
            (
                tx_input("2026-03-01", &[("5010", 500, 0), ("1000", 0, 400)], None),
                "VALIDATION",
            ),
            (
                tx_input("2026-03-01", &[("5010", 500, 500)], None),
                "VALIDATION",
            ),
            (tx_input("2026-03-01", &ok, Some(999)), "VALIDATION"),
            (tx_input("03/01/2026", &ok, None), "VALIDATION"),
            (
                tx_input("2026-03-01", &[("9999", 500, 0), ("1000", 0, 500)], None),
                "NOT_FOUND",
            ),
            (
                TransactionInput {
                    description: "  ".into(),
                    ..tx_input("2026-03-01", &ok, None)
                },
                "VALIDATION",
            ),
        ];
        for (input, code) in cases {
            let what = format!("{input:?}");
            let err = insert_transaction(&db, input).await.unwrap_err();
            assert_eq!(err.code, code, "{what}: {}", err.message);
        }

        sqlx::query("UPDATE accounts SET is_archived = 1 WHERE code = '5010'")
            .execute(&db)
            .await
            .unwrap();
        let err = insert_transaction(&db, tx_input("2026-03-01", &ok, None))
            .await
            .unwrap_err();
        assert_eq!(err.code, "VALIDATION");

        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transactions")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    fn summary(status: &str, due_date: &str, total_cents: i64, paid_cents: i64) -> InvoiceSummary {
        InvoiceSummary {
            invoice: aequi_storage::InvoiceRecord {
                id: 1,
                invoice_number: format!("INV-{status}"),
                contact_id: 1,
                status_type: status.into(),
                status_data: None,
                issue_date: "2026-01-01".into(),
                due_date: due_date.into(),
                discount_type: None,
                discount_value: None,
                notes: None,
                terms: None,
                created_at: String::new(),
                updated_at: String::new(),
            },
            totals: aequi_storage::InvoiceTotals {
                subtotal_cents: total_cents,
                total_cents,
                paid_cents,
                balance_cents: (total_cents - paid_cents).max(0),
                ..Default::default()
            },
        }
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn receivables_count_only_sent_invoices_with_a_balance() {
        let invoices = [
            summary("Sent", "2026-03-01", 10_000, 0),
            summary("Viewed", "2026-04-01", 5_000, 0),
            summary("PartiallyPaid", "2026-02-01", 8_000, 3_000),
            summary("Draft", "2026-01-15", 9_000, 0),
            summary("Paid", "2026-01-15", 7_000, 7_000),
            summary("Void", "2026-01-15", 6_000, 0),
            // Legacy invoice without line items: nothing is owed.
            summary("Sent", "2026-01-15", 0, 0),
        ];
        assert_eq!(
            receivables(&invoices, day("2026-03-15")),
            Receivables {
                outstanding: 3,
                overdue: 2,
                balance_cents: 20_000,
            }
        );
        assert_eq!(receivables(&invoices, day("2026-01-01")).overdue, 0);
        assert_eq!(receivables(&[], day("2026-01-01")), Receivables::default());
    }

    #[test]
    fn invoices_are_overdue_only_after_the_due_date() {
        let inv = summary("Sent", "2026-03-01", 10_000, 0);
        assert!(!is_overdue(&inv, day("2026-03-01")));
        assert!(is_overdue(&inv, day("2026-03-02")));
        assert!(!is_overdue(
            &summary("Paid", "2026-03-01", 10_000, 10_000),
            day("2026-04-01")
        ));
        assert!(!is_overdue(
            &summary("Sent", "03/01/2026", 10_000, 0),
            day("2026-04-01")
        ));
    }

    #[test]
    fn overdue_notice_names_a_single_invoice() {
        let a = summary("Sent", "2026-03-01", 100, 0);
        let b = summary("Viewed", "2026-03-01", 100, 0);
        assert_eq!(overdue_notice(&[]), None);
        assert_eq!(
            overdue_notice(&[&a]).as_deref(),
            Some("Invoice INV-Sent is overdue")
        );
        assert_eq!(
            overdue_notice(&[&a, &b]).as_deref(),
            Some("2 invoices are past due")
        );
    }

    #[test]
    fn payment_input_deposit_account_is_optional() {
        let input: PaymentInput = serde_json::from_value(serde_json::json!({
            "invoice_id": 1, "amount_cents": 100, "date": "2026-03-01",
        }))
        .unwrap();
        assert_eq!((input.method, input.deposit_account_code), (None, None));

        assert_eq!(non_blank(Some(" 1010 ")), Some("1010"));
        assert_eq!(non_blank(Some("  ")), None);
        assert_eq!(non_blank(None), None);
    }

    fn new_invoice(number: &str, contact_id: i64, rate_cents: i64) -> NewInvoice {
        serde_json::from_value(serde_json::json!({
            "invoice_number": number,
            "contact_id": contact_id,
            "issue_date": "2026-03-01",
            "due_date": "2026-03-31",
            "lines": [{
                "description": "Consulting",
                "quantity_hundredths": 200,
                "unit_rate_cents": rate_cents,
            }],
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn invoice_payments_post_to_the_deposit_account_and_reduce_receivables() {
        let db = test_db().await;
        let client = contact(&db, "Acme", false).await;
        let id = aequi_storage::create_invoice(&db, &new_invoice("INV-1", client, 5_000))
            .await
            .unwrap();
        aequi_storage::mark_invoice_sent(&db, id, "2026-03-01T00:00:00Z")
            .await
            .unwrap();
        let open = |db: DbPool| async move {
            let invoices = aequi_storage::list_invoice_summaries(&db).await.unwrap();
            receivables(&invoices, day("2026-04-15"))
        };
        assert_eq!(
            open(db.clone()).await,
            Receivables {
                outstanding: 1,
                overdue: 1,
                balance_cents: 10_000,
            }
        );

        let payment = |amount_cents: i64, deposit: Option<&str>, invoice_id: i64| PaymentInput {
            invoice_id,
            amount_cents,
            date: "2026-04-01".into(),
            method: Some("ACH".into()),
            deposit_account_code: deposit.map(Into::into),
        };
        let err = record_payment(&db, &payment(4_000, Some("4000"), id))
            .await
            .unwrap_err();
        assert_eq!(err.code, "VALIDATION");
        let err = record_payment(&db, &payment(4_000, None, id + 100))
            .await
            .unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.message.as_str()),
            ("NOT_FOUND", "Invoice not found")
        );

        record_payment(&db, &payment(4_000, Some("1010"), id))
            .await
            .unwrap();
        let (savings,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(tl.debit_cents), 0) FROM transaction_lines tl \
             JOIN accounts a ON a.id = tl.account_id WHERE a.code = '1010'",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(savings, 4_000);
        assert_eq!(open(db.clone()).await.balance_cents, 6_000);

        // Blank deposit account means the default (Checking).
        record_payment(&db, &payment(6_000, Some(" "), id))
            .await
            .unwrap();
        assert_eq!(open(db.clone()).await, Receivables::default());
        let detail = invoice_detail(&db, id).await.unwrap();
        assert_eq!(detail.invoice.status_type, "Paid");
        assert_eq!(detail.payments.len(), 2);

        let err = invoice_detail(&db, id + 100).await.unwrap_err();
        assert_eq!(err.code, "NOT_FOUND");
    }

    #[test]
    fn invoice_errors_name_the_missing_invoice() {
        let e = invoice_error(sqlx::Error::RowNotFound);
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("NOT_FOUND", "Invoice not found")
        );
        let e = invoice_error(sqlx::Error::InvalidArgument("only draft".into()));
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("VALIDATION", "only draft")
        );
    }

    #[tokio::test]
    async fn nec_summary_counts_bank_payments_to_contractors() {
        let db = test_db().await;
        let ana = contact(&db, "Ana Designer", true).await;
        let client = contact(&db, "Acme", false).await;
        let pay = |date: &str, from: &str, cents: i64, who: i64| {
            tx_input(date, &[("5080", cents, 0), (from, 0, cents)], Some(who))
        };
        for input in [
            pay("2026-02-01", "1000", 150_000, ana),
            pay("2026-05-01", "1010", 60_000, ana),
            // Card payments are reported on 1099-K, not by the payer.
            pay("2026-06-01", "2000", 90_000, ana),
            pay("2025-12-31", "1000", 70_000, ana),
            pay("2026-02-01", "1000", 99_000, client),
        ] {
            insert_transaction(&db, input).await.unwrap();
        }

        let summary = nec_summary(&db, 2026).await.unwrap();
        assert_eq!(summary.len(), 1);
        let entry = &summary[0];
        assert_eq!(
            (
                entry.contact_id,
                entry.contact_name.as_str(),
                entry.ytd_cents
            ),
            (ana, "Ana Designer", 210_000)
        );
        assert!(entry.over_threshold);
        assert_eq!(entry.threshold_cents, 200_000);

        let prior = nec_summary(&db, 2025).await.unwrap();
        assert_eq!(prior[0].ytd_cents, 70_000);
        assert_eq!(prior[0].threshold_cents, 60_000);
        assert!(prior[0].over_threshold);
    }

    #[tokio::test]
    async fn estimate_reports_qbi_and_the_safe_harbor_rule_from_last_years_return() {
        let db = test_db().await;
        // $120,000 of 2026 revenue, estimated for the full year.
        post(
            &db,
            "2026-02-01",
            &[("1000", 12_000_000, 0), ("4000", 0, 12_000_000)],
        )
        .await;
        let estimate = || quarterly_estimate(&db, Some(2026), Some(4), day("2026-12-31"));

        let est = estimate().await.unwrap();
        assert_eq!(est.annualized_net_profit_cents, 12_000_000);
        assert!(est.qbi_deduction_cents > 0);
        // QBI is at most 20% of profit less the deductible half of SE tax.
        assert!(est.qbi_deduction_cents * 5 <= 12_000_000 - est.se_tax_deduction_cents);
        assert!(
            est.taxable_income_cents
                < 12_000_000 - est.se_tax_deduction_cents - est.standard_deduction_cents
        );
        // Nothing known about 2025: 90% of this year's projection, at 100%.
        assert_eq!(est.prior_year_tax_cents, None);
        assert_eq!(est.safe_harbor_prior_year_percentage, "1.00");
        assert_eq!(est.safe_harbor_cents, est.total_tax_cents * 9 / 10);

        // Last year's return: $5,000 tax on an AGI over $150,000 → 110%.
        let filed = aequi_storage::TaxReturnFigures {
            total_tax_cents: Some(500_000),
            agi_cents: Some(16_000_000),
        };
        assert_eq!(save_tax_return(&db, 2025, filed).await.unwrap(), filed);
        let est = estimate().await.unwrap();
        assert_eq!(est.prior_year_tax_cents, Some(500_000));
        assert_eq!(est.safe_harbor_prior_year_percentage, "1.10");
        assert_eq!(est.safe_harbor_cents, 550_000);

        // Below the threshold → 100%.
        let filed = aequi_storage::TaxReturnFigures {
            agi_cents: Some(10_000_000),
            ..filed
        };
        save_tax_return(&db, 2025, filed).await.unwrap();
        let est = estimate().await.unwrap();
        assert_eq!(est.safe_harbor_prior_year_percentage, "1.00");
        assert_eq!(est.safe_harbor_cents, 500_000);
    }

    #[tokio::test]
    async fn tax_return_figures_round_trip_and_are_validated() {
        let db = test_db().await;
        assert_eq!(
            load_tax_return(&db, 2025).await.unwrap(),
            aequi_storage::TaxReturnFigures::default()
        );

        let figures = aequi_storage::TaxReturnFigures {
            total_tax_cents: Some(1_234_500),
            agi_cents: Some(9_800_000),
        };
        assert_eq!(save_tax_return(&db, 2025, figures).await.unwrap(), figures);
        assert_eq!(load_tax_return(&db, 2025).await.unwrap(), figures);

        // `None` clears a figure.
        let cleared = aequi_storage::TaxReturnFigures {
            agi_cents: None,
            ..figures
        };
        assert_eq!(save_tax_return(&db, 2025, cleared).await.unwrap(), cleared);

        let negative = aequi_storage::TaxReturnFigures {
            total_tax_cents: Some(-1),
            agi_cents: None,
        };
        let err = save_tax_return(&db, 2025, negative).await.unwrap_err();
        assert_eq!(err.code, "VALIDATION");
        assert_eq!(load_tax_return(&db, 2025).await.unwrap(), cleared);

        for year in [1999, 2101] {
            assert_eq!(
                load_tax_return(&db, year).await.unwrap_err().code,
                "VALIDATION"
            );
            let err = save_tax_return(&db, year, figures).await.unwrap_err();
            assert_eq!(err.code, "VALIDATION");
        }
    }

    #[test]
    fn tax_return_figures_use_snake_case_fields() {
        let figures: aequi_storage::TaxReturnFigures =
            serde_json::from_str(r#"{"total_tax_cents": 100, "agi_cents": null}"#).unwrap();
        assert_eq!(figures.total_tax_cents, Some(100));
        assert_eq!(figures.agi_cents, None);
    }

    #[test]
    fn receipts_filled_by_an_agent_are_flagged() {
        let record = |ocr_text: Option<&str>, confidence: f64| aequi_storage::ReceiptRecord {
            id: 1,
            file_hash: "abc".into(),
            file_ext: "png".into(),
            ocr_text: ocr_text.map(Into::into),
            vendor: Some("Acme".into()),
            receipt_date: Some("2026-03-01".into()),
            total_cents: Some(1_250),
            subtotal_cents: None,
            tax_cents: None,
            payment_method: None,
            confidence,
            status: "pending_review".into(),
            transaction_id: None,
            attachment_path: "/tmp/r.png".into(),
            created_at: "2026-03-01 10:00:00".into(),
            reviewed_at: None,
        };
        assert!(ReceiptOutput::from(record(None, 0.9)).filled_by_agent);
        assert!(ReceiptOutput::from(record(Some("  "), 0.9)).filled_by_agent);
        assert!(!ReceiptOutput::from(record(Some("ACME 12.50"), 0.9)).filled_by_agent);
        assert!(!ReceiptOutput::from(record(None, 0.0)).filled_by_agent);
    }
}
