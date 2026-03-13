//! Import transactions from Actual Budget JSON export.
//!
//! Actual Budget exports data as a SQLite database or JSON. This module
//! handles the JSON export format, converting Actual accounts and
//! transactions into aequi's domain types.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ActualImportError {
    #[error("JSON parse error: {0}")]
    Parse(String),
    #[error("missing required field: {0}")]
    MissingField(String),
    #[error("invalid date: {0}")]
    InvalidDate(String),
}

// ── Actual Budget JSON format ──────────────────────────────────────────────

/// An Actual Budget account from the export.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActualAccount {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub account_type: Option<String>,
    #[serde(default, alias = "offbudget")]
    pub off_budget: bool,
    #[serde(default)]
    pub closed: bool,
    /// Balance in cents (Actual uses integer amounts)
    #[serde(default)]
    pub balance: i64,
}

/// An Actual Budget transaction from the export.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActualTransaction {
    pub id: String,
    pub account: String,
    /// Date as YYYY-MM-DD string or integer (Actual uses YYYYMMDD ints)
    pub date: serde_json::Value,
    #[serde(default)]
    pub payee: Option<String>,
    #[serde(default)]
    pub payee_name: Option<String>,
    /// Amount in cents (negative = outflow, positive = inflow)
    pub amount: i64,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub category_name: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub cleared: bool,
    #[serde(default)]
    pub transfer_id: Option<String>,
}

/// Complete Actual Budget export.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActualExport {
    #[serde(default)]
    pub accounts: Vec<ActualAccount>,
    #[serde(default)]
    pub transactions: Vec<ActualTransaction>,
    #[serde(default)]
    pub categories: Vec<ActualCategory>,
    #[serde(default)]
    pub payees: Vec<ActualPayee>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActualCategory {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActualPayee {
    pub id: String,
    pub name: String,
}

// ── Converted types ────────────────────────────────────────────────────────

/// An imported transaction ready for aequi ingestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedTransaction {
    pub date: NaiveDate,
    pub description: String,
    pub amount_cents: i64,
    pub memo: Option<String>,
    pub source_account: String,
    pub category: Option<String>,
    pub is_transfer: bool,
}

/// Summary of an import operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSummary {
    pub accounts_found: usize,
    pub transactions_imported: usize,
    pub transfers_skipped: usize,
    pub errors: Vec<String>,
}

// ── Import logic ───────────────────────────────────────────────────────────

/// Parse an Actual Budget JSON export.
pub fn parse_export(json_str: &str) -> Result<ActualExport, ActualImportError> {
    serde_json::from_str(json_str).map_err(|e| ActualImportError::Parse(e.to_string()))
}

/// Parse an Actual date value (string "YYYY-MM-DD" or integer YYYYMMDD).
fn parse_actual_date(value: &serde_json::Value) -> Result<NaiveDate, ActualImportError> {
    match value {
        serde_json::Value::String(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|e| ActualImportError::InvalidDate(format!("{s}: {e}"))),
        serde_json::Value::Number(n) => {
            let d = n
                .as_i64()
                .ok_or_else(|| ActualImportError::InvalidDate(format!("{n}")))?;
            if !(10000101..=99991231).contains(&d) {
                return Err(ActualImportError::InvalidDate(format!("{d}")));
            }
            let year = (d / 10000) as i32;
            let month = ((d % 10000) / 100) as u32;
            let day = (d % 100) as u32;
            NaiveDate::from_ymd_opt(year, month, day)
                .ok_or_else(|| ActualImportError::InvalidDate(format!("{d}")))
        }
        _ => Err(ActualImportError::InvalidDate("unexpected type".into())),
    }
}

