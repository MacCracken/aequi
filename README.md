# Aequi — Open-Source Self-Employed Accounting Platform

**Status:** Pre-development planning  
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

## Architecture

The Rust backend is the application. The React frontend is a display layer only.

```
aequi/
├── Cargo.toml                  # workspace root
├── crates/
│   ├── core/                   # pure business logic (Money, ledger, tax, invoice)
│   ├── storage/                # SQLite via sqlx
│   ├── ocr/                    # Tesseract pipeline
│   ├── import/                 # OFX, QFX, CSV parsers
│   ├── pdf/                    # Typst PDF generation
│   ├── mcp/                    # MCP server (stdio transport)
│   └── app/                    # Tauri commands, state
├── src/                        # React frontend
├── rules/                      # Tax rule TOML files
└── templates/                  # Typst invoice templates
```

---

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
# Install dependencies
cargo build

# Run in development mode
cd crates/app && cargo tauri dev

# Run tests
cargo test --workspace
```

---

## License

MIT or AGPL-3.0

---

*This project follows the specification in `aequi_start_prompt.md`.*
