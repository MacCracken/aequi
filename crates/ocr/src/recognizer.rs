use thiserror::Error;

#[derive(Debug, Error)]
pub enum OcrError {
    #[error("Image decode error: {0}")]
    ImageDecode(String),
    #[error("OCR engine error: {0}")]
    Engine(String),
    #[error("Tesseract not available — build with `tesseract` feature")]
    NotAvailable,
}

/// Abstraction over an OCR backend.
/// Implementations accept raw PNG/JPEG image bytes and return the recognized text.
pub trait OcrBackend: Send + Sync {
    fn recognize(&self, image_bytes: &[u8]) -> Result<String, OcrError>;
}

// ── Mock backend (always available, used for tests) ───────────────────────────

/// Returns a pre-set string — useful for unit testing the extraction pipeline
/// without requiring Tesseract to be installed.
pub struct MockRecognizer {
    pub text: String,
}

impl MockRecognizer {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl OcrBackend for MockRecognizer {
    fn recognize(&self, _image_bytes: &[u8]) -> Result<String, OcrError> {
        Ok(self.text.clone())
    }
}

// ── Tesseract backend (optional, gated behind `tesseract` feature) ─────────────

#[cfg(feature = "tesseract")]
pub mod tesseract_backend {
    use super::{OcrBackend, OcrError};
    use leptess::LepTess;

    pub struct TesseractRecognizer {
        data_path: Option<String>,
        lang: String,
    }

    impl TesseractRecognizer {
        pub fn new(data_path: Option<String>, lang: &str) -> Self {
            Self { data_path, lang: lang.to_string() }
        }
    }

    impl OcrBackend for TesseractRecognizer {
        fn recognize(&self, image_bytes: &[u8]) -> Result<String, OcrError> {
            let mut lt = LepTess::new(self.data_path.as_deref(), &self.lang)
                .map_err(|e| OcrError::Engine(e.to_string()))?;
            lt.set_image_from_mem(image_bytes)
                .map_err(|e| OcrError::ImageDecode(e.to_string()))?;
            lt.get_utf8_text().map_err(|e| OcrError::Engine(e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_returns_preset_text() {
        let r = MockRecognizer::new("STARBUCKS\n$5.50\nVISA");
        assert_eq!(r.recognize(b"fake image data").unwrap(), "STARBUCKS\n$5.50\nVISA");
    }

    #[test]
    fn mock_ignores_image_content() {
        let r = MockRecognizer::new("hello");
        assert_eq!(r.recognize(b"anything").unwrap(), "hello");
        assert_eq!(r.recognize(b"").unwrap(), "hello");
    }
}
