# OpenLedger — Open-Source Self-Employed Accounting Platform

**Status:** Pre-development planning
**Date:** 2026-02-25
**Target:** Freelancers, sole proprietors, independent contractors (US-focused v1)

---

## 1. Problem Statement

QuickBooks Self-Employed is the dominant tool for freelancer accounting but suffers from significant limitations:

- Hard limit on connected bank accounts and transactions
- No double-entry bookkeeping (simplified model only)
- Subscription pricing with no offline option
- Data locked in a proprietary cloud
- No programmable automation or API for AI tooling
- Tax features limited to Schedule C / SE tax; no state-level support
- Cannot grow with the user into a small business

**OpenLedger** is a local-first, open-source desktop application that removes these constraints and adds a modern AI-driven automation layer via the Model Context Protocol (MCP).

---

## 2. Goals

- Full double-entry bookkeeping accessible to non-accountants via abstracted UI
- Local-first data storage (SQLite); no mandatory cloud
- Receipt intake, categorization, and extraction powered by local or remote AI
- Quarterly and annual tax calculation with Schedule C export
- Professional invoicing with payment tracking and 1099-NEC support
- MCP server exposing accounting data and operations to AI agents
- Open, auditable tax rule engine with community-maintained rule sets
- Cross-platform desktop (macOS, Windows, Linux)

---

## 3. Non-Goals (v1)

- Payroll processing
- Multi-user / team access
- Non-US tax jurisdictions (v1; v2 target)
- Inventory management
- Point of sale
- Real-time payment processing (Stripe, Square) — tracked as future integration

---

## 4. Architecture

### 4.1 Philosophy

**The Rust backend is the application.** The React frontend is a display layer only — it holds no business logic, performs no calculations, and enforces no invariants. Every meaningful operation (transaction validation, tax calculation, OCR, import parsing, PDF generation, MCP dispatch) executes in Rust and is exposed to the frontend via Tauri's typed `invoke()` command system.

This means:
- Accounting correctness is enforced at compile time via Rust's type system
- The same core library can be used headlessly (CLI, tests, MCP server) without the UI
- No state lives in React that isn't derived from the Rust backend
- Business logic is testable with standard Rust unit tests, independent of any UI framework

### 4.2 Technology Stack

| Layer | Choice | Rationale |
|---|---|---|
| Desktop shell | **Tauri v2** | Native webview wrapper; Rust process owns all logic |
| Async runtime | **Tokio** | Multi-threaded async for concurrent pipelines (OCR, import, watch) |
| Frontend | **React + TypeScript** | Display only; communicates with backend via `invoke()` |
| UI components | **shadcn/ui + Tailwind** | Accessible, unstyled-first, easy to theme |
| Database | **SQLite via sqlx** | Async, compile-time query verification, WAL mode, single-file |
| Money arithmetic | **rust_decimal** | Exact decimal arithmetic; no floating point in financial calculations |
| Date/time | **chrono** | Typed date arithmetic for fiscal periods, due dates, safe harbor |
| Serialization | **serde + serde_json** | All IPC, rule files, import formats, MCP protocol |
| Config / rule files | **toml** crate | Tax rule files, categorization rules, import column profiles |
| OCR | **leptess** (Rust bindings to Tesseract) | Fully local; bundled with the app binary |
| Image processing | **image** crate | Receipt normalization before OCR (deskew, contrast) |
| PDF generation | **Typst** (Rust-native) | Invoice and report export; template-driven |
| HTTP client | **reqwest** | Plaid (v2), Resend email, optional MCP remote endpoints |
| Email (SMTP) | **lettre** | Direct SMTP delivery; no third-party dependency required |
| MCP server | **rmcp** or custom stdio handler | Shares the same core library; zero duplication |
| OFX/QFX parsing | Custom Rust parser | SGML-subset parser; no external dependency |
| CSV parsing | **csv** crate | Configurable column-mapping profiles |
| Regex | **regex** crate | Receipt extraction patterns, categorization rules |
| Compression | **flate2** | Backup archives |
| Hashing | **sha2** | Content-addressed attachment storage |
| Testing | **cargo test + tokio::test** | All business logic tested independently of Tauri |

