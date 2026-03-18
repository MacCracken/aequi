use aequi_core::{
    Account, ContactId, Discount, FiscalYear, InvoiceId, InvoiceLine, Money, Quarter, TaxLine,
    TransactionLine, UnvalidatedTransaction, ValidatedTransaction,
};
use aequi_ocr::{MockRecognizer, ReceiptPipeline};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

use crate::AppState;

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
        CommandError {
            code: "DATABASE".into(),
            message: e.to_string(),
        }
    }
}

impl From<aequi_core::LedgerError> for CommandError {
    fn from(e: aequi_core::LedgerError) -> Self {
        CommandError {
            code: "LEDGER".into(),
            message: e.to_string(),
        }
    }
}

impl From<aequi_ocr::PipelineError> for CommandError {
    fn from(e: aequi_ocr::PipelineError) -> Self {
        CommandError {
            code: "OCR".into(),
            message: e.to_string(),
        }
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
    let db = &db;

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

    let mut lines = Vec::new();
    for line in input.lines {
        let account = aequi_storage::get_account_by_code(db, &line.account_code)
            .await?
            .ok_or_else(|| {
                CommandError::not_found(format!("Account not found: {}", line.account_code))
            })?;

        if line.debit_cents < 0 || line.credit_cents < 0 {
            return Err(CommandError::validation(
                "Debit and credit amounts must be non-negative",
            ));
        }

        let debit = Money::from_cents(line.debit_cents);
        let credit = Money::from_cents(line.credit_cents);

        lines.push(TransactionLine {
            account_id: account
                .id
                .ok_or_else(|| CommandError::internal("Account missing ID"))?,
            debit,
            credit,
            memo: line.memo,
        });
    }

    let tx = UnvalidatedTransaction {
        date,
        description,
        lines,
        memo: input.memo,
    };

    let validated = ValidatedTransaction::validate(tx)?;

    // Use a SQL transaction for atomicity
    let mut sql_tx = db.begin().await?;

    let result = sqlx::query(
        "INSERT INTO transactions (date, description, memo, balanced_total_cents) VALUES (?, ?, ?, ?) RETURNING id, date, description, memo, balanced_total_cents, created_at"
    )
    .bind(validated.date.to_string())
    .bind(&validated.description)
    .bind(&validated.memo)
    .bind(validated.balanced_total.to_cents())
    .fetch_one(&mut *sql_tx)
    .await?;

    let id: i64 = result.get("id");
    let balanced_cents: i64 = result.get("balanced_total_cents");

    for line in &validated.lines {
        sqlx::query(
            "INSERT INTO transaction_lines (transaction_id, account_id, debit_cents, credit_cents, memo) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(id)
        .bind(line.account_id.0)
        .bind(line.debit.to_cents())
        .bind(line.credit.to_cents())
        .bind(&line.memo)
        .execute(&mut *sql_tx)
        .await?;
    }

    sql_tx.commit().await?;

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
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let db = &db;

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

    Ok(query
        .into_iter()
        .map(|r| TransactionOutput {
            id: r.0,
            date: r.1,
            description: r.2,
            memo: r.3,
            balanced_total: Money::from_cents(r.4).to_string(),
            created_at: r.5,
        })
        .collect())
}

#[tauri::command]
pub async fn get_profit_loss(
    state: State<'_, Arc<Mutex<AppState>>>,
    start_date: Option<String>,
    end_date: Option<String>,
) -> Result<Vec<ProfitLossEntry>, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let db = &db;

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
        "#,
    )
    .bind(&start)
    .bind(&end)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let total_cents: i64 = r.get("total_cents");
            ProfitLossEntry {
                account_code: r.get("code"),
                account_name: r.get("name"),
                total: Money::from_cents(total_cents).to_string(),
            }
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

