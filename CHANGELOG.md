# Changelog

All notable changes to this project will be documented in this file.

## [2026.3.18] - 2026-03-18

### Added
- **Dashboard Page** (`src/pages/DashboardPage.tsx`)
  - New landing page with account summaries and recent transaction overview
  - Keyboard shortcut `Ctrl+0` for quick navigation

- **CRUD Forms**
  - Transaction create/edit forms with validated input
  - Contact create/edit forms with type selection
  - Invoice creation form with line items
  - Search and filter on accounts, transactions, contacts, and invoices pages
  - Pagination on transactions and invoices pages

- **Error Boundary** (`src/components/ErrorBoundary.tsx`)
  - React error boundary wrapping entire app with graceful fallback UI

- **Toast Notification System** (`src/components/Toast.tsx`)
  - Success, error, and info toasts with auto-dismiss
  - Used across CRUD operations for user feedback

- **Loading Skeletons** (`src/components/Skeleton.tsx`)
  - Animated placeholder skeletons on all data-fetching pages

- **Database Indexes** (`V002__indexes_and_constraints.sql`)
  - New migration adding indexes on frequently queried columns
  - Rollback support via `V002__indexes_and_constraints.down.sql`

- **Typed Command Errors**
  - `CommandError` with `code` field (VALIDATION, NOT_FOUND, DATABASE, INTERNAL)
  - Frontend error handling uses error codes for contextual messages

- **Input Validation**
  - Server-side validation on all Tauri commands with descriptive error messages
  - Frontend form validation before submission

- **Content Security Policy**
  - CSP headers configured in `tauri.conf.json` for XSS protection

- **Environment Reference** (`.env.example`)
  - Documented all environment variables for server, OIDC, Stripe, Plaid, email, MCP

- **Settings Page Expansion**
  - Backup/restore controls
  - Export (Beancount/QIF) UI
  - Schema version viewer

- **Accessibility Improvements**
  - Skip-to-content link, ARIA landmarks, `focus-visible` outlines
  - `prefers-reduced-motion` and `prefers-contrast: more` media queries
  - 44px minimum touch targets on mobile

### Changed
- Version bumped to 2026.3.18 (CalVer)
- Test suite expanded to **472 tests** (core 113, import 147, mcp 52, ocr 52, storage 44, server 29, email 27, pdf 8), 0 failures, clippy clean
- Extensive code quality pass: input sanitization, error handling hardening, dead code removal
- Storage backup hardened with path traversal checks and improved error reporting
- OIDC module hardened with key rotation retry logic and better error messages
- Stripe webhook handler hardened with constant-time signature comparison
- Plaid integration hardened with input validation and error mapping
- AI categorization module improved with confidence threshold validation
- Import profile sharing improved with TOML validation and size limits
- Email delivery improved with attachment size validation and config checks
- Graceful startup handling when database or config is missing

### Fixed
- `schedule_c.rs` quality repair for tag parsing edge case
- Frontend capture utility type safety improvements
- Dashboard data loading race conditions
- Invoice and receipt page rendering with missing optional fields
- Settings page layout issues on smaller viewports
- MCP test coverage gaps filled (52 tests, up from 33)

---

## [2026.3.13] - 2026-03-13

### Added
- **Email Delivery** (`crates/email/`)
  - New `aequi-email` crate with two backends: lettre SMTP and Resend HTTP API
  - Invoice PDF attachment with plain-text body
  - Tagged `EmailConfig` enum (SMTP or Resend) with serde JSON config
  - `send_invoice` Tauri command (reads config from `email_config` setting)
  - `POST /api/v1/invoices/{id}/send` server endpoint
  - Auto-transitions invoice status to Sent after delivery
  - 5 unit tests

- **Structured JSON Logging**
  - `tracing-bunyan-formatter` for Bunyan-compatible JSON output
  - Enable via `AEQUI_LOG_FORMAT=json` env var
  - Falls back to human-readable format when unset

