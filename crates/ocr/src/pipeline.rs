use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant, SystemTime};
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;

use crate::extract::Extractor;
use crate::hash;
use crate::preprocess;
use crate::recognizer::{OcrBackend, OcrError};
use crate::types::ExtractedReceipt;

/// Largest receipt file the pipeline will read.
pub const MAX_RECEIPT_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Not a regular file: {}", .0.display())]
    NotAFile(PathBuf),
    #[error("File too large ({:.1} MB, max {} MB)", *.size as f64 / 1_048_576.0, MAX_RECEIPT_BYTES / 1_048_576)]
    TooLarge { size: u64 },
    #[error("Image preprocessing failed: {0}")]
    Preprocess(#[from] crate::preprocess::PreprocessError),
    #[error("OCR recognition failed: {0}")]
    Ocr(#[from] OcrError),
    #[error("Receipt processing task failed: {0}")]
    Task(String),
}

/// The result of a single receipt processing run.
#[derive(Debug)]
pub struct OcrResult {
    /// SHA-256 hex digest of the original file — used as the content-addressed key.
    pub hash_hex: String,
    /// Where the original file was stored in the attachments tree.
    pub attachment_path: PathBuf,
    /// Extension the attachment was stored with, matching its content.
    pub file_ext: String,
    /// Raw OCR text output.
    pub ocr_text: String,
    /// Structured fields extracted from the OCR text.
    pub extracted: ExtractedReceipt,
}

/// Orchestrates: hash → preprocess → OCR → extract → content-store.
pub struct ReceiptPipeline<R: OcrBackend> {
    recognizer: Arc<R>,
    attachments_dir: PathBuf,
}

impl<R: OcrBackend + 'static> ReceiptPipeline<R> {
    pub fn new(recognizer: R, attachments_dir: PathBuf) -> Self {
        Self {
            recognizer: Arc::new(recognizer),
            attachments_dir,
        }
    }

    /// Process a file on disk. Only regular files up to [`MAX_RECEIPT_BYTES`]
    /// are accepted; the size is checked before anything is read.
    pub async fn process_file(&self, path: &Path) -> Result<OcrResult, PipelineError> {
        let meta = tokio::fs::metadata(path).await?;
        if !meta.is_file() {
            return Err(PipelineError::NotAFile(path.to_path_buf()));
        }
        if meta.len() > MAX_RECEIPT_BYTES {
            return Err(PipelineError::TooLarge { size: meta.len() });
        }

        // Read at most one byte past the limit in case the file grew since the stat.
        let mut data = Vec::with_capacity(meta.len() as usize);
        tokio::fs::File::open(path)
            .await?
            .take(MAX_RECEIPT_BYTES + 1)
            .read_to_end(&mut data)
            .await?;
        if data.len() as u64 > MAX_RECEIPT_BYTES {
            return Err(PipelineError::TooLarge {
                size: data.len() as u64,
            });
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        self.process_owned(data, ext).await
    }

    /// Process raw bytes (from camera capture or file read). `ext` is a hint:
    /// the stored file takes the extension of the detected image format.
    pub async fn process_bytes(&self, data: &[u8], ext: &str) -> Result<OcrResult, PipelineError> {
        self.process_owned(data.to_vec(), ext).await
    }

    async fn process_owned(&self, data: Vec<u8>, ext: &str) -> Result<OcrResult, PipelineError> {
        let recognizer = Arc::clone(&self.recognizer);
        let ext_hint = ext.to_lowercase();

        // Hashing, decoding, resizing and OCR are CPU-bound, so keep them off
        // the async runtime's worker threads.
        let (data, hash_hex, file_ext, ocr_text, extracted) =
            tokio::task::spawn_blocking(move || -> Result<_, PipelineError> {
                // 1. Hash for deduplication / content addressing.
                let hash_hex = hash::to_hex(&hash::sha256_bytes(&data));

                // 2. Preprocess. This also verifies the file really is a
                //    decodable image before anything is stored.
                let image_bytes = preprocess::prepare_for_ocr_from_bytes(&data)?;
                let file_ext = stored_extension(&data, &ext_hint);

                // 3. Run OCR.
                let ocr_text = recognizer.recognize(&image_bytes)?;

                // 4. Extract structured fields.
                let extracted = Extractor::extract(&ocr_text);

                Ok((data, hash_hex, file_ext, ocr_text, extracted))
            })
            .await
            .map_err(|e| PipelineError::Task(e.to_string()))??;

        // 5. Persist to content-addressed store.
        let dest = hash::attachment_path(&self.attachments_dir, &hash_hex, &file_ext);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&dest, &data).await?;

        Ok(OcrResult {
            hash_hex,
            attachment_path: dest,
            file_ext,
            ocr_text,
            extracted,
        })
    }
}

