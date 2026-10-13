use std::collections::HashSet;
use std::path::PathBuf;

use serde_json::json;

use crate::permissions::Permissions;
use crate::protocol::ToolResult;
use crate::tools::ToolRegistry;

async fn test_db() -> aequi_storage::DbPool {
    let db = aequi_storage::create_db(&PathBuf::from(":memory:"))
        .await
        .unwrap();
    aequi_storage::seed_default_accounts(&db).await.unwrap();
    db
}

// -----------------------------------------------------------------------
// Registry tests
// -----------------------------------------------------------------------

#[test]
fn registry_has_all_expected_tools() {
    let registry = ToolRegistry::new();
    let names: Vec<_> = registry
        .list_definitions()
        .iter()
        .map(|d| d.name.as_str())
        .collect();

    let expected = [
        "aequi_list_accounts",
        "aequi_get_account",
        "aequi_get_transactions",
        "aequi_create_transaction",
        "aequi_get_profit_loss",
        "aequi_ingest_receipt",
        "aequi_get_pending_receipts",
        "aequi_reject_receipt",
        "aequi_estimate_quarterly_tax",
        "aequi_get_schedule_c_preview",
        "aequi_record_tax_payment",
        "aequi_list_unpaid_invoices",
        "aequi_draft_invoice",
        "aequi_record_payment",
        "aequi_get_categorization_rules",
        "aequi_save_categorization_rule",
        "aequi_apply_rules",
        "aequi_get_import_profiles",
        "aequi_save_import_profile",
        "aequi_get_pending_imports",
        "aequi_create_reconciliation_session",
        "aequi_get_reconciliation_items",
        "aequi_resolve_item",
        "aequi_get_receipt_image",
        "aequi_update_receipt_fields",
        "aequi_get_invoice",
        "aequi_list_contacts",
        "aequi_post_import",
        "aequi_ignore_import",
        "aequi_get_1099_summary",
        "aequi_get_tax_return",
        "aequi_set_tax_return",
    ];

    for name in expected {
        assert!(names.contains(&name), "Missing tool: {name}");
    }
}

#[test]
fn registry_tool_count() {
    let registry = ToolRegistry::new();
    assert_eq!(registry.list_definitions().len(), 32);
}

#[test]
fn all_tools_have_descriptions() {
    let registry = ToolRegistry::new();
    for def in registry.list_definitions() {
        assert!(
            !def.description.is_empty(),
            "{} has empty description",
            def.name
        );
    }
}

#[test]
fn all_tools_have_input_schemas() {
    let registry = ToolRegistry::new();
    for def in registry.list_definitions() {
        assert!(
            def.input_schema.is_object(),
            "{} input_schema is not an object",
            def.name
        );
    }
}

#[test]
fn no_duplicate_tool_names() {
    let registry = ToolRegistry::new();
    let names: Vec<_> = registry
        .list_definitions()
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    let unique: HashSet<_> = names.iter().collect();
    assert_eq!(names.len(), unique.len(), "Duplicate tool names found");
}

// -----------------------------------------------------------------------
// Permission tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn unknown_tool_returns_error() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();
    let result = registry.call("nonexistent", json!({}), &db, &perms).await;
    assert!(result.is_error.unwrap_or(false));
}

#[tokio::test]
async fn read_only_blocks_write_tool() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions {
        read_only: true,
        disabled_tools: HashSet::new(),
    };
    let result = registry
        .call("aequi_create_transaction", json!({}), &db, &perms)
        .await;
    assert!(result.is_error.unwrap_or(false));
    assert!(result.content[0].as_text().unwrap().contains("not allowed"));
}

#[tokio::test]
async fn disabled_tool_blocked() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let mut disabled = HashSet::new();
    disabled.insert("aequi_list_accounts".to_string());
    let perms = Permissions {
        read_only: false,
        disabled_tools: disabled,
    };
    let result = registry
        .call("aequi_list_accounts", json!({}), &db, &perms)
        .await;
    assert!(result.is_error.unwrap_or(false));
}

// -----------------------------------------------------------------------
// Account tool tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn list_accounts_returns_default_accounts() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();
    let result = registry
        .call("aequi_list_accounts", json!({}), &db, &perms)
        .await;
    assert!(result.is_error.is_none());
    let text = &result.content[0].as_text().unwrap();
    assert!(text.contains("Checking"));
    assert!(text.contains("1000"));
}

#[tokio::test]
async fn get_account_by_code() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();
    let result = registry
        .call("aequi_get_account", json!({ "code": "1000" }), &db, &perms)
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].as_text().unwrap().contains("Checking"));
}

#[tokio::test]
async fn get_account_not_found() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();
    let result = registry
        .call("aequi_get_account", json!({ "code": "9999" }), &db, &perms)
        .await;
    assert!(result.is_error.unwrap_or(false));
    assert!(result.content[0].as_text().unwrap().contains("not found"));
}

// -----------------------------------------------------------------------
// Transaction tool tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn create_and_get_transaction() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_create_transaction",
            json!({
                "date": "2026-03-01",
                "description": "Client payment",
                "lines": [
                    { "account_code": "1000", "debit_cents": 50000, "credit_cents": 0 },
                    { "account_code": "4000", "debit_cents": 0, "credit_cents": 50000 }
                ]
            }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].as_text().unwrap().contains("created"));

    let list = registry
        .call(
            "aequi_get_transactions",
            json!({ "limit": 10 }),
            &db,
            &perms,
        )
        .await;
    assert!(list.is_error.is_none());
    assert!(
        list.content[0]
            .as_text()
            .unwrap()
            .contains("Client payment")
    );
}

#[tokio::test]
async fn create_transaction_unbalanced_fails() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_create_transaction",
            json!({
                "date": "2026-03-01",
                "description": "Bad",
                "lines": [
                    { "account_code": "1000", "debit_cents": 100, "credit_cents": 0 },
                    { "account_code": "4000", "debit_cents": 0, "credit_cents": 200 }
                ]
            }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.unwrap_or(false));
}

#[tokio::test]
async fn create_transaction_invalid_date_fails() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_create_transaction",
            json!({
                "date": "not-a-date",
                "description": "Bad",
                "lines": [
                    { "account_code": "1000", "debit_cents": 100, "credit_cents": 0 },
                    { "account_code": "4000", "debit_cents": 0, "credit_cents": 100 }
                ]
            }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.unwrap_or(false));
    assert!(result.content[0].as_text().unwrap().contains("YYYY-MM-DD"));
}

#[tokio::test]
async fn profit_loss_empty_ledger() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_get_profit_loss",
            json!({ "start_date": "2026-01-01", "end_date": "2026-12-31" }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    let text = &result.content[0].as_text().unwrap();
    assert!(text.contains("net_profit_cents"));
}

// -----------------------------------------------------------------------
// Receipt tool tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn pending_receipts_empty() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call("aequi_get_pending_receipts", json!({}), &db, &perms)
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].as_text().unwrap().contains("[]"));
}

#[tokio::test]
async fn ingest_receipt_file_not_found() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_ingest_receipt",
            json!({ "file_path": "/nonexistent/receipt.jpg" }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.unwrap_or(false));
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("Cannot read file")
    );
}

// -----------------------------------------------------------------------
// Tax tool tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn estimate_quarterly_tax_empty_ledger() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_estimate_quarterly_tax",
            json!({ "year": 2026, "quarter": 1 }),
            &db,
            &perms,
        )
        .await;
    assert!(
        result.is_error.is_none(),
        "{}",
        result.content[0].as_text().unwrap()
    );
    let text = &result.content[0].as_text().unwrap();
    for field in [
        "quarterly_payment",
        "annualized_net_profit",
        "standard_deduction",
        "taxable_income",
        "additional_medicare_tax",
        "through",
    ] {
        assert!(text.contains(field), "missing {field}");
    }
    let v = parse(&result);
    assert_eq!(v["qbi_deduction_cents"], 0);
    assert_eq!(v["safe_harbor_prior_year_percentage"], "1.00");
    let note = v["safe_harbor_note"].as_str().unwrap();
    assert!(
        note.contains("110% if 2025's AGI exceeded $150,000"),
        "{note}"
    );
    assert!(note.contains("aequi_set_tax_return"), "{note}");
    assert!(note.contains("no 2025 tax is on file"), "{note}");
}

#[tokio::test]
async fn schedule_c_preview_empty_ledger() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_get_schedule_c_preview",
            json!({ "year": 2026 }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].as_text().unwrap().contains("net_profit"));
}

// -----------------------------------------------------------------------
// Invoice tool tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn list_unpaid_invoices_empty() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call("aequi_list_unpaid_invoices", json!({}), &db, &perms)
        .await;
    assert!(result.is_error.is_none());
}

// -----------------------------------------------------------------------
// Rules tool tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn categorization_rules_empty() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call("aequi_get_categorization_rules", json!({}), &db, &perms)
        .await;
    assert!(result.is_error.is_none());
}

#[tokio::test]
async fn save_and_list_categorization_rule() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_save_categorization_rule",
            json!({
                "name": "GitHub",
                "priority": 10,
                "match_pattern": "GITHUB",
                "match_type": "contains",
                "account_id": 1
            }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].as_text().unwrap().contains("rule_id"));

    let list = registry
        .call("aequi_get_categorization_rules", json!({}), &db, &perms)
        .await;
    assert!(list.content[0].as_text().unwrap().contains("GITHUB"));
}

// -----------------------------------------------------------------------
// Import tool tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn import_profiles_empty() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call("aequi_get_import_profiles", json!({}), &db, &perms)
        .await;
    assert!(result.is_error.is_none());
}

#[tokio::test]
async fn save_and_list_import_profile() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_save_import_profile",
            json!({
                "name": "Chase Checking",
                "has_header": true,
                "date_column": 0,
                "description_column": 1,
                "amount_column": 2,
                "date_format": "%m/%d/%Y"
            }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].as_text().unwrap().contains("profile_id"));

    let list = registry
        .call("aequi_get_import_profiles", json!({}), &db, &perms)
        .await;
    assert!(
        list.content[0]
            .as_text()
            .unwrap()
            .contains("Chase Checking")
    );
}

// -----------------------------------------------------------------------
// Reconciliation tool tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn create_reconciliation_session() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_create_reconciliation_session",
            json!({
                "account_id": 1,
                "start_date": "2026-01-01",
                "end_date": "2026-01-31",
                "statement_balance_cents": 100000
            }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].as_text().unwrap().contains("session_id"));
}

// -----------------------------------------------------------------------
// Protocol tests
// -----------------------------------------------------------------------

#[test]
fn json_rpc_success_response() {
    let resp = crate::protocol::JsonRpcResponse::success(
        serde_json::json!(1),
        serde_json::json!({"ok": true}),
    );
    assert_eq!(resp.jsonrpc, "2.0");
    assert!(resp.result.is_some());
    assert!(resp.error.is_none());
}

#[test]
fn json_rpc_error_response() {
    let resp = crate::protocol::JsonRpcResponse::error(serde_json::json!(1), -32600, "bad".into());
    assert!(resp.result.is_none());
    assert_eq!(resp.error.as_ref().unwrap().code, -32600);
}

