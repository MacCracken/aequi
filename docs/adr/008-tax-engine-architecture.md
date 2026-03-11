# ADR-008: Tax Engine as Pure Function with TOML Rule Files

## Status
Accepted

## Context

Phase 4 adds a tax engine for US self-employed individuals. Key design decisions:

1. **How to encode tax rules** — rates, brackets, wage bases, and deduction caps change annually
2. **Where to put the computation** — in `core` (pure logic) vs `storage` (DB queries) vs `app` (command layer)
3. **How to handle the Schedule C line mapping** — connecting accounts to IRS form lines

The engine must be testable without a database, reproducible across runs for the same inputs, and easy for community contributors to update each January when new rates are published.

## Decision

### Pure function architecture

The tax engine is a pure function: `TaxRules x LedgerSnapshot → QuarterlyEstimate`. No database writes during calculation. The storage layer builds the `LedgerSnapshot` from a SQL aggregation, and the app layer persists results separately into a `tax_periods` table.

```
core/src/tax/
  mod.rs          — public exports
  rules.rs        — TaxRules struct, TOML deserialization, bracket computation
  engine.rs       — compute_quarterly_estimate(), schedule_c_preview()
  schedule_c.rs   — ScheduleCLine enum, from_tag() parser
```

### TOML rule files

Tax rules are stored in `rules/tax/us/YYYY.toml` and deserialized into `TaxRules`. The TOML schema includes:

- SE tax rates (SS, Medicare, wage base, net earnings factor, deductible fraction)
- Progressive income tax brackets (single filer)
- Standard mileage rates
- Meals deduction cap (50%)
- Simplified home office deduction rate
- Quarterly due dates

Rule files are embedded at compile time via `include_str!()` for the initial release. Future versions will load from the app's data directory to support user-installed rule updates.

### Schedule C line mapping

Each `Account` already has a `schedule_c_line: Option<String>` field (e.g., `"line_1"`, `"line_24b"`). The `ScheduleCLine` enum provides type-safe parsing via `from_tag()`. The storage layer aggregates `transaction_lines` by account's `schedule_c_line`, grouping income (net credits) and expenses (net debits) to build the `LedgerSnapshot`.

### Meals deduction cap

Business meals (Line 24b) are automatically capped at 50% during both the quarterly estimate and Schedule C preview. The cap fraction is configurable in the TOML rules file.

### SE tax wage base cap

Social Security tax is capped at the wage base ($176,100 for 2026). Medicare has no cap. The engine computes these separately and combines them.

### Safe harbor calculation

Safe harbor amount = min(100% prior year tax, 90% current year estimate). Prior year tax is pulled from the `tax_periods` table. If no prior year data exists, 90% of the current estimate is used.

## Consequences

- **Pros:**
  - Tax computation is independently testable with no database dependency
  - Community can update tax rates via a simple TOML file PR
  - The same engine works for both quarterly estimates and Schedule C preview
  - All IRS rounding rules (half-up to nearest dollar) are explicit in the Money type

- **Cons:**
  - Only single filer brackets in v1 (married filing jointly, head of household deferred to v2)
  - Rule files are currently embedded at compile time — user-installed updates require a rebuild
  - No state tax support in v1

## References
- IRS Publication 505 (Tax Withholding and Estimated Tax)
- IRS Schedule C (Form 1040)
- IRS Self-Employment Tax (Schedule SE)
