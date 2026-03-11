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

    out.push_str(&format!("{:>64} {:>12}\n", "Subtotal:", invoice.subtotal()));

    let discount = invoice.discount_amount();
    if !discount.is_zero() {
        out.push_str(&format!("{:>64} {:>12}\n", "Discount:", discount));
    }

    let tax = invoice.tax_amount();
    if !tax.is_zero() {
        for tl in &invoice.tax_lines {
            out.push_str(&format!("{:>64} {:>12}\n", format!("{}:", tl.label), tax));
        }
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
}
