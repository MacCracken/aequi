//! Schema migration system with versioned SQL files.
//!
//! Migrations are embedded at compile time and applied in order.
//! Each migration is tracked in the `schema_versions` table.

use crate::db::DbPool;
use serde::Serialize;
use sqlx::FromRow;

/// A registered migration with its SQL content.
struct Migration {
    version: i64,
    name: &'static str,
    up_sql: &'static str,
    down_sql: &'static str,
}

/// Record of an applied migration.
#[derive(Debug, FromRow, Serialize)]
pub struct SchemaVersion {
    pub version: i64,
    pub name: String,
    pub applied_at: String,
    pub checksum: String,
}

/// All known migrations, ordered by version.
fn all_migrations() -> Vec<Migration> {
    vec![Migration {
        version: 1,
        name: "initial_schema",
        up_sql: include_str!("migrations/V001__initial_schema.sql"),
        down_sql: include_str!("migrations/V001__initial_schema.down.sql"),
    }]
}

/// Compute a simple checksum for migration content.
fn checksum(sql: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    sql.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Ensure the schema_versions tracking table exists.
async fn ensure_version_table(pool: &DbPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS schema_versions (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (datetime('now')),
            checksum TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Detect pre-existing databases that were created before the migration system.
/// If core tables exist but schema_versions is empty, mark V001 as applied.
async fn bootstrap_existing_db(pool: &DbPool) -> Result<(), sqlx::Error> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM schema_versions")
        .fetch_one(pool)
        .await?;
    if row.0 > 0 {
        return Ok(()); // Already has migration history
    }

    // Check if tables from V001 already exist
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='accounts'")
            .fetch_one(pool)
            .await?;

    if row.0 == 1 {
        // Pre-existing database — record V001 as already applied
        let migrations = all_migrations();
        let v001 = &migrations[0];
        sqlx::query("INSERT INTO schema_versions (version, name, checksum) VALUES (?, ?, ?)")
            .bind(v001.version)
            .bind(v001.name)
            .bind(checksum(v001.up_sql))
            .execute(pool)
            .await?;
    }

    Ok(())
}

/// Run all pending migrations. Returns the number of migrations applied.
///
/// Handles pre-existing databases (created before migration system) by detecting
/// existing tables and marking V001 as already applied.
pub async fn run_migrations(pool: &DbPool) -> Result<usize, sqlx::Error> {
    ensure_version_table(pool).await?;
    bootstrap_existing_db(pool).await?;

    let applied: Vec<SchemaVersion> = sqlx::query_as(
        "SELECT version, name, applied_at, checksum FROM schema_versions ORDER BY version",
    )
    .fetch_all(pool)
    .await?;

    let applied_versions: std::collections::HashSet<i64> =
        applied.iter().map(|v| v.version).collect();

    let migrations = all_migrations();
    let mut count = 0;

    for migration in &migrations {
        if applied_versions.contains(&migration.version) {
            // Verify checksum matches
            if let Some(existing) = applied.iter().find(|v| v.version == migration.version) {
                let expected = checksum(migration.up_sql);
                if existing.checksum != expected {
                    return Err(sqlx::Error::Protocol(format!(
                        "Migration V{:03} ({}) checksum mismatch: expected {}, found {}. \
                         The migration file has been modified after it was applied.",
                        migration.version, migration.name, expected, existing.checksum
                    )));
                }
            }
            continue;
        }

        // Execute each statement in the migration
        for statement in split_statements(migration.up_sql) {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed).execute(pool).await?;
            }
        }

        // Record the migration
        sqlx::query("INSERT INTO schema_versions (version, name, checksum) VALUES (?, ?, ?)")
            .bind(migration.version)
            .bind(migration.name)
            .bind(checksum(migration.up_sql))
            .execute(pool)
            .await?;

        count += 1;
    }

    Ok(count)
}