#[test]
fn tool_result_text() {
    let r = ToolResult::text("hello".to_string());
    assert!(r.is_error.is_none());
    assert_eq!(r.content[0].as_text().unwrap(), "hello");
}

#[test]
fn tool_result_error() {
    let r = ToolResult::error("fail".to_string());
    assert!(r.is_error.unwrap());
    assert_eq!(r.content[0].as_text().unwrap(), "fail");
}

// -----------------------------------------------------------------------
// Audit tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn audit_log_records_tool_call() {
    let db = test_db().await;
    crate::audit::log_tool_call(&db, "test_tool", "{}", "success").await;
    let logs = aequi_storage::get_audit_log(&db, 10).await.unwrap();
    assert!(!logs.is_empty());
    assert_eq!(logs[0].tool_name, "test_tool");
    assert_eq!(logs[0].outcome, "success");
}

// -----------------------------------------------------------------------
// Invoice tool tests (additional coverage)
// -----------------------------------------------------------------------

async fn add_contact(db: &aequi_storage::DbPool, name: &str, is_contractor: bool) -> i64 {
    aequi_storage::insert_contact(
        db,
        name,
        Some("billing@example.com"),
        None,
        None,
        if is_contractor { "vendor" } else { "customer" },
        is_contractor,
        is_contractor.then_some("12-3456789"),
        None,
    )
    .await
    .unwrap()
}

/// 1.5 h at $100 (taxable) + 1 setup at $50; 10% sales tax on the taxable
/// line: subtotal $200, tax $15, total $215.
fn invoice_args(number: &str, contact_id: i64) -> serde_json::Value {
    json!({
        "invoice_number": number,
        "contact_id": contact_id,
        "issue_date": "2026-03-01",
        "due_date": "2026-03-31",
        "lines": [
            { "description": "Consulting", "quantity": 1.5, "unit_rate_cents": 10000, "taxable": true },
            { "description": "Setup", "quantity": 1, "unit_rate_cents": 5000 }
        ],
        "tax_lines": [ { "label": "Sales tax", "rate_bps": 1000 } ]
    })
}

fn parse(result: &ToolResult) -> serde_json::Value {
    let text = result.content[0].as_text().unwrap();
    assert!(!result.is_error(), "{text}");
    serde_json::from_str(text).unwrap()
}

async fn draft_invoice(db: &aequi_storage::DbPool, number: &str, contact_id: i64) -> i64 {
    let v = parse(&call(db, "aequi_draft_invoice", invoice_args(number, contact_id)).await);
    v["id"].as_i64().unwrap()
}

#[tokio::test]
async fn draft_invoice_with_line_items_computes_totals() {
    let db = test_db().await;
    let contact = add_contact(&db, "Test Client", false).await;

    let v = parse(&call(&db, "aequi_draft_invoice", invoice_args("INV-001", contact)).await);
    assert_eq!(v["status_type"], "Draft");
    assert_eq!(v["invoice_number"], "INV-001");
    assert_eq!(v["subtotal_cents"], 20000);
    assert_eq!(v["discount_cents"], 0);
    assert_eq!(v["tax_cents"], 1500);
    assert_eq!(v["total_cents"], 21500);
    assert_eq!(v["balance_cents"], 21500);
    let lines = v["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["quantity_hundredths"], 150);
    assert_eq!(lines[0]["taxable"], true);
    assert_eq!(lines[1]["taxable"], false);
    assert_eq!(v["tax_lines"][0]["rate_bps"], 1000);

    // A draft is not yet owed.
    let unpaid = parse(&call(&db, "aequi_list_unpaid_invoices", json!({})).await);
    assert_eq!(unpaid, json!([]));
}

#[tokio::test]
async fn draft_invoice_with_discount_notes_and_terms() {
    let db = test_db().await;
    let contact = add_contact(&db, "Client B", false).await;

    let mut args = invoice_args("INV-002", contact);
    args["discount"] = json!({ "type": "percentage", "bps": 1000 });
    args["notes"] = json!("Thanks!");
    args["terms"] = json!("Net 30");
    let v = parse(&call(&db, "aequi_draft_invoice", args).await);
    assert_eq!(v["discount_cents"], 2000);
    assert_eq!(v["notes"], "Thanks!");
    assert_eq!(v["terms"], "Net 30");
    let total = v["subtotal_cents"].as_i64().unwrap() - v["discount_cents"].as_i64().unwrap()
        + v["tax_cents"].as_i64().unwrap();
    assert_eq!(v["total_cents"].as_i64().unwrap(), total);

    let mut args = invoice_args("INV-003", contact);
    args["discount"] = json!({ "type": "flat", "cents": 5000 });
    let v = parse(&call(&db, "aequi_draft_invoice", args).await);
    assert_eq!(v["discount_cents"], 5000);
}

#[tokio::test]
async fn draft_invoice_missing_params_rejected() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call("aequi_draft_invoice", json!({}), &db, &perms)
        .await;
    assert!(result.is_error.unwrap_or(false));
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("Missing required argument")
    );
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM invoices")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn draft_invoice_validates_line_items() {
    let db = test_db().await;
    let contact = add_contact(&db, "Client V", false).await;

    let mut no_lines = invoice_args("INV-V", contact);
    no_lines.as_object_mut().unwrap().remove("lines");
    let mut empty_lines = invoice_args("INV-V", contact);
    empty_lines["lines"] = json!([]);
    let mut three_decimals = invoice_args("INV-V", contact);
    three_decimals["lines"][0]["quantity"] = json!(1.005);
    let mut string_qty = invoice_args("INV-V", contact);
    string_qty["lines"][0]["quantity"] = json!("2");
    let mut zero_qty = invoice_args("INV-V", contact);
    zero_qty["lines"][0]["quantity"] = json!(0);
    let mut fractional_rate = invoice_args("INV-V", contact);
    fractional_rate["lines"][0]["unit_rate_cents"] = json!(99.5);
    let mut misspelled = invoice_args("INV-V", contact);
    misspelled["lines"][1]["is_taxable"] = json!(true);
    let mut bad_tax = invoice_args("INV-V", contact);
    bad_tax["tax_lines"][0]["rate_bps"] = json!(20000);
    let mut bad_discount = invoice_args("INV-V", contact);
    bad_discount["discount"] = json!({ "type": "bogus" });
    let mut mixed_discount = invoice_args("INV-V", contact);
    mixed_discount["discount"] = json!({ "type": "flat", "bps": 100 });
    let mut bad_dates = invoice_args("INV-V", contact);
    bad_dates["due_date"] = json!("2026-02-01");

    for (args, expected) in [
        (no_lines, "Missing required argument: lines"),
        (empty_lines, "at least one line item"),
        (three_decimals, "lines[0].quantity"),
        (string_qty, "lines[0].quantity"),
        (zero_qty, "quantity must be positive"),
        (fractional_rate, "lines[0].unit_rate_cents"),
        (misspelled, "lines[1].is_taxable"),
        (bad_tax, "tax rate"),
        (bad_discount, "discount.type"),
        (mixed_discount, "discount.bps"),
        (bad_dates, "due_date"),
    ] {
        let result = call(&db, "aequi_draft_invoice", args).await;
        let text = result.content[0].as_text().unwrap();
        assert!(result.is_error(), "{expected}: {text}");
        assert!(text.contains(expected), "{expected}: {text}");
    }
    assert_eq!(count(&db, "invoices").await, 0);

    draft_invoice(&db, "INV-V", contact).await;
    let dup = call(&db, "aequi_draft_invoice", invoice_args("INV-V", contact)).await;
    assert!(dup.is_error());
    assert!(dup.content[0].as_text().unwrap().contains("already in use"));
    assert_eq!(count(&db, "invoices").await, 1);
}

#[tokio::test]
async fn get_invoice_returns_detail() {
    let db = test_db().await;
    let contact = add_contact(&db, "Client G", false).await;
    let id = draft_invoice(&db, "INV-G", contact).await;

    let v = parse(&call(&db, "aequi_get_invoice", json!({ "invoice_id": id })).await);
    assert_eq!(v["id"], id);
    assert_eq!(v["total_cents"], 21500);
    assert_eq!(v["lines"].as_array().unwrap().len(), 2);
    assert_eq!(v["payments"], json!([]));

    let missing = call(&db, "aequi_get_invoice", json!({ "invoice_id": 99 })).await;
    assert!(missing.is_error());
    assert!(
        missing.content[0]
            .as_text()
            .unwrap()
            .contains("Invoice 99 not found")
    );
}

/// Net (debit - credit) cents posted to an account by one transaction.
async fn posted(db: &aequi_storage::DbPool, transaction_id: i64, code: &str) -> i64 {
    let (net,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(tl.debit_cents - tl.credit_cents), 0) FROM transaction_lines tl \
         JOIN accounts a ON a.id = tl.account_id WHERE tl.transaction_id = ? AND a.code = ?",
    )
    .bind(transaction_id)
    .bind(code)
    .fetch_one(db)
    .await
    .unwrap();
    net
}

#[tokio::test]
async fn record_payment_posts_cash_basis_entry() {
    let db = test_db().await;
    let contact = add_contact(&db, "Pay Client", false).await;
    let invoice_id = draft_invoice(&db, "INV-PAY-001", contact).await;
    aequi_storage::mark_invoice_sent(&db, invoice_id, "2026-03-02")
        .await
        .unwrap();

    let v = parse(
        &call(
            &db,
            "aequi_record_payment",
            json!({
                "invoice_id": invoice_id,
                "amount_cents": 10000,
                "date": "2026-03-15",
                "method": "bank_transfer"
            }),
        )
        .await,
    );
    assert!(v["payment_id"].as_i64().is_some());
    assert_eq!(v["invoice_status"], "PartiallyPaid");
    assert_eq!(v["paid_cents"], 10000);
    assert_eq!(v["balance_cents"], 11500);
    let tx = v["transaction_id"].as_i64().unwrap();
    // Tax share: 10000 * 1500 / 21500 = 697.67, rounded to 698.
    assert_eq!(posted(&db, tx, "1000").await, 10000);
    assert_eq!(posted(&db, tx, "4000").await, -9302);
    assert_eq!(posted(&db, tx, "2010").await, -698);

    // Still owed: listed with its balance.
    let unpaid = parse(&call(&db, "aequi_list_unpaid_invoices", json!({})).await);
    assert_eq!(unpaid.as_array().unwrap().len(), 1);
    assert_eq!(unpaid[0]["invoice_number"], "INV-PAY-001");
    assert_eq!(unpaid[0]["balance_cents"], 11500);

    // The rest, into savings.
    let v = parse(
        &call(
            &db,
            "aequi_record_payment",
            json!({
                "invoice_id": invoice_id,
                "amount_cents": 11500,
                "date": "2026-03-20",
                "deposit_account_code": "1010"
            }),
        )
        .await,
    );
    assert_eq!(v["invoice_status"], "Paid");
    assert_eq!(v["balance_cents"], 0);
    let tx = v["transaction_id"].as_i64().unwrap();
    assert_eq!(posted(&db, tx, "1010").await, 11500);
    assert_eq!(posted(&db, tx, "1000").await, 0);

    let unpaid = parse(&call(&db, "aequi_list_unpaid_invoices", json!({})).await);
    assert_eq!(unpaid, json!([]));
}

