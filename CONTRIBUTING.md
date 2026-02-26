# Contributing to Aequi

## Prerequisites

- **Rust** (stable, via rustup) — `rustup update stable`
- **Node.js** 20+ and **pnpm** — for the React frontend
- **Tauri CLI v2** — `cargo install tauri-cli --version "^2"`
- **For mobile (Phase 3+):**
  - iOS: Xcode 15+ (macOS only)
  - Android: Android Studio + NDK r26+

## Development Setup

```bash
# Clone the repository
git clone https://github.com/anomalyco/aequi.git
cd aequi

# Install frontend dependencies
pnpm install

# Build all Rust crates
cargo build --workspace

# Run desktop app in development mode (hot-reload)
cd crates/app && cargo tauri dev
```

## Mobile Development (Phase 3+)

```bash
# Initialize iOS target (macOS only, requires Xcode)
cd crates/app && cargo tauri ios init
cargo tauri ios dev

# Initialize Android target (requires Android SDK + NDK)
cd crates/app && cargo tauri android init
cargo tauri android dev
```

The React frontend is shared between desktop and mobile. Use Tailwind's responsive
breakpoints (`sm:`, `md:`, `lg:`) for layout differences. No separate mobile codebase.

## Code Style

- Run `cargo fmt` before committing
- Run `cargo clippy -- -D warnings` to catch common mistakes
- Run `cargo test --workspace` to ensure all tests pass
- All business logic in `crates/core` and `crates/import` must have unit tests
- No `f32`/`f64` in financial calculations — use `rust_decimal::Decimal` or `Money`
- Frontend: `pnpm lint` and `pnpm typecheck` must pass

## Project Structure

```
aequi/
├── Cargo.toml              — workspace root (all crates share dependencies)
├── crates/
│   ├── core/               — pure business logic; no I/O; zero Tauri dependency
│   ├── storage/            — SQLite via sqlx; all migrations inline
│   ├── import/             — OFX/QFX/CSV parsers, match engine, rule engine
│   ├── ocr/                — Tesseract pipeline (Phase 3)
│   ├── pdf/                — Typst PDF generation (Phase 5)
│   ├── mcp/                — MCP server (Phase 6)
│   └── app/                — Tauri v2 entry point; commands only, no business logic
├── src/                    — React + TypeScript frontend (desktop + mobile)
├── docs/
│   ├── adr/                — Architecture Decision Records
│   └── development/        — Formulation doc, roadmap
└── rules/                  — Tax rule TOML files (community-maintained)
```

**Rule:** The `app` crate only contains Tauri command handlers that call into `core`/`storage`.
Business logic must never live in `app`.

## Adding a New Tauri Command

1. Add the handler in `crates/app/src/commands.rs`
2. Register it in the `invoke_handler!` macro in `crates/app/src/main.rs`
3. Add a TypeScript wrapper in `src/lib/commands.ts` (typed `invoke()`)
4. Write unit tests for any new logic in `core` or `storage`

## Submitting Changes

1. Fork the repository
2. Create a feature branch (`git checkout -b feat/my-feature`)
3. Make your changes
4. Run `cargo fmt && cargo clippy -- -D warnings && cargo test --workspace`
5. Run `pnpm lint && pnpm typecheck`
6. Submit a pull request with a description of what changed and why

## Tax Rule Files

Tax rates (SE tax, income brackets, mileage) update annually. Rule files live in
`rules/tax/us/YYYY.toml` and are schema-validated by CI. January PRs updating the
current year's file are always welcome.