    // Validate file size before processing (50 MB limit)
    const MAX_RECEIPT_SIZE: u64 = 50 * 1024 * 1024;
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|e| CommandError::validation(format!("Cannot read file: {e}")))?;
    if meta.len() > MAX_RECEIPT_SIZE {
        return Err(CommandError::validation(format!(
            "File too large ({:.1} MB, max 50 MB)",
            meta.len() as f64 / 1_048_576.0
        )));
    }

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
        e.payment_method
            .as_ref()
            .map(|f| f.value.to_string())
            .as_deref(),
        e.confidence as f64,
    )
    .await
    .map_err(|e| CommandError::internal(e.to_string()))?;

    let record = aequi_storage::get_receipt_by_id(&db, id)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
        .ok_or(CommandError::internal("Receipt not found after insert"))?;

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
    let records = aequi_storage::get_receipts_pending_review(&db)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;
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
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
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
            .await
            .map_err(|e| CommandError::internal(e.to_string()))?;
        if exists.is_none() {
            return Err(CommandError::not_found("Transaction not found"));
        }

        aequi_storage::link_receipt_to_transaction(&db, receipt_id, tx_id)
            .await
            .map_err(|e| CommandError::internal(e.to_string()))?;
    } else {
        aequi_storage::update_receipt_status(&db, receipt_id, "approved")
            .await
            .map_err(|e| CommandError::internal(e.to_string()))?;
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
    aequi_storage::update_receipt_status(&db, receipt_id, "rejected")
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;
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
    pub se_tax_cents: i64,
    pub se_tax_deduction_cents: i64,
    pub income_tax_cents: i64,
    pub total_tax_cents: i64,
    pub safe_harbor_cents: i64,
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
    let db = &db;

    let now = chrono::Utc::now().date_naive();
    let yr = year.unwrap_or(now.year() as u16);
    let qtr = quarter
        .and_then(Quarter::new)
        .unwrap_or_else(|| Quarter::new(((now.month0() / 3) + 1) as u8).unwrap_or(Quarter::Q1));

    let rules = load_tax_rules(yr)?;
    let fy = FiscalYear::new(yr);

    let prior_year_cents = aequi_storage::get_prior_year_total_tax(db, yr)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;
    let prior_year_tax = prior_year_cents.map(Money::from_cents);

    let snapshot = aequi_storage::build_ledger_snapshot(db, fy, prior_year_tax)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;

    let est = aequi_core::compute_quarterly_estimate(&rules, &snapshot, qtr);

    // Persist the estimate
    aequi_storage::upsert_tax_period(
        db,
        yr,
        quarter_to_u8(qtr),
        est.total_tax_estimate.to_cents(),
        est.se_tax_amount.to_cents(),
        est.estimated_income_tax.to_cents(),
        est.ytd_net_profit.to_cents(),
        &est.payment_due_date.to_string(),
        rules.year.value,
    )
    .await
    .map_err(|e| CommandError::internal(e.to_string()))?;

    let schedule_c_lines: Vec<ScheduleCLineOutput> = est
        .schedule_c_lines
        .iter()
        .map(|(line, amount)| ScheduleCLineOutput {
            line: format!("{line:?}"),
            label: line.label().to_string(),
            amount_cents: amount.to_cents(),
            is_income: line.is_income(),
        })
        .collect();

    Ok(QuarterlyEstimateOutput {
        year: est.year,
        quarter: est.quarter.to_string(),
        ytd_gross_income_cents: est.ytd_gross_income.to_cents(),
        ytd_total_expenses_cents: est.ytd_total_expenses.to_cents(),
        ytd_net_profit_cents: est.ytd_net_profit.to_cents(),
        se_tax_cents: est.se_tax_amount.to_cents(),
        se_tax_deduction_cents: est.se_tax_deduction.to_cents(),
        income_tax_cents: est.estimated_income_tax.to_cents(),
        total_tax_cents: est.total_tax_estimate.to_cents(),
        safe_harbor_cents: est.safe_harbor_amount.to_cents(),
        quarterly_payment_cents: est.quarterly_payment.to_cents(),
        payment_due_date: est.payment_due_date.to_string(),
        schedule_c_lines,
    })
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

    let yr = year.unwrap_or(chrono::Utc::now().date_naive().year() as u16);
    let rules = load_tax_rules(yr)?;
    let fy = FiscalYear::new(yr);

    let snapshot = aequi_storage::build_ledger_snapshot(db, fy, None)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;

    let preview = aequi_core::tax::engine::schedule_c_preview(&rules, &snapshot);

    let lines: Vec<ScheduleCLineOutput> = preview
        .lines
        .iter()
        .map(|(line, amount)| ScheduleCLineOutput {
            line: format!("{line:?}"),
            label: line.label().to_string(),
            amount_cents: amount.to_cents(),
            is_income: line.is_income(),
        })
        .collect();

    Ok(ScheduleCPreviewOutput {
        year: preview.year,
        gross_income_cents: preview.gross_income.to_cents(),
        total_expenses_cents: preview.total_expenses.to_cents(),
        net_profit_cents: preview.net_profit.to_cents(),
        lines,
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
    let contacts = aequi_storage::get_all_contacts(&db)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;
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
    .await
    .map_err(|e| CommandError {
        code: "DATABASE".into(),
        message: e.to_string(),
    })?;

    aequi_storage::get_contact_by_id(&db, id)
        .await
        .map_err(|e| CommandError {
            code: "DATABASE".into(),
            message: e.to_string(),
        })?
        .ok_or(CommandError::not_found("Contact not found after insert"))
}

// ── Invoice commands ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InvoiceInput {
    pub invoice_number: String,
    pub contact_id: i64,
    pub issue_date: String,
    pub due_date: String,
    pub notes: Option<String>,
    pub terms: Option<String>,
}

#[tauri::command]
pub async fn get_invoices(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<aequi_storage::InvoiceRecord>, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    aequi_storage::get_all_invoices(&db)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

#[tauri::command]
pub async fn create_invoice(
    state: State<'_, Arc<Mutex<AppState>>>,
    input: InvoiceInput,
) -> Result<aequi_storage::InvoiceRecord, CommandError> {
    // Input validation
    let invoice_number = input.invoice_number.trim().to_string();
    if invoice_number.is_empty() {
        return Err(CommandError::validation("Invoice number is required"));
    }
    let issue_date = NaiveDate::parse_from_str(&input.issue_date, "%Y-%m-%d")
        .map_err(|_| CommandError::validation("Invalid issue date format (expected YYYY-MM-DD)"))?;
    let due_date = NaiveDate::parse_from_str(&input.due_date, "%Y-%m-%d")
        .map_err(|_| CommandError::validation("Invalid due date format (expected YYYY-MM-DD)"))?;
    if due_date < issue_date {
        return Err(CommandError::validation(
            "Due date must be on or after issue date",
        ));
    }

    let db = {
        let s = state.lock().await;
        s.db.clone()
    };

    // Verify contact exists
    aequi_storage::get_contact_by_id(&db, input.contact_id)
        .await
        .map_err(|e| CommandError {
            code: "DATABASE".into(),
            message: e.to_string(),
        })?
        .ok_or(CommandError::not_found("Contact not found"))?;

    let id = aequi_storage::insert_invoice(
        &db,
        &invoice_number,
        input.contact_id,
        "Draft",
        None,
        &input.issue_date,
        &input.due_date,
        None,
        None,
        input.notes.as_deref(),
        input.terms.as_deref(),
    )
    .await
    .map_err(|e| CommandError {
        code: "DATABASE".into(),
        message: e.to_string(),
    })?;

    aequi_storage::get_invoice_by_id(&db, id)
        .await
        .map_err(|e| CommandError {
            code: "DATABASE".into(),
            message: e.to_string(),
        })?
        .ok_or(CommandError::not_found("Invoice not found after insert"))
}

#[tauri::command]
pub async fn get_invoice_aging(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<aequi_storage::InvoiceRecord>, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    aequi_storage::get_invoice_aging(&db)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

#[derive(Debug, Deserialize)]
pub struct PaymentInput {
    pub invoice_id: i64,
    pub amount_cents: i64,
    pub date: String,
    pub method: Option<String>,
}

#[tauri::command]
pub async fn record_invoice_payment(
    state: State<'_, Arc<Mutex<AppState>>>,
    input: PaymentInput,
) -> Result<i64, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    aequi_storage::insert_payment(
        &db,
        input.invoice_id,
        input.amount_cents,
        &input.date,
        input.method.as_deref(),
        None,
    )
    .await
    .map_err(|e| CommandError::internal(e.to_string()))
}

#[derive(Debug, Serialize)]
pub struct NecSummaryEntry {
    pub contact_id: i64,
    pub contact_name: String,
    pub ytd_cents: i64,
    pub over_threshold: bool,
}

#[tauri::command]
pub async fn get_1099_summary(
    state: State<'_, Arc<Mutex<AppState>>>,
    year: Option<u16>,
) -> Result<Vec<NecSummaryEntry>, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let yr = year.unwrap_or(chrono::Utc::now().date_naive().year() as u16);

    // Single JOIN query instead of N+1
    let rows = aequi_storage::get_contractor_ytd_payments(&db, yr)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;

    let entries = rows
        .into_iter()
        .map(|(id, name, ytd)| NecSummaryEntry {
            contact_id: id,
            contact_name: name,
            ytd_cents: ytd,
            over_threshold: aequi_core::check_1099_threshold(Money::from_cents(ytd)),
        })
        .collect();
    Ok(entries)
}

// ── Email delivery ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SendInvoiceInput {
    pub invoice_id: i64,
    pub subject: Option<String>,
}