#[tokio::test]
async fn record_payment_rejects_bad_deposit_account_and_overpayment() {
    let db = test_db().await;
    let contact = add_contact(&db, "Client C", false).await;
    let invoice_id = draft_invoice(&db, "INV-PAY-002", contact).await;

    for (args, expected) in [
        (
            json!({ "invoice_id": invoice_id, "amount_cents": 1000, "date": "2026-03-20", "deposit_account_code": "4000" }),
            "asset account",
        ),
        (
            json!({ "invoice_id": invoice_id, "amount_cents": 1000, "date": "2026-03-20", "deposit_account_code": "9999" }),
            "9999",
        ),
        (
            json!({ "invoice_id": invoice_id, "amount_cents": 21501, "date": "2026-03-20" }),
            "exceeds",
        ),
    ] {
        let result = call(&db, "aequi_record_payment", args).await;
        let text = result.content[0].as_text().unwrap();
        assert!(result.is_error(), "{text}");
        assert!(text.contains(expected), "{expected}: {text}");
    }
    assert_eq!(count(&db, "payments").await, 0);
    assert_eq!(count(&db, "transactions").await, 0);
}

#[tokio::test]
async fn list_unpaid_invoices_with_sent_status() {
    let db = test_db().await;
    let contact = add_contact(&db, "Client D", false).await;
    let sent = draft_invoice(&db, "INV-SENT-001", contact).await;
    draft_invoice(&db, "INV-DRAFT-001", contact).await;
    aequi_storage::mark_invoice_sent(&db, sent, "2026-03-02")
        .await
        .unwrap();

    let result = call(&db, "aequi_list_unpaid_invoices", json!({})).await;
    let v = parse(&result);
    let numbers: Vec<_> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["invoice_number"].as_str().unwrap())
        .collect();
    assert_eq!(numbers, ["INV-SENT-001"]);
    assert_eq!(v[0]["total_cents"], 21500);
    assert_eq!(v[0]["balance_cents"], 21500);
}

// -----------------------------------------------------------------------
// Rules tool tests (additional coverage)
// -----------------------------------------------------------------------

#[tokio::test]
async fn save_rule_with_exact_match_type() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_save_categorization_rule",
            json!({
                "name": "Exact AWS",
                "priority": 5,
                "match_pattern": "AMAZON WEB SERVICES",
                "match_type": "exact",
                "account_id": 1
            }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].as_text().unwrap().contains("rule_id"));
}

#[tokio::test]
async fn save_rule_with_regex_match_type() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_save_categorization_rule",
            json!({
                "name": "Regex SaaS",
                "priority": 20,
                "match_pattern": "SAAS.*MONTHLY",
                "match_type": "regex",
                "account_id": 2
            }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].as_text().unwrap().contains("rule_id"));
}

#[tokio::test]
async fn apply_rules_empty_batch() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    // Apply rules to a non-existent batch — should succeed with 0 matches
    let result = registry
        .call(
            "aequi_apply_rules",
            json!({ "batch_id": "nonexistent-batch" }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    let val: serde_json::Value =
        serde_json::from_str(result.content[0].as_text().unwrap()).unwrap();
    assert_eq!(val["total_pending"], 0);
    assert_eq!(val["matched"], 0);
    assert_eq!(val["unmatched"], 0);
}

#[tokio::test]
async fn apply_rules_with_no_rules_defined() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    // No rules defined, any batch should yield 0 matches
    let result = registry
        .call(
            "aequi_apply_rules",
            json!({ "batch_id": "some-batch" }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    let val: serde_json::Value =
        serde_json::from_str(result.content[0].as_text().unwrap()).unwrap();
    assert_eq!(val["matched"], 0);
}

#[tokio::test]
async fn save_multiple_rules_and_list() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    for (name, pattern, priority) in [
        ("Rule A", "PATTERN_A", 1),
        ("Rule B", "PATTERN_B", 2),
        ("Rule C", "PATTERN_C", 3),
    ] {
        let result = registry
            .call(
                "aequi_save_categorization_rule",
                json!({
                    "name": name,
                    "priority": priority,
                    "match_pattern": pattern,
                    "match_type": "contains",
                    "account_id": 1
                }),
                &db,
                &perms,
            )
            .await;
        assert!(result.is_error.is_none());
    }

    let list = registry
        .call("aequi_get_categorization_rules", json!({}), &db, &perms)
        .await;
    assert!(list.is_error.is_none());
    let text = &list.content[0].as_text().unwrap();
    assert!(text.contains("PATTERN_A"));
    assert!(text.contains("PATTERN_B"));
    assert!(text.contains("PATTERN_C"));
}

// -----------------------------------------------------------------------
// Receipt tool tests (additional coverage — path validation)
// -----------------------------------------------------------------------

#[tokio::test]
async fn ingest_receipt_empty_path() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_ingest_receipt",
            json!({ "file_path": "" }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.unwrap_or(false));
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("file_path is required")
    );
}

#[tokio::test]
async fn ingest_receipt_relative_path_rejected() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_ingest_receipt",
            json!({ "file_path": "relative/path/receipt.jpg" }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.unwrap_or(false));
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("absolute path")
    );
}

#[tokio::test]
async fn ingest_receipt_path_traversal_rejected() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_ingest_receipt",
            json!({ "file_path": "/home/user/../etc/passwd.jpg" }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.unwrap_or(false));
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("Path traversal")
    );
}

#[tokio::test]
async fn ingest_receipt_unsupported_extension() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let result = registry
        .call(
            "aequi_ingest_receipt",
            json!({ "file_path": "/home/user/receipt.exe" }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.unwrap_or(false));
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("Unsupported file type")
    );
}

#[tokio::test]
async fn ingest_receipt_supported_extensions_pass_validation() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    // These should pass validation but fail on "Cannot read file" since files don't exist
    for ext in &[
        "jpg", "jpeg", "png", "gif", "webp", "tiff", "tif", "bmp", "pdf",
    ] {
        let result = registry
            .call(
                "aequi_ingest_receipt",
                json!({ "file_path": format!("/tmp/test_receipt.{ext}") }),
                &db,
                &perms,
            )
            .await;
        assert!(result.is_error.unwrap_or(false));
        // Should get past validation to file read stage
        assert!(
            result.content[0]
                .as_text()
                .unwrap()
                .contains("Cannot read file"),
            "Extension .{ext} should pass validation but got: {}",
            result.content[0].as_text().unwrap()
        );
    }
}

#[tokio::test]
async fn ingest_receipt_no_file_path_param() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    // Missing file_path entirely — defaults to empty string
    let result = registry
        .call("aequi_ingest_receipt", json!({}), &db, &perms)
        .await;
    assert!(result.is_error.unwrap_or(false));
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("file_path is required")
    );
}

#[tokio::test]
async fn agents_cannot_approve_receipts() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    let receipt_id = aequi_storage::insert_receipt(
        &db,
        "abc123hash",
        "jpg",
        "/tmp/test.jpg",
        None,
        Some("TestVendor"),
        Some("2026-03-01"),
        Some(1500),
        None,
        None,
        None,
        0.0,
    )
    .await
    .unwrap();

    // Approval is a human step in the app: no tool is listed for it...
    let names: Vec<_> = registry
        .list_definitions()
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert!(
        !names.iter().any(|n| n.contains("approve")),
        "an approve tool is registered: {names:?}"
    );

    // ...and calling the old tool fails without changing the receipt.
    for args in [
        json!({ "receipt_id": receipt_id }),
        json!({ "receipt_id": receipt_id, "transaction_id": 1 }),
    ] {
        let result = registry
            .call("aequi_approve_receipt", args, &db, &perms)
            .await;
        assert!(result.is_error());
        assert!(
            result.content[0]
                .as_text()
                .unwrap()
                .contains("Unknown tool: aequi_approve_receipt")
        );
    }
    let receipt = aequi_storage::get_receipt_by_id(&db, receipt_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.status, "pending_review");
    assert_eq!(receipt.transaction_id, None);

    // The receipt tools tell agents where approval happens.
    for tool in ["aequi_get_pending_receipts", "aequi_update_receipt_fields"] {
        let def = registry
            .list_definitions()
            .into_iter()
            .find(|d| d.name == tool)
            .unwrap();
        assert!(
            def.description.contains("in the Aequi app"),
            "{tool}: {}",
            def.description
        );
    }
}

#[tokio::test]
async fn reject_receipt() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    aequi_storage::insert_receipt(
        &db,
        "def456hash",
        "png",
        "/tmp/test2.png",
        None,
        Some("Vendor2"),
        Some("2026-03-02"),
        Some(2000),
        None,
        None,
        None,
        0.0,
    )
    .await
    .unwrap();

    let result = registry
        .call(
            "aequi_reject_receipt",
            json!({ "receipt_id": 1 }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("Receipt rejected")
    );
}

// -----------------------------------------------------------------------
// Regression tests: settings enforcement
// -----------------------------------------------------------------------

async fn call(db: &aequi_storage::DbPool, tool: &str, args: serde_json::Value) -> ToolResult {
    ToolRegistry::new()
        .call(tool, args, db, &Permissions::default())
        .await
}

/// Row count of a table; `table` is always a literal from these tests.
async fn count(db: &aequi_storage::DbPool, table: &'static str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let (n,): (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .fetch_one(db)
        .await
        .unwrap();
    n
}

fn balanced_tx(date: &str, description: &str, cents: i64) -> serde_json::Value {
    json!({
        "date": date,
        "description": description,
        "lines": [
            { "account_code": "1000", "debit_cents": cents, "credit_cents": 0 },
            { "account_code": "4000", "debit_cents": 0, "credit_cents": cents }
        ]
    })
}

#[test]
fn write_tool_classification() {
    let registry = ToolRegistry::new();
    let mut writes = registry.write_tools();
    writes.sort_unstable();
    let mut expected = vec![
        "aequi_create_transaction",
        "aequi_ingest_receipt",
        "aequi_reject_receipt",
        "aequi_record_tax_payment",
        "aequi_draft_invoice",
        "aequi_record_payment",
        "aequi_save_categorization_rule",
        "aequi_apply_rules",
        "aequi_save_import_profile",
        "aequi_create_reconciliation_session",
        "aequi_resolve_item",
        "aequi_update_receipt_fields",
        "aequi_post_import",
        "aequi_ignore_import",
        "aequi_set_tax_return",
    ];
    expected.sort_unstable();
    assert_eq!(writes, expected);
}

#[tokio::test]
async fn disabled_setting_refuses_every_tool() {
    let db = test_db().await;
    aequi_storage::set_setting(&db, "mcp_enabled", "false")
        .await
        .unwrap();

    let read = call(&db, "aequi_list_accounts", json!({})).await;
    assert!(read.is_error());
    assert!(read.content[0].as_text().unwrap().contains("disabled"));

    let write = call(
        &db,
        "aequi_create_transaction",
        balanced_tx("2026-03-01", "X", 100),
    )
    .await;
    assert!(write.is_error());
    assert_eq!(count(&db, "transactions").await, 0);

    aequi_storage::set_setting(&db, "mcp_enabled", "true")
        .await
        .unwrap();
    assert!(!call(&db, "aequi_list_accounts", json!({})).await.is_error());
}