/// Convert an Actual export into importable transactions.
///
/// Resolves payee and category names from their respective ID lookups.
/// Transfer transactions (those with `transfer_id`) are skipped by default
/// to avoid double-counting.
pub fn convert_transactions(
    export: &ActualExport,
    skip_transfers: bool,
) -> (Vec<ImportedTransaction>, ImportSummary) {
    let payee_map: std::collections::HashMap<&str, &str> = export
        .payees
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str()))
        .collect();

    let category_map: std::collections::HashMap<&str, &str> = export
        .categories
        .iter()
        .map(|c| (c.id.as_str(), c.name.as_str()))
        .collect();

    let account_map: std::collections::HashMap<&str, &str> = export
        .accounts
        .iter()
        .map(|a| (a.id.as_str(), a.name.as_str()))
        .collect();

    let mut imported = Vec::new();
    let mut errors = Vec::new();
    let mut transfers_skipped = 0;

    for tx in &export.transactions {
        if skip_transfers && tx.transfer_id.is_some() {
            transfers_skipped += 1;
            continue;
        }

        let date = match parse_actual_date(&tx.date) {
            Ok(d) => d,
            Err(e) => {
                errors.push(format!("tx {}: {e}", tx.id));
                continue;
            }
        };

        // Resolve payee name
        let description = tx
            .payee_name
            .as_deref()
            .or_else(|| {
                tx.payee
                    .as_deref()
                    .and_then(|id| payee_map.get(id).copied())
            })
            .unwrap_or("Unknown Payee")
            .to_string();

        let source_account = account_map
            .get(tx.account.as_str())
            .unwrap_or(&tx.account.as_str())
            .to_string();

        let category = tx
            .category_name
            .as_deref()
            .or_else(|| {
                tx.category
                    .as_deref()
                    .and_then(|id| category_map.get(id).copied())
            })
            .map(String::from);

        imported.push(ImportedTransaction {
            date,
            description,
            amount_cents: tx.amount,
            memo: tx.notes.clone(),
            source_account,
            category,
            is_transfer: tx.transfer_id.is_some(),
        });
    }

    let summary = ImportSummary {
        accounts_found: export.accounts.len(),
        transactions_imported: imported.len(),
        transfers_skipped,
        errors,
    };

    (imported, summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_export_json() -> &'static str {
        r#"{
            "accounts": [
                {"id": "acc1", "name": "Checking", "balance": 150000},
                {"id": "acc2", "name": "Savings", "balance": 500000, "off_budget": true}
            ],
            "payees": [
                {"id": "pay1", "name": "Starbucks"},
                {"id": "pay2", "name": "GitHub"}
            ],
            "categories": [
                {"id": "cat1", "name": "Food & Drink"},
                {"id": "cat2", "name": "Software"}
            ],
            "transactions": [
                {
                    "id": "tx1",
                    "account": "acc1",
                    "date": "2026-03-01",
                    "payee": "pay1",
                    "amount": -550,
                    "category": "cat1",
                    "notes": "Morning coffee",
                    "cleared": true
                },
                {
                    "id": "tx2",
                    "account": "acc1",
                    "date": 20260305,
                    "payee": "pay2",
                    "amount": -1100,
                    "category": "cat2",
                    "cleared": true
                },
                {
                    "id": "tx3",
                    "account": "acc1",
                    "date": "2026-03-10",
                    "payee_name": "Client Payment",
                    "amount": 250000,
                    "cleared": false
                },
                {
                    "id": "tx4",
                    "account": "acc1",
                    "date": "2026-03-12",
                    "payee_name": "Transfer to Savings",
                    "amount": -100000,
                    "transfer_id": "tx4b"
                }
            ]
        }"#
    }

    #[test]
    fn parse_actual_export() {
        let export = parse_export(sample_export_json()).unwrap();
        assert_eq!(export.accounts.len(), 2);
        assert_eq!(export.transactions.len(), 4);
        assert_eq!(export.payees.len(), 2);
        assert_eq!(export.categories.len(), 2);
    }

    #[test]
    fn convert_resolves_names() {
        let export = parse_export(sample_export_json()).unwrap();
        let (txs, summary) = convert_transactions(&export, true);

        assert_eq!(summary.accounts_found, 2);
        assert_eq!(summary.transactions_imported, 3);
        assert_eq!(summary.transfers_skipped, 1);
        assert!(summary.errors.is_empty());

        assert_eq!(txs[0].description, "Starbucks");
        assert_eq!(txs[0].amount_cents, -550);
        assert_eq!(txs[0].category.as_deref(), Some("Food & Drink"));
        assert_eq!(txs[0].memo.as_deref(), Some("Morning coffee"));

        assert_eq!(txs[1].description, "GitHub");
        assert_eq!(txs[1].amount_cents, -1100);

        assert_eq!(txs[2].description, "Client Payment");
        assert_eq!(txs[2].amount_cents, 250000);
    }

    #[test]
    fn integer_date_format_parsed() {
        let export = parse_export(sample_export_json()).unwrap();
        let (txs, _) = convert_transactions(&export, true);

        // tx2 has date as integer 20260305
        assert_eq!(txs[1].date, NaiveDate::from_ymd_opt(2026, 3, 5).unwrap());
    }

    #[test]
    fn transfers_included_when_not_skipped() {
        let export = parse_export(sample_export_json()).unwrap();
        let (txs, summary) = convert_transactions(&export, false);

        assert_eq!(summary.transactions_imported, 4);
        assert_eq!(summary.transfers_skipped, 0);
        assert!(txs[3].is_transfer);
    }

    #[test]
    fn empty_export() {
        let json = r#"{"accounts":[],"transactions":[],"categories":[],"payees":[]}"#;
        let export = parse_export(json).unwrap();
        let (txs, summary) = convert_transactions(&export, true);
        assert_eq!(txs.len(), 0);
        assert_eq!(summary.accounts_found, 0);
    }

    #[test]
    fn invalid_json_returns_error() {
        assert!(parse_export("not json").is_err());
    }

    #[test]
    fn account_properties() {
        let export = parse_export(sample_export_json()).unwrap();
        assert_eq!(export.accounts[0].name, "Checking");
        assert!(!export.accounts[0].off_budget);
        assert!(export.accounts[1].off_budget);
    }

    #[test]
    fn source_account_resolved() {
        let export = parse_export(sample_export_json()).unwrap();
        let (txs, _) = convert_transactions(&export, true);
        assert_eq!(txs[0].source_account, "Checking");
    }
}
