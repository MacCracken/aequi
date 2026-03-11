use serde_json::json;

use aequi_core::{FiscalYear, Money, Quarter};

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::ToolRegistry;

fn load_rules(year: u16) -> Result<aequi_core::TaxRules, String> {
    let toml_str = include_str!("../../../../rules/tax/us/2026.toml");
    let rules = aequi_core::TaxRules::from_toml(toml_str).map_err(|e| e.to_string())?;
    if rules.year.value != year {
        return Err(format!("Tax rules for year {year} not available"));
    }
    Ok(rules)
}

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_estimate_quarterly_tax".to_string(),
            description: "Compute estimated quarterly tax payment".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "year": { "type": "integer" },
                    "quarter": { "type": "integer", "minimum": 1, "maximum": 4 }
                }
            }),
        },
        false,
        |db, params| async move {
            let yr = params.get("year").and_then(|v| v.as_u64()).unwrap_or(2026) as u16;
            let q = params
                .get("quarter")
                .and_then(|v| v.as_u64())
                .and_then(|v| Quarter::new(v as u8))
                .unwrap_or(Quarter::Q1);

            let rules = match load_rules(yr) {
                Ok(r) => r,
                Err(e) => return ToolResult::error(e),
            };

            let fy = FiscalYear::new(yr);
            let prior = aequi_storage::get_prior_year_total_tax(&db, yr)
                .await
                .ok()
                .flatten()
                .map(Money::from_cents);
            let snapshot = match aequi_storage::build_ledger_snapshot(&db, fy, prior).await {
                Ok(s) => s,
                Err(e) => return ToolResult::error(e.to_string()),
            };

            let est = aequi_core::compute_quarterly_estimate(&rules, &snapshot, q);
            ToolResult::text(serde_json::to_string_pretty(&est).unwrap())
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_get_schedule_c_preview".to_string(),
            description: "Get Schedule C preview with deduction-adjusted totals".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "year": { "type": "integer" } }
            }),
        },
        false,
        |db, params| async move {
            let yr = params.get("year").and_then(|v| v.as_u64()).unwrap_or(2026) as u16;
            let rules = match load_rules(yr) {
                Ok(r) => r,
                Err(e) => return ToolResult::error(e),
            };
            let fy = FiscalYear::new(yr);
            let snapshot = match aequi_storage::build_ledger_snapshot(&db, fy, None).await {
                Ok(s) => s,
                Err(e) => return ToolResult::error(e.to_string()),
            };
            let preview = aequi_core::tax::engine::schedule_c_preview(&rules, &snapshot);
            ToolResult::text(serde_json::to_string_pretty(&preview).unwrap())
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_record_tax_payment".to_string(),
            description: "Record a quarterly tax payment".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "year": { "type": "integer" },
                    "quarter": { "type": "integer" },
                    "amount_cents": { "type": "integer" },
                    "date": { "type": "string" }
                },
                "required": ["year", "quarter", "amount_cents", "date"]
            }),
        },
        true,
        |db, params| async move {
            let year = params.get("year").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
            let quarter = params.get("quarter").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
            let amount = params
                .get("amount_cents")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let date = params.get("date").and_then(|v| v.as_str()).unwrap_or("");

            match aequi_storage::record_tax_payment(&db, year, quarter, amount, date).await {
                Ok(()) => ToolResult::text("Payment recorded".to_string()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );
}