#[tokio::test]
async fn read_only_setting_blocks_every_write_tool() {
    let db = test_db().await;
    aequi_storage::set_setting(&db, "mcp_read_only", "true")
        .await
        .unwrap();

    let registry = ToolRegistry::new();
    for tool in registry.write_tools() {
        let result = registry
            .call(tool, json!({}), &db, &Permissions::default())
            .await;
        assert!(result.is_error(), "{tool} ran in read-only mode");
        assert!(
            result.content[0].as_text().unwrap().contains("read-only"),
            "{tool}: {}",
            result.content[0].as_text().unwrap()
        );
    }

    let result = call(
        &db,
        "aequi_create_transaction",
        balanced_tx("2026-03-01", "X", 100),
    )
    .await;
    assert!(result.is_error());
    assert_eq!(count(&db, "transactions").await, 0);

    // Reads still work.
    assert!(!call(&db, "aequi_list_accounts", json!({})).await.is_error());

    // Toggling the setting takes effect on the next call.
    aequi_storage::set_setting(&db, "mcp_read_only", "false")
        .await
        .unwrap();
    let result = call(
        &db,
        "aequi_create_transaction",
        balanced_tx("2026-03-01", "X", 100),
    )
    .await;
    assert!(
        !result.is_error(),
        "{}",
        result.content[0].as_text().unwrap()
    );
    assert_eq!(count(&db, "transactions").await, 1);
}

#[tokio::test]
async fn unset_settings_allow_writes() {
    let db = test_db().await;
    let result = call(
        &db,
        "aequi_create_transaction",
        balanced_tx("2026-03-01", "X", 100),
    )
    .await;
    assert!(
        !result.is_error(),
        "{}",
        result.content[0].as_text().unwrap()
    );
}

// -----------------------------------------------------------------------
// Regression tests: JSON-RPC conformance
// -----------------------------------------------------------------------

async fn test_server() -> crate::server::McpServer {
    crate::server::McpServer::new(test_db().await, Permissions::default())
}

fn to_json(resp: &crate::protocol::JsonRpcResponse) -> serde_json::Value {
    serde_json::to_value(resp).unwrap()
}

#[tokio::test]
async fn notifications_get_no_response() {
    let server = test_server().await;
    for msg in [
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#,
        r#"{"jsonrpc":"2.0","method":"some/unknown_notification"}"#,
    ] {
        assert!(
            server.handle_message(msg.as_bytes()).await.is_none(),
            "{msg}"
        );
    }
}

#[tokio::test]
async fn tool_call_sent_as_notification_is_not_executed() {
    let db = test_db().await;
    let server = crate::server::McpServer::new(db.clone(), Permissions::default());
    let msg = json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "params": { "name": "aequi_create_transaction", "arguments": balanced_tx("2026-03-01", "X", 100) }
    });
    let bytes = serde_json::to_vec(&msg).unwrap();
    assert!(server.handle_message(&bytes).await.is_none());
    assert_eq!(count(&db, "transactions").await, 0);
}

#[tokio::test]
async fn parse_error_has_null_id() {
    let server = test_server().await;
    let resp = server.handle_message(b"{not json").await.unwrap();
    let v = to_json(&resp);
    assert_eq!(v["error"]["code"], -32700);
    assert!(v.as_object().unwrap().contains_key("id"));
    assert!(v["id"].is_null());
}

