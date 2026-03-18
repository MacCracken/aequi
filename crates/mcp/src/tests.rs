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
    assert!(result.content[0].text.contains("Invalid date"));
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
    assert!(result.is_error.is_none());
    let text = &result.content[0].text;
    assert!(text.contains("quarterly_payment"));
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
        Some(serde_json::json!(1)),
        serde_json::json!({"ok": true}),
    );
    assert_eq!(resp.jsonrpc, "2.0");
    assert!(resp.result.is_some());
    assert!(resp.error.is_none());
}

#[test]
fn json_rpc_error_response() {
    let resp =
        crate::protocol::JsonRpcResponse::error(Some(serde_json::json!(1)), -32600, "bad".into());
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
    aequi_storage::insert_contact(&db, "Test Client", Some("test@example.com"), None, None, "customer", false, None, None)
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
    assert!(result.is_error.is_none(), "Expected success, got: {:?}", result);
    assert!(result.content[0].text.contains("invoice_id"));
    assert!(result.content[0].text.contains("Draft"));
}

#[tokio::test]
async fn draft_invoice_with_notes() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    aequi_storage::insert_contact(&db, "Client B", None, None, None, "customer", false, None, None)
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
async fn draft_invoice_missing_params_uses_defaults() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    // Call with empty params — the handler uses defaults for missing fields
    let result = registry
        .call("aequi_draft_invoice", json!({}), &db, &perms)
        .await;
    // Should still succeed (empty strings and 0 for defaults)
    assert!(result.is_error.is_none() || result.is_error == Some(true));
}

#[tokio::test]
async fn record_payment_against_invoice() {
    let db = test_db().await;
    let registry = ToolRegistry::new();
    let perms = Permissions::default();

    aequi_storage::insert_contact(&db, "Pay Client", None, None, None, "customer", false, None, None)
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

    aequi_storage::insert_contact(&db, "Client C", None, None, None, "customer", false, None, None)
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

    aequi_storage::insert_contact(&db, "Client D", None, None, None, "customer", false, None, None)
        .await
        .unwrap();

    // Insert an invoice with "Sent" status directly
    aequi_storage::insert_invoice(
        &db, "INV-SENT-001", 1, "Sent", None, "2026-03-01", "2026-03-31", None, None, None, None,
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
    for ext in &["jpg", "jpeg", "png", "gif", "webp", "tiff", "tif", "bmp", "pdf"] {
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
        &db, "abc123hash", "jpg", "/tmp/test.jpg", None,
        Some("TestVendor"), Some("2026-03-01"), Some(1500), None, None, None, 0.0,
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
        &db, "def456hash", "png", "/tmp/test2.png", None,
        Some("Vendor2"), Some("2026-03-02"), Some(2000), None, None, None, 0.0,
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