/// The extension to store an image under: the caller's if it is valid for the
/// detected format (so "jpeg" stays "jpeg"), otherwise the format's canonical
/// one. Never derived from untrusted input alone.
fn stored_extension(data: &[u8], hint: &str) -> String {
    let Ok(format) = image::guess_format(data) else {
        return "bin".to_string();
    };
    let exts = format.extensions_str();
    if exts.contains(&hint) {
        hint.to_string()
    } else {
        exts.first().copied().unwrap_or("bin").to_string()
    }
}

// ── Watch-folder integration ──────────────────────────────────────────────────

/// How long a file's size and modification time must stay unchanged before
/// it is considered completely written.
const INTAKE_SETTLE: Duration = Duration::from_secs(1);
const INTAKE_POLL: Duration = Duration::from_millis(250);

/// Keeps an intake folder watch alive; dropping it stops watching.
pub struct IntakeWatcher {
    _watcher: notify::RecommendedWatcher,
}

/// Watch `watch_dir` and send each new receipt file's path to `tx` once it has
/// been completely written. Files already in the folder are sent on startup.
///
/// Handles files created in place, moved/renamed in, and written in several
/// chunks. A path is sent again if it changes after being sent, so the
/// receiver should move or delete files it has processed.
pub fn spawn_intake_watcher(
    watch_dir: &Path,
    tx: mpsc::Sender<PathBuf>,
) -> notify::Result<IntakeWatcher> {
    spawn_intake_watcher_with(watch_dir, tx, INTAKE_SETTLE)
}

fn spawn_intake_watcher_with(
    watch_dir: &Path,
    tx: mpsc::Sender<PathBuf>,
    settle: Duration,
) -> notify::Result<IntakeWatcher> {
    use notify::event::{AccessKind, AccessMode};
    use notify::{EventKind, RecursiveMode, Watcher};

    // Unbounded so the notify callback never blocks or drops an event.
    let (raw_tx, raw_rx) = std_mpsc::channel::<PathBuf>();

    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(ev) = event else { return };
        let relevant = matches!(
            ev.kind,
            EventKind::Create(_)
                | EventKind::Modify(_)
                | EventKind::Access(AccessKind::Close(AccessMode::Write))
        );
        if relevant {
            for path in ev.paths {
                // Fails only once the debounce thread has exited.
                let _ = raw_tx.send(path);
            }
        }
    })?;
    watcher.watch(watch_dir, RecursiveMode::NonRecursive)?;

    // Scan after the watch is in place so nothing slips between the two.
    let existing: Vec<PathBuf> = std::fs::read_dir(watch_dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();

    std::thread::Builder::new()
        .name("aequi-intake".into())
        .spawn(move || debounce_intake(existing, raw_rx, tx, settle))
        .map_err(notify::Error::io)?;

    Ok(IntakeWatcher { _watcher: watcher })
}

/// Size + modification time, used to tell when a file has stopped changing.
type FileSignature = (u64, Option<SystemTime>);

