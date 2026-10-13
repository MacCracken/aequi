use std::collections::HashMap;

use chrono::Datelike;
use serde_json::json;

use aequi_core::{FiscalYear, Money, Quarter};
use aequi_storage::TaxReturnFigures;

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

/// How the prior-year safe harbor was worked out, and what would make it
/// exact. `prev` is the prior tax year; `has_prior_tax` is whether any
/// prior-year tax (filed or projected) was available.
fn safe_harbor_note(prev: u16, filed: TaxReturnFigures, has_prior_tax: bool) -> String {
    let mut note = format!(
        "The prior-year safe harbor is 100% of {prev}'s total tax, or 110% if {prev}'s AGI exceeded $150,000; safe_harbor_amount is the smaller of that and 90% of this year's projected tax."
    );
    let mut gaps = Vec::new();
    if filed.total_tax_cents.is_none() {
        gaps.push(if has_prior_tax {
            format!("{prev}'s tax is Aequi's own projection")
        } else {
            format!("no {prev} tax is on file, so only the 90% test applies")
        });
    }
    if filed.agi_cents.is_none() {
        gaps.push(format!("{prev}'s AGI is not entered, so 100% is assumed"));
    }
    if gaps.is_empty() {
        note.push_str(&format!(
            " It uses the total tax and AGI from your filed {prev} return."
        ));
    } else {
        note.push_str(&format!(
            " Not exact yet: {}. Enter the total tax and AGI from your filed {prev} return with aequi_set_tax_return to make it exact.",
            gaps.join("; ")
        ));
    }
    note
}

fn tax_return_json(year: u16, figures: TaxReturnFigures) -> serde_json::Value {
    json!({
        "year": year,
        "total_tax_cents": figures.total_tax_cents,
        "agi_cents": figures.agi_cents,
    })
}

