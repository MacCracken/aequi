//! Writes that post to the ledger, and the invoice / import / receipt workflows
//! built on them.
//!
//! Every function that creates a ledger transaction goes through
//! [`insert_ledger_transaction`], so validation (via
//! [`aequi_core::ValidatedTransaction`]) and account checks happen in one place.

use aequi_core::{AccountId, Money, TransactionLine, UnvalidatedTransaction, ValidatedTransaction};
use sqlx::SqliteConnection;

use crate::db::{
    DbPool, ImportedTransaction, InvoiceLineRecord, InvoiceRecord, InvoiceTaxLineRecord,
    PaymentRecord, invoice_from_records, validate_date,
};

fn invalid(msg: impl Into<String>) -> sqlx::Error {
    sqlx::Error::InvalidArgument(msg.into())
}

// ── Accounts ─────────────────────────────────────────────────────────────────

/// An account as needed for posting: id, type, and whether it is archived.
#[derive(Debug, Clone)]
pub struct PostingAccount {
    pub id: i64,
    pub code: String,
    pub account_type: String,
}

/// Look up a postable (existing, non-archived) account by code.
pub async fn postable_account(
    conn: &mut SqliteConnection,
    code: &str,
) -> Result<PostingAccount, sqlx::Error> {
    let row: Option<(i64, String, i64)> =
        sqlx::query_as("SELECT id, account_type, is_archived FROM accounts WHERE code = ?")
            .bind(code)
            .fetch_optional(&mut *conn)
            .await?;
    match row {
        None => Err(invalid(format!("account {code} does not exist"))),
        Some((_, _, archived)) if archived != 0 => {
            Err(invalid(format!("account {code} is archived")))
        }
        Some((id, account_type, _)) => Ok(PostingAccount {
            id,
            code: code.to_string(),
            account_type,
        }),
    }
}

// ── Ledger transactions ──────────────────────────────────────────────────────

