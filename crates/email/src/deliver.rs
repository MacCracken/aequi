use aequi_core::{Contact, Invoice};
use aequi_pdf::{render_invoice_pdf, render_invoice_text};
use lettre::message::{header::ContentType, Attachment, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::Serialize;

use crate::config::EmailConfig;

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    #[error("recipient has no email address")]
    NoRecipientEmail,
    #[error("PDF generation failed: {0}")]
    PdfError(String),
    #[error("failed to build email message: {0}")]
    MessageBuild(String),
    #[error("SMTP delivery failed: {0}")]
    Smtp(String),
    #[error("Resend API error: {0}")]
    Resend(String),
}

#[derive(Debug, Serialize)]
pub struct DeliveryResult {
    pub recipient: String,
    pub invoice_number: String,
    pub backend: String,
}

/// Send an invoice to the contact via email.
///
/// Attaches the invoice as a PDF and includes plain-text in the body.
pub async fn send_invoice(
    config: &EmailConfig,
    invoice: &Invoice,
    contact: &Contact,
    subject: Option<&str>,
) -> Result<DeliveryResult, DeliveryError> {
    let recipient_email = contact
        .email
        .as_deref()
        .ok_or(DeliveryError::NoRecipientEmail)?;

    let text_body = render_invoice_text(invoice, contact);
    let pdf_bytes =
        render_invoice_pdf(invoice, contact).map_err(DeliveryError::PdfError)?;

    let default_subject = format!("Invoice {}", invoice.invoice_number);
    let subject = subject.unwrap_or(&default_subject);
    let filename = format!("{}.pdf", invoice.invoice_number);

    let (from_name, from_email) = config.from_address();

    let parts = EmailParts {
        from_name,
        from_email,
        to_email: recipient_email,
        to_name: &contact.name,
        subject,
        text_body: &text_body,
        pdf_bytes: &pdf_bytes,
        filename: &filename,
    };

    let backend_name = match config {
        EmailConfig::Smtp { ref smtp, .. } => {
            send_smtp(&parts, smtp).await?;
            "smtp"
        }
        EmailConfig::Resend { ref api_key, .. } => {
            send_resend(&parts, api_key).await?;
            "resend"
        }
    };

    Ok(DeliveryResult {
        recipient: recipient_email.to_string(),
        invoice_number: invoice.invoice_number.clone(),
        backend: backend_name.to_string(),
    })
}

struct EmailParts<'a> {
    from_name: &'a str,
    from_email: &'a str,
    to_email: &'a str,
    to_name: &'a str,
    subject: &'a str,
    text_body: &'a str,
    pdf_bytes: &'a [u8],
    filename: &'a str,
}

async fn send_smtp(
    parts: &EmailParts<'_>,
    smtp: &crate::config::SmtpConfig,
) -> Result<(), DeliveryError> {
    let pdf_attachment = Attachment::new(parts.filename.to_string()).body(
        parts.pdf_bytes.to_vec(),
        ContentType::parse("application/pdf").unwrap(),
    );

    let email = Message::builder()
        .from(
            format!("{} <{}>", parts.from_name, parts.from_email)
                .parse()
                .map_err(|e| DeliveryError::MessageBuild(format!("invalid from address: {e}")))?,
        )
        .to(format!("{} <{}>", parts.to_name, parts.to_email)
            .parse()
            .map_err(|e| DeliveryError::MessageBuild(format!("invalid to address: {e}")))?)
        .subject(parts.subject)
        .multipart(
            MultiPart::mixed()
                .singlepart(SinglePart::plain(parts.text_body.to_string()))
                .singlepart(pdf_attachment),
        )
        .map_err(|e| DeliveryError::MessageBuild(e.to_string()))?;

    let creds = Credentials::new(smtp.username.clone(), smtp.password.clone());

    let transport = if smtp.starttls {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host)
            .map_err(|e| DeliveryError::Smtp(e.to_string()))?
            .port(smtp.port)
            .credentials(creds)
            .build()
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.host)
            .map_err(|e| DeliveryError::Smtp(e.to_string()))?
            .port(smtp.port)
            .credentials(creds)
            .build()
    };

    transport
        .send(email)
        .await
        .map_err(|e| DeliveryError::Smtp(e.to_string()))?;

    tracing::info!("Invoice sent via SMTP to {}", parts.to_email);
    Ok(())
}

