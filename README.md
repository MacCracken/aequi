# Aequi — Open-Source Self-Employed Accounting Platform

**Status:** In Development (Phase 2 Complete — Phase 3 in Progress)
**Target:** Freelancers, sole proprietors, independent contractors (US-focused v1)

---

## What is Aequi?

Aequi is a local-first, open-source desktop and mobile application for freelancer accounting. It provides full double-entry bookkeeping with a modern AI-driven automation layer via the Model Context Protocol (MCP).

### Why Aequi?

QuickBooks Self-Employed has significant limitations:
- Hard limit on connected bank accounts and transactions
- No double-entry bookkeeping
- Subscription pricing with no offline option
- Data locked in a proprietary cloud
- No programmable automation or API for AI tooling

Aequi removes these constraints:
- Full double-entry bookkeeping accessible via abstracted UI
- Local-first data storage (SQLite); no mandatory cloud
- Receipt intake with AI-powered extraction via MCP
- Quarterly and annual tax calculation with Schedule C export
- Professional invoicing with payment tracking
- MCP server exposing accounting data to AI agents
- Open, auditable tax rule engine

---

## Crate Structure

```
aequi/
├── Cargo.toml                  # workspace root
├── crates/
│   ├── core/                   # pure business logic (Money, Account, Transaction)
│   ├── storage/                # SQLite via sqlx
│   ├── import/                 # OFX/CSV import, matching, rules
│   ├── app/                   # Tauri v2 desktop app
│   ├── ocr/                   # receipt OCR (Phase 3)
│   ├── pdf/                   # PDF generation
│   └── mcp/                   # MCP server (Phase 6)
├── src/                        # frontend (HTML/JS)
├── docs/                       # documentation
└── CHANGELOG.md                # version history
```

## Technology Stack

| Layer | Choice |
|---|---|
| Desktop shell | Tauri v2 (macOS, Windows, Linux) |
| Mobile shell | Tauri v2 Mobile (iOS, Android) |
| Async runtime | Tokio |
| Frontend | React + TypeScript |
| UI components | shadcn/ui + Tailwind (responsive) |
| Database | SQLite via sqlx |
| Money arithmetic | rust_decimal |
| Date/time | chrono |

---

## Getting Started

```bash
# Build workspace
cargo build --workspace

# Run in development mode
cd crates/app && cargo tauri dev

# Run tests
cargo test --workspace
```

## What's Implemented

- Double-entry bookkeeping with type-safe transaction validation
- Chart of accounts with Schedule C line mappings
- SQLite storage with WAL mode
- Tauri v2 commands: get_accounts, create_transaction, get_transactions, get_profit_loss
- Basic P&L reporting
- Bank statement import (OFX/QFX/CSV)
- CSV import with saved column-mapping profiles
- Auto-match engine (date window + Levenshtein similarity)
- Categorization rule engine (TOML-based, priority-ordered)
- Reconciliation session tracking

## What's Next (Phase 3)

- Receipt OCR pipeline (Tesseract via leptess)
- Watch folder + drag-and-drop receipt intake
- iOS + Android mobile app via Tauri v2 Mobile (camera-to-receipt capture)
- Full frontend UI (desktop + mobile responsive)

---

## License

MIT or AGPL-3.0

---

*This project follows the specification in `docs/development/aequi_formalation.md`.*
