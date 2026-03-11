#[cfg(test)]
mod tests {
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
        let resp = crate::protocol::JsonRpcResponse::error(
            Some(serde_json::json!(1)),
            -32600,
            "bad".into(),
        );
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
}
