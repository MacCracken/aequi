# Aequi — Open-Source Self-Employed Accounting Platform

**Status:** In Development (Phase 1 Complete)  
**Target:** Freelancers, sole proprietors, independent contractors (US-focused v1)

---

## What is Aequi?

Aequi is a local-first, open-source desktop application for freelancer accounting. It provides full double-entry bookkeeping with a modern AI-driven automation layer via the Model Context Protocol (MCP).

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
│   └── app/                    # Tauri v2 desktop app
├── src/                        # frontend (HTML/JS)
├── docs/                       # documentation
└── CHANGELOG.md                # version history
```

## Technology Stack

| Layer | Choice |
|---|---|
| Desktop shell | Tauri v2 |
| Async runtime | Tokio |
| Frontend | React + TypeScript |
| UI components | shadcn/ui + Tailwind |
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

## What's Implemented (Phase 1)

- Double-entry bookkeeping with type-safe transaction validation
- Chart of accounts with Schedule C line mappings
- SQLite storage with WAL mode
- Tauri v2 commands: get_accounts, create_transaction, get_transactions, get_profit_loss
- Basic P&L reporting

## What's Next (Phase 2)

- Bank statement import (OFX/QFX/CSV)
- Transaction categorization rules
- Receipt OCR pipeline
- Full frontend UI

---

## License

MIT or AGPL-3.0

---

*This project follows the specification in `docs/development/aequi_formalation.md`.*
