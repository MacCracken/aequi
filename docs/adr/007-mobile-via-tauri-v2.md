# ADR-007: iOS and Android Mobile App via Tauri v2 Mobile

## Status
Accepted

## Context

Freelancers frequently need to capture receipts on-the-go using their phone camera and review account activity away from their desk. A mobile companion experience was originally scoped as a v2 feature ("mobile companion app scan → sync via local network"). However, Tauri v2 introduced first-class mobile support that allows the same Rust backend and React frontend to compile directly to iOS and Android — eliminating the need for a separate mobile codebase or a network sync layer.

Key requirements:
- Receipt capture via device camera → run through the same OCR pipeline
- Review imported transactions and receipts from a mobile device
- No separate data store or sync protocol — mobile reads/writes the same SQLite ledger
- No cloud required; app works offline by default

## Decision

We will build the iOS and Android apps using **Tauri v2 Mobile** rather than React Native, Flutter, or a separate native app.

The mobile app is not a companion app — it is the same app, compiled to a different target. It shares:
- All Rust crates (`core`, `storage`, `import`, `ocr`) — no duplication of business logic
- The React + TypeScript frontend — responsive layout via Tailwind CSS breakpoints
- The same SQLite database on the device

Mobile-specific additions:
- `tauri-plugin-camera` for camera capture
- `tauri-plugin-fs` with mobile-safe paths for the ledger and attachments directory
- Responsive UI adjustments in the React layer (bottom nav, touch-optimized lists)

## Consequences

- **Pros:**
  - Zero duplication of business logic — accounting correctness is the same on mobile and desktop
  - Single repository, single test suite covers all platforms
  - Tauri v2 `invoke()` IPC works identically on mobile and desktop — no additional API surface
  - Leverages existing `ocr` crate for on-device receipt processing (no server required)

- **Rejected alternatives:**
  - **React Native + REST API**: would require duplicating business logic or exposing a local HTTP server; adds significant complexity
  - **Flutter**: requires rewriting frontend in Dart; contradicts the "React is the display layer" principle
  - **Separate native Swift/Kotlin app**: maximum maintenance burden; rejected outright

- **Cons / risks:**
  - Tesseract (leptess) cross-compilation for iOS/Android is non-trivial; pre-built static libs may be required
  - Apple App Store review may require justification for SQLite file access patterns
  - Tauri v2 mobile is newer and less battle-tested than its desktop counterpart

## Implementation Plan

Phase 3 deliverables:
1. Add iOS and Android targets to the CI matrix
2. `cargo tauri ios init` + `cargo tauri android init` in `crates/app/`
3. Integrate `tauri-plugin-camera` for receipt capture
4. Responsive frontend adjustments (mobile nav, touch targets)
5. Pre-built Tesseract static libs for `aarch64-apple-ios` and `aarch64-linux-android`
6. App Store + Google Play listing setup

## References
- [Tauri v2 Mobile Guide](https://tauri.app/distribute/)
- [tauri-plugin-camera](https://github.com/tauri-apps/plugins-workspace)