async fn send_resend(
    parts: &EmailParts<'_>,
    api_key: &str,
) -> Result<(), DeliveryError> {
    use serde_json::json;

    let encoded = base64_encode(parts.pdf_bytes);

    let body = json!({
        "from": format!("{} <{}>", parts.from_name, parts.from_email),
        "to": [parts.to_email],
        "subject": parts.subject,
        "text": parts.text_body,
        "attachments": [{
            "filename": parts.filename,
            "content": encoded,
            "type": "application/pdf",
        }]
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| DeliveryError::Resend(e.to_string()))?;
    let resp = client
        .post("https://api.resend.com/emails")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| DeliveryError::Resend(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let mut text = resp
            .text()
            .await
            .unwrap_or_else(|_| "no body".to_string());
        text.truncate(500); // avoid leaking verbose error bodies
        return Err(DeliveryError::Resend(format!("{status}: {text}")));
    }

    tracing::info!("Invoice sent via Resend API to {}", parts.to_email);
    Ok(())
}

/// Simple base64 encoder (avoids adding a base64 crate dependency).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[(triple >> 18 & 0x3F) as usize] as char);
        result.push(CHARS[(triple >> 12 & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[(triple >> 6 & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use aequi_core::*;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    fn sample_invoice() -> Invoice {
        Invoice {
            id: None,
            invoice_number: "INV-042".to_string(),
            contact_id: ContactId(1),
            status: InvoiceStatus::Draft,
            issue_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            due_date: NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
            lines: vec![InvoiceLine {
                description: "Consulting".to_string(),
                quantity: Decimal::from(10),
                unit_rate: Money::from_cents(15000),
                taxable: false,
            }],
            discount: None,
            tax_lines: vec![],
            notes: None,
            terms: Some("Net 30".to_string()),
        }
    }

    #[test]
    fn no_email_returns_error() {
        let invoice = sample_invoice();
        let contact = Contact::new("No Email Inc", ContactType::Client);
        let config = EmailConfig::Resend {
            from_name: "Test".into(),
            from_email: "test@example.com".into(),
            api_key: "re_fake".into(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(send_invoice(&config, &invoice, &contact, None));
        assert!(matches!(result, Err(DeliveryError::NoRecipientEmail)));
    }

    #[test]
    fn base64_roundtrip() {
        let input = b"Hello, invoice PDF!";
        let encoded = base64_encode(input);
        assert_eq!(encoded, "SGVsbG8sIGludm9pY2UgUERGIQ==");
    }

    #[test]
    fn config_smtp_serde() {
        let json = r#"{
            "backend": "smtp",
            "from_name": "Aequi",
            "from_email": "invoices@example.com",
            "smtp": {
                "host": "smtp.example.com",
                "port": 587,
                "username": "user",
                "password": "pass"
            }
        }"#;
        let config: EmailConfig = serde_json::from_str(json).unwrap();
        let (name, email) = config.from_address();
        assert_eq!(name, "Aequi");
        assert_eq!(email, "invoices@example.com");
    }

    #[test]
    fn config_resend_serde() {
        let json = r#"{
            "backend": "resend",
            "from_name": "Aequi",
            "from_email": "invoices@example.com",
            "api_key": "re_test_key"
        }"#;
        let config: EmailConfig = serde_json::from_str(json).unwrap();
        let (name, email) = config.from_address();
        assert_eq!(name, "Aequi");
        assert_eq!(email, "invoices@example.com");
    }

    #[test]
    fn base64_empty() {
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn base64_one_byte() {
        assert_eq!(base64_encode(b"A"), "QQ==");
    }

    #[test]
    fn base64_two_bytes() {
        assert_eq!(base64_encode(b"AB"), "QUI=");
    }

    #[test]
    fn base64_three_bytes() {
        assert_eq!(base64_encode(b"ABC"), "QUJD");
    }

    #[test]
    fn config_debug_redacts_secrets() {
        let config = EmailConfig::Resend {
            from_name: "Test".into(),
            from_email: "test@example.com".into(),
            api_key: "re_super_secret_key".into(),
        };
        let debug = format!("{config:?}");
        assert!(!debug.contains("re_super_secret_key"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn smtp_debug_redacts_password() {
        let config = crate::config::SmtpConfig {
            host: "smtp.example.com".into(),
            port: 587,
            username: "user".into(),
            password: "s3cret".into(),
            starttls: true,
        };
        let debug = format!("{config:?}");
        assert!(!debug.contains("s3cret"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn delivery_result_serializes() {
        let r = DeliveryResult {
            recipient: "test@example.com".into(),
            invoice_number: "INV-001".into(),
            backend: "smtp".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("INV-001"));
    }
}
