# ADR-001: Use Tauri v2 as Desktop Shell

## Status
Accepted

## Context
Need a cross-platform desktop application framework that:
- Supports macOS, Windows, Linux
- Allows the React frontend to communicate with Rust backend
- Keeps binary size small (<50MB)

## Decision
We will use **Tauri v2** as the desktop shell.

## Consequences
- **Pros:**
  - Small binary size (~10MB vs 100MB+ for Electron)
  - Rust backend is the application; React is display layer only
  - Native webview on each platform
  - IPC via typed `invoke()` commands
  
- **Cons:**
  - Tesseract/OCR integration requires additional build complexity
  - Less mature ecosystem than Electron

## References
- [Tauri v2 Documentation](https://tauri.app)
