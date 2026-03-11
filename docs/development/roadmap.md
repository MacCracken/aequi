# Development Roadmap

## Completed Phases

- **Phase 1** — Core Bookkeeping (desktop shell, accounts, transactions, SQLite)
- **Phase 2** — Import + Reconciliation (OFX/CSV, auto-match, categorization rules)
- **Phase 3** — Receipt OCR + Mobile Scaffold (OCR pipeline, intake watcher, Tauri mobile)
- **Phase 4** — Tax Engine (quarterly estimates, Schedule C, SE tax, TOML rules)
- **Phase 5** — Invoicing (contacts, invoice lifecycle, payments, 1099 tracking, PDF text rendering)
- **Phase 6** — HTTP API & Containerization (Axum REST server, Bearer auth, Dockerfile)
- **Phase 7** — MCP Server (24 tools, permissions, audit log, stdio transport)
- **Phase 8** — Polish + Ecosystem (Beancount/QIF export, settings UI, data portability)

---

## Remaining Work

### Deferred from Phase 5
- Typst PDF generation (currently plain text rendering)
- lettre SMTP delivery + Resend API option
- Mobile push notifications for invoice due dates

### Deferred from Phase 6
- GHCR publish workflow (GitHub Actions)
- Structured JSON logging via `tracing-bunyan-formatter`
- OAuth/OIDC authentication (currently API key only)

### Deferred from Phase 7
- SSE/streaming transport option

### Phase 7.5 — AGNOS Marketplace Integration

**Done (AGNOS-side, in agnosticos repo):**
- ✅ Marketplace recipe (`recipes/marketplace/aequi.toml`)
- ✅ Sandbox profile in `sandbox_profiles.rs` (desktop mode, network disabled, Tesseract OCR access)
- ✅ Agnoshi intent patterns (tax estimate, schedule C, import, balances, receipts)
- ✅ Agnoshi translate module (`translate/aequi.rs`) routing to MCP bridge
- ✅ Release workflow includes bare binary + tax rules in Linux tarball

**Done (aequi-side):**
- ✅ Daimon client module (`crates/server/src/daimon.rs`) with reqwest HTTP client
- ✅ `GET /v1/discover` handshake on startup to auto-detect daimon capabilities
- ✅ Register Aequi MCP agent with daimon on startup (`POST /v1/agents/register`)
- ✅ Periodic dashboard sync to daimon (`POST /v1/dashboard/sync`, 30s interval)
- ✅ Consumer health reporting via existing `GET /health` endpoint
- ✅ Graceful shutdown signaling for background daimon task
- ✅ 8 unit tests for daimon client (serialization, deserialization, shutdown)

- ✅ MCP server spawned as Tauri sidecar process (`tauri-plugin-shell`, `externalBin`)

### Deferred from Phase 8
- AI-assisted categorization via configured MCP endpoint
- Community tax rule update workflow (PR-based, annual)
- Import profile sharing (export/import TOML)
- Backup / restore (compressed SQLite snapshot + attachments)
- Auto-updater via Tauri updater
- Keyboard shortcuts + accessibility pass (WCAG 2.1 AA)
- Schema migration system (versioned SQL files, up/down)

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
