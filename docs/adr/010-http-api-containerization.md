# ADR-010: HTTP API Server with Axum and Docker Containerization

## Status
Accepted

## Context

Phase 6 adds a headless HTTP API so aequi can run as a service — in Docker containers, on servers, or as a backend for third-party integrations. The Tauri desktop app remains the primary interface, but the HTTP API enables:

1. CI/CD automation (import transactions, check balances)
2. Docker deployment for always-on receipt intake
3. Integration with external tools and AI agents

## Decision

### Axum HTTP server

The `aequi-server` crate provides a standalone Axum server binary. It reuses `aequi-storage` for all database operations — no duplicated logic.

Configuration via environment variables:
- `AEQUI_DB_PATH` — SQLite database path (required)
- `AEQUI_PORT` — listen port (default 8060)
- `AEQUI_API_KEY` — optional Bearer token for authentication

### API design

All endpoints under `/api/v1/` with domain-organized routes:
- Accounts, Transactions, Receipts, Tax, Invoices, Contacts, Rules, Reconciliation, Reports
- `GET /health` is unauthenticated for service discovery

### Authentication

Bearer token middleware checks `Authorization: Bearer <key>` header. The `/health` endpoint is exempt. If `AEQUI_API_KEY` is not set, authentication is disabled (development mode).

### Containerization

Multi-stage Dockerfile: Rust builder stage → minimal Debian runtime. The database is stored in a `/data` volume. Port 8060 is exposed.

## Consequences

- **Pros:**
  - Same storage layer as Tauri app — no logic duplication
  - Docker deployment is a single `docker run` command
  - API key auth is simple and sufficient for single-user scenarios
  - CORS enabled for cross-origin browser access

- **Cons:**
  - No OAuth/OIDC in v1 (single API key only)
  - No rate limiting in v1
  - SQLite limits concurrent write throughput (acceptable for single-user)

## References
- Axum web framework: https://github.com/tokio-rs/axum
- Docker multi-stage builds: https://docs.docker.com/build/building/multi-stage/
