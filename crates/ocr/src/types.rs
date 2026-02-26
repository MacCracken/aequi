use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// A single extracted value with an associated confidence score (0.0–1.0).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtractedField<T> {
    pub value: T,
    /// Confidence in this extraction (0.0 = guessed, 1.0 = certain).
    pub confidence: f32,
}

impl<T> ExtractedField<T> {
    pub fn new(value: T, confidence: f32) -> Self {
        Self { value, confidence: confidence.clamp(0.0, 1.0) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PaymentMethod {
    Visa,
    Mastercard,
    Amex,
    Discover,
    Cash,
    Debit,
    Check,
    Other(String),
}

impl std::fmt::Display for PaymentMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaymentMethod::Visa => write!(f, "Visa"),
            PaymentMethod::Mastercard => write!(f, "Mastercard"),
            PaymentMethod::Amex => write!(f, "Amex"),
            PaymentMethod::Discover => write!(f, "Discover"),
            PaymentMethod::Cash => write!(f, "Cash"),
            PaymentMethod::Debit => write!(f, "Debit"),
            PaymentMethod::Check => write!(f, "Check"),
            PaymentMethod::Other(s) => write!(f, "{s}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    PendingReview,
    Approved,
    Rejected,
    Duplicate,
}

impl std::fmt::Display for ReceiptStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReceiptStatus::PendingReview => write!(f, "pending_review"),
            ReceiptStatus::Approved => write!(f, "approved"),
            ReceiptStatus::Rejected => write!(f, "rejected"),
            ReceiptStatus::Duplicate => write!(f, "duplicate"),
        }
    }
}

impl std::str::FromStr for ReceiptStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending_review" => Ok(ReceiptStatus::PendingReview),
            "approved" => Ok(ReceiptStatus::Approved),
            "rejected" => Ok(ReceiptStatus::Rejected),
            "duplicate" => Ok(ReceiptStatus::Duplicate),
            other => Err(format!("Unknown receipt status: '{other}'")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineItem {
    pub description: String,
    pub amount_cents: Option<i64>,
    pub quantity: Option<f32>,
}

/// The fully extracted, confidence-annotated representation of a receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedReceipt {
    pub vendor: Option<ExtractedField<String>>,
    pub date: Option<ExtractedField<NaiveDate>>,
    /// Amount before tax (cents).
    pub subtotal_cents: Option<ExtractedField<i64>>,
    /// Tax amount (cents).
    pub tax_cents: Option<ExtractedField<i64>>,
    /// Grand total (cents) — the primary field for transaction creation.
    pub total_cents: Option<ExtractedField<i64>>,
    pub payment_method: Option<ExtractedField<PaymentMethod>>,
    pub line_items: Vec<LineItem>,
    /// Aggregate confidence across all extracted fields (0.0–1.0).
    pub confidence: f32,
}

impl ExtractedReceipt {
    /// Whether the extraction is good enough to auto-suggest without human review.
    /// Threshold mirrors the formulation spec (0.7).
    pub fn needs_review(&self) -> bool {
        self.confidence < 0.7
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracted_field_clamps_confidence() {
        let f = ExtractedField::new("test", 1.5);
        assert_eq!(f.confidence, 1.0);
        let f = ExtractedField::new("test", -0.1);
        assert_eq!(f.confidence, 0.0);
    }

    #[test]
    fn payment_method_display() {
        assert_eq!(PaymentMethod::Visa.to_string(), "Visa");
        assert_eq!(PaymentMethod::Other("Zelle".into()).to_string(), "Zelle");
    }

    #[test]
    fn receipt_status_roundtrip() {
        use std::str::FromStr;
        assert_eq!(
            ReceiptStatus::from_str(&ReceiptStatus::PendingReview.to_string()).unwrap(),
            ReceiptStatus::PendingReview
        );
        assert_eq!(
            ReceiptStatus::from_str(&ReceiptStatus::Approved.to_string()).unwrap(),
            ReceiptStatus::Approved
        );
    }

    #[test]
    fn needs_review_threshold() {
        let low = ExtractedReceipt {
            vendor: None,
            date: None,
            subtotal_cents: None,
            tax_cents: None,
            total_cents: None,
            payment_method: None,
            line_items: vec![],
            confidence: 0.5,
        };
        assert!(low.needs_review());

        let high = ExtractedReceipt { confidence: 0.9, ..low };
        assert!(!high.needs_review());
    }
}