struct PendingFile {
    signature: Option<FileSignature>,
    stable_since: Instant,
}

/// Runs until the watcher is dropped or the receiver of `tx` goes away.
fn debounce_intake(
    initial: Vec<PathBuf>,
    raw_rx: std_mpsc::Receiver<PathBuf>,
    tx: mpsc::Sender<PathBuf>,
    settle: Duration,
) {
    let mut pending: HashMap<PathBuf, PendingFile> = HashMap::new();
    let touch = |pending: &mut HashMap<PathBuf, PendingFile>, path: PathBuf| {
        if is_hidden(&path) {
            return;
        }
        pending.insert(
            path,
            PendingFile {
                signature: None,
                stable_since: Instant::now(),
            },
        );
    };
    for path in initial {
        touch(&mut pending, path);
    }

    let mut last_check = Instant::now();
    loop {
        match raw_rx.recv_timeout(INTAKE_POLL) {
            Ok(path) => touch(&mut pending, path),
            Err(std_mpsc::RecvTimeoutError::Timeout) => {}
            Err(std_mpsc::RecvTimeoutError::Disconnected) => return,
        }
        if last_check.elapsed() < INTAKE_POLL {
            continue;
        }
        last_check = Instant::now();

        let mut ready = Vec::new();
        pending.retain(|path, state| {
            // Gone, or not a regular file: nothing to process.
            let Some(signature) = file_signature(path) else {
                return false;
            };
            if state.signature != Some(signature) {
                state.signature = Some(signature);
                state.stable_since = Instant::now();
                true
            } else if state.stable_since.elapsed() >= settle {
                ready.push(path.clone());
                false
            } else {
                true
            }
        });
        for path in ready {
            if tx.blocking_send(path).is_err() {
                return;
            }
        }
    }
}

fn file_signature(path: &Path) -> Option<FileSignature> {
    let meta = std::fs::metadata(path).ok()?;
    meta.is_file().then(|| (meta.len(), meta.modified().ok()))
}

