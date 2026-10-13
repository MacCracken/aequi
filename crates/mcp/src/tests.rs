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
        "aequi_approve_receipt",
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
    ];

    for name in expected {
        assert!(names.contains(&name), "Missing tool: {name}");
    }
}

#[test]
fn registry_tool_count() {
    let registry = ToolRegistry::new();
    assert_eq!(registry.list_definitions().len(), 24);
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
    assert!(result.content[0].text.contains("not allowed"));
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
    let text = &result.content[0].text;
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
    assert!(result.content[0].text.contains("Checking"));
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
    assert!(result.content[0].text.contains("not found"));
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
    assert!(result.content[0].text.contains("created"));

    let list = registry
        .call(
            "aequi_get_transactions",
            json!({ "limit": 10 }),
            &db,
            &perms,
        )
        .await;
    assert!(list.is_error.is_none());
    assert!(list.content[0].text.contains("Client payment"));
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
    assert!(result.content[0].text.contains("YYYY-MM-DD"));
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
    let text = &result.content[0].text;
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
    assert!(result.content[0].text.contains("[]"));
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
    assert!(result.content[0].text.contains("Cannot read file"));
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
    assert!(result.is_error.is_none(), "{}", result.content[0].text);
    let text = &result.content[0].text;
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
    assert!(result.content[0].text.contains("net_profit"));
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
    assert!(result.content[0].text.contains("rule_id"));

    let list = registry
        .call("aequi_get_categorization_rules", json!({}), &db, &perms)
        .await;
    assert!(list.content[0].text.contains("GITHUB"));
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
    assert!(result.content[0].text.contains("profile_id"));

    let list = registry
        .call("aequi_get_import_profiles", json!({}), &db, &perms)
        .await;
    assert!(list.content[0].text.contains("Chase Checking"));
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
    assert!(result.content[0].text.contains("session_id"));
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
    assert_eq!(r.content[0].text, "hello");
}

#[test]
fn tool_result_error() {
    let r = ToolResult::error("fail".to_string());
    assert!(r.is_error.unwrap());
    assert_eq!(r.content[0].text, "fail");
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

#[tokio::test]
async fn draft_invoice_and_list_unpaid() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    // Insert a contact first so we have a valid contact_id
    aequi_storage::insert_contact(
        &db,
        "Test Client",
        Some("test@example.com"),
        None,
        None,
        "customer",
        false,
        None,
        None,
    )
    .await
    .unwrap();

    let result = registry
        .call(
            "aequi_draft_invoice",
            json!({
                "invoice_number": "INV-001",
                "contact_id": 1,
                "issue_date": "2026-03-01",
                "due_date": "2026-03-31"
            }),
            &db,
            &perms,
        )
        .await;
    assert!(
        result.is_error.is_none(),
        "Expected success, got: {:?}",
        result
    );
    assert!(result.content[0].text.contains("invoice_id"));
    assert!(result.content[0].text.contains("Draft"));
}

#[tokio::test]
async fn draft_invoice_with_notes() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    aequi_storage::insert_contact(
        &db, "Client B", None, None, None, "customer", false, None, None,
    )
    .await
    .unwrap();

    let result = registry
        .call(
            "aequi_draft_invoice",
            json!({
                "invoice_number": "INV-002",
                "contact_id": 1,
                "issue_date": "2026-04-01",
                "due_date": "2026-04-30",
                "notes": "Net 30 terms"
            }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].text.contains("invoice_id"));
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
    assert!(result.content[0].text.contains("Missing required argument"));
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM invoices")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn record_payment_against_invoice() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    aequi_storage::insert_contact(
        &db,
        "Pay Client",
        None,
        None,
        None,
        "customer",
        false,
        None,
        None,
    )
    .await
    .unwrap();

    // Create a draft invoice first
    let draft = registry
        .call(
            "aequi_draft_invoice",
            json!({
                "invoice_number": "INV-PAY-001",
                "contact_id": 1,
                "issue_date": "2026-03-01",
                "due_date": "2026-03-31"
            }),
            &db,
            &perms,
        )
        .await;
    assert!(draft.is_error.is_none());
    let draft_val: serde_json::Value = serde_json::from_str(&draft.content[0].text).unwrap();
    let invoice_id = draft_val["invoice_id"].as_i64().unwrap();

    // Record a payment
    let result = registry
        .call(
            "aequi_record_payment",
            json!({
                "invoice_id": invoice_id,
                "amount_cents": 50000,
                "date": "2026-03-15",
                "method": "bank_transfer"
            }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].text.contains("payment_id"));
}

#[tokio::test]
async fn record_payment_without_method() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    aequi_storage::insert_contact(
        &db, "Client C", None, None, None, "customer", false, None, None,
    )
    .await
    .unwrap();

    let draft = registry
        .call(
            "aequi_draft_invoice",
            json!({
                "invoice_number": "INV-PAY-002",
                "contact_id": 1,
                "issue_date": "2026-03-01",
                "due_date": "2026-03-31"
            }),
            &db,
            &perms,
        )
        .await;
    let draft_val: serde_json::Value = serde_json::from_str(&draft.content[0].text).unwrap();
    let invoice_id = draft_val["invoice_id"].as_i64().unwrap();

    // Payment without method field
    let result = registry
        .call(
            "aequi_record_payment",
            json!({
                "invoice_id": invoice_id,
                "amount_cents": 25000,
                "date": "2026-03-20"
            }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].text.contains("payment_id"));
}

