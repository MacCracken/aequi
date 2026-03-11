use aequi_core::{Contact, Discount, Invoice};

/// Generate Typst markup for an invoice.
fn invoice_to_typst(invoice: &Invoice, contact: &Contact) -> String {
    let mut typ = String::new();

    // Page setup
    typ.push_str("#set page(margin: (x: 2cm, y: 2cm))\n");
    typ.push_str("#set text(size: 10pt)\n\n");

    // Header
    typ.push_str(&format!(
        "#align(right)[#text(size: 24pt, weight: \"bold\")[INVOICE]]\n\n"
    ));

    // Invoice metadata table
    typ.push_str("#grid(\n");
    typ.push_str("  columns: (1fr, auto),\n");
    typ.push_str("  [\n");
    typ.push_str(&format!(
        "    #text(weight: \"bold\")[Bill To:]\\\n"
    ));
    typ.push_str(&format!("    {}\\\n", escape(&contact.name)));
    if let Some(email) = &contact.email {
        typ.push_str(&format!("    {}\\\n", escape(email)));
    }
    if let Some(addr) = &contact.address {
        typ.push_str(&format!("    {}\\\n", escape(addr)));
    }
    typ.push_str("  ],\n");
    typ.push_str("  align(right)[\n");
    typ.push_str(&format!(
        "    #text(weight: \"bold\")[Invoice \\#:] {}\\\n",
        escape(&invoice.invoice_number)
    ));
    typ.push_str(&format!(
        "    #text(weight: \"bold\")[Date:] {}\\\n",
        invoice.issue_date
    ));
    typ.push_str(&format!(
        "    #text(weight: \"bold\")[Due:] {}\\\n",
        invoice.due_date
    ));
    typ.push_str("  ],\n");
    typ.push_str(")\n\n");

    typ.push_str("#v(1em)\n");

    // Line items table
    typ.push_str("#table(\n");
    typ.push_str("  columns: (1fr, auto, auto, auto),\n");
    typ.push_str("  align: (left, right, right, right),\n");
    typ.push_str("  stroke: none,\n");
    typ.push_str("  table.hline(),\n");
    typ.push_str("  table.header(\n");
    typ.push_str("    [*Description*], [*Qty*], [*Rate*], [*Amount*],\n");
    typ.push_str("  ),\n");
    typ.push_str("  table.hline(),\n");

    for line in &invoice.lines {
        typ.push_str(&format!(
            "  [{}], [{}], [{}], [{}],\n",
            escape(&line.description),
            line.quantity,
            money(line.unit_rate),
            money(line.amount())
        ));
    }

    typ.push_str("  table.hline(),\n");
    typ.push_str(")\n\n");

    // Totals
    typ.push_str("#align(right)[\n");
    typ.push_str("#grid(\n");
    typ.push_str("  columns: (auto, 8em),\n");
    typ.push_str("  row-gutter: 0.5em,\n");
    typ.push_str("  align: (right, right),\n");
    typ.push_str(&format!(
        "  [Subtotal:], [{}],\n",
        money(invoice.subtotal())
    ));

    let discount = invoice.discount_amount();
    if !discount.is_zero() {
        let label = match &invoice.discount {
            Some(Discount::Percentage(pct)) => format!("Discount ({pct}%):"),
            Some(Discount::Flat(_)) => "Discount:".to_string(),
            None => "Discount:".to_string(),
        };
        typ.push_str(&format!("  [{}], [−{}],\n", label, money(discount)));
    }

    let tax = invoice.tax_amount();
    if !tax.is_zero() {
        for tl in &invoice.tax_lines {
            typ.push_str(&format!(
                "  [{} ({}%):], [{}],\n",
                escape(&tl.label),
                tl.rate,
                money(tax)
            ));
        }
    }

    typ.push_str(&format!(
        "  [#text(weight: \"bold\")[Total:]], [#text(weight: \"bold\")[{}]],\n",
        money(invoice.total())
    ));
    typ.push_str(")\n");
    typ.push_str("]\n\n");

    // Terms and notes
    if let Some(terms) = &invoice.terms {
        typ.push_str(&format!(
            "#v(2em)\n#text(weight: \"bold\")[Terms:] {}\n",
            escape(terms)
        ));
    }
    if let Some(notes) = &invoice.notes {
        typ.push_str(&format!("\n#v(1em)\n{}\n", escape(notes)));
    }

    typ
}

/// Escape special Typst characters in user-provided text.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('#', "\\#")
        .replace('$', "\\$")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('@', "\\@")
        .replace('<', "\\<")
        .replace('>', "\\>")
}

/// Format a Money value safe for Typst (escape the $ sign).
fn money(m: aequi_core::Money) -> String {
    escape(&m.to_string())
}