- **OAuth/OIDC Authentication** (`crates/server/src/oidc.rs`)
  - JWKS-based JWT validation from any OIDC provider
  - Auto-discovery via `.well-known/openid-configuration`
  - Key rotation with automatic JWKS refresh on validation failure
  - Configurable via `AEQUI_OIDC_CONFIG` env var (JSON with issuer + audience)
  - Auth middleware accepts both API key and OIDC JWT Bearer tokens
  - 4 unit tests

- **Auto-Updater**
  - `tauri-plugin-updater` with GitHub Releases endpoint
  - `check_for_updates` Tauri command returning version info
  - Updater config in `tauri.conf.json`

- **Keyboard Shortcuts + Accessibility**
  - `Ctrl+1`–`Ctrl+7` navigation to all pages
  - `Ctrl+/` toggles keyboard shortcut help overlay
  - Skip-to-content link for keyboard/screen reader users
  - ARIA landmarks: `role="banner"`, `role="main"`, `aria-label` on nav regions
  - `aria-hidden="true"` on decorative SVG icons
  - `focus-visible` outline ring (2px primary color) on all interactive elements
  - 44px minimum touch target on mobile nav (WCAG 2.1 AA)
  - `prefers-reduced-motion` media query disables all animations
  - `prefers-contrast: more` media query for high-contrast mode

- **AI-Assisted Categorization** (`crates/import/src/ai_categorize.rs`)
  - `AiCategorizationConfig` with endpoint URL, API key, confidence threshold
  - `suggest_category()` calls external MCP endpoint with transaction details
  - `suggest_categories_batch()` for bulk categorization
  - Filters suggestions below configurable confidence threshold
  - 4 unit tests

- **Community Tax Rule Workflow** (`crates/core/src/tax/community.rs`)
  - `CommunityTaxRules` package format with `TaxRulesMeta` (country, jurisdiction, year, author, version)
  - `validate_submission()` checks metadata completeness, year consistency, jurisdiction validity, and rules parsability
  - `rules_path()` generates canonical file paths for tax rule files
  - Supported jurisdictions: us-federal, ca, ny, tx, fl, wa
  - 5 unit tests

- **Import Profile Sharing** (`crates/import/src/profile_sharing.rs`)
  - `SharedProfile` bundle: metadata + CSV profile + optional categorization rules
  - `export_profile()` serializes to shareable TOML
  - `import_profile()` parses and validates TOML submissions
  - 4 unit tests

- **Mobile Push Notifications**
  - `tauri-plugin-notification` for iOS/Android native notifications
  - `check_overdue_invoices` Tauri command sends notification when invoices are past due

- **Frontend API Bindings**
  - `sendInvoice()` for email delivery
  - `checkForUpdates()` for auto-updater

- **Stripe Webhook Listener** (`crates/server/src/routes/stripe.rs`)
  - `POST /api/v1/stripe/webhook` endpoint (outside auth middleware, uses Stripe signature verification)
  - HMAC-SHA256 signature verification per Stripe's webhook spec
  - Auto-creates double-entry transactions for `charge.succeeded`, `charge.refunded`, `payout.paid`
  - Stripe fees recorded as separate line items to Bank Fees account (5010)
  - Configurable via `STRIPE_WEBHOOK_SECRET` env var
  - 9 unit tests: signature verification, event mapping, deserialization

- **Actual Budget Import** (`crates/import/src/actual.rs`)
  - Parses Actual Budget JSON export format (accounts, transactions, payees, categories)
  - Supports both string dates ("YYYY-MM-DD") and integer dates (YYYYMMDD)
  - Resolves payee and category names from ID lookups
  - Transfer detection with optional skip (avoids double-counting)
  - `ImportSummary` with account/transaction counts and error details
  - 8 unit tests: parsing, name resolution, date formats, transfers, empty exports

- **Plaid Bank Sync** (`crates/import/src/plaid.rs` + `crates/server/src/routes/plaid.rs`)
  - `PlaidClient` with Link token creation, public token exchange, and transaction fetch
  - `PlaidConfig` with sandbox/development/production environment support
  - Server endpoints: `POST /plaid/link-token`, `/plaid/exchange`, `/plaid/sync`
  - Auto-creates double-entry transactions from Plaid data (checking vs expense)
  - Access token stored in settings table for persistent bank connection
  - Configurable via `AEQUI_PLAID_CONFIG` env var
  - 12 unit tests (client + routes)

