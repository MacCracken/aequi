# Development Roadmap

All core phases (1-8), deferred items, and the AGNOS marketplace integration are complete as of v2026.3.13.
UX hardening pass completed as of v2026.3.18.

---

## UX Hardening (v2026.3.18)

| Item | Status | Detail |
|---|---|---|
| Dashboard page | Done | YTD income/expenses/net profit, outstanding invoices, pending receipts, recent transactions, quarterly tax |
| CRUD forms | Done | Transaction, contact (create + edit), invoice creation forms in-app |
| Search & filter | Done | All list pages: transactions, contacts, invoices, accounts have search + type/status/date filters |
| Pagination | Done | Transaction list paginated (50 per page) |
| DB indexes (V002) | Done | 13 performance indexes on frequent query paths |
| Transaction atomicity | Done | `create_transaction` wrapped in SQL transaction |
| CSP security | Done | Content Security Policy enabled in tauri.conf.json |
| Typed errors | Done | `CommandError` has `code` + `message` fields (VALIDATION, NOT_FOUND, DATABASE, etc.) |
| Input validation | Done | Contact email, invoice dates (due >= issue), transaction descriptions, debit/credit non-negative |
| Toast notifications | Done | Global toast system for success/error/info feedback |
| Error boundary | Done | React ErrorBoundary wraps the entire app |
| Loading skeletons | Done | Skeleton screens on all pages during data load |
| Export UI | Done | Beancount and QIF export buttons in Settings with file download |
| Settings expansion | Done | Business name/EIN, export, updates check, MCP config, audit log |
| Graceful startup | Done | Replaced `.expect()` panics in app init with proper error propagation |
| `.env.example` | Done | Reference file documenting all environment variables |
| Keyboard shortcuts | Done | Ctrl+0 for dashboard, Ctrl+1-7 for pages |

---

## Post-v1 Integrations

| Integration | Status | Value |
|---|---|---|
| Stripe webhook listener | Done | Auto-record Stripe payouts, fees, and refunds |
| Actual Budget | Done | Import from Actual Budget JSON export |
| Plaid | Done | Real-time bank sync (Link flow + transaction import) |
| Wave Accounting | Done | Migration import from Wave CSV export |
| GitHub / Linear | Done | Import invoiceable work items by milestone or label |
| SecureYeoman MCP | N/A (SY-side) | Agent-driven accounting workflows (aequi MCP server already exposes 24 tools) |
| Google Drive / Dropbox | Planned | Receipt watch folder via cloud-synced directory |
| DocuSign / PandaDoc | Planned | Invoice signature collection |
| IRS FIRE system | Planned | Direct 1099-NEC e-filing |
| IRS EFTPS | Planned | Direct 1040-ES quarterly payment submission |
| State tax authorities | Planned | State quarterly estimates (CA, NY, TX, FL, WA priority) |
| iCloud / Google Drive sync | Planned | Optional encrypted ledger sync between desktop and mobile |
