use aequi_core::{
    Account, AccountId, AccountType, FiscalYear, LedgerSnapshot, Money, ScheduleCLine,
    DEFAULT_ACCOUNTS,
};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::collections::BTreeMap;
use std::path::Path;

pub type DbPool = Pool<Sqlite>;

pub async fn create_db(path: &Path) -> Result<DbPool, sqlx::Error> {
    // WAL mode supports concurrent readers + one writer.
    // 4 connections allows parallel reads without serializing everything.
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect(&format!("sqlite:{}?mode=rwc", path.display()))
        .await?;

    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA synchronous = NORMAL")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA busy_timeout = 5000")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA cache_size = -32000")
        .execute(&pool)
        .await?;

    crate::migrate::run_migrations(&pool).await?;

    Ok(pool)
}

pub async fn seed_default_accounts(pool: &DbPool) -> Result<(), sqlx::Error> {
    for (code, name, account_type, schedule_c_line) in DEFAULT_ACCOUNTS {
        let type_str = match account_type {
            AccountType::Asset => "Asset",
            AccountType::Liability => "Liability",
            AccountType::Equity => "Equity",
            AccountType::Income => "Income",
            AccountType::Expense => "Expense",
        };

        sqlx::query(
            "INSERT OR IGNORE INTO accounts (code, name, account_type, is_archetype, schedule_c_line) VALUES (?, ?, ?, 1, ?)"
        )
        .bind(code)
        .bind(name)
        .bind(type_str)
        .bind(if schedule_c_line.is_empty() { None } else { Some(*schedule_c_line) })
        .execute(pool)
        .await?;
    }

    Ok(())
}

type AccountRow = (i64, String, String, String, i64, i64, Option<String>);

fn row_to_account(r: AccountRow) -> Account {
    let account_type = match r.3.as_str() {
        "Asset" => AccountType::Asset,
        "Liability" => AccountType::Liability,
        "Equity" => AccountType::Equity,
        "Income" => AccountType::Income,
        "Expense" => AccountType::Expense,
        _ => AccountType::Asset,
    };
    Account {
        id: Some(AccountId(r.0)),
        code: r.1,
        name: r.2,
        account_type,
        is_archetype: r.4 != 0,
        is_archived: r.5 != 0,
        schedule_c_line: r.6,
    }
}

pub async fn get_all_accounts(pool: &DbPool) -> Result<Vec<Account>, sqlx::Error> {
    let rows = sqlx::query_as::<_, AccountRow>(
        "SELECT id, code, name, account_type, is_archetype, is_archived, schedule_c_line FROM accounts WHERE is_archived = 0 ORDER BY code"
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(row_to_account).collect())
}

pub async fn get_account_by_code(
    pool: &DbPool,
    code: &str,
) -> Result<Option<Account>, sqlx::Error> {
    let row = sqlx::query_as::<_, AccountRow>(
        "SELECT id, code, name, account_type, is_archetype, is_archived, schedule_c_line FROM accounts WHERE code = ?"
    )
    .bind(code)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(row_to_account))
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ImportProfile {
    pub id: i64,
    pub name: String,
    pub has_header: bool,
    pub delimiter: String,
    pub date_column: Option<i64>,
    pub description_column: Option<i64>,
    pub amount_column: Option<i64>,
    pub debit_column: Option<i64>,
    pub credit_column: Option<i64>,
    pub memo_column: Option<i64>,
    pub date_format: String,
    pub created_at: String,
}