/// Insert a validated transaction and its lines using the caller's connection
/// (normally inside a database transaction). Rejects lines that post to
/// missing or archived accounts, and an unknown `contact_id`.
pub async fn insert_ledger_transaction(
    conn: &mut SqliteConnection,
    tx: &ValidatedTransaction,
    contact_id: Option<i64>,
) -> Result<i64, sqlx::Error> {
    let total_cents = tx
        .balanced_total
        .checked_to_cents()
        .ok_or_else(|| invalid("transaction amount is out of range"))?;

    for line in &tx.lines {
        let row: Option<(i64,)> = sqlx::query_as("SELECT is_archived FROM accounts WHERE id = ?")
            .bind(line.account_id.0)
            .fetch_optional(&mut *conn)
            .await?;
        match row {
            None => {
                return Err(invalid(format!(
                    "account id {} does not exist",
                    line.account_id.0
                )));
            }
            Some((archived,)) if archived != 0 => {
                return Err(invalid(format!(
                    "account id {} is archived",
                    line.account_id.0
                )));
            }
            Some(_) => {}
        }
    }
    if let Some(cid) = contact_id {
        let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM contacts WHERE id = ?")
            .bind(cid)
            .fetch_optional(&mut *conn)
            .await?;
        if exists.is_none() {
            return Err(invalid(format!("contact {cid} does not exist")));
        }
    }

    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO transactions (date, description, memo, balanced_total_cents, contact_id) \
         VALUES (?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(tx.date.to_string())
    .bind(&tx.description)
    .bind(&tx.memo)
    .bind(total_cents)
    .bind(contact_id)
    .fetch_one(&mut *conn)
    .await?;

    for line in &tx.lines {
        sqlx::query(
            "INSERT INTO transaction_lines (transaction_id, account_id, debit_cents, credit_cents, memo) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(line.account_id.0)
        .bind(line.debit.to_cents())
        .bind(line.credit.to_cents())
        .bind(&line.memo)
        .execute(&mut *conn)
        .await?;
    }
    Ok(id)
}

/// Validate and insert a transaction atomically. Core validation errors are
/// returned as `InvalidArgument`.
pub async fn create_ledger_transaction(
    pool: &DbPool,
    tx: UnvalidatedTransaction,
    contact_id: Option<i64>,
) -> Result<i64, sqlx::Error> {
    let validated = ValidatedTransaction::validate(tx).map_err(|e| invalid(e.to_string()))?;
    let mut db_tx = pool.begin().await?;
    let id = insert_ledger_transaction(&mut db_tx, &validated, contact_id).await?;
    db_tx.commit().await?;
    Ok(id)
}

/// Build and validate a two-or-more line transaction from (account, debit, credit) cents.
fn build_transaction(
    date: chrono::NaiveDate,
    description: String,
    memo: Option<String>,
    lines: &[(i64, i64, i64)],
) -> Result<ValidatedTransaction, sqlx::Error> {
    let tx = UnvalidatedTransaction {
        date,
        description,
        lines: lines
            .iter()
            .map(|&(account_id, debit, credit)| TransactionLine {
                account_id: AccountId(account_id),
                debit: Money::from_cents(debit),
                credit: Money::from_cents(credit),
                memo: None,
            })
            .collect(),
        memo,
    };
    ValidatedTransaction::validate(tx).map_err(|e| invalid(e.to_string()))
}

// ── Invoices ─────────────────────────────────────────────────────────────────

/// Default accounts used when invoice payments are posted (cash basis).
pub const DEFAULT_DEPOSIT_ACCOUNT: &str = "1000"; // Checking
pub const INVOICE_INCOME_ACCOUNT: &str = "4000"; // Services Revenue
pub const SALES_TAX_PAYABLE_ACCOUNT: &str = "2010"; // Taxes Payable

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct NewInvoiceLine {
    pub description: String,
    /// Quantity in hundredths (150 = 1.5 units).
    pub quantity_hundredths: i64,
    pub unit_rate_cents: i64,
    #[serde(default)]
    pub taxable: bool,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct NewInvoiceTaxLine {
    pub label: String,
    /// Rate in basis points (825 = 8.25%).
    pub rate_bps: i64,
}

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NewDiscount {
    /// Basis points of the subtotal (1000 = 10%).
    Percentage {
        bps: i64,
    },
    Flat {
        cents: i64,
    },
}

/// Everything needed to create (or replace the contents of) an invoice.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct NewInvoice {
    pub invoice_number: String,
    pub contact_id: i64,
    pub issue_date: String,
    pub due_date: String,
    pub lines: Vec<NewInvoiceLine>,
    #[serde(default)]
    pub tax_lines: Vec<NewInvoiceTaxLine>,
    #[serde(default)]
    pub discount: Option<NewDiscount>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub terms: Option<String>,
}

const MAX_LINE_CENTS: i64 = 10_000_000_000_000; // $100 billion

fn validate_new_invoice(inv: &NewInvoice) -> Result<(), sqlx::Error> {
    if inv.invoice_number.trim().is_empty() {
        return Err(invalid("invoice_number must not be empty"));
    }
    validate_date("issue_date", &inv.issue_date)?;
    validate_date("due_date", &inv.due_date)?;
    if inv.due_date < inv.issue_date {
        return Err(invalid("due_date must not be before issue_date"));
    }
    if inv.lines.is_empty() {
        return Err(invalid("an invoice needs at least one line item"));
    }
    for (i, line) in inv.lines.iter().enumerate() {
        let n = i + 1;
        if line.description.trim().is_empty() {
            return Err(invalid(format!("line {n}: description must not be empty")));
        }
        if line.quantity_hundredths <= 0 {
            return Err(invalid(format!("line {n}: quantity must be positive")));
        }
        if line.unit_rate_cents < 0 {
            return Err(invalid(format!("line {n}: rate must not be negative")));
        }
        let amount = i128::from(line.quantity_hundredths) * i128::from(line.unit_rate_cents) / 100;
        if amount > i128::from(MAX_LINE_CENTS) {
            return Err(invalid(format!("line {n}: amount is too large")));
        }
    }
    for t in &inv.tax_lines {
        if t.label.trim().is_empty() {
            return Err(invalid("tax line label must not be empty"));
        }
        if !(0..=10_000).contains(&t.rate_bps) {
            return Err(invalid("tax rate must be between 0% and 100%"));
        }
    }
    match inv.discount {
        Some(NewDiscount::Percentage { bps }) if !(0..=10_000).contains(&bps) => {
            return Err(invalid("discount percentage must be between 0% and 100%"));
        }
        Some(NewDiscount::Flat { cents }) if cents < 0 => {
            return Err(invalid("discount must not be negative"));
        }
        _ => {}
    }
    Ok(())
}

async fn write_invoice_contents(
    conn: &mut SqliteConnection,
    invoice_id: i64,
    inv: &NewInvoice,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM invoice_lines WHERE invoice_id = ?")
        .bind(invoice_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM invoice_tax_lines WHERE invoice_id = ?")
        .bind(invoice_id)
        .execute(&mut *conn)
        .await?;
    for (i, line) in inv.lines.iter().enumerate() {
        sqlx::query(
            "INSERT INTO invoice_lines (invoice_id, description, quantity_hundredths, unit_rate_cents, taxable, sort_order) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(invoice_id)
        .bind(line.description.trim())
        .bind(line.quantity_hundredths)
        .bind(line.unit_rate_cents)
        .bind(line.taxable)
        .bind(i as i64)
        .execute(&mut *conn)
        .await?;
    }
    for t in &inv.tax_lines {
        sqlx::query("INSERT INTO invoice_tax_lines (invoice_id, label, rate_bps) VALUES (?, ?, ?)")
            .bind(invoice_id)
            .bind(t.label.trim())
            .bind(t.rate_bps)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

fn discount_columns(d: Option<NewDiscount>) -> (Option<&'static str>, Option<i64>) {
    match d {
        Some(NewDiscount::Percentage { bps }) => (Some("Percentage"), Some(bps)),
        Some(NewDiscount::Flat { cents }) => (Some("Flat"), Some(cents)),
        None => (None, None),
    }
}

async fn ensure_contact(conn: &mut SqliteConnection, contact_id: i64) -> Result<(), sqlx::Error> {
    let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM contacts WHERE id = ?")
        .bind(contact_id)
        .fetch_optional(&mut *conn)
        .await?;
    exists
        .map(|_| ())
        .ok_or_else(|| invalid(format!("contact {contact_id} does not exist")))
}

fn map_unique_number(e: sqlx::Error, number: &str) -> sqlx::Error {
    match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            invalid(format!("invoice number {number:?} is already in use"))
        }
        _ => e,
    }
}

/// Create a draft invoice with its line items, tax lines and discount.
pub async fn create_invoice(pool: &DbPool, inv: &NewInvoice) -> Result<i64, sqlx::Error> {
    validate_new_invoice(inv)?;
    let mut db_tx = pool.begin().await?;
    ensure_contact(&mut db_tx, inv.contact_id).await?;
    let (discount_type, discount_value) = discount_columns(inv.discount);
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO invoices (invoice_number, contact_id, status_type, issue_date, due_date, \
         discount_type, discount_value, notes, terms) VALUES (?, ?, 'Draft', ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(inv.invoice_number.trim())
    .bind(inv.contact_id)
    .bind(&inv.issue_date)
    .bind(&inv.due_date)
    .bind(discount_type)
    .bind(discount_value)
    .bind(&inv.notes)
    .bind(&inv.terms)
    .fetch_one(&mut *db_tx)
    .await
    .map_err(|e| map_unique_number(e, &inv.invoice_number))?;
    write_invoice_contents(&mut db_tx, id, inv).await?;
    db_tx.commit().await?;
    Ok(id)
}

/// Replace the contents of a draft invoice. Only `Draft` invoices can be edited;
/// once sent, an invoice is changed by voiding it and issuing a new one.
pub async fn update_draft_invoice(
    pool: &DbPool,
    id: i64,
    inv: &NewInvoice,
) -> Result<(), sqlx::Error> {
    validate_new_invoice(inv)?;
    let mut db_tx = pool.begin().await?;
    let status: Option<(String,)> = sqlx::query_as("SELECT status_type FROM invoices WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *db_tx)
        .await?;
    match status {
        None => return Err(sqlx::Error::RowNotFound),
        Some((s,)) if s != "Draft" => {
            return Err(invalid(format!(
                "only draft invoices can be edited (this one is {s})"
            )));
        }
        Some(_) => {}
    }
    ensure_contact(&mut db_tx, inv.contact_id).await?;
    let (discount_type, discount_value) = discount_columns(inv.discount);
    sqlx::query(
        "UPDATE invoices SET invoice_number = ?, contact_id = ?, issue_date = ?, due_date = ?, \
         discount_type = ?, discount_value = ?, notes = ?, terms = ?, updated_at = datetime('now') \
         WHERE id = ?",
    )
    .bind(inv.invoice_number.trim())
    .bind(inv.contact_id)
    .bind(&inv.issue_date)
    .bind(&inv.due_date)
    .bind(discount_type)
    .bind(discount_value)
    .bind(&inv.notes)
    .bind(&inv.terms)
    .bind(id)
    .execute(&mut *db_tx)
    .await
    .map_err(|e| map_unique_number(e, &inv.invoice_number))?;
    write_invoice_contents(&mut db_tx, id, inv).await?;
    db_tx.commit().await?;
    Ok(())
}

/// Void an invoice that has no payments recorded against it.
pub async fn void_invoice(pool: &DbPool, id: i64, reason: &str) -> Result<(), sqlx::Error> {
    let mut db_tx = pool.begin().await?;
    let status: Option<(String,)> = sqlx::query_as("SELECT status_type FROM invoices WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *db_tx)
        .await?;
    match status.as_ref().map(|s| s.0.as_str()) {
        None => return Err(sqlx::Error::RowNotFound),
        Some("Draft" | "Sent" | "Viewed") => {}
        Some(s) => return Err(invalid(format!("a {s} invoice cannot be voided"))),
    }
    let data = serde_json::json!({
        "voided_at": chrono::Utc::now().to_rfc3339(),
        "reason": reason,
    })
    .to_string();
    sqlx::query(
        "UPDATE invoices SET status_type = 'Void', status_data = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(data)
    .bind(id)
    .execute(&mut *db_tx)
    .await?;
    db_tx.commit().await?;
    Ok(())
}

/// Computed money totals of an invoice, in cents.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct InvoiceTotals {
    pub subtotal_cents: i64,
    pub discount_cents: i64,
    pub tax_cents: i64,
    pub total_cents: i64,
    pub paid_cents: i64,
    pub balance_cents: i64,
}

fn totals_for(
    rec: &InvoiceRecord,
    lines: &[InvoiceLineRecord],
    tax_lines: &[InvoiceTaxLineRecord],
    paid_cents: i64,
) -> Result<InvoiceTotals, sqlx::Error> {
    let invoice = invoice_from_records(rec, lines, tax_lines)?;
    let cents = |m: Money| {
        m.checked_to_cents()
            .ok_or_else(|| sqlx::Error::Protocol(format!("invoice {} total out of range", rec.id)))
    };
    let total_cents = cents(invoice.total())?;
    Ok(InvoiceTotals {
        subtotal_cents: cents(invoice.subtotal())?,
        discount_cents: cents(invoice.discount_amount())?,
        tax_cents: cents(invoice.tax_amount())?,
        total_cents,
        paid_cents,
        balance_cents: (total_cents - paid_cents).max(0),
    })
}

/// An invoice row with its computed totals, for list views.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InvoiceSummary {
    #[serde(flatten)]
    pub invoice: InvoiceRecord,
    #[serde(flatten)]
    pub totals: InvoiceTotals,
}

/// All invoices (newest first) with their totals and amounts paid.
pub async fn list_invoice_summaries(pool: &DbPool) -> Result<Vec<InvoiceSummary>, sqlx::Error> {
    let invoices = sqlx::query_as::<_, InvoiceRecord>(
        "SELECT * FROM invoices ORDER BY created_at DESC, id DESC LIMIT 5000",
    )
    .fetch_all(pool)
    .await?;
    let lines = sqlx::query_as::<_, InvoiceLineRecord>(
        "SELECT * FROM invoice_lines ORDER BY invoice_id, sort_order",
    )
    .fetch_all(pool)
    .await?;
    let tax_lines = sqlx::query_as::<_, InvoiceTaxLineRecord>("SELECT * FROM invoice_tax_lines")
        .fetch_all(pool)
        .await?;
    let paid: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT invoice_id, COALESCE(SUM(amount_cents), 0) FROM payments GROUP BY invoice_id",
    )
    .fetch_all(pool)
    .await?;
    let paid: std::collections::HashMap<i64, i64> = paid.into_iter().collect();

    invoices
        .into_iter()
        .map(|rec| {
            let l: Vec<_> = lines
                .iter()
                .filter(|l| l.invoice_id == rec.id)
                .cloned()
                .collect();
            let t: Vec<_> = tax_lines
                .iter()
                .filter(|t| t.invoice_id == rec.id)
                .cloned()
                .collect();
            let totals = totals_for(&rec, &l, &t, paid.get(&rec.id).copied().unwrap_or(0))?;
            Ok(InvoiceSummary {
                invoice: rec,
                totals,
            })
        })
        .collect()
}

/// One invoice with everything needed to display it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InvoiceDetail {
    #[serde(flatten)]
    pub invoice: InvoiceRecord,
    pub lines: Vec<InvoiceLineRecord>,
    pub tax_lines: Vec<InvoiceTaxLineRecord>,
    pub payments: Vec<PaymentRecord>,
    #[serde(flatten)]
    pub totals: InvoiceTotals,
}

pub async fn get_invoice_detail(
    pool: &DbPool,
    id: i64,
) -> Result<Option<InvoiceDetail>, sqlx::Error> {
    let Some(invoice) = crate::db::get_invoice_by_id(pool, id).await? else {
        return Ok(None);
    };
    let lines = crate::db::get_invoice_lines(pool, id).await?;
    let tax_lines = crate::db::get_invoice_tax_lines(pool, id).await?;
    let payments = crate::db::get_payments_for_invoice(pool, id).await?;
    let paid = payments.iter().map(|p| p.amount_cents).sum();
    let totals = totals_for(&invoice, &lines, &tax_lines, paid)?;
    Ok(Some(InvoiceDetail {
        invoice,
        lines,
        tax_lines,
        payments,
        totals,
    }))
}

/// Post an invoice payment to the ledger (cash basis): debit the deposit
/// account, credit Services Revenue, and credit Taxes Payable with the payment's
/// proportional share of the invoice's sales tax. Returns the transaction id.
pub(crate) async fn post_invoice_payment(
    conn: &mut SqliteConnection,
    rec: &InvoiceRecord,
    totals: &InvoiceTotals,
    amount_cents: i64,
    date: &str,
    method: Option<&str>,
    deposit_account_code: &str,
) -> Result<i64, sqlx::Error> {
    let deposit = postable_account(conn, deposit_account_code).await?;
    if deposit.account_type != "Asset" {
        return Err(invalid(format!(
            "deposit account {deposit_account_code} must be an asset account"
        )));
    }
    let income = postable_account(conn, INVOICE_INCOME_ACCOUNT).await?;

    // Share of this payment that is sales tax collected (a liability, not income).
    let tax_cents = if totals.total_cents > 0 && totals.tax_cents > 0 {
        let num = i128::from(amount_cents) * i128::from(totals.tax_cents);
        let den = i128::from(totals.total_cents);
        ((num * 2 + den) / (den * 2)) as i64 // round half up
    } else {
        0
    };
    let mut lines = vec![(deposit.id, amount_cents, 0)];
    if amount_cents - tax_cents > 0 {
        lines.push((income.id, 0, amount_cents - tax_cents));
    }
    if tax_cents > 0 {
        let tax = postable_account(conn, SALES_TAX_PAYABLE_ACCOUNT).await?;
        lines.push((tax.id, 0, tax_cents));
    }

    let date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| invalid("payment date must be YYYY-MM-DD"))?;
    let tx = build_transaction(
        date,
        format!("Payment received: invoice {}", rec.invoice_number),
        method.map(|m| format!("Paid by {m}")),
        &lines,
    )?;
    insert_ledger_transaction(conn, &tx, Some(rec.contact_id)).await
}

pub(crate) fn invoice_totals_for_payment(
    rec: &InvoiceRecord,
    lines: &[InvoiceLineRecord],
    tax_lines: &[InvoiceTaxLineRecord],
    already_paid: i64,
) -> Result<InvoiceTotals, sqlx::Error> {
    totals_for(rec, lines, tax_lines, already_paid)
}

// ── Imported transactions ────────────────────────────────────────────────────

/// Imported transactions still awaiting review (pending or categorized), oldest first.
pub async fn get_pending_imports(
    pool: &DbPool,
    limit: i64,
) -> Result<Vec<ImportedTransaction>, sqlx::Error> {
    sqlx::query_as::<_, ImportedTransaction>(
        "SELECT * FROM imported_transactions WHERE status IN ('pending', 'categorized') \
         ORDER BY date, id LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Post a reviewed import to the ledger as a two-line transaction between the
/// bank/card account it came from and a category account.
///
/// Sign convention: negative `amount_cents` is money out of `bank_account_code`
/// (debit category, credit bank); positive is money in. When
/// `category_account_code` is `None`, the account of the categorization rule
/// the import was matched to is used. Returns the new transaction id.
pub async fn post_imported_transaction(
    pool: &DbPool,
    imported_id: i64,
    bank_account_code: &str,
    category_account_code: Option<&str>,
    contact_id: Option<i64>,
) -> Result<i64, sqlx::Error> {
    let mut db_tx = pool.begin().await?;
    let row = sqlx::query_as::<_, ImportedTransaction>(
        "SELECT * FROM imported_transactions WHERE id = ?",
    )
    .bind(imported_id)
    .fetch_optional(&mut *db_tx)
    .await?
    .ok_or(sqlx::Error::RowNotFound)?;
    match row.status.as_str() {
        "pending" | "categorized" => {}
        "matched" => {
            return Err(invalid(
                "this import matches an existing ledger transaction; posting it would duplicate it",
            ));
        }
        s => return Err(invalid(format!("this import is already {s}"))),
    }
    if row.amount_cents == 0 {
        return Err(invalid("cannot post a zero-amount import"));
    }

    let bank = postable_account(&mut db_tx, bank_account_code).await?;
    if bank.account_type != "Asset" && bank.account_type != "Liability" {
        return Err(invalid(format!(
            "account {bank_account_code} must be a bank (asset) or card (liability) account"
        )));
    }
    let category = match category_account_code {
        Some(code) => postable_account(&mut db_tx, code).await?,
        None => {
            let rule_account: Option<(String,)> = sqlx::query_as(
                "SELECT a.code FROM categorization_rules r JOIN accounts a ON a.id = r.account_id \
                 WHERE r.id = ?",
            )
            .bind(row.category_rule_id)
            .fetch_optional(&mut *db_tx)
            .await?;
            let Some((code,)) = rule_account else {
                return Err(invalid(
                    "no category account given and the import has no categorization rule",
                ));
            };
            postable_account(&mut db_tx, &code).await?
        }
    };
    if category.id == bank.id {
        return Err(invalid("category and bank account must differ"));
    }

    let amount = row.amount_cents.unsigned_abs() as i64;
    let lines = if row.amount_cents < 0 {
        [(category.id, amount, 0), (bank.id, 0, amount)]
    } else {
        [(bank.id, amount, 0), (category.id, 0, amount)]
    };
    let date = chrono::NaiveDate::parse_from_str(&row.date, "%Y-%m-%d")
        .map_err(|_| invalid(format!("import {imported_id} has an invalid date")))?;
    let tx = build_transaction(date, row.description.clone(), row.memo.clone(), &lines)?;
    let tx_id = insert_ledger_transaction(&mut db_tx, &tx, contact_id).await?;

    sqlx::query(
        "UPDATE imported_transactions SET status = 'posted', matched_transaction_id = ? WHERE id = ?",
    )
    .bind(tx_id)
    .bind(imported_id)
    .execute(&mut *db_tx)
    .await?;
    db_tx.commit().await?;
    Ok(tx_id)
}

/// Dismiss an import (e.g. a transfer already recorded another way).
pub async fn ignore_imported_transaction(
    pool: &DbPool,
    imported_id: i64,
) -> Result<(), sqlx::Error> {
    let result = sqlx::query(
        "UPDATE imported_transactions SET status = 'ignored' \
         WHERE id = ? AND status IN ('pending', 'categorized', 'matched')",
    )
    .bind(imported_id)
    .execute(pool)
    .await?;
    if result.rows_affected() > 0 {
        return Ok(());
    }
    let exists: Option<(String,)> =
        sqlx::query_as("SELECT status FROM imported_transactions WHERE id = ?")
            .bind(imported_id)
            .fetch_optional(pool)
            .await?;
    match exists {
        None => Err(sqlx::Error::RowNotFound),
        Some((s,)) => Err(invalid(format!("this import is already {s}"))),
    }
}

// ── Receipts ─────────────────────────────────────────────────────────────────

/// Fields read off a receipt (by OCR or an AI agent).
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ReceiptFields {
    pub vendor: Option<String>,
    pub receipt_date: Option<String>,
    pub total_cents: Option<i64>,
    pub subtotal_cents: Option<i64>,
    pub tax_cents: Option<i64>,
    pub payment_method: Option<String>,
}

/// Set the extracted fields of a receipt that is still pending review.
/// `confidence` is clamped to 0..=1.
pub async fn update_receipt_fields(
    pool: &DbPool,
    receipt_id: i64,
    fields: &ReceiptFields,
    confidence: f64,
) -> Result<(), sqlx::Error> {
    if let Some(d) = &fields.receipt_date {
        validate_date("receipt_date", d)?;
    }
    for (name, v) in [
        ("total_cents", fields.total_cents),
        ("subtotal_cents", fields.subtotal_cents),
        ("tax_cents", fields.tax_cents),
    ] {
        if v.is_some_and(|c| c.abs() > MAX_LINE_CENTS) {
            return Err(invalid(format!("{name} is out of range")));
        }
    }
    if !confidence.is_finite() {
        return Err(invalid("confidence must be a number between 0 and 1"));
    }
    let result = sqlx::query(
        "UPDATE receipts SET vendor = ?, receipt_date = ?, total_cents = ?, subtotal_cents = ?, \
         tax_cents = ?, payment_method = ?, confidence = ? WHERE id = ? AND status = 'pending_review'",
    )
    .bind(fields.vendor.as_deref().map(str::trim).filter(|v| !v.is_empty()))
    .bind(&fields.receipt_date)
    .bind(fields.total_cents)
    .bind(fields.subtotal_cents)
    .bind(fields.tax_cents)
    .bind(fields.payment_method.as_deref().map(str::trim).filter(|v| !v.is_empty()))
    .bind(confidence.clamp(0.0, 1.0))
    .bind(receipt_id)
    .execute(pool)
    .await?;
    if result.rows_affected() > 0 {
        return Ok(());
    }
    match crate::db::get_receipt_by_id(pool, receipt_id).await? {
        None => Err(sqlx::Error::RowNotFound),
        Some(r) => Err(invalid(format!(
            "receipt {receipt_id} is {} and can no longer be edited",
            r.status
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{
        ImportedTransaction, create_db, get_account_by_code, get_invoice_by_id,
        insert_imported_transaction, insert_payment, insert_receipt, save_categorization_rule,
        seed_default_accounts,
    };

    async fn pool() -> DbPool {
        let pool = create_db(std::path::Path::new(":memory:")).await.unwrap();
        seed_default_accounts(&pool).await.unwrap();
        pool
    }

    async fn contact(pool: &DbPool) -> i64 {
        crate::db::insert_contact(pool, "Acme", None, None, None, "Client", false, None, None)
            .await
            .unwrap()
    }

    fn new_invoice(contact_id: i64, number: &str) -> NewInvoice {
        NewInvoice {
            invoice_number: number.into(),
            contact_id,
            issue_date: "2026-03-01".into(),
            due_date: "2026-03-31".into(),
            lines: vec![
                NewInvoiceLine {
                    description: "Consulting".into(),
                    quantity_hundredths: 1000, // 10 hours
                    unit_rate_cents: 10000,    // $100/h
                    taxable: false,
                },
                NewInvoiceLine {
                    description: "Hardware".into(),
                    quantity_hundredths: 100,
                    unit_rate_cents: 20000,
                    taxable: true,
                },
            ],
            tax_lines: vec![NewInvoiceTaxLine {
                label: "Sales tax".into(),
                rate_bps: 1000, // 10%
            }],
            discount: None,
            notes: None,
            terms: Some("Net 30".into()),
        }
    }

    /// (account code, debit, credit) for every line of a transaction.
    async fn lines_of(pool: &DbPool, tx_id: i64) -> Vec<(String, i64, i64)> {
        sqlx::query_as(
            "SELECT a.code, tl.debit_cents, tl.credit_cents FROM transaction_lines tl \
             JOIN accounts a ON a.id = tl.account_id WHERE tl.transaction_id = ? ORDER BY tl.id",
        )
        .bind(tx_id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn invoice_totals_include_lines_tax_and_discount() {
        let pool = pool().await;
        let c = contact(&pool).await;
        let mut inv = new_invoice(c, "INV-1");
        inv.discount = Some(NewDiscount::Percentage { bps: 1000 });
        let id = create_invoice(&pool, &inv).await.unwrap();

        let detail = get_invoice_detail(&pool, id).await.unwrap().unwrap();
        assert_eq!(detail.invoice.status_type, "Draft");
        assert_eq!(detail.lines.len(), 2);
        // $1,000 + $200 = $1,200; 10% off = $120; taxable $200 - 10% = $180 → $18 tax.
        assert_eq!(detail.totals.subtotal_cents, 120000);
        assert_eq!(detail.totals.discount_cents, 12000);
        assert_eq!(detail.totals.tax_cents, 1800);
        assert_eq!(detail.totals.total_cents, 109800);
        assert_eq!(detail.totals.balance_cents, 109800);

        let list = list_invoice_summaries(&pool).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].totals.total_cents, 109800);
    }

    #[tokio::test]
    async fn invoice_validation() {
        let pool = pool().await;
        let c = contact(&pool).await;
        let mut inv = new_invoice(c, "INV-1");
        inv.lines.clear();
        assert!(create_invoice(&pool, &inv).await.is_err());

        let mut inv = new_invoice(c, "INV-1");
        inv.tax_lines[0].rate_bps = 20_000;
        assert!(create_invoice(&pool, &inv).await.is_err());

        let mut inv = new_invoice(c, "INV-1");
        inv.lines[0].quantity_hundredths = 0;
        assert!(create_invoice(&pool, &inv).await.is_err());

        assert!(
            create_invoice(&pool, &new_invoice(999, "INV-1"))
                .await
                .is_err()
        );

        create_invoice(&pool, &new_invoice(c, "INV-1"))
            .await
            .unwrap();
        let dup = create_invoice(&pool, &new_invoice(c, "INV-1")).await;
        assert!(
            matches!(dup, Err(sqlx::Error::InvalidArgument(m)) if m.contains("already in use"))
        );
    }

    #[tokio::test]
    async fn only_drafts_can_be_edited_and_paid_invoices_not_voided() {
        let pool = pool().await;
        let c = contact(&pool).await;
        let id = create_invoice(&pool, &new_invoice(c, "INV-1"))
            .await
            .unwrap();

        let mut edited = new_invoice(c, "INV-1");
        edited.lines.truncate(1);
        update_draft_invoice(&pool, id, &edited).await.unwrap();
        let detail = get_invoice_detail(&pool, id).await.unwrap().unwrap();
        assert_eq!(detail.lines.len(), 1);
        assert_eq!(detail.totals.total_cents, 100000);

        crate::db::mark_invoice_sent(&pool, id, "2026-03-01T00:00:00Z")
            .await
            .unwrap();
        assert!(update_draft_invoice(&pool, id, &edited).await.is_err());

        insert_payment(&pool, id, 1000, "2026-03-05", None, None)
            .await
            .unwrap();
        assert!(void_invoice(&pool, id, "mistake").await.is_err());

        let other = create_invoice(&pool, &new_invoice(c, "INV-2"))
            .await
            .unwrap();
        void_invoice(&pool, other, "duplicate").await.unwrap();
        let rec = get_invoice_by_id(&pool, other).await.unwrap().unwrap();
        assert_eq!(rec.status_type, "Void");
    }

    #[tokio::test]
    async fn payments_post_cash_basis_with_sales_tax_split() {
        let pool = pool().await;
        let c = contact(&pool).await;
        // $1,000 consulting + $200 taxable hardware + $20 tax = $1,220.
        let id = create_invoice(&pool, &new_invoice(c, "INV-1"))
            .await
            .unwrap();
        crate::db::mark_invoice_sent(&pool, id, "2026-03-01T00:00:00Z")
            .await
            .unwrap();

        // Pay half into Savings.
        let pid = insert_payment(&pool, id, 61000, "2026-03-10", Some("ACH"), Some("1010"))
            .await
            .unwrap();
        let (tx_id,): (Option<i64>,) =
            sqlx::query_as("SELECT transaction_id FROM payments WHERE id = ?")
                .bind(pid)
                .fetch_one(&pool)
                .await
                .unwrap();
        let tx_id = tx_id.expect("payment is linked to its ledger entry");
        assert_eq!(
            lines_of(&pool, tx_id).await,
            vec![
                ("1010".to_string(), 61000, 0),
                ("4000".to_string(), 0, 60000),
                ("2010".to_string(), 0, 1000),
            ]
        );
        let (contact_id,): (Option<i64>,) =
            sqlx::query_as("SELECT contact_id FROM transactions WHERE id = ?")
                .bind(tx_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(contact_id, Some(c));

        // A deposit account must be an asset; nothing is recorded on failure.
        assert!(
            insert_payment(&pool, id, 100, "2026-03-11", None, Some("4000"))
                .await
                .is_err()
        );
        let detail = get_invoice_detail(&pool, id).await.unwrap().unwrap();
        assert_eq!(detail.payments.len(), 1);
        assert_eq!(detail.invoice.status_type, "PartiallyPaid");
    }

    async fn import(pool: &DbPool, amount_cents: i64) -> i64 {
        insert_imported_transaction(
            pool,
            &ImportedTransaction {
                id: 0,
                source_type: "plaid".into(),
                source_id: Some(format!("p{amount_cents}")),
                import_batch_id: "b1".into(),
                date: "2026-04-02".into(),
                description: "COFFEE SHOP".into(),
                amount_cents,
                debit_cents: None,
                credit_cents: None,
                memo: None,
                matched_transaction_id: None,
                category_rule_id: None,
                status: "pending".into(),
                created_at: String::new(),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn posting_imports_follows_the_sign_convention() {
        let pool = pool().await;
        let out = import(&pool, -450).await;
        let inn = import(&pool, 10000).await;
        assert_eq!(get_pending_imports(&pool, 10).await.unwrap().len(), 2);

        let tx = post_imported_transaction(&pool, out, "1000", Some("5020"), None)
            .await
            .unwrap();
        assert_eq!(
            lines_of(&pool, tx).await,
            vec![("5020".to_string(), 450, 0), ("1000".to_string(), 0, 450)]
        );
        let tx = post_imported_transaction(&pool, inn, "1000", Some("4020"), None)
            .await
            .unwrap();
        assert_eq!(
            lines_of(&pool, tx).await,
            vec![
                ("1000".to_string(), 10000, 0),
                ("4020".to_string(), 0, 10000)
            ]
        );

        // Posted imports leave the queue and can't be posted twice.
        assert!(get_pending_imports(&pool, 10).await.unwrap().is_empty());
        assert!(
            post_imported_transaction(&pool, out, "1000", Some("5020"), None)
                .await
                .is_err()
        );
        assert!(matches!(
            post_imported_transaction(&pool, 999, "1000", Some("5020"), None).await,
            Err(sqlx::Error::RowNotFound)
        ));
    }

    #[tokio::test]
    async fn posting_uses_the_rule_account_and_ignore_dismisses() {
        let pool = pool().await;
        let meals = get_account_by_code(&pool, "5020")
            .await
            .unwrap()
            .unwrap()
            .id
            .unwrap();
        let rule = save_categorization_rule(
            &pool,
            &crate::db::CategorizationRule {
                id: 0,
                name: "Coffee".into(),
                priority: 1,
                match_pattern: "COFFEE".into(),
                match_type: "contains".into(),
                account_id: meals.0,
                created_at: String::new(),
            },
        )
        .await
        .unwrap();
        let id = import(&pool, -450).await;
        // No rule yet and no category given.
        assert!(
            post_imported_transaction(&pool, id, "1000", None, None)
                .await
                .is_err()
        );
        crate::db::mark_imported_transaction_categorized(&pool, id, rule)
            .await
            .unwrap();
        let tx = post_imported_transaction(&pool, id, "1000", None, None)
            .await
            .unwrap();
        assert_eq!(lines_of(&pool, tx).await[0].0, "5020");

        let other = import(&pool, -100).await;
        ignore_imported_transaction(&pool, other).await.unwrap();
        assert!(get_pending_imports(&pool, 10).await.unwrap().is_empty());
        assert!(ignore_imported_transaction(&pool, other).await.is_err());
    }

    #[tokio::test]
    async fn receipt_fields_can_be_set_while_pending() {
        let pool = pool().await;
        let id = insert_receipt(
            &pool,
            "abc",
            "jpg",
            "/tmp/a.jpg",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            0.0,
        )
        .await
        .unwrap();
        let fields = ReceiptFields {
            vendor: Some("  Blue Bottle ".into()),
            receipt_date: Some("2026-04-02".into()),
            total_cents: Some(650),
            subtotal_cents: Some(600),
            tax_cents: Some(50),
            payment_method: Some("Visa".into()),
        };
        update_receipt_fields(&pool, id, &fields, 1.7)
            .await
            .unwrap();
        let r = crate::db::get_receipt_by_id(&pool, id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.vendor.as_deref(), Some("Blue Bottle"));
        assert_eq!(r.total_cents, Some(650));
        assert_eq!(r.confidence, 1.0);

        let bad = ReceiptFields {
            receipt_date: Some("4/2/2026".into()),
            ..fields.clone()
        };
        assert!(update_receipt_fields(&pool, id, &bad, 0.9).await.is_err());

        crate::db::update_receipt_status(&pool, id, "approved")
            .await
            .unwrap();
        assert!(
            update_receipt_fields(&pool, id, &fields, 0.9)
                .await
                .is_err()
        );
        assert!(matches!(
            update_receipt_fields(&pool, 999, &fields, 0.9).await,
            Err(sqlx::Error::RowNotFound)
        ));
    }
}
