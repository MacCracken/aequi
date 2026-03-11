use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Money;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InvoiceStatus {
    Draft,
    Sent {
        sent_at: DateTime<Utc>,
    },
    Viewed {
        first_viewed_at: DateTime<Utc>,
    },
    PartiallyPaid {
        paid_amount: Money,
        last_payment_at: DateTime<Utc>,
    },
    Paid {
        paid_at: DateTime<Utc>,
    },
    Void {
        voided_at: DateTime<Utc>,
        reason: String,
    },
}

#[derive(Debug, Clone, Error)]
pub enum InvoiceError {
    #[error("Invalid status transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },
    #[error("Invoice not found: {0}")]
    NotFound(i64),
    #[error("Payment exceeds amount due")]
    OverPayment,
    #[error("Invoice has no lines")]
    EmptyInvoice,
}

impl InvoiceStatus {
    pub fn label(&self) -> &'static str {
        match self {
            InvoiceStatus::Draft => "Draft",
            InvoiceStatus::Sent { .. } => "Sent",
            InvoiceStatus::Viewed { .. } => "Viewed",
            InvoiceStatus::PartiallyPaid { .. } => "Partially Paid",
            InvoiceStatus::Paid { .. } => "Paid",
            InvoiceStatus::Void { .. } => "Void",
        }
    }

    /// Whether this invoice is overdue (computed, not stored).
    pub fn is_overdue(&self, due_date: NaiveDate, today: NaiveDate) -> bool {
        match self {
            InvoiceStatus::Sent { .. }
            | InvoiceStatus::Viewed { .. }
            | InvoiceStatus::PartiallyPaid { .. } => today > due_date,
            _ => false,
        }
    }

    pub fn can_transition_to(&self, target: &InvoiceStatus) -> bool {
        matches!(
            (self, target),
            (InvoiceStatus::Draft, InvoiceStatus::Sent { .. })
                | (InvoiceStatus::Draft, InvoiceStatus::Void { .. })
                | (InvoiceStatus::Sent { .. }, InvoiceStatus::Viewed { .. })
                | (
                    InvoiceStatus::Sent { .. },
                    InvoiceStatus::PartiallyPaid { .. }
                )
                | (InvoiceStatus::Sent { .. }, InvoiceStatus::Paid { .. })
                | (InvoiceStatus::Sent { .. }, InvoiceStatus::Void { .. })
                | (
                    InvoiceStatus::Viewed { .. },
                    InvoiceStatus::PartiallyPaid { .. }
                )
                | (InvoiceStatus::Viewed { .. }, InvoiceStatus::Paid { .. })
                | (InvoiceStatus::Viewed { .. }, InvoiceStatus::Void { .. })
                | (
                    InvoiceStatus::PartiallyPaid { .. },
                    InvoiceStatus::Paid { .. }
                )
                | (
                    InvoiceStatus::PartiallyPaid { .. },
                    InvoiceStatus::PartiallyPaid { .. }
                )
                | (
                    InvoiceStatus::PartiallyPaid { .. },
                    InvoiceStatus::Void { .. }
                )
        )
    }

    pub fn transition(self, target: InvoiceStatus) -> Result<InvoiceStatus, InvoiceError> {
        if self.can_transition_to(&target) {
            Ok(target)
        } else {
            Err(InvoiceError::InvalidTransition {
                from: self.label().to_string(),
                to: target.label().to_string(),
            })
        }
    }
}

