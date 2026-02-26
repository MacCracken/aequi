use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::extract::Extractor;
use crate::hash;
use crate::preprocess;
use crate::recognizer::{OcrBackend, OcrError};
use crate::types::ExtractedReceipt;

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image preprocessing failed: {0}")]
    Preprocess(#[from] crate::preprocess::PreprocessError),
    #[error("OCR recognition failed: {0}")]
    Ocr(#[from] OcrError),
}

/// The result of a single receipt processing run.
#[derive(Debug)]
pub struct OcrResult {
    /// SHA-256 hex digest of the original file — used as the content-addressed key.
    pub hash_hex: String,
    /// Where the original file was stored in the attachments tree.
    pub attachment_path: PathBuf,
    /// Raw OCR text output.
    pub ocr_text: String,
    /// Structured fields extracted from the OCR text.
    pub extracted: ExtractedReceipt,
}

/// Orchestrates: hash → dedup check → content-store → preprocess → OCR → extract.
pub struct ReceiptPipeline<R: OcrBackend> {
    recognizer: R,
    attachments_dir: PathBuf,
}

impl<R: OcrBackend> ReceiptPipeline<R> {
    pub fn new(recognizer: R, attachments_dir: PathBuf) -> Self {
        Self { recognizer, attachments_dir }
    }

    /// Process a file on disk.
    pub async fn process_file(&self, path: &Path) -> Result<OcrResult, PipelineError> {
        let bytes = tokio::fs::read(path).await?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin")
            .to_lowercase();
        self.process_bytes(&bytes, &ext).await
    }

    /// Process raw bytes (from camera capture or file read).
    pub async fn process_bytes(
        &self,
        data: &[u8],
        ext: &str,
    ) -> Result<OcrResult, PipelineError> {
        // 1. Hash for deduplication / content addressing.
        let hash = hash::sha256_bytes(data);
        let hash_hex = hash::to_hex(&hash);

        // 2. Persist to content-addressed store.
        let dest = hash::attachment_path(&self.attachments_dir, &hash_hex, ext);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&dest, data).await?;

        // 3. Preprocess image.
        let image_bytes = preprocess::prepare_for_ocr_from_bytes(data)?;

        // 4. Run OCR.
        let ocr_text = self.recognizer.recognize(&image_bytes)?;

        // 5. Extract structured fields.
        let extracted = Extractor::extract(&ocr_text);

        Ok(OcrResult {
            hash_hex,
            attachment_path: dest,
            ocr_text,
            extracted,
        })
    }
}

// ── Watch-folder integration ──────────────────────────────────────────────────

/// Spawn a notify watcher on `watch_dir` that sends new file paths to `tx`.
/// Returns the watcher — it must be kept alive for watching to continue.
pub fn spawn_intake_watcher(
    watch_dir: &Path,
    tx: mpsc::Sender<PathBuf>,
) -> notify::Result<impl notify::Watcher> {
    use notify::{EventKind, RecursiveMode, Watcher};

    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(ev) = event {
            if matches!(ev.kind, EventKind::Create(_)) {
                for path in ev.paths {
                    let _ = tx.try_send(path);
                }
            }
        }
    })?;

    watcher.watch(watch_dir, RecursiveMode::NonRecursive)?;
    Ok(watcher)
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
        let pipeline = ReceiptPipeline::new(
            MockRecognizer::new("irrelevant"),
            dir.path().to_path_buf(),
        );
        let data = tiny_png();

        let r1 = pipeline.process_bytes(&data, "png").await.unwrap();
        let r2 = pipeline.process_bytes(&data, "png").await.unwrap();

        assert_eq!(r1.hash_hex, r2.hash_hex);
        assert_eq!(r1.attachment_path, r2.attachment_path);
    }
}
