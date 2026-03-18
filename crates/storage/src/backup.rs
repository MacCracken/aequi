//! Backup and restore for Aequi data.
//!
//! Creates a compressed tarball containing the SQLite database snapshot
//! and the attachments directory. Restore extracts to a target location.

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Metadata written into the backup archive as `manifest.json`.
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct BackupManifest {
    pub version: String,
    pub created_at: String,
    pub schema_version: i64,
    pub db_size_bytes: u64,
    pub attachment_count: u64,
}

/// Create a compressed backup archive at `output_path`.
///
/// The archive contains:
/// - `manifest.json` — backup metadata
/// - `ledger.db` — SQLite database snapshot (via VACUUM INTO)
/// - `attachments/` — all receipt attachments (if directory exists and is non-empty)
///
/// Uses SQLite's `VACUUM INTO` for a consistent point-in-time snapshot,
/// avoiding issues with WAL files and concurrent access.
pub async fn create_backup(
    pool: &crate::db::DbPool,
    _db_path: &Path,
    attachments_dir: &Path,
    output_path: &Path,
    app_version: &str,
) -> Result<BackupManifest, BackupError> {
    // Create a clean snapshot of the database
    let temp_dir = output_path.parent().unwrap_or(Path::new("."));
    let snapshot_path = temp_dir.join(".aequi-backup-snapshot.db");

    // Clean up any previous failed snapshot
    let _ = fs::remove_file(&snapshot_path);

    // VACUUM INTO creates an atomic, consistent copy.
    // Canonicalize and validate the path to prevent injection via crafted filenames.
    let canonical = snapshot_path
        .canonicalize()
        .or_else(|_| {
            // File doesn't exist yet; canonicalize parent and append filename
            let parent = snapshot_path
                .parent()
                .unwrap_or(Path::new("."))
                .canonicalize()
                .map_err(|e| BackupError::Io(e.to_string()))?;
            Ok::<PathBuf, BackupError>(parent.join(snapshot_path.file_name().unwrap_or_default()))
        })?;
    let path_str = canonical.to_string_lossy();
    // Reject paths containing single quotes to prevent SQL injection
    if path_str.contains('\'') {
        return Err(BackupError::Io("Backup path must not contain single quotes".into()));
    }
    sqlx::query(&format!("VACUUM INTO '{path_str}'"))
        .execute(pool)
        .await
        .map_err(|e| BackupError::Database(e.to_string()))?;

    let db_size = fs::metadata(&snapshot_path).map(|m| m.len()).unwrap_or(0);

    // Count attachments
    let attachment_count = count_files(attachments_dir);

    // Get schema version
    let schema_version = crate::migrate::current_version(pool).await.unwrap_or(0);

    let manifest = BackupManifest {
        version: app_version.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        schema_version,
        db_size_bytes: db_size,
        attachment_count,
    };

    // Build the tar.gz archive
    let output_file = fs::File::create(output_path)
        .map_err(|e| BackupError::Io(format!("Failed to create backup file: {e}")))?;
    let encoder = GzEncoder::new(output_file, Compression::default());
    let mut archive = tar::Builder::new(encoder);

    // Add manifest
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| BackupError::Io(format!("Failed to serialize manifest: {e}")))?;
    let manifest_bytes = manifest_json.as_bytes();
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest_bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive
        .append_data(&mut header, "manifest.json", manifest_bytes)
        .map_err(|e| BackupError::Io(format!("Failed to add manifest: {e}")))?;

    // Add database snapshot
    archive
        .append_path_with_name(&snapshot_path, "ledger.db")
        .map_err(|e| BackupError::Io(format!("Failed to add database: {e}")))?;

    // Add attachments directory
    if attachments_dir.is_dir() && attachment_count > 0 {
        archive
            .append_dir_all("attachments", attachments_dir)
            .map_err(|e| BackupError::Io(format!("Failed to add attachments: {e}")))?;
    }

    archive
        .finish()
        .map_err(|e| BackupError::Io(format!("Failed to finalize archive: {e}")))?;

    // Clean up snapshot
    let _ = fs::remove_file(&snapshot_path);

    Ok(manifest)
}

/// Restore from a backup archive.
///
/// Extracts the database and attachments to `target_dir`.
/// Returns the path to the restored database file.
///
/// **Warning**: This overwrites existing data in `target_dir`.
pub fn restore_backup(
    archive_path: &Path,
    target_dir: &Path,
) -> Result<RestoreResult, BackupError> {
    let file = fs::File::open(archive_path)
        .map_err(|e| BackupError::Io(format!("Failed to open backup: {e}")))?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    fs::create_dir_all(target_dir)
        .map_err(|e| BackupError::Io(format!("Failed to create target directory: {e}")))?;

    let mut manifest: Option<BackupManifest> = None;

    for entry in archive
        .entries()
        .map_err(|e| BackupError::Io(format!("Failed to read archive: {e}")))?
    {
        let mut entry = entry.map_err(|e| BackupError::Io(format!("Failed to read entry: {e}")))?;

        let path = entry
            .path()
            .map_err(|e| BackupError::Io(format!("Invalid path in archive: {e}")))?
            .to_path_buf();

        // Security: reject absolute paths and path traversal
        if path.is_absolute()
            || path
                .components()
                .any(|c| c == std::path::Component::ParentDir)
        {
            return Err(BackupError::InvalidArchive(
                "Archive contains path traversal".to_string(),
            ));
        }

        if path == Path::new("manifest.json") {
            let mut buf = String::new();
            io::Read::read_to_string(&mut entry, &mut buf)
                .map_err(|e| BackupError::Io(format!("Failed to read manifest: {e}")))?;
            manifest = Some(
                serde_json::from_str(&buf)
                    .map_err(|e| BackupError::InvalidArchive(format!("Invalid manifest: {e}")))?,
            );
        } else {
            let target = target_dir.join(&path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| BackupError::Io(format!("Failed to create dir: {e}")))?;
            }
            entry.unpack(&target).map_err(|e| {
                BackupError::Io(format!("Failed to extract {}: {e}", path.display()))
            })?;
        }
    }

    let manifest = manifest
        .ok_or_else(|| BackupError::InvalidArchive("Archive missing manifest.json".to_string()))?;

    Ok(RestoreResult {
        db_path: target_dir.join("ledger.db"),
        attachments_dir: target_dir.join("attachments"),
        manifest,
    })
}

