# ADR-004: Rust Backend is the Application

## Status
Accepted

## Context
Business logic must be enforceable at compile time and testable without UI.

## Decision
The Rust backend is the application. React is a display layer only:
- All business logic in Rust (core crate)
- React communicates via Tauri's `invoke()` commands
- No business logic in frontend
- MCP server shares same core library

## Consequences
- **Pros:**
  - Accounting correctness enforced by Rust type system
  - Same core used by CLI, tests, and MCP server
  - Business logic fully testable without UI
  
- **Cons:**
  - More initial setup required
  - All features need Rust implementation first

## References
- [Tauri invoke()](https://tauri.app/distribute/tauri-command)
