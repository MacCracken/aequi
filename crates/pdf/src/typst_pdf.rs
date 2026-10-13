use aequi_core::{Contact, Discount, Invoice, Money};
use rust_decimal::Decimal;

/// Generate Typst markup for an invoice.
///
/// User-provided text (names, addresses, descriptions, terms, notes) is never
/// spliced into markup: each value becomes a Typst string literal (see
/// [`typst_str`]) and is inserted as plain text, so markup characters in it
/// (`[`, `]`, `#`, `//`, `*`, `=` at line start, …) are shown verbatim.
fn invoice_to_typst(invoice: &Invoice, contact: &Contact) -> String {
    let mut typ = String::new();

    // Page setup
    typ.push_str("#set page(margin: (x: 2cm, y: 2cm))\n");
    typ.push_str("#set text(size: 10pt)\n\n");

    // Header
    typ.push_str("#align(right)[#text(size: 24pt, weight: \"bold\")[INVOICE]]\n\n");

    // Invoice metadata table
    typ.push_str("#grid(\n");
    typ.push_str("  columns: (1fr, auto),\n");
    typ.push_str("  [\n");
    typ.push_str("    #text(weight: \"bold\")[Bill To:] \\\n");
    typ.push_str(&format!("    #{} \\\n", typst_str(&contact.name)));
    if let Some(email) = &contact.email {
        typ.push_str(&format!("    #{} \\\n", typst_str(email)));
    }
    if let Some(addr) = &contact.address {
        typ.push_str(&format!("    #{} \\\n", typst_str(addr)));
    }
    typ.push_str("  ],\n");
    typ.push_str("  align(right)[\n");
    typ.push_str(&format!(
        "    #text(weight: \"bold\")[Invoice \\#:] #{} \\\n",
        typst_str(&invoice.invoice_number)
    ));
    typ.push_str(&format!(
        "    #text(weight: \"bold\")[Date:] #{} \\\n",
        typst_str(&invoice.issue_date.to_string())
    ));
    typ.push_str(&format!(
        "    #text(weight: \"bold\")[Due:] #{} \\\n",
        typst_str(&invoice.due_date.to_string())
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
            "  {}, {}, {}, {},\n",
            typst_str(&line.description),
            typst_str(&line.quantity.normalize().to_string()),
            typst_str(&line.unit_rate.to_string()),
            typst_str(&line.amount().to_string())
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
    for (label, amount) in totals_rows(invoice) {
        typ.push_str(&format!(
            "  {}, {},\n",
            typst_str(&label),
            typst_str(&amount)
        ));
    }
    typ.push_str(&format!(
        "  text(weight: \"bold\")[Total:], text(weight: \"bold\", {}),\n",
        typst_str(&invoice.total().to_string())
    ));
    typ.push_str(")\n");
    typ.push_str("]\n\n");

    // Terms and notes
    if let Some(terms) = &invoice.terms {
        typ.push_str(&format!(
            "#v(2em)\n#text(weight: \"bold\")[Terms:] #{}\n",
            typst_str(terms)
        ));
    }
    if let Some(notes) = &invoice.notes {
        typ.push_str(&format!("\n#v(1em)\n#{}\n", typst_str(notes)));
    }

    typ
}

/// Subtotal, discount and per-tax-line rows (label, amount) shared by the PDF
/// and plain-text renderers. Rates are stored as fractions (0.0825) and shown
/// as percentages (8.25%); each tax line shows its own amount.
pub(crate) fn totals_rows(invoice: &Invoice) -> Vec<(String, String)> {
    let mut rows = vec![("Subtotal:".to_string(), invoice.subtotal().to_string())];

    let discount = invoice.discount_amount();
    if !discount.is_zero() {
        let label = match &invoice.discount {
            Some(Discount::Percentage(pct)) => format!("Discount ({}%):", percent(*pct)),
            Some(Discount::Flat(_)) | None => "Discount:".to_string(),
        };
        rows.push((label, format!("−{discount}")));
    }

    if !invoice.tax_amount().is_zero() {
        let taxable = invoice.taxable_subtotal();
        for tl in &invoice.tax_lines {
            let line_tax: Money = taxable * tl.rate;
            rows.push((
                format!("{} ({}%):", tl.label, percent(tl.rate)),
                line_tax.to_string(),
            ));
        }
    }

    rows
}

/// A fractional rate as a percentage without trailing zeros: 0.0825 -> "8.25".
pub(crate) fn percent(rate: Decimal) -> String {
    (rate * Decimal::ONE_HUNDRED).normalize().to_string()
}

/// Quote `s` as a Typst string literal. Inside a string only backslash and
/// double quote are special, so escaping those (plus control characters)
/// makes any input inert; `#"…"` then inserts it as plain text.
fn typst_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn compile_invoice(
    invoice: &Invoice,
    contact: &Contact,
) -> Result<typst_layout::PagedDocument, String> {
    let typst_source = invoice_to_typst(invoice, contact);

    // Without fonts Typst silently drops every glyph and the PDF has no text.
    // Use the fonts bundled with typst-assets (not system fonts) so output is
    // identical on desktop, in the server container, and in CI.
    let engine = typst_as_lib::TypstEngine::builder()
        .main_file(typst_source)
        .search_fonts_with(
            typst_as_lib::typst_kit_options::TypstKitFontOptions::new().include_system_fonts(false),
        )
        .build();

    engine
        .compile::<typst_layout::PagedDocument>()
        .output
        .map_err(|e| format!("Typst compile error: {e}"))
}

/// Render an invoice as a PDF byte vector using Typst.
///
/// Returns `Ok(pdf_bytes)` on success, or an error string.
pub fn render_invoice_pdf(invoice: &Invoice, contact: &Contact) -> Result<Vec<u8>, String> {
    let doc = compile_invoice(invoice, contact)?;
    let options = typst_pdf::PdfOptions::default();
    typst_pdf::pdf(&doc, &options).map_err(|e| format!("PDF generation error: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aequi_core::*;
    use chrono::{NaiveDate, Utc};
    use rust_decimal::Decimal;
    use typst_library::layout::{Frame, FrameItem};

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

    /// All rendered text runs with their font size in points.
    fn text_runs(doc: &typst_layout::PagedDocument) -> Vec<(String, f64)> {
        fn walk(frame: &Frame, out: &mut Vec<(String, f64)>) {
            for (_, item) in frame.items() {
                match item {
                    FrameItem::Group(group) => walk(&group.frame, out),
                    FrameItem::Text(text) => out.push((text.text.to_string(), text.size.to_pt())),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        for page in doc.pages() {
            walk(&page.frame, &mut out);
        }
        out
    }

    /// Rendered text with all whitespace removed (runs split at spaces/lines).
    fn rendered_text(doc: &typst_layout::PagedDocument) -> String {
        text_runs(doc)
            .into_iter()
            .map(|(t, _)| t)
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    fn squash(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn typst_markup_contains_invoice_data() {
        let contact = sample_contact();
        let invoice = sample_invoice();
        let typ = invoice_to_typst(&invoice, &contact);

        assert!(typ.contains("INV-001"));
        assert!(typ.contains("Acme Corp"));
        assert!(typ.contains("billing@acme.com"));
        assert!(typ.contains("Web Development"));
        assert!(typ.contains("Design Consultation"));
        assert!(typ.contains("Net 30"));
        assert!(typ.contains("Thank you for your business!"));
    }

    #[test]
    fn typst_markup_with_discount() {
        let contact = sample_contact();
        let mut invoice = sample_invoice();
        invoice.discount = Some(Discount::Percentage(Decimal::new(10, 2))); // 10%

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
            rate: Decimal::new(825, 4), // 8.25%
        }];

        let typ = invoice_to_typst(&invoice, &contact);
        assert!(typ.contains("Sales Tax (8.25%):"));
    }

    #[test]
    fn tax_rows_show_percentages_and_each_lines_own_amount() {
        let mut invoice = sample_invoice();
        invoice.lines[0].taxable = true; // $6,000.00 taxable
        invoice.tax_lines = vec![
            TaxLine {
                label: "State".to_string(),
                rate: Decimal::new(625, 4), // 6.25%
            },
            TaxLine {
                label: "City".to_string(),
                rate: Decimal::new(2, 2), // 2%
            },
        ];
        let rows = totals_rows(&invoice);
        assert!(rows.contains(&("State (6.25%):".to_string(), "$375.00".to_string())));
        assert!(rows.contains(&("City (2%):".to_string(), "$120.00".to_string())));
        assert_eq!(invoice.tax_amount(), Money::from_cents(49_500));
    }

    #[test]
    fn typst_str_escapes_only_string_specials() {
        assert_eq!(typst_str("test@email.com"), r#""test@email.com""#);
        assert_eq!(typst_str(r#"a\b"c"#), r#""a\\b\"c""#);
        assert_eq!(typst_str("l1\nl2\t\u{7}"), r#""l1\nl2\t\u{7}""#);
    }

    #[test]
    fn hostile_text_compiles_and_renders_literally() {
        let hostile = [
            "x], text(size: 400pt)[FAKE], [",
            "Net 30 // 2% in 10 days",
            "Back`tick and /* open comment",
            "= Heading",
            "- list item",
            "+ enum item",
            "#panic(\"boom\")",
            "$x^2$ *bold* _em_ @ref <label> ~ \\ \"q\"",
            ")\n#set text(size: 300pt)\n#(",
        ];
        for h in hostile {
            let mut contact = sample_contact();
            contact.name = h.to_string();
            contact.address = Some(h.to_string());
            let mut invoice = sample_invoice();
            invoice.invoice_number = h.to_string();
            invoice.lines[0].description = h.to_string();
            invoice.terms = Some(h.to_string());
            invoice.notes = Some(h.to_string());
            invoice.tax_lines = vec![TaxLine {
                label: h.to_string(),
                rate: Decimal::new(5, 2),
            }];
            invoice.lines[0].taxable = true;

            let doc = compile_invoice(&invoice, &contact)
                .unwrap_or_else(|e| panic!("hostile input {h:?} broke compilation: {e}"));

            // The text appears verbatim (so nothing was interpreted or dropped)...
            let text = rendered_text(&doc);
            assert!(
                text.contains(&squash(h)),
                "{h:?} not rendered literally in {text:?}"
            );
            // ...and no injected styling took effect.
            let max = text_runs(&doc)
                .iter()
                .map(|(_, size)| *size)
                .fold(0.0, f64::max);
            assert!(max <= 24.0, "injected font size {max}pt for {h:?}");

            render_invoice_pdf(&invoice, &contact).expect("PDF export failed");
        }
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
    fn render_pdf_embeds_fonts() {
        // Regression: with no fonts loaded Typst drops all text and still
        // produces a "valid" PDF, so check that a font program is embedded.
        let pdf = render_invoice_pdf(&sample_invoice(), &sample_contact()).unwrap();
        assert!(
            pdf.windows(8).any(|w| w == b"FontFile"),
            "PDF has no embedded fonts, so no text was rendered"
        );
    }

    #[test]
    fn render_pdf_with_discount_and_tax() {
        let contact = sample_contact();
        let mut invoice = sample_invoice();
        invoice.discount = Some(Discount::Flat(Money::from_cents(5000)));
        invoice.lines[0].taxable = true;
        invoice.tax_lines = vec![TaxLine {
            label: "Tax".to_string(),
            rate: Decimal::new(10, 2),
        }];

        let doc = compile_invoice(&invoice, &contact).expect("compile failed");
        let text = rendered_text(&doc);
        assert!(text.contains("Tax(10%):"), "{text}");
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
            status: InvoiceStatus::Sent {
                sent_at: Utc::now(),
            },
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
