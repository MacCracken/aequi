use serde_json::json;

use aequi_core::{Money, TransactionLine, UnvalidatedTransaction};

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::args::{Args, ToolError, check_date_order};
use crate::tools::{Access, ToolRegistry};

const MAX_LIMIT: i64 = 1000;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_get_transactions".to_string(),
            description: "List recent transactions, newest first, optionally filtered by date range (inclusive)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "start_date": { "type": "string", "description": "YYYY-MM-DD (inclusive)" },
                    "end_date": { "type": "string", "description": "YYYY-MM-DD (inclusive)" },
                    "limit": { "type": "integer", "default": 50, "minimum": 1, "maximum": MAX_LIMIT }
                }
            }),
        },
        Access::Read,
        |db, params| async move {
            let args = Args::new(&params);
            let start = args.opt_date("start_date")?;
            let end = args.opt_date("end_date")?;
            if let (Some(s), Some(e)) = (start, end) {
                check_date_order(s, e)?;
            }
            let limit = args.opt_i64("limit")?.unwrap_or(50);
            if !(1..=MAX_LIMIT).contains(&limit) {
                return Err(ToolError(format!(
                    "Argument limit must be between 1 and {MAX_LIMIT}, got {limit}"
                )));
            }

            // Unset bounds bind NULL and drop out of the filter.
            let rows = sqlx::query_as::<_, (i64, String, String, Option<String>, i64, Option<i64>)>(
                "SELECT id, date, description, memo, balanced_total_cents, contact_id FROM transactions \
                 WHERE (?1 IS NULL OR date >= ?1) AND (?2 IS NULL OR date <= ?2) \
                 ORDER BY date DESC, id DESC LIMIT ?3",
            )
            .bind(start)
            .bind(end)
            .bind(limit)
            .fetch_all(&db)
            .await?;

            let txs: Vec<_> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.0, "date": r.1, "description": r.2,
                        "memo": r.3, "balanced_total_cents": r.4, "contact_id": r.5
                    })
                })
                .collect();
            Ok(ToolResult::json(&txs))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_create_transaction".to_string(),
            description: "Create a balanced double-entry transaction. Each line has either a debit or a credit (whole cents, positive); total debits must equal total credits. Set contact_id to the payee or payer (see aequi_list_contacts); payments to contractors from a bank account count toward their 1099-NEC".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "date": { "type": "string", "description": "YYYY-MM-DD" },
                    "description": { "type": "string" },
                    "memo": { "type": "string" },
                    "contact_id": { "type": "integer", "description": "Payee or payer contact id" },
                    "lines": {
                        "type": "array",
                        "minItems": 2,
                        "items": {
                            "type": "object",
                            "properties": {
                                "account_code": { "type": "string" },
                                "debit_cents": { "type": "integer", "minimum": 0 },
                                "credit_cents": { "type": "integer", "minimum": 0 },
                                "memo": { "type": "string" }
                            },
                            "required": ["account_code", "debit_cents", "credit_cents"]
                        }
                    }
                },
                "required": ["date", "description", "lines"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let args = Args::new(&params);
            let date_str = args.date("date")?;
            let date = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                .map_err(|e| ToolError(format!("Invalid date: {e}")))?;
            let description = args.str("description")?.to_string();
            let memo = args.opt_str("memo")?.map(str::to_string);
            let contact_id = args.opt_id("contact_id")?;

            let mut lines = Vec::new();
            for (i, line) in args.array("lines")?.iter().enumerate() {
                let line = Args::object(line, format!("lines[{i}]"))?;
                line.only(&["account_code", "debit_cents", "credit_cents", "memo"])?;
                let code = line.str("account_code")?;
                let account = aequi_storage::get_account_by_code(&db, code)
                    .await?
                    .ok_or_else(|| ToolError(format!("Account {code} not found")))?;
                if account.is_archived {
                    return Err(ToolError(format!("Account {code} is archived")));
                }
                let account_id = account
                    .id
                    .ok_or_else(|| ToolError(format!("Account {code} has no id")))?;
                lines.push(TransactionLine {
                    account_id,
                    debit: Money::from_cents(line.cents("debit_cents")?),
                    credit: Money::from_cents(line.cents("credit_cents")?),
                    memo: line.opt_str("memo")?.map(str::to_string),
                });
            }

            let tx = UnvalidatedTransaction {
                date,
                description,
                lines,
                memo,
            };
            // Validation (balance, line rules, accounts, contact) and the
            // atomic insert live in aequi-storage.
            let id = aequi_storage::create_ledger_transaction(&db, tx, contact_id)
                .await
                .map_err(|e| match e {
                    sqlx::Error::InvalidArgument(msg) => {
                        ToolError(format!("Invalid transaction: {msg}"))
                    }
                    other => ToolError::from(other),
                })?;
            Ok(ToolResult::text(
                json!({ "id": id, "status": "created" }).to_string(),
            ))
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
        Access::Read,
        |db, params| async move {
            let args = Args::new(&params);
            let today = chrono::Local::now().date_naive();
            let default_start = format!("{}-01-01", today.format("%Y"));
            let today_str = today.to_string();
            let start = args.opt_date("start_date")?.unwrap_or(&default_start);
            let end = args.opt_date("end_date")?.unwrap_or(&today_str);
            check_date_order(start, end)?;

            let income = sqlx::query_as::<_, (String, String, i64)>(
                r#"SELECT a.code, a.name, COALESCE(SUM(tl.credit_cents - tl.debit_cents), 0) AS net
                   FROM accounts a
                   JOIN transaction_lines tl ON tl.account_id = a.id
                   JOIN transactions t ON t.id = tl.transaction_id
                   WHERE a.account_type = 'Income' AND t.date >= ? AND t.date <= ?
                   GROUP BY a.id ORDER BY a.code"#,
            )
            .bind(start)
            .bind(end)
            .fetch_all(&db)
            .await?;

            let expenses = sqlx::query_as::<_, (String, String, i64)>(
                r#"SELECT a.code, a.name, COALESCE(SUM(tl.debit_cents - tl.credit_cents), 0) AS net
                   FROM accounts a
                   JOIN transaction_lines tl ON tl.account_id = a.id
                   JOIN transactions t ON t.id = tl.transaction_id
                   WHERE a.account_type = 'Expense' AND t.date >= ? AND t.date <= ?
                   GROUP BY a.id ORDER BY a.code"#,
            )
            .bind(start)
            .bind(end)
            .fetch_all(&db)
            .await?;

            let total_income: i64 = income.iter().map(|r| r.2).sum();
            let total_expenses: i64 = expenses.iter().map(|r| r.2).sum();
            let net_profit = total_income - total_expenses;

            let income_lines: Vec<_> = income
                .iter()
                .map(|r| json!({ "code": r.0, "name": r.1, "amount_cents": r.2 }))
                .collect();
            let expense_lines: Vec<_> = expenses
                .iter()
                .map(|r| json!({ "code": r.0, "name": r.1, "amount_cents": r.2 }))
                .collect();

            Ok(ToolResult::json(&json!({
                "period": { "start": start, "end": end },
                "income": income_lines,
                "expenses": expense_lines,
                "total_income_cents": total_income,
                "total_expenses_cents": total_expenses,
                "net_profit_cents": net_profit
            })))
        },
    );
}
