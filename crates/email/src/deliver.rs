use aequi_core::{Contact, Invoice};
use aequi_pdf::{render_invoice_pdf, render_invoice_text};
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
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

    let default_subject = format!("Invoice {}", invoice.invoice_number);
    let subject = subject.unwrap_or(&default_subject);
    let (from_name, from_email) = config.from_address();

    // Reject CRLF in header-sensitive fields to prevent email header injection
    // (lettre refuses them too, but the Resend API path never goes through lettre).
    for (field, value) in [
        ("subject", subject),
        ("contact name", contact.name.as_str()),
        ("invoice number", invoice.invoice_number.as_str()),
        ("recipient email", recipient_email),
        ("sender name", from_name),
        ("sender email", from_email),
    ] {
        if value.contains('\r') || value.contains('\n') {
            return Err(DeliveryError::MessageBuild(format!(
                "Invalid {field}: must not contain line breaks"
            )));
        }
    }

    let from = mailbox(from_name, from_email, "sender")?;
    let to = mailbox(&contact.name, recipient_email, "recipient")?;

    let text_body = render_invoice_text(invoice, contact);
    // Typst layout is CPU-bound; keep it off the async worker threads.
    let pdf_bytes = {
        let (invoice, contact) = (invoice.clone(), contact.clone());
        tokio::task::spawn_blocking(move || render_invoice_pdf(&invoice, &contact))
            .await
            .map_err(|e| DeliveryError::PdfError(format!("render task failed: {e}")))?
            .map_err(DeliveryError::PdfError)?
    };

    let filename = format!("{}.pdf", invoice.invoice_number);

    let parts = EmailParts {
        from,
        to,
        subject,
        text_body: &text_body,
        pdf_bytes: &pdf_bytes,
        filename: &filename,
    };

    let backend_name = match config {
        EmailConfig::Smtp { smtp, .. } => {
            send_smtp(&parts, smtp).await?;
            "smtp"
        }
        EmailConfig::Resend { api_key, .. } => {
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

/// Build a mailbox from a display name and address. The name is kept as data
/// (lettre quotes/encodes it), so "Acme, Inc." or quotes in a name are fine.
fn mailbox(name: &str, email: &str, role: &str) -> Result<Mailbox, DeliveryError> {
    let address: Address = email
        .trim()
        .parse()
        .map_err(|e| DeliveryError::MessageBuild(format!("invalid {role} address: {e}")))?;
    let name = name.trim();
    Ok(Mailbox::new(
        (!name.is_empty()).then(|| name.to_string()),
        address,
    ))
}

/// RFC 5322 form of a mailbox (`"Acme, Inc." <billing@acme.test>`), for APIs
/// that take the header value as a string.
fn mailbox_header(mailbox: &Mailbox) -> Result<String, DeliveryError> {
    use std::fmt::Write;
    let mut out = String::new();
    write!(out, "{mailbox}")
        .map_err(|_| DeliveryError::MessageBuild("unencodable mailbox".to_string()))?;
    Ok(out)
}

/// Truncate to at most `max_bytes` without splitting a UTF-8 character.
fn truncate_on_char_boundary(s: &mut String, max_bytes: usize) {
    let end = s.floor_char_boundary(max_bytes);
    s.truncate(end);
}

struct EmailParts<'a> {
    from: Mailbox,
    to: Mailbox,
    subject: &'a str,
    text_body: &'a str,
    pdf_bytes: &'a [u8],
    filename: &'a str,
}

fn build_smtp_message(parts: &EmailParts<'_>) -> Result<Message, DeliveryError> {
    let pdf_attachment = Attachment::new(parts.filename.to_string()).body(
        parts.pdf_bytes.to_vec(),
        ContentType::parse("application/pdf").unwrap(),
    );

    Message::builder()
        .from(parts.from.clone())
        .to(parts.to.clone())
        .subject(parts.subject)
        .multipart(
            MultiPart::mixed()
                .singlepart(SinglePart::plain(parts.text_body.to_string()))
                .singlepart(pdf_attachment),
        )
        .map_err(|e| DeliveryError::MessageBuild(e.to_string()))
}

async fn send_smtp(
    parts: &EmailParts<'_>,
    smtp: &crate::config::SmtpConfig,
) -> Result<(), DeliveryError> {
    let email = build_smtp_message(parts)?;

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

    tracing::info!("Invoice sent via SMTP to {}", parts.to.email);
    Ok(())
}

async fn send_resend(parts: &EmailParts<'_>, api_key: &str) -> Result<(), DeliveryError> {
    use serde_json::json;

    let encoded = base64_encode(parts.pdf_bytes);

    let body = json!({
        "from": mailbox_header(&parts.from)?,
        "to": [parts.to.email.to_string()],
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
        let mut text = resp.text().await.unwrap_or_else(|_| "no body".to_string());
        truncate_on_char_boundary(&mut text, 500); // avoid leaking verbose error bodies
        return Err(DeliveryError::Resend(format!("{status}: {text}")));
    }

    tracing::info!("Invoice sent via Resend API to {}", parts.to.email);
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

    #[test]
    fn render_text_contains_invoice_number() {
        let invoice = sample_invoice();
        let contact = Contact::new("Acme Corp", ContactType::Client);
        let text = render_invoice_text(&invoice, &contact);
        assert!(text.contains("INVOICE INV-042"));
    }

    #[test]
    fn render_text_contains_contact_name() {
        let invoice = sample_invoice();
        let contact = Contact::new("Acme Corp", ContactType::Client);
        let text = render_invoice_text(&invoice, &contact);
        assert!(text.contains("Bill To: Acme Corp"));
    }

    #[test]
    fn render_text_contains_dates() {
        let invoice = sample_invoice();
        let contact = Contact::new("Acme Corp", ContactType::Client);
        let text = render_invoice_text(&invoice, &contact);
        assert!(text.contains("2026-03-01"));
        assert!(text.contains("2026-03-31"));
    }

    #[test]
    fn render_text_contains_line_items() {
        let invoice = sample_invoice();
        let contact = Contact::new("Acme Corp", ContactType::Client);
        let text = render_invoice_text(&invoice, &contact);
        assert!(text.contains("Consulting"));
    }

    #[test]
    fn render_text_contains_terms() {
        let invoice = sample_invoice();
        let contact = Contact::new("Acme Corp", ContactType::Client);
        let text = render_invoice_text(&invoice, &contact);
        assert!(text.contains("Net 30"));
    }

    #[test]
    fn render_text_includes_email_when_present() {
        let invoice = sample_invoice();
        let mut contact = Contact::new("Acme Corp", ContactType::Client);
        contact.email = Some("billing@acme.com".into());
        let text = render_invoice_text(&invoice, &contact);
        assert!(text.contains("billing@acme.com"));
    }

    #[test]
    fn render_text_includes_address_when_present() {
        let invoice = sample_invoice();
        let mut contact = Contact::new("Acme Corp", ContactType::Client);
        contact.address = Some("123 Main St, Springfield".into());
        let text = render_invoice_text(&invoice, &contact);
        assert!(text.contains("123 Main St, Springfield"));
    }

    #[test]
    fn render_pdf_produces_bytes() {
        let invoice = sample_invoice();
        let contact = Contact::new("Acme Corp", ContactType::Client);
        let pdf = render_invoice_pdf(&invoice, &contact);
        assert!(pdf.is_ok());
        let bytes = pdf.unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn crlf_in_subject_rejected() {
        let invoice = sample_invoice();
        let mut contact = Contact::new("Acme Corp", ContactType::Client);
        contact.email = Some("test@example.com".into());
        let config = EmailConfig::Resend {
            from_name: "Test".into(),
            from_email: "test@example.com".into(),
            api_key: "re_fake".into(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(send_invoice(
            &config,
            &invoice,
            &contact,
            Some("Bad\r\nSubject"),
        ));
        assert!(matches!(result, Err(DeliveryError::MessageBuild(_))));
        if let Err(DeliveryError::MessageBuild(msg)) = result {
            assert!(msg.contains("subject"));
            assert!(msg.contains("line breaks"));
        }
    }

    #[test]
    fn crlf_in_contact_name_rejected() {
        let invoice = sample_invoice();
        let mut contact = Contact::new("Evil\nName", ContactType::Client);
        contact.email = Some("test@example.com".into());
        let config = EmailConfig::Resend {
            from_name: "Test".into(),
            from_email: "test@example.com".into(),
            api_key: "re_fake".into(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(send_invoice(&config, &invoice, &contact, None));
        assert!(matches!(result, Err(DeliveryError::MessageBuild(_))));
        if let Err(DeliveryError::MessageBuild(msg)) = result {
            assert!(msg.contains("contact name"));
        }
    }

    #[test]
    fn crlf_cr_in_invoice_number_rejected() {
        let mut invoice = sample_invoice();
        invoice.invoice_number = "INV\r-042".to_string();
        let mut contact = Contact::new("Acme Corp", ContactType::Client);
        contact.email = Some("test@example.com".into());
        let config = EmailConfig::Resend {
            from_name: "Test".into(),
            from_email: "test@example.com".into(),
            api_key: "re_fake".into(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        // When no explicit subject is given, default subject is "Invoice {number}",
        // which also contains \r, so subject check may fire first.
        let result = rt.block_on(send_invoice(&config, &invoice, &contact, None));
        assert!(matches!(result, Err(DeliveryError::MessageBuild(_))));
        if let Err(DeliveryError::MessageBuild(msg)) = result {
            assert!(msg.contains("line breaks"));
        }
    }

    #[test]
    fn crlf_in_invoice_number_with_explicit_subject() {
        let mut invoice = sample_invoice();
        invoice.invoice_number = "INV\n-042".to_string();
        let mut contact = Contact::new("Acme Corp", ContactType::Client);
        contact.email = Some("test@example.com".into());
        let config = EmailConfig::Resend {
            from_name: "Test".into(),
            from_email: "test@example.com".into(),
            api_key: "re_fake".into(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        // With clean explicit subject, the invoice number check fires
        let result = rt.block_on(send_invoice(
            &config,
            &invoice,
            &contact,
            Some("Clean Subject"),
        ));
        assert!(matches!(result, Err(DeliveryError::MessageBuild(_))));
        if let Err(DeliveryError::MessageBuild(msg)) = result {
            assert!(msg.contains("invoice number"));
        }
    }

    fn parts<'a>(from: Mailbox, to: Mailbox, pdf: &'a [u8]) -> EmailParts<'a> {
        EmailParts {
            from,
            to,
            subject: "Invoice INV-1",
            text_body: "body",
            pdf_bytes: pdf,
            filename: "INV-1.pdf",
        }
    }

    #[test]
    fn display_names_with_specials_are_quoted() {
        let from = mailbox("Acme, Inc.", "billing@acme.test", "sender").unwrap();
        assert_eq!(
            mailbox_header(&from).unwrap(),
            r#""Acme, Inc." <billing@acme.test>"#
        );
        let quoted = mailbox(r#"Bob "The Builder" <x>"#, "bob@x.test", "recipient").unwrap();
        assert_eq!(
            mailbox_header(&quoted).unwrap(),
            r#""Bob \"The Builder\" <x>" <bob@x.test>"#
        );
        let bare = mailbox("", "a@b.test", "sender").unwrap();
        assert_eq!(mailbox_header(&bare).unwrap(), "a@b.test");
    }

    #[test]
    fn smtp_message_builds_with_comma_in_names() {
        let from = mailbox("Acme, Inc.", "billing@acme.test", "sender").unwrap();
        let to = mailbox("Doe, Jane", "jane@client.test", "recipient").unwrap();
        let msg = build_smtp_message(&parts(from, to, b"%PDF-")).unwrap();
        let raw = String::from_utf8(msg.formatted()).unwrap();
        // lettre RFC 2047-encodes display names containing specials.
        let from = format!(
            "From: =?utf-8?b?{}?= <billing@acme.test>",
            base64_encode(b"Acme, Inc.")
        );
        let to = format!(
            "To: =?utf-8?b?{}?= <jane@client.test>",
            base64_encode(b"Doe, Jane")
        );
        assert!(raw.contains(&from), "{raw}");
        assert!(raw.contains(&to), "{raw}");
    }

    #[test]
    fn invalid_recipient_address_rejected() {
        assert!(matches!(
            mailbox("X", "not an address", "recipient"),
            Err(DeliveryError::MessageBuild(_))
        ));
    }

    #[test]
    fn crlf_in_recipient_email_rejected() {
        let invoice = sample_invoice();
        let mut contact = Contact::new("Acme Corp", ContactType::Client);
        contact.email = Some("a@b.test\r\nBcc: evil@x.test".into());
        let config = EmailConfig::Resend {
            from_name: "Test".into(),
            from_email: "test@example.com".into(),
            api_key: "re_fake".into(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(send_invoice(&config, &invoice, &contact, None));
        assert!(
            matches!(result, Err(DeliveryError::MessageBuild(ref m)) if m.contains("recipient email"))
        );
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        // Byte 500 falls inside a two-byte 'é'; String::truncate would panic.
        let mut s = format!("a{}", "é".repeat(300));
        truncate_on_char_boundary(&mut s, 500);
        assert_eq!(s.len(), 499);
        let mut short = "short".to_string();
        truncate_on_char_boundary(&mut short, 500);
        assert_eq!(short, "short");
    }

    #[test]
    fn config_rejects_unknown_fields() {
        // The old .env.example used "from"; a typo must fail, not be ignored.
        let json = r#"{"backend":"resend","api_key":"re_x","from":"a@b.test"}"#;
        assert!(serde_json::from_str::<EmailConfig>(json).is_err());
        let json = r#"{"backend":"resend","api_key":"re_x","from_name":"A","from_email":"a@b.test","form":"x"}"#;
        assert!(serde_json::from_str::<EmailConfig>(json).is_err());
    }

    #[test]
    fn base64_longer_input() {
        // "Man" is the classic base64 test vector
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
    }

    #[test]
    fn base64_binary_data() {
        let data: Vec<u8> = (0..=255).collect();
        let encoded = base64_encode(&data);
        // Should be valid base64 (length multiple of 4, only valid chars)
        assert_eq!(encoded.len() % 4, 0);
        assert!(
            encoded
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
        );
    }

    #[test]
    fn delivery_error_display() {
        let e1 = DeliveryError::NoRecipientEmail;
        assert_eq!(e1.to_string(), "recipient has no email address");

        let e2 = DeliveryError::PdfError("typst failed".into());
        assert_eq!(e2.to_string(), "PDF generation failed: typst failed");

        let e3 = DeliveryError::MessageBuild("bad header".into());
        assert_eq!(e3.to_string(), "failed to build email message: bad header");

        let e4 = DeliveryError::Smtp("timeout".into());
        assert_eq!(e4.to_string(), "SMTP delivery failed: timeout");

        let e5 = DeliveryError::Resend("429 rate limited".into());
        assert_eq!(e5.to_string(), "Resend API error: 429 rate limited");
    }

    #[test]
    fn delivery_result_all_fields() {
        let r = DeliveryResult {
            recipient: "alice@example.com".into(),
            invoice_number: "INV-099".into(),
            backend: "resend".into(),
        };
        let json: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(json["recipient"], "alice@example.com");
        assert_eq!(json["invoice_number"], "INV-099");
        assert_eq!(json["backend"], "resend");
    }
}