/// Reconstruct a domain Invoice from storage records.
fn records_to_invoice(
    rec: &aequi_storage::InvoiceRecord,
    lines: &[aequi_storage::InvoiceLineRecord],
    tax_lines: &[aequi_storage::InvoiceTaxLineRecord],
) -> aequi_core::Invoice {
    let discount = match (rec.discount_type.as_deref(), rec.discount_value) {
        (Some("Percentage"), Some(bps)) => Some(Discount::Percentage(Decimal::new(bps, 2))),
        (Some("Flat"), Some(cents)) => Some(Discount::Flat(Money::from_cents(cents))),
        _ => None,
    };

    aequi_core::Invoice {
        id: Some(InvoiceId(rec.id)),
        invoice_number: rec.invoice_number.clone(),
        contact_id: ContactId(rec.contact_id),
        status: aequi_core::InvoiceStatus::Draft,
        issue_date: NaiveDate::parse_from_str(&rec.issue_date, "%Y-%m-%d")
            .unwrap_or_else(|_| chrono::Utc::now().date_naive()),
        due_date: NaiveDate::parse_from_str(&rec.due_date, "%Y-%m-%d")
            .unwrap_or_else(|_| chrono::Utc::now().date_naive()),
        lines: lines
            .iter()
            .map(|l| InvoiceLine {
                description: l.description.clone(),
                quantity: Decimal::new(l.quantity_hundredths, 2),
                unit_rate: Money::from_cents(l.unit_rate_cents),
                taxable: l.taxable,
            })
            .collect(),
        discount,
        tax_lines: tax_lines
            .iter()
            .map(|t| TaxLine {
                label: t.label.clone(),
                rate: Decimal::new(t.rate_bps, 4),
            })
            .collect(),
        notes: rec.notes.clone(),
        terms: rec.terms.clone(),
    }
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
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
        .ok_or(CommandError::internal(
            "Email not configured — set email_config in Settings",
        ))?;

    let config: aequi_email::EmailConfig = serde_json::from_str(&config_json)
        .map_err(|e| CommandError::internal(format!("Invalid email config: {e}")))?;

    let rec = aequi_storage::get_invoice_by_id(&db, input.invoice_id)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
        .ok_or(CommandError::internal("Invoice not found"))?;

    let lines = aequi_storage::get_invoice_lines(&db, input.invoice_id)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;
    let tax_lines = aequi_storage::get_invoice_tax_lines(&db, input.invoice_id)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;
    let invoice = records_to_invoice(&rec, &lines, &tax_lines);

    let contact_rec = aequi_storage::get_contact_by_id(&db, rec.contact_id)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
        .ok_or(CommandError::internal("Contact not found"))?;
    let contact = record_to_contact(&contact_rec);

    let result = aequi_email::send_invoice(&config, &invoice, &contact, input.subject.as_deref())
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;

    // Update invoice status to Sent
    let sent_data = serde_json::json!({ "sent_at": chrono::Utc::now().to_rfc3339() }).to_string();
    let _ =
        aequi_storage::update_invoice_status(&db, input.invoice_id, "Sent", Some(&sent_data)).await;

    Ok(result)
}

