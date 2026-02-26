# ADR-001: Use Tauri v2 as Desktop and Mobile Shell

## Status
Accepted

## Context
Need a cross-platform application framework that:
- Supports macOS, Windows, Linux (desktop)
- Supports iOS and Android (mobile) — same Rust backend, no separate codebase
- Allows the React frontend to communicate with Rust backend via typed IPC
- Keeps binary size small (<50MB)

## Decision
We will use **Tauri v2** as the application shell for both desktop and mobile targets.

Tauri v2 introduced first-class mobile support (`cargo tauri ios` / `cargo tauri android`). The same Rust workspace and React frontend compiles to all five targets without maintaining a separate mobile codebase.

## Consequences
- **Pros:**
  - Small binary size (~10MB vs 100MB+ for Electron)
  - Rust backend is the application; React is display layer only
  - Native webview on each platform (WKWebView on iOS, WebView on Android)
  - IPC via typed `invoke()` commands — identical API on desktop and mobile
  - Single codebase for all platforms (desktop + iOS + Android)
  - Mobile camera and filesystem access via official Tauri plugins

- **Cons:**
  - Tesseract/OCR integration requires additional build complexity, especially on mobile
  - Mobile targets require Xcode (iOS) and Android SDK (Android) in CI
  - Less mature mobile ecosystem than React Native or Flutter

## Mobile Build Targets
- iOS: `aarch64-apple-ios` (device), `aarch64-apple-ios-sim` (simulator)
- Android: `aarch64-linux-android`, `armv7-linux-androideabi`

Added in Phase 3 via:
```bash
cargo tauri ios init
cargo tauri android init
```

## References
- [Tauri v2 Documentation](https://tauri.app)
- [Tauri Mobile Guide](https://tauri.app/distribute/)
