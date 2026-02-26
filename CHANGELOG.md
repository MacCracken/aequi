# Changelog

All notable changes to this project will be documented in this file.

## [0.1.0] - 2026-02-26

### Added
- **Core Crate**
  - `Money` type with decimal arithmetic, cents conversion
  - `Account`, `AccountId`, `AccountType` for chart of accounts
  - `UnvalidatedTransaction`, `ValidatedTransaction` with balance validation
  - `LedgerError` for type-safe error handling
  - Default chart of accounts with Schedule C line mappings
  - `FiscalYear`, `Quarter`, `DateRange` for period management

- **Storage Crate**
  - SQLite database setup with WAL mode
  - Schema migrations for accounts, transactions, transaction_lines, settings, fiscal_periods
  - Default account seeding
  - Account CRUD operations

- **App Crate**
  - Tauri v2 desktop application setup
  - `get_accounts` command
  - `create_transaction` command with validation
  - `get_transactions` command
  - `get_profit_loss` command

### Dependencies
- tauri v2
- sqlx v0.8 with SQLite
- rust_decimal v1.36
- chrono v0.4

### Known Issues
- No frontend UI yet (Phase 1 backend complete)
- No OFX/CSV import (Phase 2)
- No receipt OCR (Phase 3)
- No tax engine (Phase 4)
- No invoicing (Phase 5)
- No MCP server (Phase 6)