/// Result of a successful restore operation.
pub struct RestoreResult {
    pub db_path: PathBuf,
    pub attachments_dir: PathBuf,
    pub manifest: BackupManifest,
}

/// Count files recursively in a directory.
fn count_files(dir: &Path) -> u64 {
    if !dir.is_dir() {
        return 0;
    }
    walkdir(dir)
}

fn walkdir(dir: &Path) -> u64 {
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                count += 1;
            } else if path.is_dir() {
                count += walkdir(&path);
            }
        }
    }
    count
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("Database error: {0}")]
    Database(String),
    #[error("IO error: {0}")]
    Io(String),
    #[error("Invalid archive: {0}")]
    InvalidArchive(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool_at(path: &Path) -> crate::db::DbPool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite:{}?mode=rwc", path.display()))
            .await
            .unwrap();
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&pool)
            .await
            .unwrap();
        crate::migrate::run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn backup_creates_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("ledger.db");
        let attachments = tmp.path().join("attachments");
        fs::create_dir_all(&attachments).unwrap();
        let output = tmp.path().join("backup.tar.gz");

        let pool = test_pool_at(&db_path).await;

        // Insert test data
        sqlx::query("INSERT OR IGNORE INTO accounts (code, name, account_type) VALUES ('1000', 'Cash', 'Asset')")
            .execute(&pool)
            .await
            .unwrap();

        let manifest = create_backup(&pool, &db_path, &attachments, &output, "2026.3.10")
            .await
            .unwrap();

        assert!(output.exists());
        assert_eq!(manifest.version, "2026.3.10");
        assert!(manifest.schema_version >= 1);
        assert!(manifest.db_size_bytes > 0);
    }

    #[tokio::test]
    async fn backup_and_restore_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("ledger.db");
        let attachments = tmp.path().join("attachments");
        fs::create_dir_all(&attachments).unwrap();

        // Create a test attachment
        fs::write(attachments.join("test.jpg"), b"fake image data").unwrap();

        let pool = test_pool_at(&db_path).await;
        sqlx::query("INSERT OR IGNORE INTO accounts (code, name, account_type) VALUES ('2000', 'Revenue', 'Income')")
            .execute(&pool)
            .await
            .unwrap();

        let output = tmp.path().join("backup.tar.gz");
        create_backup(&pool, &db_path, &attachments, &output, "2026.3.10")
            .await
            .unwrap();

        // Restore to a new location
        let restore_dir = tmp.path().join("restored");
        let result = restore_backup(&output, &restore_dir).unwrap();

        assert!(result.db_path.exists());
        assert_eq!(result.manifest.version, "2026.3.10");
        assert_eq!(result.manifest.attachment_count, 1);

        // Verify the restored database has the data
        let restored_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite:{}?mode=rwc", result.db_path.display()))
            .await
            .unwrap();
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM accounts WHERE code = '2000'")
            .fetch_one(&restored_pool)
            .await
            .unwrap();
        assert_eq!(row.0, 1);

        // Verify attachment was restored
        assert!(result.attachments_dir.join("test.jpg").exists());
    }

    #[tokio::test]
    async fn backup_empty_attachments() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("ledger.db");
        let attachments = tmp.path().join("attachments");
        fs::create_dir_all(&attachments).unwrap();
        let output = tmp.path().join("backup.tar.gz");

        let pool = test_pool_at(&db_path).await;
        let manifest = create_backup(&pool, &db_path, &attachments, &output, "2026.3.10")
            .await
            .unwrap();

        assert_eq!(manifest.attachment_count, 0);
        assert!(output.exists());
    }

    #[test]
    fn restore_rejects_invalid_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let bad_file = tmp.path().join("bad.tar.gz");
        fs::write(&bad_file, b"not a valid archive").unwrap();

        let result = restore_backup(&bad_file, &tmp.path().join("out"));
        assert!(result.is_err());
    }

    #[test]
    fn count_files_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(count_files(tmp.path()), 0);
    }

    #[test]
    fn count_files_with_nested() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("sub")).unwrap();
        fs::write(tmp.path().join("a.txt"), b"a").unwrap();
        fs::write(tmp.path().join("sub/b.txt"), b"b").unwrap();
        assert_eq!(count_files(tmp.path()), 2);
    }

    #[test]
    fn count_files_nonexistent() {
        assert_eq!(count_files(Path::new("/nonexistent/path")), 0);
    }
}
