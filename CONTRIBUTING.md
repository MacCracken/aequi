# Contributing to Aequi

## Prerequisites

- **Rust** 1.94+ (stable, via rustup; edition 2024) — `rustup update stable`
- **Node.js** 22.12+ (24 LTS recommended) and **npm** — for the React frontend
- **Tauri CLI v2** — `cargo install tauri-cli --version "^2"`
- **For mobile (Phase 3+):**
  - iOS: Xcode 15+ (macOS only)
  - Android: Android Studio + NDK r26+

## Development Setup

```bash
# Clone the repository
git clone https://github.com/MacCracken/aequi.git
cd aequi

# Install frontend dependencies
npm ci

# Build all Rust crates
cargo build --workspace

# Run desktop app in development mode (hot-reload)
cd crates/app && cargo tauri dev
```

## Releases and update signing

Pushing a `YYYY.M.D` tag (or running the *Release Automation* workflow) builds
signed installers for macOS, Windows and Linux, attaches them with a signed
`latest.json` to a GitHub release, and publishes it as the latest release.
Installed apps update from
`https://github.com/MacCracken/aequi/releases/latest/download/latest.json`.

One-time setup (the release workflow refuses to run without it):

```bash
# Generates a key pair; keep the private key and its password out of the repo.
cargo tauri signer generate -w ~/.tauri/aequi.key
```

1. Put the printed **public** key in `plugins.updater.pubkey` in
   `crates/app/tauri.conf.json` and commit it.
2. Add repository secrets `TAURI_SIGNING_PRIVATE_KEY` (contents of
   `~/.tauri/aequi.key`) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Losing the private key means existing installs can no longer verify updates.

Local `cargo tauri build` also signs updater bundles. Without the key, build
unsigned bundles with:

```bash
cargo tauri build --config '{"bundle":{"createUpdaterArtifacts":false}}'
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
- Frontend: `npx tsc --noEmit` must pass

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
2. Register it in the `tauri::generate_handler!` list in `crates/app/src/lib.rs`
3. Add a TypeScript wrapper in `src/lib/api.ts` (typed `invoke()`)
4. Write unit tests for any new logic in `core` or `storage`

## Submitting Changes

1. Fork the repository
2. Create a feature branch (`git checkout -b feat/my-feature`)
3. Make your changes
4. Run `cargo fmt && cargo clippy -- -D warnings && cargo test --workspace`
5. Run `npx tsc --noEmit`
6. Submit a pull request with a description of what changed and why

## Tax Rule Files

Tax rates (SE tax, income brackets, mileage) update annually. Rule files live in
`rules/tax/us/YYYY.toml` and are schema-validated by CI. January PRs updating the
current year's file are always welcome.
