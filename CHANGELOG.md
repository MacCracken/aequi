# Changelog

All notable changes to this project will be documented in this file.

## [0.3.0] - 2026-02-26

### Added
- **OCR Crate** (`aequi-ocr`)
  - `OcrBackend` trait with `MockRecognizer` (tests) and optional `TesseractRecognizer` (`tesseract` feature)
  - Image preprocessing: grayscale + contrast stretch via `image` crate; auto-resize for images > 2800 px
  - SHA-256 content-addressed attachment store (`attachments/XX/HASH.ext` layout)
  - `Extractor` with per-field confidence scores: vendor, date, amounts, payment method
  - `ReceiptPipeline<R>`: hash → dedup → content-store → preprocess → OCR → extract
  - `spawn_intake_watcher`: notify-based watch-folder feeding an `mpsc` channel
  - 35 unit tests

- **Storage Crate** — Phase 3 additions
  - `receipts` table: file hash, OCR text, extracted fields, status, confidence, transaction link
  - `receipt_line_items` table for per-line receipt data
  - CRUD: `insert_receipt`, `get_receipt_by_id`, `get_receipts_pending_review`,
    `update_receipt_status`, `link_receipt_to_transaction`, `check_receipt_duplicate`
  - `ReceiptRecord` sqlx row type

- **App Crate** — Phase 3 additions
  - `AppState` extended with `attachments_dir` and `receipt_tx` pipeline channel
  - Background Tokio task consuming the receipt pipeline channel
  - Watch-folder task on `~/.aequi/intake/` (auto-created on startup)
  - Tauri commands: `ingest_receipt`, `get_pending_receipts`, `approve_receipt`, `reject_receipt`
  - `tauri-plugin-dialog` + `tauri-plugin-fs` for file picking on desktop and mobile
  - Capability files: `capabilities/default.json` (desktop) and `capabilities/mobile.json` (iOS/Android)
  - iOS min version 13.0, Android minSdkVersion 24 in `tauri.conf.json`

### Completed
- Phase 3: Receipt OCR pipeline + mobile scaffold

---

## [0.2.0] - 2026-02-26

### Added
- **Import Crate**
  - OFX/QFX parser for bank statement import
  - CSV importer with column-mapping profiles
  - Auto-match engine (date window + Levenshtein similarity)
  - Categorization rule engine (TOML-based, priority-ordered)
  - Fuzzy matching with configurable thresholds

- **Storage Crate**
  - Import profiles table with saved mappings
  - Imported transactions table with status tracking
  - Categorization rules table
  - Reconciliation sessions and items tables
  - CRUD operations for all new tables

### Completed
- Phase 1: Desktop Shell (Tauri v2, core bookkeeping, SQLite storage)
- Phase 2: Import + Reconciliation

### Planned (Phase 3)
- Receipt OCR pipeline
- iOS + Android mobile app via Tauri v2 mobile (camera receipt capture)

---

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
- No frontend UI yet
- No receipt OCR (Phase 3)
- No tax engine (Phase 4)
- No invoicing (Phase 5)
- No MCP server (Phase 6)
