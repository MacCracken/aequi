# ADR-012: Data Export — Beancount and QIF Formats

## Status
Accepted

## Context

Phase 8 adds data portability. Users need to export their ledger for:

1. Migration to other accounting tools
2. Backup in a human-readable format
3. Integration with plain-text accounting workflows (e.g., `beancount`, `hledger`)

## Decision

### Beancount export

`export_beancount()` generates valid Beancount files with:
- Account declarations (`open` directives) with type mapping (Asset → Assets, Liability → Liabilities, etc.)
- Transaction entries with date, narration, and posting lines
- Account names sanitized to CamelCase (Beancount requirement)
- Amounts formatted as `USD` currency

### QIF export

`export_qif()` generates Quicken Interchange Format files with:
- Account type headers (`!Type:Bank`, `!Type:CCard`, etc.)
- Transaction records with `D` (date in MM/DD/YYYY), `T` (amount), `P` (payee), `^` (separator)

Both exporters are pure functions in `crates/core/src/export/` taking account and transaction data as input — no database dependency.

## Consequences

- **Pros:**
  - Beancount is the standard for plain-text accounting — wide tool compatibility
  - QIF support covers legacy tools (Quicken, GnuCash import)
  - Pure functions are trivially testable
  - No vendor lock-in — users can always extract their data

- **Cons:**
  - QIF format is limited (no multi-currency, no split transactions in standard spec)
  - Beancount export doesn't include balance assertions (would require end-of-period snapshots)

## References
- Beancount syntax: https://beancount.github.io/docs/beancount_language_syntax.html
- QIF format: https://en.wikipedia.org/wiki/Quicken_Interchange_Format