// ── Export commands ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn export_beancount(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<String, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let accounts = aequi_storage::get_all_accounts(&db)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;

    // For now export with empty transactions — full transaction fetch to be added
    Ok(aequi_core::export::beancount::export_beancount(
        &accounts,
        &[],
    ))
}

#[tauri::command]
pub async fn export_qif(_state: State<'_, Arc<Mutex<AppState>>>) -> Result<String, CommandError> {
    Ok(aequi_core::export::qif::export_qif(
        &[],
        aequi_core::AccountType::Asset,
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
    aequi_storage::get_setting(&db, &key)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
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
    aequi_storage::set_setting(&db, &key, &value)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
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
    aequi_storage::get_audit_log(&db, limit.unwrap_or(100))
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

#[tauri::command]
pub async fn create_backup(
    state: State<'_, Arc<Mutex<AppState>>>,
    output_path: String,
) -> Result<aequi_storage::backup::BackupManifest, CommandError> {
    let (db, db_path, attachments_dir) = {
        let s = state.lock().await;
        (s.db.clone(), s.db_path.clone(), s.attachments_dir.clone())
    };
    let output = std::path::PathBuf::from(&output_path);
    aequi_storage::backup::create_backup(
        &db,
        &db_path,
        &attachments_dir,
        &output,
        env!("CARGO_PKG_VERSION"),
    )
    .await
    .map_err(|e| CommandError::internal(e.to_string()))
}

#[tauri::command]
pub async fn restore_backup(
    archive_path: String,
    target_dir: String,
) -> Result<String, CommandError> {
    let result = aequi_storage::backup::restore_backup(
        std::path::Path::new(&archive_path),
        std::path::Path::new(&target_dir),
    )
    .map_err(|e| CommandError::internal(e.to_string()))?;
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
    aequi_storage::migrate::get_schema_versions(&db)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

// ── Update commands ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct UpdateStatus {
    pub update_available: bool,
    pub current_version: String,
    pub latest_version: Option<String>,
}

#[tauri::command]
pub async fn check_for_updates(app: tauri::AppHandle) -> Result<UpdateStatus, CommandError> {
    let current = app.package_info().version.to_string();

    match app
        .updater()
        .map_err(|e: tauri_plugin_updater::Error| CommandError::internal(e.to_string()))?
        .check()
        .await
    {
        Ok(Some(update)) => Ok(UpdateStatus {
            update_available: true,
            current_version: current,
            latest_version: Some(update.version.clone()),
        }),
        Ok(None) => Ok(UpdateStatus {
            update_available: false,
            current_version: current,
            latest_version: None,
        }),
        Err(e) => {
            tracing::warn!("Update check failed: {e}");
            Ok(UpdateStatus {
                update_available: false,
                current_version: current,
                latest_version: None,
            })
        }
    }
}

// ── Notification commands ───────────────────────────────────────────────────

#[tauri::command]
pub async fn check_overdue_invoices(
    app: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<u32, CommandError> {
    let db = {
        let s = state.lock().await;
        s.db.clone()
    };
    let aging = aequi_storage::get_invoice_aging(&db)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;

    let today = chrono::Utc::now().date_naive();
    let overdue: Vec<_> = aging
        .iter()
        .filter(|inv| {
            NaiveDate::parse_from_str(&inv.due_date, "%Y-%m-%d")
                .map(|d| d < today)
                .unwrap_or(false)
        })
        .collect();

    let count = overdue.len() as u32;

    if count > 0 {
        let body = if count == 1 {
            format!("Invoice {} is overdue", overdue[0].invoice_number)
        } else {
            format!("{count} invoices are past due")
        };

        let _ = app
            .notification()
            .builder()
            .title("Overdue Invoices")
            .body(&body)
            .show();
    }

    Ok(count)
}

// ── Dashboard commands ──────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DashboardSummary {
    pub ytd_income_cents: i64,
    pub ytd_expenses_cents: i64,
    pub ytd_net_profit_cents: i64,
    pub outstanding_invoices: u32,
    pub overdue_invoices: u32,
    pub pending_receipts: u32,
    pub total_accounts: u32,
    pub total_transactions: u32,
    pub recent_transactions: Vec<TransactionOutput>,
    pub quarterly_tax_due_cents: Option<i64>,
    pub next_tax_due_date: Option<String>,
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

    let now = chrono::Utc::now().date_naive();
    let year_start = NaiveDate::from_ymd_opt(now.year(), 1, 1)
        .unwrap()
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

    // Outstanding + overdue invoices
    let aging = aequi_storage::get_invoice_aging(db).await?;
    let outstanding = aging.len() as u32;
    let overdue = aging
        .iter()
        .filter(|inv| {
            NaiveDate::parse_from_str(&inv.due_date, "%Y-%m-%d")
                .map(|d| d < now)
                .unwrap_or(false)
        })
        .count() as u32;

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
    let recent = sqlx::query_as::<_, (i64, String, String, Option<String>, i64, String)>(
        "SELECT id, date, description, memo, balanced_total_cents, created_at FROM transactions ORDER BY date DESC, id DESC LIMIT 5"
    )
    .fetch_all(db)
    .await?;

    let recent_transactions: Vec<TransactionOutput> = recent
        .into_iter()
        .map(|r| TransactionOutput {
            id: r.0,
            date: r.1,
            description: r.2,
            memo: r.3,
            balanced_total: Money::from_cents(r.4).to_string(),
            created_at: r.5,
        })
        .collect();

    // Current quarter tax estimate
    let current_quarter = ((now.month0() / 3) + 1) as u8;
    let yr = now.year() as u16;
    let tax_periods = aequi_storage::get_tax_periods(db, yr).await?;
    let current_period = tax_periods
        .iter()
        .find(|p| p.quarter == current_quarter as i64);

    Ok(DashboardSummary {
        ytd_income_cents: ytd_income,
        ytd_expenses_cents: ytd_expenses,
        ytd_net_profit_cents: ytd_income - ytd_expenses,
        outstanding_invoices: outstanding,
        overdue_invoices: overdue,
        pending_receipts,
        total_accounts: accounts.0 as u32,
        total_transactions: tx_count.0 as u32,
        recent_transactions,
        quarterly_tax_due_cents: current_period.map(|p| p.estimated_tax_cents),
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
    .await
    .map_err(|e| CommandError {
        code: "DATABASE".into(),
        message: e.to_string(),
    })?;

    aequi_storage::get_contact_by_id(&db, input.id)
        .await
        .map_err(|e| CommandError {
            code: "DATABASE".into(),
            message: e.to_string(),
        })?
        .ok_or(CommandError::not_found("Contact not found"))
}
