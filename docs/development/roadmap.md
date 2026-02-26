# Development Roadmap

## Phase 3 — Receipt Pipeline + Mobile App

- Receipt review queue UI with attachment viewer (image + PDF)
- Receipt-to-transaction linking
- **iOS + Android mobile app via Tauri v2 Mobile**
  - Camera capture via `<input type="file" capture="camera">` → OCR pipeline
  - Responsive React frontend — single codebase, Tailwind breakpoints
  - App Store + Google Play distribution targets

**Deliverable:** User can photograph receipts on their phone and review/approve transactions on desktop or mobile.

---

## Phase 4 — Tax Engine

- `core/src/tax/` module: `TaxRules`, `TaxEngine::compute_quarterly_estimate()`
- TOML rule file loader with version validation
- SE tax, safe harbor, income bracket lookup
- Schedule C line mapping and aggregation
- Schedule C preview report + Typst PDF export
- Quarterly tax dashboard widget with countdown to next due date

**Deliverable:** User can see their estimated quarterly payment and Schedule C preview at any time.

---

## Phase 5 — Invoicing

- Contact management with contractor flag and YTD tracker
- Invoice CRUD + line items + discount + tax lines
- `InvoiceStatus` state machine with validated transitions
- Typst PDF generation (customizable template)
- lettre SMTP delivery + Resend API option
- Payment recording (full + partial)
- Invoice aging report
- 1099-NEC threshold warnings

**Deliverable:** User can create and send invoices and track payments.

---

## Phase 6 — MCP Server

- `mcp` crate: stdio transport, JSON-RPC framing via tokio-util codec
- Tool and resource handlers wired to `core` + `storage`
- Settings UI: enable/disable server, per-tool write permissions, read-only mode
- Audit log integration
- MCP server spawned as Tauri sidecar process

**Deliverable:** Any MCP-compatible AI agent (Claude Desktop, SecureYeoman, etc.) can query and operate the accounting system.

---

## Phase 7 — Polish + Ecosystem

- AI-assisted categorization via configured MCP endpoint (optional)
- Community tax rule update workflow (PR-based, annual)
- Import profile sharing (export/import TOML)
- Backup / restore (compressed SQLite snapshot + attachments)
- Data export: Beancount format (plain-text portability), QIF (legacy import)
- Auto-updater via Tauri updater
- Keyboard shortcuts + accessibility pass (WCAG 2.1 AA)
- Mobile: push notifications for invoice due dates and quarterly tax reminders

---

## Post-v1 Integrations

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
| Wave Accounting | Migration import |
| Actual Budget | Import from Actual |
| SecureYeoman MCP | Agent-driven accounting workflows |
| iCloud / Google Drive sync | Optional encrypted ledger sync between desktop and mobile |