- **Wave Accounting Import** (`crates/import/src/wave.rs`)
  - Parses Wave's CSV export format (Transaction ID, Date, Account, Type, Amount, etc.)
  - Converts dollar amounts to cents, parses MM/DD/YYYY dates
  - `WaveImportSummary` with account counts, transaction counts, date range
  - 7 unit tests

- **GitHub / Linear Work Items** (`crates/import/src/work_items.rs`)
  - `WorkItemSource` enum: GitHub (owner/repo/token) or Linear (API key/team)
  - `fetch_work_items()` calls GitHub REST API or Linear GraphQL
  - `WorkItemFilter` by milestone, label, date, assignee
  - `estimate_line_items()` converts work items to invoice line estimates with hourly rates
  - 6 unit tests

### Changed
- Version bumped to 2026.3.13 (CalVer)
- Server state now includes optional `email_config` and `oidc` fields
- Auth middleware supports API key + OIDC JWT (previously API key only)
- Import crate gains `reqwest` and `serde_json` dependencies for AI categorization
- App crate gains `rust_decimal` dependency for invoice record reconstruction
- All deferred roadmap items complete — roadmap now shows only post-v1 integrations
- Stripe, Actual Budget, Plaid, Wave, and GitHub/Linear integrations added as post-v1 features
- Server crate now depends on `aequi-import` for Plaid client

---

## [2026.3.10] - 2026-03-10

### Added
- **Invoice Engine** (`core/src/invoice/`)
  - `Contact` model with `ContactType` (Client, Vendor, Contractor) and auto `is_contractor` flag
  - `Invoice` struct with line items, `Discount` (Percentage/Flat), `TaxLine`, computed totals
  - `InvoiceStatus` state machine with validated transitions (Draft → Sent → Viewed → Paid)
  - `Payment` recording (full + partial)
  - `compute_ytd_payments()` and `check_1099_threshold()` for 1099-NEC tracking ($600)
  - 34 unit tests covering lifecycle transitions, computation, contacts, payments

- **Data Export** (`core/src/export/`)
  - Beancount format export with account declarations and sanitized names
  - QIF (Quicken Interchange Format) export with account type mapping
  - 6 unit tests

- **Invoice PDF** (`pdf/src/`)
  - Plain text invoice rendering (`invoice_pdf.rs`)
  - **Typst PDF generation** (`typst_pdf.rs`)
    - Professional invoice PDF rendering via Typst typesetting engine
    - Full invoice layout: header, bill-to, line items table, subtotal/discount/tax/total
    - Typst special character escaping (prevents math mode injection from $ in currency)
    - 8 tests: markup generation, discount/tax variants, PDF byte output validation

- **Storage Crate** — Phase 5-8 additions
  - New tables: `contacts`, `invoices`, `invoice_lines`, `invoice_tax_lines`, `payments`, `audit_log`
  - Contact CRUD: `insert_contact`, `update_contact`, `get_all_contacts`, `get_contact_by_id`, `get_contractors`
  - Invoice CRUD: `insert_invoice`, `update_invoice_status`, `get_all_invoices`, `get_invoice_by_id`, `get_invoices_by_status`
  - Invoice lines and tax lines: `insert_invoice_line`, `get_invoice_lines`, `insert_invoice_tax_line`, `get_invoice_tax_lines`
  - Payments: `insert_payment`, `get_payments_for_invoice`, `get_ytd_payments_to_contact`
  - Invoice aging: `get_invoice_aging`
  - Audit log: `insert_audit_log`, `get_audit_log`
  - Settings: `get_setting`, `set_setting`
  - `serde::Serialize` added to all record types for API serialization
  - **Schema Migration System** (`storage/src/migrate.rs`)
    - Versioned SQL migration files with up/down support
    - `schema_versions` table tracks applied migrations with checksums
    - Automatic bootstrap for pre-existing databases (detects existing tables)
    - Checksum verification prevents running modified migrations
    - Statement splitter handles comments, string literals, multi-statement files
    - `run_migrations()`, `rollback_last()`, `get_schema_versions()`, `current_version()`
    - 14 unit tests: apply, idempotent, rollback, reapply, bootstrap, statement splitting
  - **Backup / Restore** (`storage/src/backup.rs`)
    - Compressed `.tar.gz` archive containing SQLite snapshot + attachments
    - `VACUUM INTO` for consistent point-in-time database snapshot (safe with WAL mode)
    - `manifest.json` with version, schema version, timestamps, file counts
    - Path traversal protection on restore (rejects absolute paths and `..`)
    - `create_backup()`, `restore_backup()` with `BackupManifest` metadata
    - 7 unit tests: create, roundtrip, empty attachments, invalid archive, file counting

