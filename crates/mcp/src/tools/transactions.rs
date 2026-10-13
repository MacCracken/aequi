use serde_json::json;

use aequi_core::{Money, TransactionLine, UnvalidatedTransaction, ValidatedTransaction};

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
            let rows = sqlx::query_as::<_, (i64, String, String, Option<String>, i64)>(
                "SELECT id, date, description, memo, balanced_total_cents FROM transactions \
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
                        "memo": r.3, "balanced_total_cents": r.4
                    })
                })
                .collect();
            Ok(ToolResult::json(&txs))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_create_transaction".to_string(),
            description: "Create a balanced double-entry transaction. Each line has either a debit or a credit (whole cents, positive); total debits must equal total credits".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "date": { "type": "string", "description": "YYYY-MM-DD" },
                    "description": { "type": "string" },
                    "memo": { "type": "string" },
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

            let mut lines = Vec::new();
            for (i, line) in args.array("lines")?.iter().enumerate() {
                let line = Args::object(line, format!("lines[{i}]"))?;
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
            let validated = ValidatedTransaction::validate(tx)
                .map_err(|e| ToolError(format!("Invalid transaction: {e}")))?;

            let id = insert_transaction(&db, &validated).await?;
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

/// Insert the header and all lines in one database transaction, so a failed
/// line insert can't leave an unbalanced header behind.
async fn insert_transaction(
    db: &aequi_storage::DbPool,
    tx: &ValidatedTransaction,
) -> Result<i64, sqlx::Error> {
    let mut db_tx = db.begin().await?;

    let (id,) = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO transactions (date, description, memo, balanced_total_cents) VALUES (?, ?, ?, ?) RETURNING id",
    )
    .bind(tx.date.to_string())
    .bind(&tx.description)
    .bind(&tx.memo)
    .bind(tx.balanced_total.to_cents())
    .fetch_one(&mut *db_tx)
    .await?;

    for line in &tx.lines {
        sqlx::query(
            "INSERT INTO transaction_lines (transaction_id, account_id, debit_cents, credit_cents, memo) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(line.account_id.0)
        .bind(line.debit.to_cents())
        .bind(line.credit.to_cents())
        .bind(&line.memo)
        .execute(&mut *db_tx)
        .await?;
    }

    // Dropping `db_tx` on any early return above rolls everything back.
    db_tx.commit().await?;
    Ok(id)
}