/// Roll back the most recently applied migration.
/// Returns the version that was rolled back, or None if no migrations to roll back.
pub async fn rollback_last(pool: &DbPool) -> Result<Option<i64>, sqlx::Error> {
    ensure_version_table(pool).await?;

    let latest: Option<SchemaVersion> = sqlx::query_as(
        "SELECT version, name, applied_at, checksum FROM schema_versions ORDER BY version DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;

    let Some(latest) = latest else {
        return Ok(None);
    };

    let migrations = all_migrations();
    let migration = migrations
        .iter()
        .find(|m| m.version == latest.version)
        .ok_or_else(|| {
            sqlx::Error::Protocol(format!(
                "No down migration found for V{:03}",
                latest.version
            ))
        })?;

    for statement in split_statements(migration.down_sql) {
        let trimmed = statement.trim();
        if !trimmed.is_empty() {
            sqlx::query(trimmed).execute(pool).await?;
        }
    }

    sqlx::query("DELETE FROM schema_versions WHERE version = ?")
        .bind(latest.version)
        .execute(pool)
        .await?;

    Ok(Some(latest.version))
}

/// Get all applied schema versions.
pub async fn get_schema_versions(pool: &DbPool) -> Result<Vec<SchemaVersion>, sqlx::Error> {
    ensure_version_table(pool).await?;
    sqlx::query_as(
        "SELECT version, name, applied_at, checksum FROM schema_versions ORDER BY version",
    )
    .fetch_all(pool)
    .await
}

/// Get the current schema version number, or 0 if no migrations applied.
pub async fn current_version(pool: &DbPool) -> Result<i64, sqlx::Error> {
    ensure_version_table(pool).await?;
    let row: Option<(i64,)> = sqlx::query_as("SELECT MAX(version) FROM schema_versions")
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.0).unwrap_or(0))
}