- **HTTP API Server** (`crates/server/`)
  - Axum REST server on port 8060 with domain-organized routes
  - Bearer token auth middleware (`AEQUI_API_KEY` env var)
  - Endpoints: accounts, transactions, receipts, tax, invoices, contacts, payments, rules, reconciliation, reports
  - `GET /health` unauthenticated endpoint for service discovery
  - CORS headers for cross-origin access
  - **AGNOS Daimon Integration** (`crates/server/src/daimon.rs`)
    - `DaimonClient` with reqwest HTTP client for AGNOS orchestrator communication
    - `GET /v1/discover` handshake on startup to detect daimon capabilities
    - `POST /v1/agents/register` agent registration with capabilities and metadata
    - `POST /v1/dashboard/sync` periodic heartbeat (30s interval) with agent status
    - Background tokio task with graceful shutdown via `watch` channel
    - Registration retry with backoff (5 attempts, 10s delay)
    - Configurable daimon URL via `AEQUI_DAIMON_URL` env var (default `127.0.0.1:8090`)
    - 8 unit tests: serialization, deserialization, client creation, shutdown signal

- **MCP Server** (`crates/mcp/`)
  - Stdio JSON-RPC 2.0 transport with `initialize`, `tools/list`, `tools/call`
  - `ToolRegistry` with generic `register()` for async handler closures
  - 24 tools across 8 domains (accounts, transactions, receipts, tax, invoices, rules, import, reconciliation)
  - `Permissions` system: `read_only` mode and per-tool `disabled_tools` blocklist
  - SHA-256 audit logging of tool invocations
  - **SSE Transport** (`sse` feature flag)
    - HTTP+SSE transport per MCP specification 2024-11-05
    - `GET /sse` opens event stream with session ID; `POST /message?sessionId=` sends JSON-RPC requests
    - Multi-client support via per-session channels
    - Configurable via `AEQUI_MCP_TRANSPORT=sse` and `AEQUI_MCP_PORT` (default 8061)
    - 7 tests: event stream, session management, tool list, tool call, unknown method, unknown session, cleanup
  - 40 unit tests total: registry, permissions, accounts, transactions, receipts, tax, invoices, rules, import, reconciliation, protocol, audit, SSE transport

- **MCP Sidecar** (`crates/app/`)
  - `aequi-mcp` binary spawned as Tauri sidecar process on desktop startup
  - `tauri-plugin-shell` for process management with `externalBin` configuration
  - Sidecar receives `AEQUI_DB_PATH` env var to share database with main app
  - Stderr logging forwarded to app tracing system
  - Graceful lifecycle — sidecar exits when the Tauri app window closes

- **App Crate** — 12 new Tauri commands
  - `get_contacts`, `create_contact`, `get_invoices`, `create_invoice`
  - `get_invoice_aging`, `record_invoice_payment`, `get_1099_summary`
  - `export_beancount`, `export_qif`
  - `get_setting`, `set_setting`, `get_audit_log`, `get_schema_versions`
  - `create_backup`, `restore_backup`
  - Total: 25 Tauri commands

- **Frontend** — 3 new pages
  - Invoices page with status badges and All/Aging tab toggle
  - Contacts page with type badges and contractor indicators
  - Settings page with MCP server toggle, read-only mode, and audit log viewer
  - Navigation: 7 links (Accounts, Transactions, Receipts, Tax, Invoices, Contacts, Settings)

