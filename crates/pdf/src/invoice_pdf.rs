use aequi_core::{Contact, Invoice};

/// Render an invoice as formatted plain text.
///
/// This serves as the invoice rendering engine. A future version will
/// integrate Typst for actual PDF generation; for now this produces a
/// human-readable text representation suitable for email bodies and
/// plain-text export.
pub fn render_invoice_text(invoice: &Invoice, contact: &Contact) -> String {
    let mut out = String::new();

    out.push_str(&format!("INVOICE {}\n", invoice.invoice_number));
    out.push_str(&format!("Date: {}\n", invoice.issue_date));
    out.push_str(&format!("Due:  {}\n", invoice.due_date));
    out.push_str(&format!("\nBill To: {}\n", contact.name));
    if let Some(email) = &contact.email {
        out.push_str(&format!("         {email}\n"));
    }
    if let Some(addr) = &contact.address {
        out.push_str(&format!("         {addr}\n"));
    }

    out.push('\n');
    out.push_str(&format!(
        "{:<40} {:>8} {:>12} {:>12}\n",
        "Description", "Qty", "Rate", "Amount"
    ));
    out.push_str(&"-".repeat(76));
    out.push('\n');

    for line in &invoice.lines {
        out.push_str(&format!(
            "{:<40} {:>8} {:>12} {:>12}\n",
            line.description,
            line.quantity,
            line.unit_rate,
            line.amount()
        ));
    }

    out.push_str(&"-".repeat(76));
    out.push('\n');

    for (label, amount) in crate::typst_pdf::totals_rows(invoice) {
        out.push_str(&format!("{label:>64} {amount:>12}\n"));
    }

    out.push_str(&format!("{:>64} {:>12}\n", "TOTAL:", invoice.total()));

    if let Some(terms) = &invoice.terms {
        out.push_str(&format!("\nTerms: {terms}\n"));
    }
    if let Some(notes) = &invoice.notes {
        out.push_str(&format!("\n{notes}\n"));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aequi_core::*;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    #[test]
    fn render_basic_invoice() {
        let contact = Contact::new("Acme Corp", ContactType::Client);
        let invoice = Invoice {
            id: None,
            invoice_number: "INV-001".to_string(),
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
        };

        let text = render_invoice_text(&invoice, &contact);
        assert!(text.contains("INVOICE INV-001"));
        assert!(text.contains("Acme Corp"));
        assert!(text.contains("Consulting"));
        assert!(text.contains("Net 30"));
        assert!(text.contains("$1500.00"));
    }

    #[test]
    fn tax_lines_show_rate_as_percent_and_own_amount() {
        let contact = Contact::new("Acme Corp", ContactType::Client);
        let invoice = Invoice {
            id: None,
            invoice_number: "INV-003".to_string(),
            contact_id: ContactId(1),
            status: InvoiceStatus::Draft,
            issue_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            due_date: NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
            lines: vec![InvoiceLine {
                description: "Widgets".to_string(),
                quantity: Decimal::from(1),
                unit_rate: Money::from_cents(100_000),
                taxable: true,
            }],
            discount: Some(Discount::Percentage(Decimal::new(1, 1))), // 10%
            tax_lines: vec![
                TaxLine {
                    label: "State".to_string(),
                    rate: Decimal::new(6, 2), // 6%
                },
                TaxLine {
                    label: "County".to_string(),
                    rate: Decimal::new(225, 4), // 2.25%
                },
            ],
            notes: None,
            terms: None,
        };

        let text = render_invoice_text(&invoice, &contact);
        assert!(text.contains("Discount (10%):"), "{text}");
        // Taxable $900.00 after discount: 6% = $54.00, 2.25% = $20.25.
        assert!(text.contains("State (6%):"), "{text}");
        assert!(text.contains("$54.00"), "{text}");
        assert!(text.contains("County (2.25%):"), "{text}");
        assert!(text.contains("$20.25"), "{text}");
        assert!(
            !text.contains("$74.25"),
            "total tax printed on a tax line: {text}"
        );
    }
}