/// Render an invoice as a PDF byte vector using Typst.
///
/// Returns `Ok(pdf_bytes)` on success, or an error string.
pub fn render_invoice_pdf(invoice: &Invoice, contact: &Contact) -> Result<Vec<u8>, String> {
    let typst_source = invoice_to_typst(invoice, contact);

    let engine = typst_as_lib::TypstEngine::builder()
        .main_file(typst_source)
        .build();

    let result = engine.compile::<typst::layout::PagedDocument>();

    let doc = result.output.map_err(|e| format!("Typst compile error: {e}"))?;

    let options = typst_pdf::PdfOptions::default();
    typst_pdf::pdf(&doc, &options).map_err(|e| format!("PDF generation error: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aequi_core::*;
    use chrono::{NaiveDate, Utc};
    use rust_decimal::Decimal;

    fn sample_contact() -> Contact {
        let mut c = Contact::new("Acme Corp", ContactType::Client);
        c.email = Some("billing@acme.com".to_string());
        c.address = Some("123 Main St, Springfield, IL 62701".to_string());
        c
    }

    fn sample_invoice() -> Invoice {
        Invoice {
            id: None,
            invoice_number: "INV-001".to_string(),
            contact_id: ContactId(1),
            status: InvoiceStatus::Draft,
            issue_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            due_date: NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
            lines: vec![
                InvoiceLine {
                    description: "Web Development".to_string(),
                    quantity: Decimal::from(40),
                    unit_rate: Money::from_cents(15000),
                    taxable: false,
                },
                InvoiceLine {
                    description: "Design Consultation".to_string(),
                    quantity: Decimal::from(8),
                    unit_rate: Money::from_cents(20000),
                    taxable: false,
                },
            ],
            discount: None,
            tax_lines: vec![],
            notes: Some("Thank you for your business!".to_string()),
            terms: Some("Net 30".to_string()),
        }
    }

    #[test]
    fn typst_markup_contains_invoice_data() {
        let contact = sample_contact();
        let invoice = sample_invoice();
        let typ = invoice_to_typst(&invoice, &contact);

        assert!(typ.contains("INV-001"));
        assert!(typ.contains("Acme Corp"));
        assert!(typ.contains("billing\\@acme.com"));
        assert!(typ.contains("Web Development"));
        assert!(typ.contains("Design Consultation"));
        assert!(typ.contains("Net 30"));
        assert!(typ.contains("Thank you for your business!"));
    }

    #[test]
    fn typst_markup_with_discount() {
        let contact = sample_contact();
        let mut invoice = sample_invoice();
        invoice.discount = Some(Discount::Percentage(Decimal::from(10)));

        let typ = invoice_to_typst(&invoice, &contact);
        assert!(typ.contains("Discount (10%)"));
    }

    #[test]
    fn typst_markup_with_tax() {
        let contact = sample_contact();
        let mut invoice = sample_invoice();
        invoice.lines[0].taxable = true;
        invoice.tax_lines = vec![TaxLine {
            label: "Sales Tax".to_string(),
            rate: Decimal::new(825, 2), // 8.25%
        }];

        let typ = invoice_to_typst(&invoice, &contact);
        assert!(typ.contains("Sales Tax"));
    }

    #[test]
    fn escape_special_chars() {
        assert_eq!(escape("test@email.com"), "test\\@email.com");
        assert_eq!(escape("$100"), "\\$100");
        assert_eq!(escape("a#b"), "a\\#b");
    }

    #[test]
    fn render_pdf_produces_bytes() {
        let contact = sample_contact();
        let invoice = sample_invoice();
        let pdf = render_invoice_pdf(&invoice, &contact).expect("PDF render failed");

        // PDF files start with %PDF
        assert!(pdf.len() > 100, "PDF too small: {} bytes", pdf.len());
        assert_eq!(&pdf[0..5], b"%PDF-", "Not a valid PDF header");
    }

    #[test]
    fn render_pdf_with_discount_and_tax() {
        let contact = sample_contact();
        let mut invoice = sample_invoice();
        invoice.discount = Some(Discount::Flat(Money::from_cents(5000)));
        invoice.lines[0].taxable = true;
        invoice.tax_lines = vec![TaxLine {
            label: "Tax".to_string(),
            rate: Decimal::new(10, 0),
        }];

        let pdf = render_invoice_pdf(&invoice, &contact).expect("PDF render failed");
        assert_eq!(&pdf[0..5], b"%PDF-");
    }

    #[test]
    fn render_pdf_single_line_item() {
        let contact = Contact::new("Simple Client", ContactType::Client);
        let invoice = Invoice {
            id: None,
            invoice_number: "INV-002".to_string(),
            contact_id: ContactId(2),
            status: InvoiceStatus::Sent { sent_at: Utc::now() },
            issue_date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            due_date: NaiveDate::from_ymd_opt(2026, 4, 30).unwrap(),
            lines: vec![InvoiceLine {
                description: "Consulting".to_string(),
                quantity: Decimal::from(1),
                unit_rate: Money::from_cents(100000),
                taxable: false,
            }],
            discount: None,
            tax_lines: vec![],
            notes: None,
            terms: None,
        };

        let pdf = render_invoice_pdf(&invoice, &contact).expect("PDF render failed");
        assert_eq!(&pdf[0..5], b"%PDF-");
    }
}