- **Containerization**
  - Multi-stage Dockerfile (Rust 1.85 builder → minimal Debian runtime)
  - `.dockerignore` for target/, node_modules/, dist/
  - SQLite `?mode=rwc` fix for database creation in fresh containers
  - **GHCR Publish Workflow** (`docker-publish.yml`)
    - Automated container build and push to `ghcr.io` on release tags
    - Docker Buildx with GHA layer caching for fast rebuilds
    - Version-tagged and `latest` image tags via metadata action
    - Manual workflow dispatch for ad-hoc publishes

### Fixed
- SQLite connection string now uses `?mode=rwc` to create database file if missing (fixes Docker startup)
- Unused `formatCents` import in InvoicesPage.tsx (fixes TypeScript CI check)
- CI workflows now build `aequi-mcp` sidecar before workspace build/clippy/docs steps

- **Documentation**
  - ADR-009: Invoice Engine with Contact Model and Lifecycle State Machine
  - ADR-010: HTTP API Server with Axum and Docker Containerization
  - ADR-011: MCP Server with Tool Registry and Permission System
  - ADR-012: Data Export — Beancount and QIF Formats

- **Tax Engine** (`core/src/tax/`)
  - `TaxRules` struct with TOML deserialization and validation
  - `TaxRules::compute_income_tax()` — progressive bracket computation
  - `compute_quarterly_estimate()` — pure function: TaxRules x LedgerSnapshot → QuarterlyEstimate
  - `schedule_c_preview()` — Schedule C line totals with deduction caps applied
  - `ScheduleCLine` enum with `from_tag()` parser for all Schedule C lines
  - `LedgerSnapshot` — point-in-time ledger aggregation by Schedule C line
  - SE tax calculation with Social Security wage base cap ($176,100)
  - 50% SE tax deduction from AGI
  - Safe harbor: min(100% prior year, 90% current year estimate)
  - Meals deduction cap (50%) applied automatically to Line 24b
  - IRS-style rounding (half-up to nearest dollar) via `Money::round_to_dollar()`
  - 25 unit tests covering brackets, SE tax, wage base cap, loss scenarios, safe harbor

- **Tax Rule File** (`rules/tax/us/2026.toml`)
  - 2026 US tax rates: SE tax, income brackets (single filer), mileage, meals cap
  - Quarterly due dates
  - Community-maintainable TOML format with inline documentation

- **Money type enhancements**
  - `Mul<Decimal>` implementation for rate calculations
  - `as_decimal()` accessor for intermediate precision
  - `round_to_dollar()` with IRS MidpointAwayFromZero rounding

- **Storage Crate** — Phase 4 additions
  - `tax_periods` table: quarterly estimates, SE tax, income tax, payments
  - `build_ledger_snapshot()` — SQL aggregation by Schedule C line
  - `upsert_tax_period()`, `record_tax_payment()`, `get_tax_periods()`
  - `get_prior_year_total_tax()` for safe harbor calculation

- **App Crate** — Phase 4 additions
  - `estimate_quarterly_tax` Tauri command (auto-detects current year/quarter)
  - `get_schedule_c_preview` Tauri command
  - Tax rules loaded via `include_str!()` for embedded distribution

- **Frontend** — Tax Center
  - Tax page with Quarterly Estimate and Schedule C Preview tabs
  - Quarterly payment callout with due date countdown
  - Summary cards: YTD income, expenses, net profit, SE tax, income tax
  - Schedule C line-by-line breakdown (income and expense sections)
  - Navigation: "Tax" added to desktop top nav and mobile bottom nav

- **Documentation**
  - ADR-008: Tax Engine as Pure Function with TOML Rule Files
  - ADR-009: Invoice Engine with Contact Model and Lifecycle State Machine
  - ADR-010: HTTP API Server with Axum and Docker Containerization
  - ADR-011: MCP Server with Tool Registry and Permission System
  - ADR-012: Data Export — Beancount and QIF Formats

### Completed
- Phase 4: Tax Engine
- Phase 5: Invoicing
- Phase 6: HTTP API & Containerization
- Phase 7: MCP Server
- Phase 8: Polish + Ecosystem (partial — exports and settings)

---

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
- No invoicing (Phase 5)
- No HTTP API / containerization (Phase 6)
- No MCP server (Phase 7)
