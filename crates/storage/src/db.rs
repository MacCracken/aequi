use aequi_core::{Account, AccountId, AccountType, DEFAULT_ACCOUNTS};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::path::Path;

pub type DbPool = Pool<Sqlite>;

pub async fn create_db(path: &Path) -> Result<DbPool, sqlx::Error> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite:{}", path.display()))
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

    run_migrations(&pool).await?;

    Ok(pool)
}

async fn run_migrations(pool: &DbPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS accounts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            code TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            account_type TEXT NOT NULL,
            is_archetype INTEGER NOT NULL DEFAULT 0,
            is_archived INTEGER NOT NULL DEFAULT 0,
            schedule_c_line TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS transactions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            date TEXT NOT NULL,
            description TEXT NOT NULL,
            memo TEXT,
            balanced_total_cents INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS transaction_lines (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            transaction_id INTEGER NOT NULL,
            account_id INTEGER NOT NULL,
            debit_cents INTEGER NOT NULL DEFAULT 0,
            credit_cents INTEGER NOT NULL DEFAULT 0,
            memo TEXT,
            FOREIGN KEY (transaction_id) REFERENCES transactions(id) ON DELETE CASCADE,
            FOREIGN KEY (account_id) REFERENCES accounts(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS fiscal_periods (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            year INTEGER NOT NULL UNIQUE,
            start_date TEXT NOT NULL,
            end_date TEXT NOT NULL,
            is_closed INTEGER NOT NULL DEFAULT 0
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS import_profiles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            has_header INTEGER NOT NULL DEFAULT 1,
            delimiter TEXT NOT NULL DEFAULT ',',
            date_column INTEGER,
            description_column INTEGER,
            amount_column INTEGER,
            debit_column INTEGER,
            credit_column INTEGER,
            memo_column INTEGER,
            date_format TEXT NOT NULL DEFAULT '%Y-%m-%d',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS imported_transactions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_type TEXT NOT NULL,
            source_id TEXT,
            import_batch_id TEXT NOT NULL,
            date TEXT NOT NULL,
            description TEXT NOT NULL,
            amount_cents INTEGER NOT NULL,
            debit_cents INTEGER,
            credit_cents INTEGER,
            memo TEXT,
            matched_transaction_id INTEGER,
            category_rule_id INTEGER,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (matched_transaction_id) REFERENCES transactions(id),
            FOREIGN KEY (category_rule_id) REFERENCES categorization_rules(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS categorization_rules (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            priority INTEGER NOT NULL DEFAULT 0,
            match_pattern TEXT NOT NULL,
            match_type TEXT NOT NULL DEFAULT 'contains',
            account_id INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (account_id) REFERENCES accounts(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS reconciliation_sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            start_date TEXT NOT NULL,
            end_date TEXT NOT NULL,
            statement_balance_cents INTEGER NOT NULL,
            is_completed INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (account_id) REFERENCES accounts(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS reconciliation_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id INTEGER NOT NULL,
            imported_transaction_id INTEGER,
            transaction_id INTEGER,
            match_type TEXT NOT NULL,
            difference_cents INTEGER NOT NULL DEFAULT 0,
            is_resolved INTEGER NOT NULL DEFAULT 0,
            resolution_notes TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (session_id) REFERENCES reconciliation_sessions(id),
            FOREIGN KEY (imported_transaction_id) REFERENCES imported_transactions(id),
            FOREIGN KEY (transaction_id) REFERENCES transactions(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS receipts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_hash TEXT NOT NULL UNIQUE,
            file_ext TEXT NOT NULL DEFAULT 'jpg',
            ocr_text TEXT,
            vendor TEXT,
            receipt_date TEXT,
            total_cents INTEGER,
            subtotal_cents INTEGER,
            tax_cents INTEGER,
            payment_method TEXT,
            confidence REAL NOT NULL DEFAULT 0.0,
            status TEXT NOT NULL DEFAULT 'pending_review',
            transaction_id INTEGER,
            attachment_path TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            reviewed_at TEXT,
            FOREIGN KEY (transaction_id) REFERENCES transactions(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS receipt_line_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            receipt_id INTEGER NOT NULL,
            description TEXT NOT NULL,
            amount_cents INTEGER,
            quantity REAL,
            FOREIGN KEY (receipt_id) REFERENCES receipts(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
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

pub async fn get_account_by_code(pool: &DbPool, code: &str) -> Result<Option<Account>, sqlx::Error> {
    let row = sqlx::query_as::<_, AccountRow>(
        "SELECT id, code, name, account_type, is_archetype, is_archived, schedule_c_line FROM accounts WHERE code = ?"
    )
    .bind(code)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(row_to_account))
}

#[derive(Debug, Clone, sqlx::FromRow)]
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

pub async fn save_import_profile(pool: &DbPool, profile: &ImportProfile) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        r#"INSERT INTO import_profiles 
           (name, has_header, delimiter, date_column, description_column, 
            amount_column, debit_column, credit_column, memo_column, date_format)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#
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
    let rows = sqlx::query_as::<_, ImportProfile>(
        "SELECT * FROM import_profiles ORDER BY name"
    )
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

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CategorizationRule {
    pub id: i64,
    pub name: String,
    pub priority: i32,
    pub match_pattern: String,
    pub match_type: String,
    pub account_id: i64,
    pub created_at: String,
}

pub async fn save_categorization_rule(pool: &DbPool, rule: &CategorizationRule) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        r#"INSERT INTO categorization_rules (name, priority, match_pattern, match_type, account_id)
           VALUES (?, ?, ?, ?, ?)"#
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

pub async fn get_categorization_rules(pool: &DbPool) -> Result<Vec<CategorizationRule>, sqlx::Error> {
    let rows = sqlx::query_as::<_, CategorizationRule>(
        "SELECT * FROM categorization_rules ORDER BY priority DESC"
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

#[derive(Debug, Clone, sqlx::FromRow)]
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
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#
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
           VALUES (?, ?, ?, ?)"#
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
        "SELECT * FROM reconciliation_sessions WHERE account_id = ? ORDER BY created_at DESC"
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

#[derive(Debug, Clone, sqlx::FromRow)]
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
           VALUES (?, ?, ?, ?, ?)"#
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
        "UPDATE reconciliation_items SET is_resolved = 1, resolution_notes = ? WHERE id = ?"
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
        "SELECT * FROM reconciliation_items WHERE session_id = ? ORDER BY created_at"
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

#[derive(Debug, Clone, sqlx::FromRow)]
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

pub async fn get_receipts_pending_review(
    pool: &DbPool,
) -> Result<Vec<ReceiptRecord>, sqlx::Error> {
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
    sqlx::query(
        "UPDATE receipts SET status = ?, reviewed_at = datetime('now') WHERE id = ?",
    )
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