/// Dotfiles are OS metadata (.DS_Store) or in-progress temp files.
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recognizer::MockRecognizer;
    use image::{DynamicImage, GrayImage, ImageBuffer, Luma};
    use std::io::Cursor;

    fn tiny_png() -> Vec<u8> {
        let img: GrayImage = ImageBuffer::from_fn(4, 4, |_, _| Luma([200u8]));
        let mut buf = Vec::new();
        DynamicImage::ImageLuma8(img)
            .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    #[tokio::test]
    async fn process_bytes_produces_ocr_result() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = ReceiptPipeline::new(
            MockRecognizer::new("STARBUCKS\n2024-01-15\nTotal $5.50\nVISA"),
            dir.path().to_path_buf(),
        );

        let result = pipeline.process_bytes(&tiny_png(), "png").await.unwrap();

        // Hash must be 64 hex chars.
        assert_eq!(result.hash_hex.len(), 64);
        // Attachment stored at expected path.
        assert!(result.attachment_path.exists());
        // Extraction worked.
        assert!(result.extracted.total_cents.is_some());
        assert_eq!(result.extracted.total_cents.unwrap().value, 550);
    }

    #[tokio::test]
    async fn process_bytes_dedup_path_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline =
            ReceiptPipeline::new(MockRecognizer::new("irrelevant"), dir.path().to_path_buf());
        let data = tiny_png();

        let r1 = pipeline.process_bytes(&data, "png").await.unwrap();
        let r2 = pipeline.process_bytes(&data, "png").await.unwrap();

        assert_eq!(r1.hash_hex, r2.hash_hex);
        assert_eq!(r1.attachment_path, r2.attachment_path);
    }

    #[tokio::test]
    async fn process_file_reads_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let attach_dir = tempfile::tempdir().unwrap();
        let pipeline = ReceiptPipeline::new(
            MockRecognizer::new("ACME CORP\n2026-01-10\nTotal $12.34"),
            attach_dir.path().to_path_buf(),
        );

        // Write a tiny PNG to disk
        let file_path = dir.path().join("receipt.png");
        let data = tiny_png();
        std::fs::write(&file_path, &data).unwrap();

        let result = pipeline.process_file(&file_path).await.unwrap();
        assert_eq!(result.hash_hex.len(), 64);
        assert!(result.attachment_path.exists());
        assert!(result.ocr_text.contains("ACME CORP"));
        assert_eq!(result.extracted.total_cents.unwrap().value, 1234);
    }

    #[tokio::test]
    async fn process_bytes_different_data_different_hash() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = ReceiptPipeline::new(MockRecognizer::new("text"), dir.path().to_path_buf());

        let img1 = tiny_png();

        // Create a slightly different image
        let img2 = {
            let img: GrayImage = ImageBuffer::from_fn(4, 4, |_, _| Luma([100u8]));
            let mut buf = Vec::new();
            DynamicImage::ImageLuma8(img)
                .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
                .unwrap();
            buf
        };

        let r1 = pipeline.process_bytes(&img1, "png").await.unwrap();
        let r2 = pipeline.process_bytes(&img2, "png").await.unwrap();

        assert_ne!(r1.hash_hex, r2.hash_hex);
        assert_ne!(r1.attachment_path, r2.attachment_path);
    }

    #[tokio::test]
    async fn process_bytes_extension_follows_content() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = ReceiptPipeline::new(MockRecognizer::new("text"), dir.path().to_path_buf());
        let data = tiny_png();

        let r1 = pipeline.process_bytes(&data, "PNG").await.unwrap();
        let r2 = pipeline.process_bytes(&data, "jpg").await.unwrap();
        let r3 = pipeline.process_bytes(&data, "html").await.unwrap();

        // A mislabeled file is stored under the extension of its real format.
        assert_eq!(r1.hash_hex, r2.hash_hex);
        assert_eq!(r1.attachment_path, r2.attachment_path);
        assert_eq!(r1.attachment_path, r3.attachment_path);
        assert_eq!(r1.file_ext, "png");
        assert!(r1.attachment_path.to_str().unwrap().ends_with(".png"));
    }

    #[tokio::test]
    async fn process_bytes_rejects_non_image_without_storing() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = ReceiptPipeline::new(MockRecognizer::new("text"), dir.path().to_path_buf());

        let err = pipeline
            .process_bytes(b"%PDF-1.7\nnot an image", "pdf")
            .await
            .unwrap_err();
        assert!(matches!(err, PipelineError::Preprocess(_)), "{err}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn process_file_rejects_directory() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = ReceiptPipeline::new(MockRecognizer::new("text"), dir.path().to_path_buf());

        let err = pipeline.process_file(dir.path()).await.unwrap_err();
        assert!(matches!(err, PipelineError::NotAFile(_)), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_file_rejects_device_file() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = ReceiptPipeline::new(MockRecognizer::new("text"), dir.path().to_path_buf());

        let err = pipeline
            .process_file(Path::new("/dev/zero"))
            .await
            .unwrap_err();
        assert!(matches!(err, PipelineError::NotAFile(_)), "{err}");
    }

    #[tokio::test]
    async fn process_file_rejects_oversized_file_without_reading() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = ReceiptPipeline::new(MockRecognizer::new("text"), dir.path().to_path_buf());

        // Sparse file: no data is written, only the length is set.
        let path = dir.path().join("huge.png");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_RECEIPT_BYTES + 1).unwrap();

        let err = pipeline.process_file(&path).await.unwrap_err();
        assert!(matches!(err, PipelineError::TooLarge { .. }), "{err}");
    }

    #[tokio::test]
    async fn process_bytes_creates_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = ReceiptPipeline::new(MockRecognizer::new("text"), dir.path().to_path_buf());
        let data = tiny_png();

        let result = pipeline.process_bytes(&data, "png").await.unwrap();

        // The attachment path should be in a subdirectory based on first 2 hex chars
        let parent = result.attachment_path.parent().unwrap();
        let subdir_name = parent.file_name().unwrap().to_str().unwrap();
        assert_eq!(subdir_name.len(), 2);
        assert_eq!(subdir_name, &result.hash_hex[..2]);
    }

    #[tokio::test]
    async fn process_bytes_stores_file_content() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = ReceiptPipeline::new(MockRecognizer::new("text"), dir.path().to_path_buf());
        let data = tiny_png();

        let result = pipeline.process_bytes(&data, "png").await.unwrap();

        // Verify the stored file has the same content
        let stored = std::fs::read(&result.attachment_path).unwrap();
        assert_eq!(stored, data);
    }

    #[tokio::test]
    async fn process_bytes_extraction_fields() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = ReceiptPipeline::new(
            MockRecognizer::new("123 Main Street\nSTARBUCKS COFFEE\n2024-01-15\nTotal $5.50\nVISA"),
            dir.path().to_path_buf(),
        );

        let result = pipeline.process_bytes(&tiny_png(), "png").await.unwrap();

        assert!(result.extracted.vendor.is_some());
        assert_eq!(result.extracted.vendor.unwrap().value, "STARBUCKS COFFEE");
        assert!(result.extracted.date.is_some());
        assert!(result.extracted.payment_method.is_some());
    }

    #[tokio::test]
    async fn process_file_nonexistent_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = ReceiptPipeline::new(MockRecognizer::new("text"), dir.path().to_path_buf());

        let result = pipeline
            .process_file(Path::new("/nonexistent/file.png"))
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PipelineError::Io(_)));
    }

    // ── Intake watcher ───────────────────────────────────────────────────────

    const SETTLE: Duration = Duration::from_millis(300);

    async fn next_name(rx: &mut mpsc::Receiver<PathBuf>) -> Option<String> {
        tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .ok()
            .flatten()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
    }

    #[tokio::test]
    async fn watcher_sends_files_present_at_startup() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("old.png"), tiny_png()).unwrap();
        std::fs::write(dir.path().join(".DS_Store"), b"x").unwrap();

        let (tx, mut rx) = mpsc::channel(1);
        let _w = spawn_intake_watcher_with(dir.path(), tx, SETTLE).unwrap();

        assert_eq!(next_name(&mut rx).await.as_deref(), Some("old.png"));
    }

    #[tokio::test]
    async fn watcher_sends_new_and_moved_in_files() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::channel(1);
        let _w = spawn_intake_watcher_with(dir.path(), tx, SETTLE).unwrap();

        std::fs::write(dir.path().join("new.png"), tiny_png()).unwrap();
        assert_eq!(next_name(&mut rx).await.as_deref(), Some("new.png"));

        let staged = outside.path().join("moved.png");
        std::fs::write(&staged, tiny_png()).unwrap();
        std::fs::rename(&staged, dir.path().join("moved.png")).unwrap();
        assert_eq!(next_name(&mut rx).await.as_deref(), Some("moved.png"));
    }

    #[tokio::test]
    async fn watcher_waits_until_file_is_completely_written() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::channel(1);
        let _w = spawn_intake_watcher_with(dir.path(), tx, SETTLE).unwrap();

        let path = dir.path().join("slow.png");
        let data = tiny_png();
        let (head, tail) = data.split_at(data.len() / 2);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(head).unwrap();
        file.flush().unwrap();
        std::thread::sleep(SETTLE / 2);
        file.write_all(tail).unwrap();
        drop(file);

        assert_eq!(next_name(&mut rx).await.as_deref(), Some("slow.png"));
        assert_eq!(std::fs::read(&path).unwrap(), data);
        // Sent once, not once per write.
        assert!(
            tokio::time::timeout(SETTLE * 3, rx.recv()).await.is_err(),
            "file was sent twice"
        );
    }
}