/// Split SQL text into individual statements on semicolons.
/// Handles comments and avoids splitting inside string literals.
fn split_statements(sql: &str) -> Vec<&str> {
    let mut statements = Vec::new();
    let mut start = 0;
    let bytes = sql.as_bytes();
    let mut i = 0;
    let len = bytes.len();

    while i < len {
        match bytes[i] {
            b'\'' => {
                // Skip string literal
                i += 1;
                while i < len {
                    if bytes[i] == b'\'' {
                        if i + 1 < len && bytes[i + 1] == b'\'' {
                            i += 2; // escaped quote
                        } else {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                i += 1;
            }
            b'-' if i + 1 < len && bytes[i + 1] == b'-' => {
                // Skip line comment
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b';' => {
                let stmt = &sql[start..i];
                if !stmt.trim().is_empty() {
                    statements.push(stmt);
                }
                start = i + 1;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    // Trailing statement without semicolon
    let trailing = &sql[start..];
    if !trailing.trim().is_empty() {
        statements.push(trailing);
    }

    statements
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> DbPool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn migrations_apply_on_fresh_db() {
        let pool = test_pool().await;
        let count = run_migrations(&pool).await.unwrap();
        assert_eq!(count, 1, "Should apply 1 migration on fresh db");

        // Verify tables exist
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='accounts'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, 1);
    }

    #[tokio::test]
    async fn migrations_idempotent() {
        let pool = test_pool().await;
        let count1 = run_migrations(&pool).await.unwrap();
        assert_eq!(count1, 1);

        let count2 = run_migrations(&pool).await.unwrap();
        assert_eq!(count2, 0, "Should skip already-applied migrations");
    }

    #[tokio::test]
    async fn version_tracking() {
        let pool = test_pool().await;
        run_migrations(&pool).await.unwrap();

        let ver = current_version(&pool).await.unwrap();
        assert_eq!(ver, 1);

        let versions = get_schema_versions(&pool).await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].name, "initial_schema");
    }

    #[tokio::test]
    async fn rollback_removes_tables() {
        let pool = test_pool().await;
        run_migrations(&pool).await.unwrap();

        let rolled = rollback_last(&pool).await.unwrap();
        assert_eq!(rolled, Some(1));

        // accounts table should be gone
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='accounts'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, 0);

        // Version should be 0
        let ver = current_version(&pool).await.unwrap();
        assert_eq!(ver, 0);
    }

    #[tokio::test]
    async fn rollback_then_reapply() {
        let pool = test_pool().await;
        run_migrations(&pool).await.unwrap();
        rollback_last(&pool).await.unwrap();

        let count = run_migrations(&pool).await.unwrap();
        assert_eq!(count, 1, "Should re-apply after rollback");

        let ver = current_version(&pool).await.unwrap();
        assert_eq!(ver, 1);
    }

    #[tokio::test]
    async fn rollback_empty_db() {
        let pool = test_pool().await;
        let rolled = rollback_last(&pool).await.unwrap();
        assert_eq!(rolled, None);
    }

    #[tokio::test]
    async fn all_17_tables_created() {
        let pool = test_pool().await;
        run_migrations(&pool).await.unwrap();

        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type='table' AND name != 'schema_versions' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        let names: Vec<&str> = tables.iter().map(|t| t.0.as_str()).collect();
        assert!(names.contains(&"accounts"));
        assert!(names.contains(&"transactions"));
        assert!(names.contains(&"transaction_lines"));
        assert!(names.contains(&"settings"));
        assert!(names.contains(&"fiscal_periods"));
        assert!(names.contains(&"import_profiles"));
        assert!(names.contains(&"imported_transactions"));
        assert!(names.contains(&"categorization_rules"));
        assert!(names.contains(&"reconciliation_sessions"));
        assert!(names.contains(&"reconciliation_items"));
        assert!(names.contains(&"receipts"));
        assert!(names.contains(&"receipt_line_items"));
        assert!(names.contains(&"contacts"));
        assert!(names.contains(&"invoices"));
        assert!(names.contains(&"invoice_lines"));
        assert!(names.contains(&"invoice_tax_lines"));
        assert!(names.contains(&"payments"));
        assert!(names.contains(&"audit_log"));
        assert!(names.contains(&"tax_periods"));
        // 19 domain tables + sqlite_sequence (from AUTOINCREMENT)
        assert_eq!(
            names.len(),
            20,
            "Should have 20 tables (19 domain + sqlite_sequence)"
        );
    }

    #[test]
    fn split_statements_basic() {
        let sql = "CREATE TABLE a (id INT); CREATE TABLE b (id INT);";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 2);
    }

    #[test]
    fn split_statements_with_comments() {
        let sql = "-- comment\nCREATE TABLE a (id INT);\n-- another\nCREATE TABLE b (id INT);";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 2);
    }

    #[test]
    fn split_statements_with_string_semicolons() {
        let sql = "INSERT INTO t (v) VALUES ('hello; world');";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 1);
        assert!(stmts[0].contains("hello; world"));
    }

    #[test]
    fn split_statements_trailing_no_semicolon() {
        let sql = "CREATE TABLE a (id INT)";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 1);
    }

    #[tokio::test]
    async fn bootstrap_existing_database() {
        let pool = test_pool().await;

        // Simulate a pre-migration database: create accounts table directly
        sqlx::query("CREATE TABLE accounts (id INTEGER PRIMARY KEY, code TEXT NOT NULL UNIQUE, name TEXT NOT NULL, account_type TEXT NOT NULL, is_archetype INTEGER NOT NULL DEFAULT 0, is_archived INTEGER NOT NULL DEFAULT 0, schedule_c_line TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')))")
            .execute(&pool)
            .await
            .unwrap();

        // Run migrations — should detect existing tables and skip V001
        let count = run_migrations(&pool).await.unwrap();
        assert_eq!(count, 0, "Should not re-apply V001 on existing database");

        let ver = current_version(&pool).await.unwrap();
        assert_eq!(ver, 1, "Should mark V001 as applied via bootstrap");
    }

    #[test]
    fn all_migrations_ordered() {
        let migrations = all_migrations();
        for (i, m) in migrations.iter().enumerate() {
            assert_eq!(m.version, (i + 1) as i64, "Migrations must be sequential");
        }
    }

    #[test]
    fn checksum_deterministic() {
        let sql = "CREATE TABLE test (id INT);";
        assert_eq!(checksum(sql), checksum(sql));
    }
}