impl std::fmt::Display for InvoiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn draft_to_sent() {
        let status = InvoiceStatus::Draft;
        let result = status.transition(InvoiceStatus::Sent { sent_at: now() });
        assert!(result.is_ok());
    }

    #[test]
    fn draft_to_void() {
        let status = InvoiceStatus::Draft;
        let result = status.transition(InvoiceStatus::Void {
            voided_at: now(),
            reason: "Cancelled".to_string(),
        });
        assert!(result.is_ok());
    }

    #[test]
    fn draft_to_paid_rejected() {
        let status = InvoiceStatus::Draft;
        let result = status.transition(InvoiceStatus::Paid { paid_at: now() });
        assert!(result.is_err());
    }

    #[test]
    fn sent_to_viewed() {
        let status = InvoiceStatus::Sent { sent_at: now() };
        let result = status.transition(InvoiceStatus::Viewed {
            first_viewed_at: now(),
        });
        assert!(result.is_ok());
    }

    #[test]
    fn sent_to_paid() {
        let status = InvoiceStatus::Sent { sent_at: now() };
        let result = status.transition(InvoiceStatus::Paid { paid_at: now() });
        assert!(result.is_ok());
    }

    #[test]
    fn sent_to_partially_paid() {
        let status = InvoiceStatus::Sent { sent_at: now() };
        let result = status.transition(InvoiceStatus::PartiallyPaid {
            paid_amount: Money::from_cents(5000),
            last_payment_at: now(),
        });
        assert!(result.is_ok());
    }

    #[test]
    fn viewed_to_paid() {
        let status = InvoiceStatus::Viewed {
            first_viewed_at: now(),
        };
        let result = status.transition(InvoiceStatus::Paid { paid_at: now() });
        assert!(result.is_ok());
    }

    #[test]
    fn partially_paid_to_paid() {
        let status = InvoiceStatus::PartiallyPaid {
            paid_amount: Money::from_cents(5000),
            last_payment_at: now(),
        };
        let result = status.transition(InvoiceStatus::Paid { paid_at: now() });
        assert!(result.is_ok());
    }

    #[test]
    fn partially_paid_to_partially_paid() {
        let status = InvoiceStatus::PartiallyPaid {
            paid_amount: Money::from_cents(5000),
            last_payment_at: now(),
        };
        let result = status.transition(InvoiceStatus::PartiallyPaid {
            paid_amount: Money::from_cents(8000),
            last_payment_at: now(),
        });
        assert!(result.is_ok());
    }

    #[test]
    fn paid_to_anything_rejected() {
        let status = InvoiceStatus::Paid { paid_at: now() };
        assert!(status.clone().transition(InvoiceStatus::Draft).is_err());
        assert!(status
            .clone()
            .transition(InvoiceStatus::Sent { sent_at: now() })
            .is_err());
        assert!(status
            .transition(InvoiceStatus::Void {
                voided_at: now(),
                reason: "test".to_string(),
            })
            .is_err());
    }

    #[test]
    fn void_to_anything_rejected() {
        let status = InvoiceStatus::Void {
            voided_at: now(),
            reason: "test".to_string(),
        };
        assert!(status.clone().transition(InvoiceStatus::Draft).is_err());
        assert!(status
            .transition(InvoiceStatus::Sent { sent_at: now() })
            .is_err());
    }

    #[test]
    fn is_overdue() {
        let due = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 3, 5).unwrap();

        let sent = InvoiceStatus::Sent { sent_at: now() };
        assert!(sent.is_overdue(due, today));

        let draft = InvoiceStatus::Draft;
        assert!(!draft.is_overdue(due, today));

        let paid = InvoiceStatus::Paid { paid_at: now() };
        assert!(!paid.is_overdue(due, today));
    }

    #[test]
    fn not_overdue_before_due_date() {
        let due = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();

        let sent = InvoiceStatus::Sent { sent_at: now() };
        assert!(!sent.is_overdue(due, today));
    }

    #[test]
    fn status_labels() {
        assert_eq!(InvoiceStatus::Draft.label(), "Draft");
        assert_eq!(InvoiceStatus::Sent { sent_at: now() }.label(), "Sent");
        assert_eq!(InvoiceStatus::Paid { paid_at: now() }.label(), "Paid");
    }

    #[test]
    fn status_display() {
        assert_eq!(InvoiceStatus::Draft.to_string(), "Draft");
    }
}