/// A filed return's year: it must be a valid tax year that has already ended.
fn return_year(args: &Args) -> Result<u16, ToolError> {
    let year = args.year("year")?;
    let current = current_year();
    if year >= current {
        return Err(ToolError(format!(
            "year is the tax year of a filed return and must be before {current}, got {year} (the return filed in {current} is for {})",
            current - 1
        )));
    }
    Ok(year)
}

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_estimate_quarterly_tax".to_string(),
            description: "Compute the estimated tax payment for a quarter. Year-to-date profit through the end of the installment period (or today, if earlier) is annualized; totals are full-year projections and quarterly_payment is a quarter of the projected tax. safe_harbor_amount uses last year's tax at 100%, or 110% when last year's AGI exceeded $150,000; safe_harbor_note says whether it is based on last year's filed return (see aequi_set_tax_return)".to_string(),
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
            let prior_return = aequi_storage::get_tax_return_figures(&db, yr - 1).await?;
            let prior = aequi_storage::get_prior_year_total_tax(&db, yr)
                .await?
                .map(Money::from_cents);
            let through = aequi_core::estimate_through(fy, q, chrono::Local::now().date_naive());
            let snapshot = aequi_storage::build_ledger_snapshot(&db, fy, through, prior).await?;

            let est = aequi_core::compute_quarterly_estimate(&rules, &snapshot, q);
            let qbi_deduction_cents = est
                .qbi_deduction
                .checked_to_cents()
                .ok_or_else(|| ToolError::new("QBI deduction is out of range"))?;
            let mut out = serde_json::to_value(&est)
                .map_err(|e| ToolError(format!("Failed to serialize estimate: {e}")))?;
            if let Some(obj) = out.as_object_mut() {
                obj.insert("through".to_string(), json!(through));
                obj.insert("qbi_deduction_cents".to_string(), json!(qbi_deduction_cents));
                obj.insert(
                    "safe_harbor_prior_year_percentage".to_string(),
                    json!(format!("{:.2}", est.safe_harbor_prior_year_percentage)),
                );
                obj.insert(
                    "safe_harbor_note".to_string(),
                    json!(safe_harbor_note(yr - 1, prior_return, prior.is_some())),
                );
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

    registry.register(
        ToolDefinition {
            name: "aequi_get_tax_return".to_string(),
            description: "Get the figures entered from a filed tax return: total_tax_cents (the total tax on the return) and agi_cents (adjusted gross income), in whole cents; null means not entered. Last year's return makes the prior-year safe harbor in aequi_estimate_quarterly_tax exact: 100% of its total tax, or 110% if its AGI exceeded $150,000".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "year": { "type": "integer", "description": "Tax year of the return (a past year), e.g. 2025 for the return filed in 2026" }
                },
                "required": ["year"]
            }),
        },
        Access::Read,
        |db, params| async move {
            let year = return_year(&Args::new(&params))?;
            let figures = aequi_storage::get_tax_return_figures(&db, year).await?;
            Ok(ToolResult::json(&tax_return_json(year, figures)))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_set_tax_return".to_string(),
            description: "Save figures from a filed tax return so the next year's prior-year safe harbor is exact. Copy them from the return; don't estimate. An omitted field is left unchanged and an explicit null clears the stored value; pass at least one of total_tax_cents and agi_cents. Returns the stored figures".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "year": { "type": "integer", "description": "Tax year of the return (a past year), e.g. 2025 for the return filed in 2026" },
                    "total_tax_cents": { "type": ["integer", "null"], "minimum": 0, "description": "Total tax shown on the return, in cents. Omit to keep, null to clear" },
                    "agi_cents": { "type": ["integer", "null"], "description": "Adjusted gross income on the return, in cents (may be negative). Omit to keep, null to clear" }
                },
                "required": ["year"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let args = Args::new(&params);
            let year = return_year(&args)?;
            let total_tax = args.nullable_cents("total_tax_cents")?;
            let agi = args.nullable_cents("agi_cents")?;
            if total_tax.is_none() && agi.is_none() {
                return Err(ToolError::new(
                    "Nothing to change: pass total_tax_cents and/or agi_cents (null clears a stored value)",
                ));
            }
            if let Some(Some(c)) = total_tax
                && c < 0
            {
                return Err(ToolError(format!(
                    "total_tax_cents must not be negative, got {c}"
                )));
            }

            let current = aequi_storage::get_tax_return_figures(&db, year).await?;
            let figures = TaxReturnFigures {
                total_tax_cents: total_tax.unwrap_or(current.total_tax_cents),
                agi_cents: agi.unwrap_or(current.agi_cents),
            };
            aequi_storage::set_tax_return_figures(&db, year, figures).await?;
            let stored = aequi_storage::get_tax_return_figures(&db, year).await?;
            Ok(ToolResult::json(&tax_return_json(year, stored)))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_get_1099_summary".to_string(),
            description: "1099-NEC summary for a year: how much you paid each contact flagged as a contractor, counting payments from bank/cash accounts on transactions whose contact is that contractor (card and payment-app payments are excluded; those go on the processor's 1099-K). over_threshold means a 1099-NEC is likely due; has_tax_id shows whether a TIN is on file".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "year": { "type": "integer", "description": "Calendar year (default: current year)" }
                }
            }),
        },
        Access::Read,
        |db, params| async move {
            let year = Args::new(&params)
                .opt_year("year")?
                .unwrap_or_else(current_year);
            let threshold = aequi_core::nec_threshold(year);
            let has_tax_id: HashMap<i64, bool> = aequi_storage::get_contractors(&db)
                .await?
                .into_iter()
                .map(|c| (c.id, c.tax_id.is_some_and(|t| !t.trim().is_empty())))
                .collect();

            let payments = aequi_storage::get_contractor_ytd_payments(&db, year).await?;
            let contractors: Vec<_> = payments
                .iter()
                .map(|(id, name, cents)| {
                    json!({
                        "contact_id": id,
                        "name": name,
                        "paid_cents": cents,
                        "over_threshold":
                            aequi_core::check_1099_threshold(Money::from_cents(*cents), year),
                        "has_tax_id": has_tax_id.get(id).copied().unwrap_or(false)
                    })
                })
                .collect();
            let over = contractors
                .iter()
                .filter(|c| c["over_threshold"] == true)
                .count();

            Ok(ToolResult::json(&json!({
                "year": year,
                "threshold_cents": threshold.to_cents(),
                "contractors_over_threshold": over,
                "contractors": contractors
            })))
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_harbor_note_says_what_is_missing() {
        let filed = |total_tax_cents, agi_cents| TaxReturnFigures {
            total_tax_cents,
            agi_cents,
        };

        let note = safe_harbor_note(2025, filed(Some(1), Some(1)), true);
        assert!(
            note.contains("110% if 2025's AGI exceeded $150,000"),
            "{note}"
        );
        assert!(note.contains("from your filed 2025 return"), "{note}");
        assert!(!note.contains("Not exact"), "{note}");

        let note = safe_harbor_note(2025, filed(None, Some(1)), true);
        assert!(
            note.contains("2025's tax is Aequi's own projection"),
            "{note}"
        );
        assert!(!note.contains("AGI is not entered"), "{note}");
        assert!(note.contains("aequi_set_tax_return"), "{note}");

        let note = safe_harbor_note(2025, filed(Some(1), None), true);
        assert!(
            note.contains("AGI is not entered, so 100% is assumed"),
            "{note}"
        );
        assert!(!note.contains("projection"), "{note}");

        let note = safe_harbor_note(2025, filed(None, None), false);
        assert!(note.contains("no 2025 tax is on file"), "{note}");
        assert!(note.contains("AGI is not entered"), "{note}");
    }
}