#[tokio::test]
async fn invalid_requests_get_32600() {
    let server = test_server().await;
    for (msg, id) in [
        (r#"[]"#, json!(null)),
        (r#"42"#, json!(null)),
        (r#"{"foo":1}"#, json!(null)),
        (r#"{"jsonrpc":"2.0","id":7}"#, json!(7)),
        (r#"{"jsonrpc":"1.0","id":"a","method":"ping"}"#, json!("a")),
        (r#"{"jsonrpc":"2.0","id":3,"method":5}"#, json!(3)),
        (
            r#"{"jsonrpc":"2.0","id":4,"method":"ping","params":"x"}"#,
            json!(4),
        ),
        (
            r#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#,
            json!(null),
        ),
    ] {
        let resp = server.handle_message(msg.as_bytes()).await.unwrap();
        let v = to_json(&resp);
        assert_eq!(v["error"]["code"], -32600, "{msg}");
        assert_eq!(v["id"], id, "{msg}");
    }
}

#[tokio::test]
async fn ping_returns_empty_result() {
    let server = test_server().await;
    let resp = server
        .handle_message(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)
        .await
        .unwrap();
    assert_eq!(resp.id, json!(1));
    assert_eq!(resp.result, Some(json!({})));
}

#[tokio::test]
async fn unknown_method_with_id_is_32601() {
    let server = test_server().await;
    let resp = server
        .handle_message(br#"{"jsonrpc":"2.0","id":"x","method":"bogus"}"#)
        .await
        .unwrap();
    assert_eq!(resp.id, json!("x"));
    assert_eq!(resp.error.unwrap().code, -32601);
}

#[tokio::test]
async fn null_id_is_a_request() {
    let server = test_server().await;
    let resp = server
        .handle_message(br#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#)
        .await
        .unwrap();
    assert!(resp.id.is_null());
    assert!(resp.result.is_some());
}

#[tokio::test]
async fn tools_call_without_name_is_invalid_params() {
    let server = test_server().await;
    let resp = server
        .handle_message(br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#)
        .await
        .unwrap();
    assert_eq!(resp.error.unwrap().code, -32602);
}

#[tokio::test]
async fn stdio_survives_invalid_utf8_and_skips_notifications() {
    let server = test_server().await;
    let mut input = Vec::new();
    input.extend_from_slice(b"\xff\xfe garbage\n");
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n");
    input.extend_from_slice(b"\r\n");
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\r\n");
    let mut output = Vec::new();

    crate::server::serve_lines(&server, &input[..], &mut output)
        .await
        .unwrap();

    let text = String::from_utf8(output).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert_eq!(lines[0]["error"]["code"], -32700);
    assert!(lines[0]["id"].is_null());
    assert_eq!(lines[1]["id"], 2);
    assert_eq!(lines[1]["result"], json!({}));
}

// -----------------------------------------------------------------------
// Regression tests: argument validation
// -----------------------------------------------------------------------

#[tokio::test]
async fn fractional_cents_rejected() {
    let db = test_db().await;
    let result = call(
        &db,
        "aequi_create_transaction",
        balanced_tx("2026-03-01", "X", 0),
    )
    .await;
    assert!(result.is_error());

    let args = json!({
        "date": "2026-03-01",
        "description": "Fraction",
        "lines": [
            { "account_code": "1000", "debit_cents": 49.99, "credit_cents": 0 },
            { "account_code": "4000", "debit_cents": 0, "credit_cents": 49.99 }
        ]
    });
    let result = call(&db, "aequi_create_transaction", args).await;
    assert!(result.is_error());
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("lines[0].debit_cents")
    );
    assert_eq!(count(&db, "transactions").await, 0);
}

#[tokio::test]
async fn integral_float_cents_accepted() {
    let db = test_db().await;
    let args = json!({
        "date": "2026-03-01",
        "description": "Float",
        "lines": [
            { "account_code": "1000", "debit_cents": 5000.0, "credit_cents": 0 },
            { "account_code": "4000", "debit_cents": 0, "credit_cents": 5000.0 }
        ]
    });
    let result = call(&db, "aequi_create_transaction", args).await;
    assert!(
        !result.is_error(),
        "{}",
        result.content[0].as_text().unwrap()
    );
}

#[tokio::test]
async fn core_validation_errors_surface() {
    let db = test_db().await;
    for lines in [
        // negative
        json!([
            { "account_code": "1000", "debit_cents": -100, "credit_cents": 0 },
            { "account_code": "4000", "debit_cents": 0, "credit_cents": -100 }
        ]),
        // both sides on one line
        json!([
            { "account_code": "1000", "debit_cents": 100, "credit_cents": 100 },
            { "account_code": "4000", "debit_cents": 0, "credit_cents": 0 }
        ]),
        // single line
        json!([{ "account_code": "1000", "debit_cents": 100, "credit_cents": 0 }]),
        // missing credit_cents
        json!([
            { "account_code": "1000", "debit_cents": 100 },
            { "account_code": "4000", "debit_cents": 0, "credit_cents": 100 }
        ]),
    ] {
        let args = json!({ "date": "2026-03-01", "description": "Bad", "lines": lines });
        let result = call(&db, "aequi_create_transaction", args).await;
        assert!(result.is_error(), "{lines}");
    }
    assert_eq!(count(&db, "transactions").await, 0);
}

#[tokio::test]
async fn archived_account_rejected() {
    let db = test_db().await;
    sqlx::query("UPDATE accounts SET is_archived = 1 WHERE code = '4000'")
        .execute(&db)
        .await
        .unwrap();
    let result = call(
        &db,
        "aequi_create_transaction",
        balanced_tx("2026-03-01", "X", 100),
    )
    .await;
    assert!(result.is_error());
    assert!(result.content[0].as_text().unwrap().contains("archived"));
}

#[tokio::test]
async fn transaction_insert_is_atomic() {
    let db = test_db().await;
    // Make the second line's insert fail after the header was written.
    sqlx::query(
        "CREATE TRIGGER fail_income_line BEFORE INSERT ON transaction_lines \
         WHEN NEW.account_id = (SELECT id FROM accounts WHERE code = '4000') \
         BEGIN SELECT RAISE(ABORT, 'boom'); END",
    )
    .execute(&db)
    .await
    .unwrap();

    let result = call(
        &db,
        "aequi_create_transaction",
        balanced_tx("2026-03-01", "X", 100),
    )
    .await;
    assert!(result.is_error());
    assert!(result.content[0].as_text().unwrap().contains("boom"));
    assert_eq!(count(&db, "transactions").await, 0);
    assert_eq!(count(&db, "transaction_lines").await, 0);
}

#[tokio::test]
async fn unknown_arguments_rejected() {
    let db = test_db().await;
    let result = call(
        &db,
        "aequi_get_transactions",
        json!({ "start_dat": "2026-01-01" }),
    )
    .await;
    assert!(result.is_error());
    assert!(result.content[0].as_text().unwrap().contains("start_dat"));

    let result = call(&db, "aequi_list_accounts", json!([1, 2])).await;
    assert!(result.is_error());
}

#[tokio::test]
async fn save_rule_requires_pattern_and_valid_type() {
    let db = test_db().await;
    let base = json!({
        "name": "R", "priority": 1, "match_pattern": "X",
        "match_type": "contains", "account_id": 1
    });

    let mut missing = base.clone();
    missing.as_object_mut().unwrap().remove("match_pattern");
    assert!(
        call(&db, "aequi_save_categorization_rule", missing)
            .await
            .is_error()
    );

    let mut empty = base.clone();
    empty["match_pattern"] = json!("");
    assert!(
        call(&db, "aequi_save_categorization_rule", empty)
            .await
            .is_error()
    );

    let mut bad_type = base.clone();
    bad_type["match_type"] = json!("startswith");
    assert!(
        call(&db, "aequi_save_categorization_rule", bad_type)
            .await
            .is_error()
    );

    let mut bad_regex = base.clone();
    bad_regex["match_type"] = json!("regex");
    bad_regex["match_pattern"] = json!("(unclosed");
    let result = call(&db, "aequi_save_categorization_rule", bad_regex).await;
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("Invalid regex")
    );

    let mut bad_account = base.clone();
    bad_account["account_id"] = json!(9999);
    assert!(
        call(&db, "aequi_save_categorization_rule", bad_account)
            .await
            .is_error()
    );

    assert_eq!(count(&db, "categorization_rules").await, 0);
}

#[tokio::test]
async fn tax_payment_args_validated() {
    let db = test_db().await;
    let ok = json!({ "year": 2026, "quarter": 2, "amount_cents": 125000, "date": "2026-06-15" });

    for (field, value) in [
        ("amount_cents", json!(1250.5)),
        ("amount_cents", json!(0)),
        ("amount_cents", json!(-5)),
        ("quarter", json!(5)),
        ("quarter", json!(0)),
        ("year", json!(67562)),
        ("date", json!("06/15/2026")),
    ] {
        let mut args = ok.clone();
        args[field] = value.clone();
        let result = call(&db, "aequi_record_tax_payment", args).await;
        assert!(result.is_error(), "{field}={value} accepted");
    }
    assert_eq!(count(&db, "tax_periods").await, 0);

    // Works with no prior estimate, and payments accumulate.
    assert!(
        !call(&db, "aequi_record_tax_payment", ok.clone())
            .await
            .is_error()
    );
    assert!(!call(&db, "aequi_record_tax_payment", ok).await.is_error());
    let periods = aequi_storage::get_tax_periods(&db, 2026).await.unwrap();
    assert_eq!(periods.len(), 1);
    assert_eq!(periods[0].payment_recorded_cents, 250000);
}

#[tokio::test]
async fn estimate_requires_valid_quarter() {
    let db = test_db().await;
    for args in [
        json!({ "year": 2026, "quarter": 5 }),
        json!({ "year": 2026 }),
        json!({ "year": 2026, "quarter": 1.5 }),
    ] {
        assert!(
            call(&db, "aequi_estimate_quarterly_tax", args.clone())
                .await
                .is_error(),
            "{args}"
        );
    }
}

fn this_year() -> i64 {
    chrono::Datelike::year(&chrono::Local::now().date_naive()).into()
}

#[tokio::test]
async fn tax_return_figures_round_trip() {
    let db = test_db().await;
    let get = |year: i64| call(&db, "aequi_get_tax_return", json!({ "year": year }));

    // Nothing entered yet.
    assert_eq!(
        parse(&get(2025).await),
        json!({ "year": 2025, "total_tax_cents": null, "agi_cents": null })
    );

    let v = parse(
        &call(
            &db,
            "aequi_set_tax_return",
            json!({ "year": 2025, "total_tax_cents": 1_500_000, "agi_cents": 20_000_000 }),
        )
        .await,
    );
    assert_eq!(
        v,
        json!({ "year": 2025, "total_tax_cents": 1_500_000, "agi_cents": 20_000_000 })
    );

    // An omitted field is left unchanged; AGI may be negative.
    let v = parse(
        &call(
            &db,
            "aequi_set_tax_return",
            json!({ "year": 2025, "agi_cents": -500_000 }),
        )
        .await,
    );
    assert_eq!(
        v,
        json!({ "year": 2025, "total_tax_cents": 1_500_000, "agi_cents": -500_000 })
    );

    // An explicit null clears the stored value.
    let v = parse(
        &call(
            &db,
            "aequi_set_tax_return",
            json!({ "year": 2025, "total_tax_cents": null }),
        )
        .await,
    );
    assert_eq!(
        v,
        json!({ "year": 2025, "total_tax_cents": null, "agi_cents": -500_000 })
    );
    assert_eq!(parse(&get(2025).await), v);
    assert_eq!(
        aequi_storage::get_tax_return_figures(&db, 2025)
            .await
            .unwrap(),
        aequi_storage::TaxReturnFigures {
            total_tax_cents: None,
            agi_cents: Some(-500_000),
        }
    );

    // Each year is separate.
    assert_eq!(
        parse(&get(2024).await),
        json!({ "year": 2024, "total_tax_cents": null, "agi_cents": null })
    );
}

#[tokio::test]
async fn tax_return_args_validated() {
    let db = test_db().await;
    let this_year = this_year();

    for args in [
        json!({ "total_tax_cents": 100 }),
        json!({ "year": 1999, "total_tax_cents": 100 }),
        json!({ "year": 67562, "total_tax_cents": 100 }),
        json!({ "year": this_year, "total_tax_cents": 100 }),
        json!({ "year": this_year + 1, "total_tax_cents": 100 }),
        json!({ "year": 2025, "total_tax_cents": -1 }),
        json!({ "year": 2025, "total_tax_cents": 12.5 }),
        json!({ "year": 2025, "total_tax_cents": "1500" }),
        json!({ "year": 2025, "agi_cents": 1.5 }),
        json!({ "year": 2025 }),
        json!({ "year": 2025, "total_tax": 100 }),
    ] {
        let result = call(&db, "aequi_set_tax_return", args.clone()).await;
        assert!(result.is_error(), "accepted {args}");
    }
    for year in [2025, this_year as u16] {
        assert_eq!(
            aequi_storage::get_tax_return_figures(&db, year)
                .await
                .unwrap(),
            aequi_storage::TaxReturnFigures::default()
        );
    }

    let result = call(&db, "aequi_set_tax_return", json!({ "year": 2025 })).await;
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("Nothing to change")
    );
    let result = call(
        &db,
        "aequi_set_tax_return",
        json!({ "year": this_year, "agi_cents": 100 }),
    )
    .await;
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains(&format!("must be before {this_year}")),
        "{}",
        result.content[0].as_text().unwrap()
    );

    for args in [
        json!({}),
        json!({ "year": 1999 }),
        json!({ "year": this_year }),
        json!({ "year": 2025.5 }),
    ] {
        let result = call(&db, "aequi_get_tax_return", args.clone()).await;
        assert!(result.is_error(), "accepted {args}");
    }
}

#[tokio::test]
async fn estimate_uses_filed_prior_year_return() {
    let db = test_db().await;
    // $25,000 of Q1 revenue ($100,000 annualized): below the QBI limits, and
    // 90% of this year's projected tax is above last year's tax, so the
    // prior-year test decides the safe harbor.
    assert!(
        !call(
            &db,
            "aequi_create_transaction",
            balanced_tx("2026-01-15", "Big client", 2_500_000),
        )
        .await
        .is_error()
    );
    let estimate = || async {
        parse(
            &call(
                &db,
                "aequi_estimate_quarterly_tax",
                json!({ "year": 2026, "quarter": 1 }),
            )
            .await,
        )
    };
    let dollars = |v: &serde_json::Value, field: &str| -> rust_decimal::Decimal {
        v[field].as_str().unwrap().parse().unwrap()
    };
    let set = |args: serde_json::Value| call(&db, "aequi_set_tax_return", args);

    // QBI deduction is reported in cents too.
    let v = estimate().await;
    let qbi_cents = v["qbi_deduction_cents"].as_i64().unwrap();
    assert!(qbi_cents > 0);
    assert_eq!(
        rust_decimal::Decimal::from(qbi_cents),
        dollars(&v, "qbi_deduction") * rust_decimal::Decimal::ONE_HUNDRED
    );

    // Last year's AGI above $150,000: 110% of last year's tax.
    assert!(
        !set(json!({ "year": 2025, "total_tax_cents": 1_500_000, "agi_cents": 20_000_000 }))
            .await
            .is_error()
    );
    let v = estimate().await;
    assert_eq!(v["safe_harbor_prior_year_percentage"], "1.10");
    assert_eq!(
        dollars(&v, "safe_harbor_amount"),
        rust_decimal::Decimal::from(16_500)
    );
    let note = v["safe_harbor_note"].as_str().unwrap();
    assert!(note.contains("from your filed 2025 return"), "{note}");
    assert!(!note.contains("Not exact"), "{note}");

    // At exactly $150,000: 100%.
    assert!(
        !set(json!({ "year": 2025, "agi_cents": 15_000_000 }))
            .await
            .is_error()
    );
    let v = estimate().await;
    assert_eq!(v["safe_harbor_prior_year_percentage"], "1.00");
    assert_eq!(
        dollars(&v, "safe_harbor_amount"),
        rust_decimal::Decimal::from(15_000)
    );

    // Without last year's AGI, 100% is assumed and the note says so.
    assert!(
        !set(json!({ "year": 2025, "agi_cents": null }))
            .await
            .is_error()
    );
    let v = estimate().await;
    assert_eq!(v["safe_harbor_prior_year_percentage"], "1.00");
    let note = v["safe_harbor_note"].as_str().unwrap();
    assert!(note.contains("2025's AGI is not entered"), "{note}");
    assert!(note.contains("aequi_set_tax_return"), "{note}");
}

#[tokio::test]
async fn dates_are_validated() {
    let db = test_db().await;
    aequi_storage::insert_contact(&db, "C", None, None, None, "customer", false, None, None)
        .await
        .unwrap();

    let cases = [
        ("aequi_get_profit_loss", json!({ "start_date": "2026-1-1" })),
        (
            "aequi_get_profit_loss",
            json!({ "start_date": "2026-03-01", "end_date": "2026-02-01" }),
        ),
        ("aequi_get_transactions", json!({ "end_date": "yesterday" })),
        (
            "aequi_draft_invoice",
            json!({ "invoice_number": "I-1", "contact_id": 1, "issue_date": "2026-02-30", "due_date": "2026-03-31" }),
        ),
        (
            "aequi_create_reconciliation_session",
            json!({ "account_id": 1, "start_date": "01/01/2026", "end_date": "2026-01-31", "statement_balance_cents": 0 }),
        ),
        (
            "aequi_ingest_receipt",
            json!({ "file_path": "/nonexistent/receipt.jpg", "date": "March 1" }),
        ),
    ];
    for (tool, args) in cases {
        let result = call(&db, tool, args.clone()).await;
        assert!(result.is_error(), "{tool} accepted {args}");
    }
}

// -----------------------------------------------------------------------
// Regression tests: date filters
// -----------------------------------------------------------------------

#[tokio::test]
async fn get_transactions_filters_by_date() {
    let db = test_db().await;
    for (date, desc) in [
        ("2026-01-15", "January"),
        ("2026-02-15", "February"),
        ("2026-03-15", "March"),
    ] {
        let r = call(
            &db,
            "aequi_create_transaction",
            balanced_tx(date, desc, 100),
        )
        .await;
        assert!(!r.is_error());
    }

    let descriptions = |r: ToolResult| -> Vec<String> {
        let v: serde_json::Value = serde_json::from_str(r.content[0].as_text().unwrap()).unwrap();
        v.as_array()
            .unwrap()
            .iter()
            .map(|t| t["description"].as_str().unwrap().to_string())
            .collect()
    };

    let r = call(
        &db,
        "aequi_get_transactions",
        json!({ "start_date": "2026-02-01", "end_date": "2026-02-28" }),
    )
    .await;
    assert_eq!(descriptions(r), ["February"]);

    let r = call(
        &db,
        "aequi_get_transactions",
        json!({ "start_date": "2026-02-15" }),
    )
    .await;
    assert_eq!(descriptions(r), ["March", "February"]);

    let r = call(
        &db,
        "aequi_get_transactions",
        json!({ "end_date": "2026-02-14" }),
    )
    .await;
    assert_eq!(descriptions(r), ["January"]);

    let r = call(&db, "aequi_get_transactions", json!({ "limit": 1 })).await;
    assert_eq!(descriptions(r), ["March"]);

    assert!(
        call(&db, "aequi_get_transactions", json!({ "limit": 0 }))
            .await
            .is_error()
    );
}

// -----------------------------------------------------------------------
// Regression tests: not-found and error propagation
// -----------------------------------------------------------------------

#[tokio::test]
async fn nonexistent_ids_report_not_found() {
    let db = test_db().await;
    for (tool, args) in [
        ("aequi_reject_receipt", json!({ "receipt_id": 99 })),
        (
            "aequi_resolve_item",
            json!({ "item_id": 99, "notes": "ok" }),
        ),
        (
            "aequi_record_payment",
            json!({ "invoice_id": 99, "amount_cents": 100, "date": "2026-03-01" }),
        ),
        (
            "aequi_get_reconciliation_items",
            json!({ "session_id": 99 }),
        ),
        ("aequi_draft_invoice", invoice_args("I-9", 99)),
        ("aequi_get_invoice", json!({ "invoice_id": 99 })),
        ("aequi_get_receipt_image", json!({ "receipt_id": 99 })),
        (
            "aequi_update_receipt_fields",
            json!({ "receipt_id": 99, "vendor": "X", "confidence": 0.5 }),
        ),
        (
            "aequi_post_import",
            json!({ "imported_id": 99, "bank_account_code": "1000", "category_account_code": "5010" }),
        ),
        ("aequi_ignore_import", json!({ "imported_id": 99 })),
        (
            "aequi_create_reconciliation_session",
            json!({ "account_id": 9999, "start_date": "2026-01-01", "end_date": "2026-01-31", "statement_balance_cents": 0 }),
        ),
    ] {
        let result = call(&db, tool, args).await;
        assert!(result.is_error(), "{tool}");
        assert!(
            result.content[0].as_text().unwrap().contains("not found"),
            "{tool}: {}",
            result.content[0].as_text().unwrap()
        );
    }
}

// -----------------------------------------------------------------------
// Regression tests: rule matching
// -----------------------------------------------------------------------

async fn insert_pending_import(db: &aequi_storage::DbPool, batch: &str, description: &str) -> i64 {
    insert_import(db, batch, description, -1000).await
}

async fn insert_import(
    db: &aequi_storage::DbPool,
    batch: &str,
    description: &str,
    amount_cents: i64,
) -> i64 {
    aequi_storage::insert_imported_transaction(
        db,
        &aequi_storage::ImportedTransaction {
            id: 0,
            source_type: "csv".to_string(),
            source_id: None,
            import_batch_id: batch.to_string(),
            date: "2026-03-01".to_string(),
            description: description.to_string(),
            amount_cents,
            debit_cents: None,
            credit_cents: None,
            memo: None,
            matched_transaction_id: None,
            category_rule_id: None,
            status: "pending".to_string(),
            created_at: String::new(),
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn apply_rules_uses_regex_and_fuzzy_semantics() {
    let db = test_db().await;
    let saas = insert_pending_import(&db, "b1", "SAAS TOOL MONTHLY").await;
    let literal = insert_pending_import(&db, "b1", "charge SAAS.*MONTHLY").await;
    let aws = insert_pending_import(&db, "b1", "AMAZON WEB SERVICE").await;
    let other = insert_pending_import(&db, "b1", "COFFEE SHOP").await;

    for (name, pattern, match_type) in [
        ("SaaS", "^SAAS .* MONTHLY$", "regex"),
        ("AWS", "amazon web services", "fuzzy"),
    ] {
        let r = call(
            &db,
            "aequi_save_categorization_rule",
            json!({
                "name": name, "priority": 1, "match_pattern": pattern,
                "match_type": match_type, "account_id": 1
            }),
        )
        .await;
        assert!(!r.is_error(), "{}", r.content[0].as_text().unwrap());
    }

    let r = call(&db, "aequi_apply_rules", json!({ "batch_id": "b1" })).await;
    assert!(!r.is_error(), "{}", r.content[0].as_text().unwrap());
    let v: serde_json::Value = serde_json::from_str(r.content[0].as_text().unwrap()).unwrap();
    assert_eq!(v["total_pending"], 4);
    assert_eq!(v["matched"], 2);
    assert_eq!(v["unmatched"], 2);

    let status = |id: i64| {
        let db = db.clone();
        async move {
            let (s,): (String,) =
                sqlx::query_as("SELECT status FROM imported_transactions WHERE id = ?")
                    .bind(id)
                    .fetch_one(&db)
                    .await
                    .unwrap();
            s
        }
    };
    assert_eq!(status(saas).await, "categorized");
    assert_eq!(status(aws).await, "categorized");
    assert_eq!(status(literal).await, "pending");
    assert_eq!(status(other).await, "pending");
}

#[tokio::test]
async fn apply_rules_skips_invalid_stored_rules() {
    let db = test_db().await;
    insert_pending_import(&db, "b2", "anything").await;
    // A rule stored by another path without validation.
    aequi_storage::save_categorization_rule(
        &db,
        &aequi_storage::CategorizationRule {
            id: 0,
            name: "broken".to_string(),
            priority: 1,
            match_pattern: "(".to_string(),
            match_type: "regex".to_string(),
            account_id: 1,
            created_at: String::new(),
        },
    )
    .await
    .unwrap();

    let r = call(&db, "aequi_apply_rules", json!({ "batch_id": "b2" })).await;
    assert!(!r.is_error());
    let v: serde_json::Value = serde_json::from_str(r.content[0].as_text().unwrap()).unwrap();
    assert_eq!(v["matched"], 0);
    assert_eq!(v["skipped_rules"].as_array().unwrap().len(), 1);
}

// -----------------------------------------------------------------------
// Regression tests: receipt ingestion
// -----------------------------------------------------------------------

async fn file_db(dir: &std::path::Path) -> aequi_storage::DbPool {
    let db = aequi_storage::create_db(&dir.join("ledger.db"))
        .await
        .unwrap();
    aequi_storage::seed_default_accounts(&db).await.unwrap();
    db
}

#[tokio::test]
async fn ingest_receipt_copies_into_attachments() {
    let data_dir = tempfile::tempdir().unwrap();
    let source_dir = tempfile::tempdir().unwrap();
    let db = file_db(data_dir.path()).await;

    let source = source_dir.path().join("Receipt.PNG");
    std::fs::write(&source, b"fake png bytes").unwrap();

    let result = call(
        &db,
        "aequi_ingest_receipt",
        json!({ "file_path": source.to_str().unwrap(), "vendor": "Shop", "date": "2026-03-01", "total_cents": 1234 }),
    )
    .await;
    assert!(
        !result.is_error(),
        "{}",
        result.content[0].as_text().unwrap()
    );
    let v: serde_json::Value = serde_json::from_str(result.content[0].as_text().unwrap()).unwrap();
    let hash = v["file_hash"].as_str().unwrap();

    let expected = data_dir
        .path()
        .canonicalize()
        .unwrap()
        .join("attachments")
        .join(&hash[..2])
        .join(format!("{hash}.png"));
    let stored = std::path::PathBuf::from(v["attachment_path"].as_str().unwrap());
    assert_eq!(stored.canonicalize().unwrap(), expected);
    assert_eq!(std::fs::read(&stored).unwrap(), b"fake png bytes");

    let receipt = aequi_storage::get_receipt_by_id(&db, v["receipt_id"].as_i64().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.attachment_path,
        v["attachment_path"].as_str().unwrap()
    );
    assert_eq!(receipt.file_ext, "png");

    // No temp files left behind.
    let leftovers: Vec<_> = std::fs::read_dir(data_dir.path().join("attachments"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());

    // Same content again is a duplicate.
    let again = source_dir.path().join("copy.png");
    std::fs::write(&again, b"fake png bytes").unwrap();
    let dup = call(
        &db,
        "aequi_ingest_receipt",
        json!({ "file_path": again.to_str().unwrap() }),
    )
    .await;
    assert!(dup.is_error());
    assert!(
        dup.content[0]
            .as_text()
            .unwrap()
            .contains("Duplicate receipt")
    );
}

#[tokio::test]
async fn ingest_receipt_rejects_directories_and_large_files() {
    let data_dir = tempfile::tempdir().unwrap();
    let db = file_db(data_dir.path()).await;

    let dir_path = data_dir.path().join("folder.jpg");
    std::fs::create_dir(&dir_path).unwrap();
    let result = call(
        &db,
        "aequi_ingest_receipt",
        json!({ "file_path": dir_path.to_str().unwrap() }),
    )
    .await;
    assert!(result.is_error());
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("regular file")
    );

    // Sparse file just over the cap.
    let big = data_dir.path().join("big.pdf");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(crate::tools::MAX_RECEIPT_BYTES + 1).unwrap();
    drop(f);
    let result = call(
        &db,
        "aequi_ingest_receipt",
        json!({ "file_path": big.to_str().unwrap() }),
    )
    .await;
    assert!(result.is_error());
    assert!(result.content[0].as_text().unwrap().contains("too large"));
    assert_eq!(count(&db, "receipts").await, 0);
}

#[tokio::test]
async fn ingest_receipt_needs_file_backed_ledger() {
    let db = test_db().await;
    let source_dir = tempfile::tempdir().unwrap();
    let source = source_dir.path().join("r.jpg");
    std::fs::write(&source, b"jpg").unwrap();
    let result = call(
        &db,
        "aequi_ingest_receipt",
        json!({ "file_path": source.to_str().unwrap() }),
    )
    .await;
    assert!(result.is_error());
    assert!(result.content[0].as_text().unwrap().contains("file-backed"));
}

#[tokio::test]
async fn failed_tool_calls_are_audited_as_errors() {
    let db = test_db().await;
    let server = crate::server::McpServer::new(db.clone(), Permissions::default());
    let msg = br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"aequi_get_account","arguments":{}}}"#;
    let resp = server.handle_message(msg).await.unwrap();
    assert_eq!(resp.result.unwrap()["isError"], true);
    let logs = aequi_storage::get_audit_log(&db, 10).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].tool_name, "aequi_get_account");
    assert_eq!(logs[0].outcome, "error");
}

// -----------------------------------------------------------------------
// Receipt extraction by an agent: image access and field updates
// -----------------------------------------------------------------------

const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR not really a png";

async fn insert_test_receipt(db: &aequi_storage::DbPool, hash: &str, path: &str) -> i64 {
    aequi_storage::insert_receipt(
        db,
        hash,
        "png",
        path,
        Some("OCR TEXT"),
        Some("OCR Vendr"),
        None,
        Some(1000),
        None,
        None,
        None,
        0.4,
    )
    .await
    .unwrap()
}

fn decode_base64(data: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap()
}

#[tokio::test]
async fn receipt_image_returned_as_image_block() {
    let data_dir = tempfile::tempdir().unwrap();
    let source_dir = tempfile::tempdir().unwrap();
    let db = file_db(data_dir.path()).await;

    // The extension says JPEG; the bytes say PNG, and the bytes win.
    let source = source_dir.path().join("receipt.jpg");
    std::fs::write(&source, PNG_BYTES).unwrap();
    let v = parse(
        &call(
            &db,
            "aequi_ingest_receipt",
            json!({ "file_path": source.to_str().unwrap(), "vendor": "Shop" }),
        )
        .await,
    );
    let id = v["receipt_id"].as_i64().unwrap();

    let result = call(&db, "aequi_get_receipt_image", json!({ "receipt_id": id })).await;
    let info = parse(&result);
    assert_eq!(info["receipt_id"], id);
    assert_eq!(info["vendor"], "Shop");
    assert_eq!(info["status"], "pending_review");
    assert_eq!(info["mime_type"], "image/png");
    assert_eq!(info["size_bytes"], PNG_BYTES.len());
    assert_eq!(result.content.len(), 2);
    match &result.content[1] {
        crate::protocol::ToolContent::Image { data, mime_type } => {
            assert_eq!(mime_type, "image/png");
            assert_eq!(decode_base64(data), PNG_BYTES);
        }
        other => panic!("expected an image block, got {other:?}"),
    }

    // MCP wire format.
    let server = crate::server::McpServer::new(db.clone(), Permissions::default());
    let msg = format!(
        r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"aequi_get_receipt_image","arguments":{{"receipt_id":{id}}}}}}}"#
    );
    let resp = to_json(&server.handle_message(msg.as_bytes()).await.unwrap());
    let content = &resp["result"]["content"];
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["mimeType"], "image/png");
    assert_eq!(
        decode_base64(content[1]["data"].as_str().unwrap()),
        PNG_BYTES
    );
    assert!(resp["result"].get("isError").is_none());
}