pub async fn save_import_profile(
    pool: &DbPool,
    profile: &ImportProfile,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        r#"INSERT INTO import_profiles 
           (name, has_header, delimiter, date_column, description_column, 
            amount_column, debit_column, credit_column, memo_column, date_format)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(&profile.name)
    .bind(profile.has_header)
    .bind(&profile.delimiter)
    .bind(profile.date_column)
    .bind(profile.description_column)
    .bind(profile.amount_column)
    .bind(profile.debit_column)
    .bind(profile.credit_column)
    .bind(profile.memo_column)
    .bind(&profile.date_format)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

pub async fn get_import_profiles(pool: &DbPool) -> Result<Vec<ImportProfile>, sqlx::Error> {
    let rows = sqlx::query_as::<_, ImportProfile>("SELECT * FROM import_profiles ORDER BY name")
        .fetch_all(pool)
        .await?;

    Ok(rows)
}

pub async fn delete_import_profile(pool: &DbPool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM import_profiles WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct CategorizationRule {
    pub id: i64,
    pub name: String,
    pub priority: i32,
    pub match_pattern: String,
    pub match_type: String,
    pub account_id: i64,
    pub created_at: String,
}

pub async fn save_categorization_rule(
    pool: &DbPool,
    rule: &CategorizationRule,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        r#"INSERT INTO categorization_rules (name, priority, match_pattern, match_type, account_id)
           VALUES (?, ?, ?, ?, ?)"#,
    )
    .bind(&rule.name)
    .bind(rule.priority)
    .bind(&rule.match_pattern)
    .bind(&rule.match_type)
    .bind(rule.account_id)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

pub async fn get_categorization_rules(
    pool: &DbPool,
) -> Result<Vec<CategorizationRule>, sqlx::Error> {
    let rows = sqlx::query_as::<_, CategorizationRule>(
        "SELECT * FROM categorization_rules ORDER BY priority DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn delete_categorization_rule(pool: &DbPool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM categorization_rules WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ImportedTransaction {
    pub id: i64,
    pub source_type: String,
    pub source_id: Option<String>,
    pub import_batch_id: String,
    pub date: String,
    pub description: String,
    pub amount_cents: i64,
    pub debit_cents: Option<i64>,
    pub credit_cents: Option<i64>,
    pub memo: Option<String>,
    pub matched_transaction_id: Option<i64>,
    pub category_rule_id: Option<i64>,
    pub status: String,
    pub created_at: String,
}

pub async fn insert_imported_transaction(
    pool: &DbPool,
    tx: &ImportedTransaction,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        r#"INSERT INTO imported_transactions 
           (source_type, source_id, import_batch_id, date, description, 
            amount_cents, debit_cents, credit_cents, memo, status)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(&tx.source_type)
    .bind(&tx.source_id)
    .bind(&tx.import_batch_id)
    .bind(&tx.date)
    .bind(&tx.description)
    .bind(tx.amount_cents)
    .bind(tx.debit_cents)
    .bind(tx.credit_cents)
    .bind(&tx.memo)
    .bind(&tx.status)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

pub async fn get_pending_imported_transactions(
    pool: &DbPool,
    batch_id: &str,
) -> Result<Vec<ImportedTransaction>, sqlx::Error> {
    let rows = sqlx::query_as::<_, ImportedTransaction>(
        "SELECT * FROM imported_transactions WHERE import_batch_id = ? AND status = 'pending' ORDER BY date"
    )
    .bind(batch_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn mark_imported_transaction_matched(
    pool: &DbPool,
    id: i64,
    transaction_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE imported_transactions SET matched_transaction_id = ?, status = 'matched' WHERE id = ?"
    )
    .bind(transaction_id)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn mark_imported_transaction_categorized(
    pool: &DbPool,
    id: i64,
    rule_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE imported_transactions SET category_rule_id = ?, status = 'categorized' WHERE id = ?"
    )
    .bind(rule_id)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_imported_transactions_for_review(
    pool: &DbPool,
    batch_id: &str,
) -> Result<Vec<ImportedTransaction>, sqlx::Error> {
    let rows = sqlx::query_as::<_, ImportedTransaction>(
        "SELECT * FROM imported_transactions WHERE import_batch_id = ? AND status IN ('pending', 'categorized') ORDER BY date"
    )
    .bind(batch_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReconciliationSession {
    pub id: i64,
    pub account_id: i64,
    pub start_date: String,
    pub end_date: String,
    pub statement_balance_cents: i64,
    pub is_completed: bool,
    pub created_at: String,
}

pub async fn create_reconciliation_session(
    pool: &DbPool,
    account_id: i64,
    start_date: &str,
    end_date: &str,
    statement_balance_cents: i64,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        r#"INSERT INTO reconciliation_sessions 
           (account_id, start_date, end_date, statement_balance_cents)
           VALUES (?, ?, ?, ?)"#,
    )
    .bind(account_id)
    .bind(start_date)
    .bind(end_date)
    .bind(statement_balance_cents)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

pub async fn get_reconciliation_sessions(
    pool: &DbPool,
    account_id: i64,
) -> Result<Vec<ReconciliationSession>, sqlx::Error> {
    let rows = sqlx::query_as::<_, ReconciliationSession>(
        "SELECT * FROM reconciliation_sessions WHERE account_id = ? ORDER BY created_at DESC",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn complete_reconciliation_session(
    pool: &DbPool,
    session_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE reconciliation_sessions SET is_completed = 1 WHERE id = ?")
        .bind(session_id)
        .execute(pool)
        .await?;

    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct ReconciliationItem {
    pub id: i64,
    pub session_id: i64,
    pub imported_transaction_id: Option<i64>,
    pub transaction_id: Option<i64>,
    pub match_type: String,
    pub difference_cents: i64,
    pub is_resolved: bool,
    pub resolution_notes: Option<String>,
    pub created_at: String,
}

pub async fn add_reconciliation_item(
    pool: &DbPool,
    session_id: i64,
    imported_transaction_id: Option<i64>,
    transaction_id: Option<i64>,
    match_type: &str,
    difference_cents: i64,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        r#"INSERT INTO reconciliation_items 
           (session_id, imported_transaction_id, transaction_id, match_type, difference_cents)
           VALUES (?, ?, ?, ?, ?)"#,
    )
    .bind(session_id)
    .bind(imported_transaction_id)
    .bind(transaction_id)
    .bind(match_type)
    .bind(difference_cents)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

pub async fn resolve_reconciliation_item(
    pool: &DbPool,
    item_id: i64,
    resolution_notes: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE reconciliation_items SET is_resolved = 1, resolution_notes = ? WHERE id = ?",
    )
    .bind(resolution_notes)
    .bind(item_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_reconciliation_items(
    pool: &DbPool,
    session_id: i64,
) -> Result<Vec<ReconciliationItem>, sqlx::Error> {
    let rows = sqlx::query_as::<_, ReconciliationItem>(
        "SELECT * FROM reconciliation_items WHERE session_id = ? ORDER BY created_at",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn get_unresolved_reconciliation_items(
    pool: &DbPool,
    session_id: i64,
) -> Result<Vec<ReconciliationItem>, sqlx::Error> {
    let rows = sqlx::query_as::<_, ReconciliationItem>(
        "SELECT * FROM reconciliation_items WHERE session_id = ? AND is_resolved = 0 ORDER BY created_at"
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ── Receipt storage ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct ReceiptRecord {
    pub id: i64,
    pub file_hash: String,
    pub file_ext: String,
    pub ocr_text: Option<String>,
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
    pub created_at: String,
    pub reviewed_at: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_receipt(
    pool: &DbPool,
    file_hash: &str,
    file_ext: &str,
    attachment_path: &str,
    ocr_text: Option<&str>,
    vendor: Option<&str>,
    receipt_date: Option<&str>,
    total_cents: Option<i64>,
    subtotal_cents: Option<i64>,
    tax_cents: Option<i64>,
    payment_method: Option<&str>,
    confidence: f64,
) -> Result<i64, sqlx::Error> {
    // Silently ignore exact duplicates (same file imported twice).
    let result = sqlx::query(
        r#"INSERT OR IGNORE INTO receipts
           (file_hash, file_ext, attachment_path, ocr_text, vendor, receipt_date,
            total_cents, subtotal_cents, tax_cents, payment_method, confidence)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(file_hash)
    .bind(file_ext)
    .bind(attachment_path)
    .bind(ocr_text)
    .bind(vendor)
    .bind(receipt_date)
    .bind(total_cents)
    .bind(subtotal_cents)
    .bind(tax_cents)
    .bind(payment_method)
    .bind(confidence)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        // Duplicate — return the existing id.
        let row = sqlx::query_as::<_, (i64,)>("SELECT id FROM receipts WHERE file_hash = ?")
            .bind(file_hash)
            .fetch_one(pool)
            .await?;
        return Ok(row.0);
    }

    Ok(result.last_insert_rowid())
}

pub async fn get_receipt_by_id(
    pool: &DbPool,
    id: i64,
) -> Result<Option<ReceiptRecord>, sqlx::Error> {
    let row = sqlx::query_as::<_, ReceiptRecord>("SELECT * FROM receipts WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn get_receipts_pending_review(pool: &DbPool) -> Result<Vec<ReceiptRecord>, sqlx::Error> {
    let rows = sqlx::query_as::<_, ReceiptRecord>(
        "SELECT * FROM receipts WHERE status = 'pending_review' ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update_receipt_status(
    pool: &DbPool,
    id: i64,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE receipts SET status = ?, reviewed_at = datetime('now') WHERE id = ?")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn link_receipt_to_transaction(
    pool: &DbPool,
    receipt_id: i64,
    transaction_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE receipts SET transaction_id = ?, status = 'approved', reviewed_at = datetime('now') WHERE id = ?",
    )
    .bind(transaction_id)
    .bind(receipt_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn check_receipt_duplicate(
    pool: &DbPool,
    file_hash: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query_as::<_, (i64,)>("SELECT id FROM receipts WHERE file_hash = ?")
        .bind(file_hash)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.0))
}

// ── Tax engine storage ───────────────────────────────────────────────────────

/// Build a LedgerSnapshot for a given fiscal year by aggregating transaction_lines
/// grouped by the account's schedule_c_line tag.
pub async fn build_ledger_snapshot(
    pool: &DbPool,
    year: FiscalYear,
    prior_year_tax: Option<Money>,
) -> Result<LedgerSnapshot, sqlx::Error> {
    let start = year.start_date().to_string();
    let end = year.end_date().to_string();

    // Income: credit_cents - debit_cents (net credit = revenue)
    // Expenses: debit_cents - credit_cents (net debit = cost)
    let rows = sqlx::query_as::<_, (String, String, i64, i64)>(
        r#"
        SELECT a.schedule_c_line, a.account_type,
            COALESCE(SUM(tl.debit_cents), 0) as total_debit,
            COALESCE(SUM(tl.credit_cents), 0) as total_credit
        FROM accounts a
        JOIN transaction_lines tl ON a.id = tl.account_id
        JOIN transactions t ON tl.transaction_id = t.id
        WHERE a.schedule_c_line IS NOT NULL
            AND a.schedule_c_line != ''
            AND t.date >= ? AND t.date <= ?
        GROUP BY a.schedule_c_line, a.account_type
        "#,
    )
    .bind(&start)
    .bind(&end)
    .fetch_all(pool)
    .await?;

    let mut line_totals = BTreeMap::new();
    for (tag, account_type, total_debit, total_credit) in rows {
        if let Some(line) = ScheduleCLine::from_tag(&tag) {
            let amount = match account_type.as_str() {
                "Income" => Money::from_cents(total_credit - total_debit),
                "Expense" => Money::from_cents(total_debit - total_credit),
                _ => Money::zero(),
            };
            if !amount.is_zero() {
                let entry = line_totals.entry(line).or_insert(Money::zero());
                *entry = *entry + amount;
            }
        }
    }

    Ok(LedgerSnapshot {
        year,
        line_totals,
        prior_year_tax,
    })
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaxPeriodRecord {
    pub id: i64,
    pub year: i64,
    pub quarter: i64,
    pub estimated_tax_cents: i64,
    pub se_tax_cents: i64,
    pub income_tax_cents: i64,
    pub net_profit_cents: i64,
    pub payment_recorded_cents: i64,
    pub payment_date: Option<String>,
    pub due_date: String,
    pub rules_year: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_tax_period(
    pool: &DbPool,
    year: u16,
    quarter: u8,
    estimated_tax_cents: i64,
    se_tax_cents: i64,
    income_tax_cents: i64,
    net_profit_cents: i64,
    due_date: &str,
    rules_year: u16,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        r#"INSERT INTO tax_periods
           (year, quarter, estimated_tax_cents, se_tax_cents, income_tax_cents,
            net_profit_cents, due_date, rules_year)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT(year, quarter) DO UPDATE SET
             estimated_tax_cents = excluded.estimated_tax_cents,
             se_tax_cents = excluded.se_tax_cents,
             income_tax_cents = excluded.income_tax_cents,
             net_profit_cents = excluded.net_profit_cents,
             due_date = excluded.due_date,
             rules_year = excluded.rules_year,
             updated_at = datetime('now')
        "#,
    )
    .bind(year as i64)
    .bind(quarter as i64)
    .bind(estimated_tax_cents)
    .bind(se_tax_cents)
    .bind(income_tax_cents)
    .bind(net_profit_cents)
    .bind(due_date)
    .bind(rules_year as i64)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

pub async fn record_tax_payment(
    pool: &DbPool,
    year: u16,
    quarter: u8,
    payment_cents: i64,
    payment_date: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE tax_periods
           SET payment_recorded_cents = ?,
               payment_date = ?,
               updated_at = datetime('now')
           WHERE year = ? AND quarter = ?"#,
    )
    .bind(payment_cents)
    .bind(payment_date)
    .bind(year as i64)
    .bind(quarter as i64)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_tax_periods(
    pool: &DbPool,
    year: u16,
) -> Result<Vec<TaxPeriodRecord>, sqlx::Error> {
    let rows = sqlx::query_as::<_, TaxPeriodRecord>(
        "SELECT * FROM tax_periods WHERE year = ? ORDER BY quarter",
    )
    .bind(year as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_prior_year_total_tax(
    pool: &DbPool,
    year: u16,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query_as::<_, (i64,)>(
        "SELECT COALESCE(SUM(estimated_tax_cents), 0) FROM tax_periods WHERE year = ?",
    )
    .bind((year - 1) as i64)
    .fetch_optional(pool)
    .await?;

    match row {
        Some((total,)) if total > 0 => Ok(Some(total)),
        _ => Ok(None),
    }
}

// ── Contact storage ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct ContactRecord {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub contact_type: String,
    pub is_contractor: bool,
    pub tax_id: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_contact(
    pool: &DbPool,
    name: &str,
    email: Option<&str>,
    phone: Option<&str>,
    address: Option<&str>,
    contact_type: &str,
    is_contractor: bool,
    tax_id: Option<&str>,
    notes: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        r#"INSERT INTO contacts (name, email, phone, address, contact_type, is_contractor, tax_id, notes)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(name)
    .bind(email)
    .bind(phone)
    .bind(address)
    .bind(contact_type)
    .bind(is_contractor)
    .bind(tax_id)
    .bind(notes)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

#[allow(clippy::too_many_arguments)]
pub async fn update_contact(
    pool: &DbPool,
    id: i64,
    name: &str,
    email: Option<&str>,
    phone: Option<&str>,
    address: Option<&str>,
    contact_type: &str,
    is_contractor: bool,
    tax_id: Option<&str>,
    notes: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE contacts SET name=?, email=?, phone=?, address=?, contact_type=?,
           is_contractor=?, tax_id=?, notes=? WHERE id=?"#,
    )
    .bind(name)
    .bind(email)
    .bind(phone)
    .bind(address)
    .bind(contact_type)
    .bind(is_contractor)
    .bind(tax_id)
    .bind(notes)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_all_contacts(pool: &DbPool) -> Result<Vec<ContactRecord>, sqlx::Error> {
    sqlx::query_as::<_, ContactRecord>("SELECT * FROM contacts ORDER BY name LIMIT 5000")
        .fetch_all(pool)
        .await
}

pub async fn get_contact_by_id(
    pool: &DbPool,
    id: i64,
) -> Result<Option<ContactRecord>, sqlx::Error> {
    sqlx::query_as::<_, ContactRecord>("SELECT * FROM contacts WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn get_contractors(pool: &DbPool) -> Result<Vec<ContactRecord>, sqlx::Error> {
    sqlx::query_as::<_, ContactRecord>(
        "SELECT * FROM contacts WHERE is_contractor = 1 ORDER BY name",
    )
    .fetch_all(pool)
    .await
}

// ── Invoice storage ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct InvoiceRecord {
    pub id: i64,
    pub invoice_number: String,
    pub contact_id: i64,
    pub status_type: String,
    pub status_data: Option<String>,
    pub issue_date: String,
    pub due_date: String,
    pub discount_type: Option<String>,
    pub discount_value: Option<i64>,
    pub notes: Option<String>,
    pub terms: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct InvoiceLineRecord {
    pub id: i64,
    pub invoice_id: i64,
    pub description: String,
    pub quantity_hundredths: i64,
    pub unit_rate_cents: i64,
    pub taxable: bool,
    pub sort_order: i64,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct InvoiceTaxLineRecord {
    pub id: i64,
    pub invoice_id: i64,
    pub label: String,
    pub rate_bps: i64,
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_invoice(
    pool: &DbPool,
    invoice_number: &str,
    contact_id: i64,
    status_type: &str,
    status_data: Option<&str>,
    issue_date: &str,
    due_date: &str,
    discount_type: Option<&str>,
    discount_value: Option<i64>,
    notes: Option<&str>,
    terms: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        r#"INSERT INTO invoices (invoice_number, contact_id, status_type, status_data,
           issue_date, due_date, discount_type, discount_value, notes, terms)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(invoice_number)
    .bind(contact_id)
    .bind(status_type)
    .bind(status_data)
    .bind(issue_date)
    .bind(due_date)
    .bind(discount_type)
    .bind(discount_value)
    .bind(notes)
    .bind(terms)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

pub async fn update_invoice_status(
    pool: &DbPool,
    id: i64,
    status_type: &str,
    status_data: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE invoices SET status_type=?, status_data=?, updated_at=datetime('now') WHERE id=?",
    )
    .bind(status_type)
    .bind(status_data)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_all_invoices(pool: &DbPool) -> Result<Vec<InvoiceRecord>, sqlx::Error> {
    sqlx::query_as::<_, InvoiceRecord>(
        "SELECT * FROM invoices ORDER BY created_at DESC LIMIT 5000",
    )
    .fetch_all(pool)
    .await
}

pub async fn get_invoice_by_id(
    pool: &DbPool,
    id: i64,
) -> Result<Option<InvoiceRecord>, sqlx::Error> {
    sqlx::query_as::<_, InvoiceRecord>("SELECT * FROM invoices WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn get_invoices_by_status(
    pool: &DbPool,
    status_type: &str,
) -> Result<Vec<InvoiceRecord>, sqlx::Error> {
    sqlx::query_as::<_, InvoiceRecord>(
        "SELECT * FROM invoices WHERE status_type = ? ORDER BY due_date",
    )
    .bind(status_type)
    .fetch_all(pool)
    .await
}

pub async fn insert_invoice_line(
    pool: &DbPool,
    invoice_id: i64,
    description: &str,
    quantity_hundredths: i64,
    unit_rate_cents: i64,
    taxable: bool,
    sort_order: i64,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        r#"INSERT INTO invoice_lines (invoice_id, description, quantity_hundredths,
           unit_rate_cents, taxable, sort_order) VALUES (?, ?, ?, ?, ?, ?)"#,
    )
    .bind(invoice_id)
    .bind(description)
    .bind(quantity_hundredths)
    .bind(unit_rate_cents)
    .bind(taxable)
    .bind(sort_order)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

pub async fn get_invoice_lines(
    pool: &DbPool,
    invoice_id: i64,
) -> Result<Vec<InvoiceLineRecord>, sqlx::Error> {
    sqlx::query_as::<_, InvoiceLineRecord>(
        "SELECT * FROM invoice_lines WHERE invoice_id = ? ORDER BY sort_order",
    )
    .bind(invoice_id)
    .fetch_all(pool)
    .await
}

pub async fn insert_invoice_tax_line(
    pool: &DbPool,
    invoice_id: i64,
    label: &str,
    rate_bps: i64,
) -> Result<i64, sqlx::Error> {
    let result =
        sqlx::query("INSERT INTO invoice_tax_lines (invoice_id, label, rate_bps) VALUES (?, ?, ?)")
            .bind(invoice_id)
            .bind(label)
            .bind(rate_bps)
            .execute(pool)
            .await?;
    Ok(result.last_insert_rowid())
}

pub async fn get_invoice_tax_lines(
    pool: &DbPool,
    invoice_id: i64,
) -> Result<Vec<InvoiceTaxLineRecord>, sqlx::Error> {
    sqlx::query_as::<_, InvoiceTaxLineRecord>(
        "SELECT * FROM invoice_tax_lines WHERE invoice_id = ?",
    )
    .bind(invoice_id)
    .fetch_all(pool)
    .await
}

// ── Payment storage ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct PaymentRecord {
    pub id: i64,
    pub invoice_id: i64,
    pub amount_cents: i64,
    pub date: String,
    pub method: Option<String>,
    pub transaction_id: Option<i64>,
    pub created_at: String,
}

pub async fn insert_payment(
    pool: &DbPool,
    invoice_id: i64,
    amount_cents: i64,
    date: &str,
    method: Option<&str>,
    transaction_id: Option<i64>,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        r#"INSERT INTO payments (invoice_id, amount_cents, date, method, transaction_id)
           VALUES (?, ?, ?, ?, ?)"#,
    )
    .bind(invoice_id)
    .bind(amount_cents)
    .bind(date)
    .bind(method)
    .bind(transaction_id)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

pub async fn get_payments_for_invoice(
    pool: &DbPool,
    invoice_id: i64,
) -> Result<Vec<PaymentRecord>, sqlx::Error> {
    sqlx::query_as::<_, PaymentRecord>("SELECT * FROM payments WHERE invoice_id = ? ORDER BY date")
        .bind(invoice_id)
        .fetch_all(pool)
        .await
}

pub async fn get_ytd_payments_to_contact(
    pool: &DbPool,
    contact_id: i64,
    year: u16,
) -> Result<i64, sqlx::Error> {
    let start = format!("{year}-01-01");
    let end = format!("{year}-12-31");
    let row = sqlx::query_as::<_, (i64,)>(
        r#"SELECT COALESCE(SUM(p.amount_cents), 0)
           FROM payments p
           JOIN invoices i ON p.invoice_id = i.id
           WHERE i.contact_id = ? AND p.date >= ? AND p.date <= ?"#,
    )
    .bind(contact_id)
    .bind(&start)
    .bind(&end)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// Get YTD payments for all contractors in a single query (avoids N+1).
pub async fn get_contractor_ytd_payments(
    pool: &DbPool,
    year: u16,
) -> Result<Vec<(i64, String, i64)>, sqlx::Error> {
    let start = format!("{year}-01-01");
    let end = format!("{year}-12-31");
    sqlx::query_as::<_, (i64, String, i64)>(
        r#"SELECT c.id, c.name, COALESCE(SUM(p.amount_cents), 0) as ytd
           FROM contacts c
           LEFT JOIN invoices i ON i.contact_id = c.id
           LEFT JOIN payments p ON p.invoice_id = i.id AND p.date >= ? AND p.date <= ?
           WHERE c.is_contractor = 1
           GROUP BY c.id, c.name
           ORDER BY c.name"#,
    )
    .bind(&start)
    .bind(&end)
    .fetch_all(pool)
    .await
}

/// Invoice aging report: returns invoices bucketed by days overdue.
pub async fn get_invoice_aging(pool: &DbPool) -> Result<Vec<InvoiceRecord>, sqlx::Error> {
    sqlx::query_as::<_, InvoiceRecord>(
        r#"SELECT * FROM invoices
           WHERE status_type IN ('Sent', 'Viewed', 'PartiallyPaid')
           ORDER BY due_date"#,
    )
    .fetch_all(pool)
    .await
}

// ── Audit log ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct AuditLogRecord {
    pub id: i64,
    pub timestamp: String,
    pub tool_name: String,
    pub input_hash: Option<String>,
    pub outcome: String,
    pub details: Option<String>,
}

pub async fn insert_audit_log(
    pool: &DbPool,
    tool_name: &str,
    input_hash: Option<&str>,
    outcome: &str,
    details: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO audit_log (tool_name, input_hash, outcome, details) VALUES (?, ?, ?, ?)",
    )
    .bind(tool_name)
    .bind(input_hash)
    .bind(outcome)
    .bind(details)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

pub async fn get_audit_log(pool: &DbPool, limit: i64) -> Result<Vec<AuditLogRecord>, sqlx::Error> {
    sqlx::query_as::<_, AuditLogRecord>("SELECT * FROM audit_log ORDER BY timestamp DESC LIMIT ?")
        .bind(limit)
        .fetch_all(pool)
        .await
}

// ── Settings storage ─────────────────────────────────────────────────────────

pub async fn get_setting(pool: &DbPool, key: &str) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.0))
}

pub async fn set_setting(pool: &DbPool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}
