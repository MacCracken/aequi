use serde_json::json;

use aequi_core::{Money, TransactionLine, UnvalidatedTransaction, ValidatedTransaction};

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_get_transactions".to_string(),
            description: "List recent transactions, optionally filtered by date range".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "start_date": { "type": "string", "description": "YYYY-MM-DD" },
                    "end_date": { "type": "string", "description": "YYYY-MM-DD" },
                    "limit": { "type": "integer", "default": 50 }
                }
            }),
        },
        false,
        |db, params| async move {
            let limit = params.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
            let rows = sqlx::query_as::<_, (i64, String, String, Option<String>, i64)>(
                "SELECT id, date, description, memo, balanced_total_cents FROM transactions ORDER BY date DESC LIMIT ?"
            )
            .bind(limit)
            .fetch_all(&db)
            .await;

            match rows {
                Ok(rows) => {
                    let txs: Vec<_> = rows.iter().map(|r| json!({
                        "id": r.0, "date": r.1, "description": r.2,
                        "memo": r.3, "balanced_total_cents": r.4
                    })).collect();
                    ToolResult::text(serde_json::to_string_pretty(&txs).unwrap())
                }
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_create_transaction".to_string(),
            description: "Create a double-entry transaction".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "date": { "type": "string", "description": "YYYY-MM-DD" },
                    "description": { "type": "string" },
                    "lines": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "account_code": { "type": "string" },
                                "debit_cents": { "type": "integer" },
                                "credit_cents": { "type": "integer" }
                            },
                            "required": ["account_code", "debit_cents", "credit_cents"]
                        }
                    }
                },
                "required": ["date", "description", "lines"]
            }),
        },
        true,
        |db, params| async move {
            let date_str = params.get("date").and_then(|v| v.as_str()).unwrap_or("");
            let date = match chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                Ok(d) => d,
                Err(e) => return ToolResult::error(format!("Invalid date: {e}")),
            };

            let description = params.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let lines_val = match params.get("lines").and_then(|v| v.as_array()) {
                Some(l) => l,
                None => return ToolResult::error("lines is required".to_string()),
            };

            let mut lines = Vec::new();
            for line in lines_val {
                let code = line.get("account_code").and_then(|v| v.as_str()).unwrap_or("");
                let account = match aequi_storage::get_account_by_code(&db, code).await {
                    Ok(Some(a)) => a,
                    Ok(None) => return ToolResult::error(format!("Account {code} not found")),
                    Err(e) => return ToolResult::error(e.to_string()),
                };
                let debit = line.get("debit_cents").and_then(|v| v.as_i64()).unwrap_or(0);
                let credit = line.get("credit_cents").and_then(|v| v.as_i64()).unwrap_or(0);
                lines.push(TransactionLine {
                    account_id: account.id.unwrap(),
                    debit: Money::from_cents(debit),
                    credit: Money::from_cents(credit),
                    memo: None,
                });
            }

            let tx = UnvalidatedTransaction { date, description, lines, memo: None };
            let validated = match ValidatedTransaction::validate(tx) {
                Ok(v) => v,
                Err(e) => return ToolResult::error(e.to_string()),
            };

            let row = sqlx::query_as::<_, (i64,)>(
                "INSERT INTO transactions (date, description, memo, balanced_total_cents) VALUES (?, ?, ?, ?) RETURNING id"
            )
            .bind(validated.date.to_string())
            .bind(&validated.description)
            .bind(&validated.memo)
            .bind(validated.balanced_total.to_cents())
            .fetch_one(&db)
            .await;

            let id = match row {
                Ok(r) => r.0,
                Err(e) => return ToolResult::error(e.to_string()),
            };

            for line in &validated.lines {
                let _ = sqlx::query(
                    "INSERT INTO transaction_lines (transaction_id, account_id, debit_cents, credit_cents, memo) VALUES (?, ?, ?, ?, ?)"
                )
                .bind(id)
                .bind(line.account_id.0)
                .bind(line.debit.to_cents())
                .bind(line.credit.to_cents())
                .bind(&line.memo)
                .execute(&db)
                .await;
            }

            ToolResult::text(json!({ "id": id, "status": "created" }).to_string())
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_get_profit_loss".to_string(),
            description: "Get profit & loss summary for a date range".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "start_date": { "type": "string", "description": "YYYY-MM-DD (default: Jan 1 current year)" },
                    "end_date": { "type": "string", "description": "YYYY-MM-DD (default: today)" }
                }
            }),
        },
        false,
        |db, params| async move {
            let today = chrono::Utc::now().date_naive();
            let default_start = format!("{}-01-01", today.format("%Y"));
            let start = params.get("start_date").and_then(|v| v.as_str()).unwrap_or(&default_start);
            let today_str = today.to_string();
            let end = params.get("end_date").and_then(|v| v.as_str()).unwrap_or(&today_str);

            let income_rows = sqlx::query_as::<_, (String, String, i64)>(
                r#"SELECT a.code, a.name, COALESCE(SUM(tl.credit_cents - tl.debit_cents), 0) AS net
                   FROM accounts a
                   JOIN transaction_lines tl ON tl.account_id = a.id
                   JOIN transactions t ON t.id = tl.transaction_id
                   WHERE a.account_type = 'Income' AND t.date >= ? AND t.date <= ?
                   GROUP BY a.id ORDER BY a.code"#
            )
            .bind(start).bind(end)
            .fetch_all(&db).await;

            let expense_rows = sqlx::query_as::<_, (String, String, i64)>(
                r#"SELECT a.code, a.name, COALESCE(SUM(tl.debit_cents - tl.credit_cents), 0) AS net
                   FROM accounts a
                   JOIN transaction_lines tl ON tl.account_id = a.id
                   JOIN transactions t ON t.id = tl.transaction_id
                   WHERE a.account_type = 'Expense' AND t.date >= ? AND t.date <= ?
                   GROUP BY a.id ORDER BY a.code"#
            )
            .bind(start).bind(end)
            .fetch_all(&db).await;

            let (income, expenses) = match (income_rows, expense_rows) {
                (Ok(i), Ok(e)) => (i, e),
                (Err(e), _) | (_, Err(e)) => return ToolResult::error(e.to_string()),
            };

            let total_income: i64 = income.iter().map(|r| r.2).sum();
            let total_expenses: i64 = expenses.iter().map(|r| r.2).sum();
            let net_profit = total_income - total_expenses;

            let income_lines: Vec<_> = income.iter().map(|r| json!({
                "code": r.0, "name": r.1, "amount_cents": r.2
            })).collect();
            let expense_lines: Vec<_> = expenses.iter().map(|r| json!({
                "code": r.0, "name": r.1, "amount_cents": r.2
            })).collect();

            ToolResult::text(serde_json::to_string_pretty(&json!({
                "period": { "start": start, "end": end },
                "income": income_lines,
                "expenses": expense_lines,
                "total_income_cents": total_income,
                "total_expenses_cents": total_expenses,
                "net_profit_cents": net_profit
            })).unwrap())
        },
    );
}