#[tokio::test]
async fn receipt_pdf_returned_as_embedded_resource() {
    let data_dir = tempfile::tempdir().unwrap();
    let source_dir = tempfile::tempdir().unwrap();
    let db = file_db(data_dir.path()).await;
    let pdf = b"%PDF-1.4\n% fake receipt\n";
    let source = source_dir.path().join("invoice.pdf");
    std::fs::write(&source, pdf).unwrap();
    let id = parse(
        &call(
            &db,
            "aequi_ingest_receipt",
            json!({ "file_path": source.to_str().unwrap() }),
        )
        .await,
    )["receipt_id"]
        .as_i64()
        .unwrap();

    let result = call(&db, "aequi_get_receipt_image", json!({ "receipt_id": id })).await;
    assert_eq!(parse(&result)["mime_type"], "application/pdf");
    let wire = serde_json::to_value(&result).unwrap();
    let block = &wire["content"][1];
    assert_eq!(block["type"], "resource");
    assert_eq!(block["resource"]["mimeType"], "application/pdf");
    assert_eq!(block["resource"]["uri"], format!("aequi://receipts/{id}"));
    assert_eq!(
        decode_base64(block["resource"]["blob"].as_str().unwrap()),
        pdf
    );
}

#[tokio::test]
async fn receipt_image_refuses_paths_outside_attachments() {
    let data_dir = tempfile::tempdir().unwrap();
    let outside_dir = tempfile::tempdir().unwrap();
    let db = file_db(data_dir.path()).await;
    let attachments = data_dir.path().join("attachments");
    std::fs::create_dir_all(&attachments).unwrap();

    let outside = outside_dir.path().join("secret.png");
    std::fs::write(&outside, PNG_BYTES).unwrap();
    let mut paths = vec![
        outside.clone(),
        // `..` that climbs out to the ledger itself.
        attachments.join("..").join("ledger.db"),
    ];
    #[cfg(unix)]
    {
        let link = attachments.join("link.png");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        paths.push(link);
    }

    for (i, path) in paths.iter().enumerate() {
        let id = insert_test_receipt(&db, &format!("out-{i}"), path.to_str().unwrap()).await;
        let result = call(&db, "aequi_get_receipt_image", json!({ "receipt_id": id })).await;
        let text = result.content[0].as_text().unwrap();
        assert!(result.is_error(), "{}: {text}", path.display());
        assert!(text.contains("outside"), "{}: {text}", path.display());
        assert_eq!(result.content.len(), 1);
    }
}

