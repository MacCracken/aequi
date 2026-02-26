pub mod extract;
pub mod hash;
pub mod pipeline;
pub mod preprocess;
pub mod recognizer;
pub mod types;

pub use extract::Extractor;
pub use hash::{sha256_bytes, sha256_file, to_hex};
pub use pipeline::{OcrResult, PipelineError, ReceiptPipeline};
pub use preprocess::{prepare_for_ocr, PreprocessError};
pub use recognizer::{MockRecognizer, OcrBackend, OcrError};
pub use types::{ExtractedField, ExtractedReceipt, LineItem, PaymentMethod, ReceiptStatus};
