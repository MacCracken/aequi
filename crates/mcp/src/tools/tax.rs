use chrono::Datelike;
use serde_json::json;

use aequi_core::{FiscalYear, Money, Quarter};

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::args::{Args, ToolError};
use crate::tools::{Access, ToolRegistry};

fn load_rules(year: u16) -> Result<aequi_core::TaxRules, ToolError> {
    let toml_str = include_str!("../../../../rules/tax/us/2026.toml");
    let rules = aequi_core::TaxRules::from_toml(toml_str)
        .map_err(|e| ToolError(format!("Invalid tax rules: {e}")))?;
    if rules.year.value != year {
        return Err(ToolError(format!(
            "Tax rules for year {year} not available"
        )));
    }
    Ok(rules)
}

fn quarter_number(q: Quarter) -> u8 {
    match q {
        Quarter::Q1 => 1,
        Quarter::Q2 => 2,
        Quarter::Q3 => 3,
        Quarter::Q4 => 4,
    }
}

fn current_year() -> u16 {
    chrono::Local::now().date_naive().year() as u16
}

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_estimate_quarterly_tax".to_string(),
            description: "Compute the estimated tax payment for a quarter. Year-to-date profit through the end of the installment period (or today, if earlier) is annualized; totals are full-year projections and quarterly_payment is a quarter of the projected tax".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "year": { "type": "integer", "description": "Tax year (default: current year)" },
                    "quarter": { "type": "integer", "minimum": 1, "maximum": 4 }
                },
                "required": ["quarter"]
            }),
        },
        Access::Read,
        |db, params| async move {
            let args = Args::new(&params);
            let yr = args.opt_year("year")?.unwrap_or_else(current_year);
            let q = args.quarter("quarter")?;
            let rules = load_rules(yr)?;

            let fy = FiscalYear::new(yr);
            let prior = aequi_storage::get_prior_year_total_tax(&db, yr)
                .await?
                .map(Money::from_cents);
            let through = aequi_core::estimate_through(fy, q, chrono::Local::now().date_naive());
            let snapshot = aequi_storage::build_ledger_snapshot(&db, fy, through, prior).await?;

            let est = aequi_core::compute_quarterly_estimate(&rules, &snapshot, q);
            let mut out = serde_json::to_value(&est)
                .map_err(|e| ToolError(format!("Failed to serialize estimate: {e}")))?;
            if let Some(obj) = out.as_object_mut() {
                obj.insert("through".to_string(), json!(through));
            }
            Ok(ToolResult::json(&out))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_get_schedule_c_preview".to_string(),
            description: "Get Schedule C preview with deduction-adjusted totals for a full year"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "year": { "type": "integer", "description": "Tax year (default: current year)" }
                }
            }),
        },
        Access::Read,
        |db, params| async move {
            let yr = Args::new(&params)
                .opt_year("year")?
                .unwrap_or_else(current_year);
            let rules = load_rules(yr)?;
            let fy = FiscalYear::new(yr);
            let snapshot =
                aequi_storage::build_ledger_snapshot(&db, fy, fy.end_date(), None).await?;
            let preview = aequi_core::tax::engine::schedule_c_preview(&rules, &snapshot);
            Ok(ToolResult::json(&preview))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_record_tax_payment".to_string(),
            description: "Record an estimated tax payment for a quarter. Payments for the same quarter accumulate".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "year": { "type": "integer" },
                    "quarter": { "type": "integer", "minimum": 1, "maximum": 4 },
                    "amount_cents": { "type": "integer", "minimum": 1 },
                    "date": { "type": "string", "description": "Payment date YYYY-MM-DD" }
                },
                "required": ["year", "quarter", "amount_cents", "date"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let args = Args::new(&params);
            let year = args.year("year")?;
            let quarter = args.quarter("quarter")?;
            let amount = args.cents("amount_cents")?;
            if amount <= 0 {
                return Err(ToolError(format!(
                    "amount_cents must be positive, got {amount}"
                )));
            }
            let date = args.date("date")?;
            let q = quarter_number(quarter);

            aequi_storage::record_tax_payment(&db, year, q, amount, date).await?;
            Ok(ToolResult::text(
                json!({
                    "status": "recorded",
                    "year": year,
                    "quarter": q,
                    "amount_cents": amount,
                    "date": date
                })
                .to_string(),
            ))
        },
    );
}