#[tokio::test]
async fn receipt_image_refuses_bad_files() {
    let data_dir = tempfile::tempdir().unwrap();
    let db = file_db(data_dir.path()).await;
    let attachments = data_dir.path().join("attachments");
    std::fs::create_dir_all(&attachments).unwrap();

    let dir = attachments.join("folder.png");
    std::fs::create_dir(&dir).unwrap();
    let big = attachments.join("big.png");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(crate::tools::MAX_RECEIPT_IMAGE_BYTES + 1)
        .unwrap();
    drop(f);
    let text_file = attachments.join("notes.png");
    std::fs::write(&text_file, b"just some text").unwrap();
    let missing = attachments.join("missing.png");

    for (i, (path, expected)) in [
        (dir, "not a regular file"),
        (big, "larger than 20 MB"),
        (text_file, "not a supported image"),
        (missing, "can't be read"),
    ]
    .iter()
    .enumerate()
    {
        let id = insert_test_receipt(&db, &format!("bad-{i}"), path.to_str().unwrap()).await;
        let result = call(&db, "aequi_get_receipt_image", json!({ "receipt_id": id })).await;
        let text = result.content[0].as_text().unwrap();
        assert!(result.is_error(), "{expected}: {text}");
        assert!(text.contains(expected), "{expected}: {text}");
    }
}

#[tokio::test]
async fn receipt_image_needs_file_backed_ledger() {
    let db = test_db().await;
    let id = insert_test_receipt(&db, "mem", "/tmp/whatever.png").await;
    let result = call(&db, "aequi_get_receipt_image", json!({ "receipt_id": id })).await;
    assert!(result.is_error());
    assert!(result.content[0].as_text().unwrap().contains("file-backed"));
}

#[tokio::test]
async fn update_receipt_fields_merges_and_stays_pending() {
    let db = test_db().await;
    let id = insert_test_receipt(&db, "h-upd", "/tmp/none.png").await;

    let v = parse(
        &call(
            &db,
            "aequi_update_receipt_fields",
            json!({
                "receipt_id": id, "vendor": "Blue Bottle", "receipt_date": "2026-03-04",
                "total_cents": 1250, "tax_cents": 100, "confidence": 0.9
            }),
        )
        .await,
    );
    assert_eq!(v["vendor"], "Blue Bottle");
    assert_eq!(v["receipt_date"], "2026-03-04");
    assert_eq!(v["total_cents"], 1250);
    assert_eq!(v["tax_cents"], 100);
    assert!(v["subtotal_cents"].is_null());
    assert_eq!(v["confidence"], 0.9);
    assert_eq!(v["status"], "pending_review");

    // Omitted fields keep their values.
    let v = parse(
        &call(
            &db,
            "aequi_update_receipt_fields",
            json!({ "receipt_id": id, "payment_method": "Visa ending 1234", "confidence": 0.95 }),
        )
        .await,
    );
    assert_eq!(v["vendor"], "Blue Bottle");
    assert_eq!(v["total_cents"], 1250);
    assert_eq!(v["payment_method"], "Visa ending 1234");
    assert_eq!(v["confidence"], 0.95);

    let pending = parse(&call(&db, "aequi_get_pending_receipts", json!({})).await);
    assert_eq!(pending.as_array().unwrap().len(), 1);
    let row = &pending[0];
    assert_eq!(row["receipt_id"], id);
    assert_eq!(row["status"], "pending_review");
    assert_eq!(row["vendor"], "Blue Bottle");
    assert_eq!(row["total_cents"], 1250);
    assert_eq!(row["confidence"], 0.95);
    assert_eq!(row["ocr_text"], "OCR TEXT");
}

#[tokio::test]
async fn update_receipt_fields_validates() {
    let db = test_db().await;
    let id = insert_test_receipt(&db, "h-val", "/tmp/none.png").await;

    for (args, expected) in [
        (
            json!({ "receipt_id": id, "vendor": "X" }),
            "Missing required argument: confidence",
        ),
        (
            json!({ "receipt_id": id, "confidence": 1.5 }),
            "between 0 and 1",
        ),
        (
            json!({ "receipt_id": id, "confidence": "high" }),
            "confidence",
        ),
        (
            json!({ "receipt_id": id, "confidence": 0.5, "total_cents": -5 }),
            "must not be negative",
        ),
        (
            json!({ "receipt_id": id, "confidence": 0.5, "total_cents": 12.5 }),
            "whole number of cents",
        ),
        (
            json!({ "receipt_id": id, "confidence": 0.5, "receipt_date": "03/04/2026" }),
            "receipt_date",
        ),
        (
            json!({ "receipt_id": id, "confidence": 0.5, "vendr": "X" }),
            "vendr",
        ),
    ] {
        let result = call(&db, "aequi_update_receipt_fields", args).await;
        let text = result.content[0].as_text().unwrap();
        assert!(result.is_error(), "{expected}: {text}");
        assert!(text.contains(expected), "{expected}: {text}");
    }
    let receipt = aequi_storage::get_receipt_by_id(&db, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.vendor.as_deref(), Some("OCR Vendr"));
    assert_eq!(receipt.confidence, 0.4);

    // Once a person has decided, the agent can't change it.
    assert!(
        !call(&db, "aequi_reject_receipt", json!({ "receipt_id": id }))
            .await
            .is_error()
    );
    let result = call(
        &db,
        "aequi_update_receipt_fields",
        json!({ "receipt_id": id, "vendor": "Y", "confidence": 0.5 }),
    )
    .await;
    assert!(result.is_error());
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("can no longer be edited")
    );
}

#[tokio::test]
async fn read_only_blocks_new_write_tools_with_valid_arguments() {
    let db = test_db().await;
    let contact = add_contact(&db, "RO Client", false).await;
    let receipt = insert_test_receipt(&db, "h-ro", "/tmp/none.png").await;
    let import = insert_pending_import(&db, "b1", "COFFEE").await;
    aequi_storage::set_setting(&db, "mcp_read_only", "true")
        .await
        .unwrap();

    for (tool, args) in [
        (
            "aequi_update_receipt_fields",
            json!({ "receipt_id": receipt, "vendor": "Changed", "confidence": 0.9 }),
        ),
        (
            "aequi_post_import",
            json!({ "imported_id": import, "bank_account_code": "1000", "category_account_code": "5100" }),
        ),
        ("aequi_ignore_import", json!({ "imported_id": import })),
        ("aequi_draft_invoice", invoice_args("INV-RO", contact)),
        (
            "aequi_set_tax_return",
            json!({ "year": 2025, "total_tax_cents": 1_500_000, "agi_cents": 20_000_000 }),
        ),
    ] {
        let result = call(&db, tool, args).await;
        let text = result.content[0].as_text().unwrap();
        assert!(result.is_error(), "{tool} ran in read-only mode");
        assert!(text.contains("read-only"), "{tool}: {text}");
    }

    let receipt = aequi_storage::get_receipt_by_id(&db, receipt)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.vendor.as_deref(), Some("OCR Vendr"));
    let pending = parse(&call(&db, "aequi_get_pending_imports", json!({})).await);
    assert_eq!(pending[0]["id"], import);
    assert_eq!(pending[0]["status"], "pending");
    assert_eq!(count(&db, "transactions").await, 0);
    assert_eq!(count(&db, "invoices").await, 0);
    assert_eq!(
        aequi_storage::get_tax_return_figures(&db, 2025)
            .await
            .unwrap(),
        aequi_storage::TaxReturnFigures::default()
    );

    // The new read tools still work.
    for (tool, args) in [
        ("aequi_list_contacts", json!({})),
        ("aequi_get_1099_summary", json!({})),
        ("aequi_get_tax_return", json!({ "year": 2025 })),
    ] {
        let result = call(&db, tool, args).await;
        assert!(!result.is_error(), "{tool}");
    }
}

