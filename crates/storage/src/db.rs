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

pub async fn get_all_accounts(pool: &DbPool) -> Result<Vec<Account>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, String, String, String, i64, i64, Option<String>)>(
        "SELECT id, code, name, account_type, is_archetype, is_archived, schedule_c_line FROM accounts WHERE is_archived = 0 ORDER BY code"
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r| {
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
    }).collect())
}

pub async fn get_account_by_code(pool: &DbPool, code: &str) -> Result<Option<Account>, sqlx::Error> {
    let row = sqlx::query_as::<_, (i64, String, String, String, i64, i64, Option<String>)>(
        "SELECT id, code, name, account_type, is_archetype, is_archived, schedule_c_line FROM accounts WHERE code = ?"
    )
    .bind(code)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
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
    }))
}