### 4.3 Crate Structure

The backend is split into a workspace so each layer is independently testable and the MCP server can link against core without pulling in Tauri:

```
openledger/
  Cargo.toml                  — workspace root
  crates/
    core/                     — pure business logic, no I/O
      src/
        ledger/               — accounts, transactions, validation
        tax/                  — rule engine, SE tax, quarterly estimates
        invoice/              — invoice model, lifecycle, line items
        receipt/              — extraction types, confidence scoring
        money.rs              — Money<Decimal> newtype, currency
        period.rs             — FiscalYear, Quarter, DateRange
    storage/                  — sqlx database layer
      src/
        migrations/           — embedded SQL migrations
        db/                   — typed query modules per domain
    ocr/                      — Tesseract pipeline, image preprocessing
    import/                   — OFX, QFX, CSV parsers
    pdf/                      — Typst PDF generation
    mcp/                      — MCP server (stdio transport)
    app/                      — Tauri commands, event emitters, state
  src/                        — React frontend
  rules/                      — Tax rule TOML files (community-maintained)
  templates/                  — Typst invoice templates
```

`core` has zero I/O dependencies and compiles in under 5 seconds. All other crates depend on `core`. The `app` crate depends on everything and is the Tauri entry point.

### 4.4 Money Arithmetic

All monetary values are stored as `i64` (integer cents) in the database and represented as `rust_decimal::Decimal` in memory. No `f32` or `f64` anywhere in the financial stack.

```rust
// core/src/money.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Money(Decimal);  // always 2 decimal places, USD v1

impl Money {
    pub fn from_cents(cents: i64) -> Self { ... }
    pub fn to_cents(self) -> i64 { ... }
    pub fn checked_add(self, rhs: Self) -> Option<Self> { ... }
}
```

Tax calculations use `Decimal` throughout with explicit rounding modes matching IRS rounding rules (half-up to nearest dollar for final figures).

### 4.5 Type-Safe Transaction Validation

The ledger engine uses Rust's type system to make invalid transactions unrepresentable:

```rust
// core/src/ledger/transaction.rs
pub struct UnvalidatedTransaction {
    pub date: NaiveDate,
    pub description: String,
    pub lines: Vec<UnvalidatedLine>,
    pub memo: Option<String>,
}

pub struct ValidatedTransaction {
    // private constructor — only created by validate()
    inner: UnvalidatedTransaction,
    balanced_total: Money,
}

pub fn validate(tx: UnvalidatedTransaction) -> Result<ValidatedTransaction, LedgerError> {
    // checks: at least 2 lines, debits == credits, accounts exist, period open
}

pub enum LedgerError {
    Unbalanced { debit_total: Money, credit_total: Money },
    EmptyTransaction,
    AccountNotFound(AccountId),
    ClosedPeriod(NaiveDate),
    ArchivedAccount(AccountId),
}
```

The storage layer only accepts `ValidatedTransaction` — the type boundary is the contract.

### 4.6 Data Layer

```
~/.openledger/
  ledger.db          — SQLite (WAL mode, foreign keys enforced)
  attachments/       — Content-addressed by SHA-256 hash (receipts, PDFs)
  exports/           — Generated PDFs, CSV exports (ephemeral, regeneratable)
  rules/             — User-defined categorization rules (TOML)
  mcp-server.sock    — Unix socket for MCP server (optional, local only)
  backups/           — Compressed point-in-time backups
```

**SQLite configuration:**
```sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA synchronous = NORMAL;   -- safe with WAL; faster than FULL
PRAGMA busy_timeout = 5000;
PRAGMA cache_size = -32000;    -- 32MB page cache
```