// -----------------------------------------------------------------------
// Contacts and payees
// -----------------------------------------------------------------------

#[tokio::test]
async fn list_contacts_returns_ids_for_other_tools() {
    let db = test_db().await;
    let dev = add_contact(&db, "Dev Co", true).await;
    let acme = add_contact(&db, "Acme", false).await;

    let result = call(&db, "aequi_list_contacts", json!({})).await;
    let v = parse(&result);
    assert_eq!(
        v,
        json!([
            { "id": acme, "name": "Acme", "contact_type": "customer", "is_contractor": false, "email": "billing@example.com" },
            { "id": dev, "name": "Dev Co", "contact_type": "vendor", "is_contractor": true, "email": "billing@example.com" }
        ])
    );
    // Tax ids are not exposed.
    assert!(!result.content[0].as_text().unwrap().contains("12-3456789"));
}

#[tokio::test]
async fn create_transaction_with_contact() {
    let db = test_db().await;
    let contact = add_contact(&db, "Dev Co", true).await;

    let mut args = balanced_tx("2026-03-01", "Paid Dev Co", 5000);
    args["contact_id"] = json!(contact);
    let v = parse(&call(&db, "aequi_create_transaction", args.clone()).await);
    let id = v["id"].as_i64().unwrap();
    let listed = parse(&call(&db, "aequi_get_transactions", json!({})).await);
    assert_eq!(listed[0]["id"], id);
    assert_eq!(listed[0]["contact_id"], contact);

    args["contact_id"] = json!(99);
    let result = call(&db, "aequi_create_transaction", args).await;
    assert!(result.is_error());
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("contact 99 does not exist"),
        "{}",
        result.content[0].as_text().unwrap()
    );

    let mut args = balanced_tx("2026-03-01", "Typo", 5000);
    args["lines"][0]["debit"] = json!(5000);
    let result = call(&db, "aequi_create_transaction", args).await;
    assert!(result.is_error());
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("lines[0].debit")
    );
    assert_eq!(count(&db, "transactions").await, 1);
}

// -----------------------------------------------------------------------
// Import review
// -----------------------------------------------------------------------

async fn import_status(db: &aequi_storage::DbPool, id: i64) -> String {
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM imported_transactions WHERE id = ?")
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap();
    status
}

#[tokio::test]
async fn pending_imports_across_batches() {
    let db = test_db().await;
    let a = insert_pending_import(&db, "b1", "AMAZON MARKETPLACE").await;
    let b = insert_pending_import(&db, "b2", "COFFEE SHOP").await;

    let ids = |v: &serde_json::Value| -> Vec<i64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_i64().unwrap())
            .collect()
    };
    let all = parse(&call(&db, "aequi_get_pending_imports", json!({})).await);
    assert_eq!(ids(&all), [a, b]);
    let only_b2 = parse(
        &call(
            &db,
            "aequi_get_pending_imports",
            json!({ "batch_id": "b2" }),
        )
        .await,
    );
    assert_eq!(ids(&only_b2), [b]);
    let limited = parse(&call(&db, "aequi_get_pending_imports", json!({ "limit": 1 })).await);
    assert_eq!(ids(&limited), [a]);
    for limit in [0, 1001] {
        let result = call(&db, "aequi_get_pending_imports", json!({ "limit": limit })).await;
        assert!(result.is_error(), "limit {limit}");
    }

    // A matched categorization rule shows up as the suggested category.
    let office = aequi_storage::get_account_by_code(&db, "5100")
        .await
        .unwrap()
        .unwrap()
        .id
        .unwrap()
        .0;
    let r = call(
        &db,
        "aequi_save_categorization_rule",
        json!({ "name": "Amazon", "priority": 1, "match_pattern": "amazon", "match_type": "contains", "account_id": office }),
    )
    .await;
    assert!(!r.is_error());
    assert!(
        !call(&db, "aequi_apply_rules", json!({ "batch_id": "b1" }))
            .await
            .is_error()
    );
    let all = parse(&call(&db, "aequi_get_pending_imports", json!({})).await);
    assert_eq!(all[0]["status"], "categorized");
    assert_eq!(all[0]["suggested_category"]["account_code"], "5100");
    assert_eq!(
        all[0]["suggested_category"]["account_name"],
        "Office Supplies"
    );
    assert!(all[1]["suggested_category"].is_null());
}

#[tokio::test]
async fn post_import_creates_ledger_entry() {
    let db = test_db().await;
    let payee = add_contact(&db, "Office Depot", false).await;
    let out = insert_pending_import(&db, "b1", "OFFICE DEPOT").await;

    let v = parse(
        &call(
            &db,
            "aequi_post_import",
            json!({ "imported_id": out, "bank_account_code": "1000", "category_account_code": "5100", "contact_id": payee }),
        )
        .await,
    );
    assert_eq!(v["status"], "posted");
    let tx = v["transaction_id"].as_i64().unwrap();
    // Negative import: money out of checking into the expense.
    assert_eq!(posted(&db, tx, "5100").await, 1000);
    assert_eq!(posted(&db, tx, "1000").await, -1000);
    let (contact_id,): (Option<i64>,) =
        sqlx::query_as("SELECT contact_id FROM transactions WHERE id = ?")
            .bind(tx)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(contact_id, Some(payee));
    assert_eq!(import_status(&db, out).await, "posted");

    let again = call(
        &db,
        "aequi_post_import",
        json!({ "imported_id": out, "bank_account_code": "1000", "category_account_code": "5100" }),
    )
    .await;
    assert!(again.is_error());
    assert!(
        again.content[0]
            .as_text()
            .unwrap()
            .contains("already posted")
    );

    // Positive import: money into checking from income.
    let incoming = insert_import(&db, "b1", "CLIENT ACH", 5000).await;
    let v = parse(
        &call(
            &db,
            "aequi_post_import",
            json!({ "imported_id": incoming, "bank_account_code": "1000", "category_account_code": "4000" }),
        )
        .await,
    );
    let tx = v["transaction_id"].as_i64().unwrap();
    assert_eq!(posted(&db, tx, "1000").await, 5000);
    assert_eq!(posted(&db, tx, "4000").await, -5000);

    // No category and no rule, or a non-bank account: refused, nothing posted.
    let mystery = insert_import(&db, "b1", "MYSTERY", -200).await;
    for (args, expected) in [
        (
            json!({ "imported_id": mystery, "bank_account_code": "1000" }),
            "no categorization rule",
        ),
        (
            json!({ "imported_id": mystery, "bank_account_code": "5100", "category_account_code": "5010" }),
            "bank (asset) or card (liability)",
        ),
    ] {
        let result = call(&db, "aequi_post_import", args).await;
        let text = result.content[0].as_text().unwrap();
        assert!(result.is_error(), "{expected}: {text}");
        assert!(text.contains(expected), "{expected}: {text}");
    }
    assert_eq!(import_status(&db, mystery).await, "pending");
    assert_eq!(count(&db, "transactions").await, 2);
}

#[tokio::test]
async fn ignore_import_dismisses_it() {
    let db = test_db().await;
    let id = insert_pending_import(&db, "b1", "TRANSFER TO SAVINGS").await;

    let v = parse(&call(&db, "aequi_ignore_import", json!({ "imported_id": id })).await);
    assert_eq!(v["status"], "ignored");
    assert_eq!(import_status(&db, id).await, "ignored");
    let pending = parse(&call(&db, "aequi_get_pending_imports", json!({})).await);
    assert_eq!(pending, json!([]));

    let again = call(&db, "aequi_ignore_import", json!({ "imported_id": id })).await;
    assert!(again.is_error());
    assert!(
        again.content[0]
            .as_text()
            .unwrap()
            .contains("already ignored")
    );
    let post = call(
        &db,
        "aequi_post_import",
        json!({ "imported_id": id, "bank_account_code": "1000", "category_account_code": "5100" }),
    )
    .await;
    assert!(post.is_error());
    assert_eq!(count(&db, "transactions").await, 0);
}

// -----------------------------------------------------------------------
// 1099-NEC summary
// -----------------------------------------------------------------------

#[tokio::test]
async fn summary_1099_counts_bank_payments_to_contractors() {
    let db = test_db().await;
    let big = add_contact(&db, "Big Contractor", true).await;
    let small = aequi_storage::insert_contact(
        &db,
        "Small Contractor",
        None,
        None,
        None,
        "vendor",
        true,
        None,
        None,
    )
    .await
    .unwrap();
    let client = add_contact(&db, "Client", false).await;

    let pay = |contact: i64, cents: i64, from: &'static str, date: &'static str| {
        let db = db.clone();
        async move {
            let args = json!({
                "date": date,
                "description": "Contract work",
                "contact_id": contact,
                "lines": [
                    { "account_code": "5080", "debit_cents": cents, "credit_cents": 0 },
                    { "account_code": from, "debit_cents": 0, "credit_cents": cents }
                ]
            });
            parse(&call(&db, "aequi_create_transaction", args).await);
        }
    };
    pay(big, 150_000, "1000", "2026-02-01").await;
    pay(big, 60_000, "1000", "2026-05-01").await;
    pay(big, 500_000, "2000", "2026-06-01").await; // by card: 1099-K, not NEC
    pay(big, 70_000, "1000", "2025-12-31").await;
    pay(small, 50_000, "1000", "2026-03-01").await;
    pay(client, 900_000, "1000", "2026-03-01").await; // not a contractor

    let v = parse(&call(&db, "aequi_get_1099_summary", json!({ "year": 2026 })).await);
    assert_eq!(v["year"], 2026);
    assert_eq!(v["threshold_cents"], 200_000);
    assert_eq!(v["contractors_over_threshold"], 1);
    assert_eq!(
        v["contractors"],
        json!([
            { "contact_id": big, "name": "Big Contractor", "paid_cents": 210_000, "over_threshold": true, "has_tax_id": true },
            { "contact_id": small, "name": "Small Contractor", "paid_cents": 50_000, "over_threshold": false, "has_tax_id": false }
        ])
    );

    let v = parse(&call(&db, "aequi_get_1099_summary", json!({ "year": 2025 })).await);
    assert_eq!(v["threshold_cents"], 60_000);
    assert_eq!(v["contractors"][0]["paid_cents"], 70_000);
    assert_eq!(v["contractors"][0]["over_threshold"], true);
    assert_eq!(v["contractors"][1]["paid_cents"], 0);

    assert!(
        !call(&db, "aequi_get_1099_summary", json!({}))
            .await
            .is_error()
    );
    assert!(
        call(&db, "aequi_get_1099_summary", json!({ "year": 1999 }))
            .await
            .is_error()
    );
}