#[tokio::test]
async fn list_unpaid_invoices_with_sent_status() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    aequi_storage::insert_contact(
        &db, "Client D", None, None, None, "customer", false, None, None,
    )
    .await
    .unwrap();

    // Insert an invoice with "Sent" status directly
    aequi_storage::insert_invoice(
        &db,
        "INV-SENT-001",
        1,
        "Sent",
        None,
        "2026-03-01",
        "2026-03-31",
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let result = registry
        .call("aequi_list_unpaid_invoices", json!({}), &db, &perms)
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].text.contains("INV-SENT-001"));
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
    assert!(result.content[0].text.contains("rule_id"));
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
    assert!(result.content[0].text.contains("rule_id"));
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
    let val: serde_json::Value = serde_json::from_str(&result.content[0].text).unwrap();
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
    let val: serde_json::Value = serde_json::from_str(&result.content[0].text).unwrap();
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
    let text = &list.content[0].text;
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
    assert!(result.content[0].text.contains("file_path is required"));
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
    assert!(result.content[0].text.contains("absolute path"));
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
    assert!(result.content[0].text.contains("Path traversal"));
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
    assert!(result.content[0].text.contains("Unsupported file type"));
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
            result.content[0].text.contains("Cannot read file"),
            "Extension .{ext} should pass validation but got: {}",
            result.content[0].text
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
    assert!(result.content[0].text.contains("file_path is required"));
}

#[tokio::test]
async fn approve_receipt_without_transaction() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    // Insert a receipt directly for testing
    aequi_storage::insert_receipt(
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

    let result = registry
        .call(
            "aequi_approve_receipt",
            json!({ "receipt_id": 1 }),
            &db,
            &perms,
        )
        .await;
    assert!(result.is_error.is_none());
    assert!(result.content[0].text.contains("Receipt approved"));
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
    assert!(result.content[0].text.contains("Receipt rejected"));
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
        "aequi_approve_receipt",
        "aequi_reject_receipt",
        "aequi_record_tax_payment",
        "aequi_draft_invoice",
        "aequi_record_payment",
        "aequi_save_categorization_rule",
        "aequi_apply_rules",
        "aequi_save_import_profile",
        "aequi_create_reconciliation_session",
        "aequi_resolve_item",
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
    assert!(read.content[0].text.contains("disabled"));

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
            result.content[0].text.contains("read-only"),
            "{tool}: {}",
            result.content[0].text
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
    assert!(!result.is_error(), "{}", result.content[0].text);
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
    assert!(!result.is_error(), "{}", result.content[0].text);
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
    assert!(result.content[0].text.contains("lines[0].debit_cents"));
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
    assert!(!result.is_error(), "{}", result.content[0].text);
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
    assert!(result.content[0].text.contains("archived"));
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
    assert!(result.content[0].text.contains("boom"));
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
    assert!(result.content[0].text.contains("start_dat"));

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
    assert!(result.content[0].text.contains("Invalid regex"));

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
        let v: serde_json::Value = serde_json::from_str(&r.content[0].text).unwrap();
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
        ("aequi_approve_receipt", json!({ "receipt_id": 99 })),
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
        (
            "aequi_draft_invoice",
            json!({ "invoice_number": "I-9", "contact_id": 99, "issue_date": "2026-03-01", "due_date": "2026-03-31" }),
        ),
        (
            "aequi_create_reconciliation_session",
            json!({ "account_id": 9999, "start_date": "2026-01-01", "end_date": "2026-01-31", "statement_balance_cents": 0 }),
        ),
    ] {
        let result = call(&db, tool, args).await;
        assert!(result.is_error(), "{tool}");
        assert!(
            result.content[0].text.contains("not found"),
            "{tool}: {}",
            result.content[0].text
        );
    }
}

#[tokio::test]
async fn approve_receipt_with_missing_transaction_fails() {
    let db = test_db().await;
    aequi_storage::insert_receipt(
        &db,
        "h1",
        "jpg",
        "/tmp/x.jpg",
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
    let result = call(
        &db,
        "aequi_approve_receipt",
        json!({ "receipt_id": 1, "transaction_id": 42 }),
    )
    .await;
    assert!(result.is_error());
    assert!(result.content[0].text.contains("Transaction 42 not found"));
}

// -----------------------------------------------------------------------
// Regression tests: rule matching
// -----------------------------------------------------------------------

async fn insert_pending_import(db: &aequi_storage::DbPool, batch: &str, description: &str) -> i64 {
    aequi_storage::insert_imported_transaction(
        db,
        &aequi_storage::ImportedTransaction {
            id: 0,
            source_type: "csv".to_string(),
            source_id: None,
            import_batch_id: batch.to_string(),
            date: "2026-03-01".to_string(),
            description: description.to_string(),
            amount_cents: -1000,
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
        assert!(!r.is_error(), "{}", r.content[0].text);
    }

    let r = call(&db, "aequi_apply_rules", json!({ "batch_id": "b1" })).await;
    assert!(!r.is_error(), "{}", r.content[0].text);
    let v: serde_json::Value = serde_json::from_str(&r.content[0].text).unwrap();
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
    let v: serde_json::Value = serde_json::from_str(&r.content[0].text).unwrap();
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
    assert!(!result.is_error(), "{}", result.content[0].text);
    let v: serde_json::Value = serde_json::from_str(&result.content[0].text).unwrap();
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
    assert!(dup.content[0].text.contains("Duplicate receipt"));
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
    assert!(result.content[0].text.contains("regular file"));

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
    assert!(result.content[0].text.contains("too large"));
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
    assert!(result.content[0].text.contains("file-backed"));
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