**Schema (double-entry):**
- `accounts` — chart of accounts with type, schedule_c_line, is_archetype
- `transactions` — journal entry headers (date, description, reconciled_at)
- `transaction_lines` — debit/credit legs; amount stored as integer cents
- `receipts` — file hash, ocr_text, extracted JSON, confidence, review status
- `invoices` / `invoice_lines` — full lifecycle state machine
- `contacts` — clients and vendors, contractor flag, ytd_payments
- `tax_periods` — quarterly summaries, safe_harbor_amount, payment_recorded
- `mileage_log` — trips with deductible_amount computed at insert
- `import_batches` / `import_rows` — raw import state before commit
- `categorization_rules` — ordered rule set, pattern + account mapping
- `audit_log` — MCP tool calls, bulk imports, tax period closes
- `settings` — key/value store for all user preferences

### 4.7 Async Pipeline Architecture

The Tokio runtime runs three persistent background tasks alongside the main Tauri process:

```
tokio runtime
  ├── watch_task        — inotify/FSEvents watch on configured intake folders
  │                       new file → enqueue to receipt_pipeline channel
  ├── receipt_pipeline  — bounded channel (backpressure); processes receipts:
  │                       hash → dedup check → OCR → extract → persist → notify UI
  └── sync_task         — periodic: backup reminder, Plaid sync (v2), rule updates
```

Frontend receives real-time updates via Tauri events emitted from these tasks. No polling.

### 4.8 Process Model

```
openledger process (Tauri)
  ├── Rust core (all business logic)
  │   ├── LedgerEngine       — validates + commits transactions
  │   ├── ImportPipeline     — OFX/QFX/CSV → UnvalidatedTransaction[]
  │   ├── ReceiptPipeline    — file → OCR → ExtractedReceipt → review queue
  │   ├── TaxEngine          — rule-driven SE tax + quarterly estimates
  │   ├── InvoiceEngine      — lifecycle, PDF generation, email dispatch
  │   ├── ReportEngine       — SQL aggregations → typed report structs
  │   └── McpServer          — stdio child process or unix socket server
  └── React frontend (display + input only)
      ├── Dashboard
      ├── Transactions
      ├── Receipts (review queue)
      ├── Invoices
      ├── Tax Center
      ├── Reports
      └── Settings
```

---

## 5. Module Specifications

### 5.1 Bookkeeping Engine

**Chart of Accounts**

Default accounts follow standard self-employed structure:

```
Assets
  1000 Checking
  1010 Savings
  1020 Accounts Receivable
  1030 Undeposited Funds
Liabilities
  2000 Credit Card
  2010 Taxes Payable
Equity
  3000 Owner's Equity
  3100 Owner's Draw
Income
  4000 Services Revenue
  4010 Product Sales
  4020 Other Income
Expenses
  5000 Advertising & Marketing
  5010 Bank Fees
  5020 Business Meals (50% deductible)
  5030 Education & Training
  5040 Equipment
  5050 Home Office
  5060 Insurance
  5070 Internet & Phone
  5080 Legal & Professional
  5090 Mileage
  5100 Office Supplies
  5110 Software & Subscriptions
  5120 Travel
  5130 Utilities
  5140 Vehicle Expenses
  5900 Miscellaneous
```

Users can add, rename, or archive accounts. Archetype accounts (referenced by the tax engine's Schedule C mapping) cannot be deleted, only renamed.

**Transaction Validation**

Every saved transaction must satisfy:
1. Sum of all debit legs == sum of all credit legs (enforced by `validate()` return type)
2. At least two lines (one debit, one credit)
3. Date is valid and within an open fiscal period
4. All account references exist and are not archived

The UI presents a simplified "from account / to account" model for common cases (expense, income, transfer). A power mode exposes the full journal entry with explicit debit/credit columns.

**Reconciliation**

Import-then-match workflow:
1. User imports bank statement (OFX/QFX/CSV)
2. System attempts auto-match against existing transactions (date ±3 days, exact amount, description similarity via Levenshtein distance)
3. Unmatched imports go to **Review Queue**
4. User confirms matches or creates new transactions from unmatched rows
5. Reconciliation state stored per transaction-line per account

### 5.2 Import Pipeline

**Supported formats (v1):**
- OFX 1.x / 2.x — custom Rust SGML-subset parser (no external XML dep)
- QFX — same parser with minor extension handling
- CSV — configurable column mapping; profiles saved per financial institution

**Import flow:**
1. Parse file into `Vec<RawImportRow>` (all parsing errors collected, not fatal)
2. Deduplicate: SHA-256 of (date + amount_cents + description); skip already-imported rows
3. Apply categorization rules in priority order → `Option<AccountId>`
4. If no rule match and MCP endpoint configured: request AI suggestion (async, non-blocking)
5. Present rows in review UI with suggested category highlighted
6. User approves / edits / skips each row
7. Approved rows: `validate()` → `storage::commit_batch()` in a single transaction
8. Import batch record retained for audit (never deleted)

**Categorization rule format (TOML):**

```toml
[[rules]]
priority = 10
match_description = "GITHUB"          # substring, case-insensitive
match_amount_min = -100.00            # optional range filter
assign_account = "5110"               # Software & Subscriptions
memo = "GitHub subscription"

[[rules]]
priority = 20
match_description_regex = "^AMZN|AMAZON"
assign_account = "5100"               # Office Supplies (default; AI may override)
```

**Plaid (v2):**
- OAuth connection flow via Tauri embedded browser window
- Incremental sync via `/transactions/sync` endpoint
- Institution-specific field normalization via a JSON quirks table
- Access tokens stored encrypted in SQLite (not in plaintext settings)

### 5.3 Receipt Pipeline

**Intake methods:**
- Drag-and-drop onto the desktop app window
- Watch folder (tokio `inotify`/`FSEvents` via `notify` crate)
- MCP tool call (`accounting_ingest_receipt` — see §6)
- Mobile companion app scan → sync via local network (v2)
- Email forwarding address (v2; requires optional sync component)

**Processing steps (async, Tokio task):**
1. File received → compute SHA-256 → check for duplicate in `receipts` table
2. Copy to `attachments/{hash[0..2]}/{hash}.{ext}` (content-addressed, immutable)
3. Image preprocessing (if raster): deskew, normalize contrast via `image` crate
4. Tesseract OCR → raw text stored as `ocr_text` (never discarded)
5. Extraction pass (regex + heuristics on OCR text):
   - Vendor name (top-of-receipt lines, common patterns)
   - Date (multiple format patterns)
   - Subtotal / tax / total (currency pattern matching)
   - Line items (best effort; low confidence flagged)
   - Payment method (VISA/MC/AMEX patterns)
6. Confidence score (0.0–1.0) computed per field; aggregate receipt confidence
7. If MCP configured and confidence < 0.7: send ocr_text to AI for structured re-extraction
8. Receipt record persisted with `status = 'pending_review'`
9. Tauri event emitted → frontend updates review queue badge

**Privacy:** Steps 1–8 are entirely local. AI-assisted extraction (step 7) only runs if the user has explicitly configured an MCP endpoint and opted in.

### 5.4 Tax Engine

**Rule-driven architecture:**

The tax engine is a pure function: `TaxRules × LedgerSnapshot → TaxEstimate`. No database writes during calculation; results are persisted separately as `tax_periods`.

```rust
// core/src/tax/engine.rs
pub fn compute_quarterly_estimate(
    rules: &TaxRules,
    snapshot: &LedgerSnapshot,
    quarter: Quarter,
) -> QuarterlyEstimate { ... }

pub struct QuarterlyEstimate {
    pub ytd_gross_income: Money,
    pub ytd_total_expenses: Money,
    pub ytd_net_profit: Money,
    pub se_tax_base: Money,          // net_profit × 0.9235
    pub se_tax_amount: Money,        // se_tax_base × 0.153
    pub se_tax_deduction: Money,     // se_tax_amount × 0.50
    pub adjusted_net_income: Money,
    pub estimated_income_tax: Money, // based on bracket lookup
    pub total_tax_estimate: Money,
    pub safe_harbor_amount: Money,
    pub payment_due_date: NaiveDate,
    pub schedule_c_lines: BTreeMap<ScheduleCLine, Money>,
}
```

**US Self-Employment Tax v1 scope:**

| Feature | Notes |
|---|---|
| SE tax calculation | 15.3% (12.4% SS + 2.9% Medicare) on 92.35% of net profit |
| SE tax deduction | 50% of SE tax deducted from AGI (Schedule 1 line 15) |
| Quarterly estimates | Safe harbor: 100% of prior year liability or 90% of current year |
| Home office deduction | Simplified method ($5/sqft, max 300 sqft) or actual expenses (user selects) |
| Vehicle / mileage | Standard rate from current rule file or actual vehicle expenses |
| Meals deduction | 50% cap applied automatically to account 5020 at Schedule C aggregation |
| Schedule C line mapping | Each account has a `schedule_c_line` tag; engine aggregates per line |
| Schedule C PDF export | Pre-filled against IRS Form C-2024 PDF template via Typst overlay |

**Tax rule files:**

Stored in `rules/tax/us/YYYY.toml` — community-maintainable, PR-updated each January:

```toml
[year]
value = 2026

[se_tax]
ss_rate = 0.124
medicare_rate = 0.029
ss_wage_base = 176100         # updated annually
net_earnings_factor = 0.9235
deductible_fraction = 0.50

[income_brackets]
# Single filer 2026 ordinary income brackets
[[income_brackets.single]]
floor = 0
ceiling = 11925
rate = 0.10
[[income_brackets.single]]
floor = 11925
ceiling = 48475
rate = 0.12
# ... etc

[mileage]
business_cents_per_mile = 70   # updated annually
medical_cents_per_mile = 21
charity_cents_per_mile = 14

[meals_deduction_cap]
fraction = 0.50

[home_office_simplified]
rate_per_sqft = 5.00
max_sqft = 300

[quarterly_due_dates]
q1 = "2026-04-15"
q2 = "2026-06-15"
q3 = "2026-09-15"
q4 = "2027-01-15"
```

**v2 tax scope additions:**
- State income tax estimates (starting with CA, NY, TX, FL, WA)
- QBI deduction (Section 199A) for qualifying business income
- Home office depreciation (39-year straight-line)
- Section 179 / bonus depreciation for equipment purchases
- Multi-state allocation for remote workers with multiple state nexus

### 5.5 Invoice Engine

**Invoice lifecycle (state machine in Rust enum):**

```rust
pub enum InvoiceStatus {
    Draft,
    Sent { sent_at: DateTime<Utc> },
    Viewed { first_viewed_at: DateTime<Utc> },
    PartiallyPaid { paid_amount: Money, last_payment_at: DateTime<Utc> },
    Paid { paid_at: DateTime<Utc> },
    Overdue,   // computed from due_date, not stored
    Void { voided_at: DateTime<Utc>, reason: String },
}
```

Transitions are validated; e.g. `Void` cannot transition to `Sent`. `Overdue` is a computed view state, not a stored value.

**Features:**
- Customizable templates (Typst: logo, colors, payment terms, footer text)
- Line items: quantity, unit rate, description, taxable flag
- Discount: percentage or flat amount
- Tax lines: configurable rate(s), labeled (e.g. "Sales Tax 8.5%")
- Net-30/60/90 or custom due date
- PDF generation via Typst (sub-100ms on modern hardware)
- Email delivery: lettre (direct SMTP) or Resend API (configurable)
- Payment recording: full or partial, multiple payments per invoice
- Recurring schedule: weekly / monthly / custom interval, auto-draft or auto-send
- Estimate-to-invoice conversion (creates invoice preserving estimate line items)

**1099-NEC tracking:**
- Each contact flagged as `client`, `vendor`, or `contractor`
- YTD payments to each contractor aggregated at invoice commit time
- Warning surfaced in UI when any contractor crosses $600 in a calendar year
- 1099-NEC summary report (v1); direct e-filing via IRS FIRE system (v2)

### 5.6 Reporting

The report engine runs SQL aggregations against the `transaction_lines` table and returns typed Rust structs that are serialized to the frontend. All reports are computed on demand (no materialized views in v1); query time on a 5-year ledger is expected to be <50ms.

**Standard reports (v1):**

| Report | Description |
|---|---|
| Profit & Loss | Income minus expenses for any date range, grouped by account |
| Balance Sheet | Assets, liabilities, equity snapshot at any date |
| Cash Flow | Operating / investing / financing summary |
| Schedule C Preview | Tax-line-mapped P&L with deduction caps applied |
| Quarterly Tax Summary | YTD + estimated payment due + safe harbor amount |
| Invoice Aging | Outstanding invoices bucketed by 0-30, 31-60, 61-90, 90+ days |
| Expense by Category | Sorted by amount, with optional YoY comparison |
| Mileage Log | Trips, purposes, running deductible total |
| Top Clients | Revenue by client for any period |
| 1099-NEC Summary | Contractors at/above $600 threshold for any calendar year |

All reports exportable as CSV (raw data) and PDF (formatted via Typst).

---

## 6. MCP Server

The MCP server is an optional component sharing the same `core` and `storage` crates as the main app. It runs as either:
- A **stdio subprocess** spawned by Tauri (default; works with any MCP client including Claude Desktop and SecureYeoman)
- A **Unix socket server** for persistent local connections

Because it links against the same library, there is zero duplication of business logic between the desktop app and the MCP interface.

### 6.1 Tools

```
accounting_categorize_transaction
  Input:  description, amount, merchant?, date?
  Output: suggested_account, confidence, reasoning

accounting_extract_receipt
  Input:  file_path (image or PDF)
  Output: vendor, date, total, tax, line_items[], suggested_account

accounting_ingest_receipt
  Input:  file_path
  Output: receipt_id, extracted_fields, confidence, queued_for_review

accounting_draft_invoice
  Input:  client_name, description, amount, due_days?
  Output: invoice_id (draft), preview_fields

accounting_record_payment
  Input:  invoice_id, amount, date, method?
  Output: invoice_status, transaction_id

accounting_estimate_quarterly_tax
  Input:  quarter?, year?
  Output: ytd_net_profit, se_tax_amount, payment_due, due_date, safe_harbor_amount

accounting_query_ledger
  Input:  natural language query (e.g. "total spent on software Q3 2025")
  Output: sql_result (structured), summary (plain English)

accounting_get_account_balance
  Input:  account_name or account_code
  Output: balance, currency, as_of_date

accounting_list_unpaid_invoices
  Input:  (none)
  Output: invoices[], total_outstanding

accounting_find_deductions
  Input:  year?
  Output: categories[], potential_missed[], flagged_items[]

accounting_log_mileage
  Input:  date, origin, destination, purpose, miles?
  Output: trip_id, deductible_amount
```

### 6.2 Resources

```
accounting://summary                        — YTD P&L snapshot
accounting://transactions/{date-range}      — Ledger entries (ISO 8601 range)
accounting://accounts                       — Full chart of accounts
accounting://invoices/open                  — Unpaid and overdue invoices
accounting://tax/{year}/schedule-c          — Schedule C line totals
accounting://tax/{year}/quarterly           — Quarterly estimate breakdown
accounting://contacts                       — Clients and vendors
accounting://receipts/review-queue          — Receipts pending confirmation
```

### 6.3 Security Model

- MCP server binds to localhost only by default; remote requires explicit opt-in + API key
- Write tools (`accounting_ingest_receipt`, `accounting_record_payment`, `accounting_draft_invoice`, `accounting_log_mileage`) can be individually disabled in Settings
- Read-only mode toggle disables all write tools globally
- All MCP tool calls recorded in `audit_log` with timestamp, tool name, input hash, and outcome

---

## 7. Rust Crate Dependencies (Key)

```toml
# Core accounting (no I/O)
rust_decimal = { version = "1", features = ["serde"] }
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
thiserror = "1"

# Storage
sqlx = { version = "0.8", features = ["sqlite", "runtime-tokio", "macros", "chrono", "rust_decimal"] }
tokio = { version = "1", features = ["full"] }

# OCR + image
leptess = "0.14"         # Tesseract bindings
image = "0.25"

# Import parsing
csv = "1"
encoding_rs = "0.8"      # handle non-UTF8 OFX files (some banks)
regex = "1"

# PDF + email
typst = "0.11"
lettre = { version = "0.11", features = ["tokio1", "smtp-transport"] }
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }

# Hashing + compression
sha2 = "0.10"
flate2 = "1"

# Config / rules
toml = "0.8"

# MCP
serde_json = "1"
tokio-util = { version = "0.7", features = ["codec"] }  # framing for stdio MCP

# Tauri
tauri = { version = "2", features = ["shell-open"] }
tauri-build = "2"
```

All network TLS via **rustls** — no OpenSSL dependency, simpler cross-platform builds.

---

## 8. Build Phases

### Phase 1 — Foundation (Core Data + UI Shell)
- Cargo workspace + Tauri v2 scaffold
- `core` crate: `Money`, `Account`, `UnvalidatedTransaction`, `ValidatedTransaction`, `LedgerError`
- `storage` crate: SQLite setup, migrations, chart of accounts CRUD
- Tauri `invoke()` commands wired to storage
- Manual transaction entry UI (simplified + power mode)
- Basic P&L report
- First-run setup wizard (business name, fiscal year, base currency)

**Deliverable:** User can manually track income and expenses and see a P&L.

### Phase 2 — Import + Reconciliation
- OFX/QFX parser (custom SGML subset in Rust)
- CSV importer with saved column-mapping profiles
- Auto-match engine (date window + Levenshtein description similarity)
- Categorization rule engine (TOML rules, priority-ordered)
- Review queue UI (approve / edit / skip)
- Duplicate detection and reconciliation state tracking

**Deliverable:** User can import bank statements and reconcile their account.

### Phase 3 — Receipt Pipeline
- `ocr` crate: leptess bindings, image preprocessing, confidence scoring
- Async Tokio pipeline with bounded channel
- Watch folder via `notify` crate (cross-platform FSEvents/inotify)
- Extraction heuristics (regex patterns for vendor / date / amounts)
- Receipt review queue UI with attachment viewer (image + PDF)
- Receipt-to-transaction linking

**Deliverable:** User can photograph or drop receipts and have them auto-extracted.

### Phase 4 — Tax Engine
- `core/src/tax/` module: `TaxRules`, `TaxEngine::compute_quarterly_estimate()`
- TOML rule file loader with version validation
- SE tax, safe harbor, income bracket lookup
- Schedule C line mapping and aggregation
- Schedule C preview report + Typst PDF export
- Quarterly tax dashboard widget with countdown to next due date

**Deliverable:** User can see their estimated quarterly payment and Schedule C preview at any time.

### Phase 5 — Invoicing
- Contact management with contractor flag and YTD tracker
- Invoice CRUD + line items + discount + tax lines
- `InvoiceStatus` state machine with validated transitions
- Typst PDF generation (customizable template)
- lettre SMTP delivery + Resend API option
- Payment recording (full + partial)
- Invoice aging report
- 1099-NEC threshold warnings

**Deliverable:** User can create and send invoices and track payments.

### Phase 6 — MCP Server
- `mcp` crate: stdio transport, JSON-RPC framing via tokio-util codec
- Tool and resource handlers wired to `core` + `storage`
- Settings UI: enable/disable server, per-tool write permissions, read-only mode
- Audit log integration
- MCP server spawned as Tauri sidecar process

**Deliverable:** Any MCP-compatible AI agent (Claude Desktop, SecureYeoman, etc.) can query and operate the accounting system.

### Phase 7 — Polish + Ecosystem
- AI-assisted categorization via configured MCP endpoint (optional)
- Community tax rule update workflow (PR-based, annual)
- Import profile sharing (export/import TOML)
- Backup / restore (compressed SQLite snapshot + attachments)
- Data export: Beancount format (plain-text portability), QIF (legacy import)
- Auto-updater via Tauri updater
- Keyboard shortcuts + accessibility pass (WCAG 2.1 AA)

---

## 9. Open-Source Strategy

### License
**AGPL-3.0** — ensures any hosted or SaaS derivative remains open. The `core` crate published separately under **MIT** to encourage embedding in other tools.

### Repository Structure
```
openledger/
  Cargo.toml              — workspace
  crates/
    core/
    storage/
    ocr/
    import/
    pdf/
    mcp/
    app/
  src/                    — React frontend
  rules/                  — Tax rule TOML files (community-maintained)
  templates/              — Typst invoice templates
  docs/                   — User guide, architecture, contributing
  .github/                — CI (cargo test + clippy + tsc), issue templates
```

### Community Rule Maintenance
Tax rates and mileage rates change annually. The `rules/tax/` directory accepts pull requests each January with updated `YYYY.toml` files — low barrier to contribution, high value to all users. Rule files are schema-validated by CI on every PR.

### Non-Goals for the Open-Source Model
- No telemetry or analytics
- No required user accounts or cloud sync
- No commercial "Pro" tier gating features — all features open

---

## 10. Future Integrations (Post-v1)

| Integration | Value |
|---|---|
| Plaid | Real-time bank sync without manual OFX export |
| Stripe webhook listener | Auto-record Stripe payouts, fees, and refunds |
| GitHub / Linear | Import invoiceable work items by milestone or label |
| Google Drive / Dropbox | Receipt watch folder via cloud-synced directory |
| DocuSign / PandaDoc | Invoice signature collection |
| IRS FIRE system | Direct 1099-NEC e-filing |
| IRS EFTPS | Direct 1040-ES quarterly payment submission |
| State tax authorities | State quarterly estimates (CA, NY, TX, FL, WA priority) |
| Wave Accounting | Migration import (Wave CSV export → OpenLedger) |
| Actual Budget | Import from Actual (personal → business transaction split) |
| SecureYeoman MCP | Agent-driven accounting workflows, receipt intake automation |

---

## 11. Risks and Mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| Tax rule errors causing underpayment | Medium | "Informational only — consult a CPA" disclaimer on all tax output; community review of rule files; no auto-filing in v1 |
| Double-entry complexity alienating non-accountants | High | Default simplified from/to UI hides debits/credits; power mode is opt-in; good onboarding |
| OCR accuracy on low-quality receipts | High | Confidence scoring per field; human confirmation required before commit; easy manual override |
| Plaid auth reliability / pricing changes | Medium | OFX import always available as a fallback; Plaid gated to v2 |
| Keeping tax rule files current | Medium | Community PR workflow; CI validates schema; "rules last updated" indicator in UI |
| SQLite corruption on hard crash | Low | WAL mode + `synchronous = NORMAL` is crash-safe; periodic backup prompts |
| leptess (Tesseract) cross-platform build complexity | Medium | Bundle pre-built Tesseract data files; use musl for Linux; CI matrix for all three platforms |
| Typst API stability | Low | Pin to minor version; Typst is stable for document generation use cases |

---

## 12. Success Metrics (v1)

- Handles a full year of freelance accounting for a single-person business without requiring any external tool
- Schedule C preview matches a CPA-prepared return within 2% on a representative test dataset
- Receipt extraction accuracy >85% for clear photos of standard US retail receipts
- Import and reconciliation handles OFX from the 10 largest US retail banks without manual column mapping
- All financial calculations use exact decimal arithmetic (zero floating-point operations on money values)
- MCP server passes a standard MCP conformance test suite
- `cargo test` coverage >90% on the `core` crate
- Cold start time <2 seconds on mid-range hardware (2020 MacBook Pro, mid-range Windows laptop)
- Release binaries: <50MB on all platforms (Tauri, no bundled Node/Chromium)

---

*Document owner: TBD — to be moved to project repository on creation.*
