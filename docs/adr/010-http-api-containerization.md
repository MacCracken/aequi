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
- `AEQUI_API_KEY` — Bearer token for authentication (or `AEQUI_OIDC_CONFIG`; one is required unless bound to loopback)

### API design

All endpoints under `/api/v1/` with domain-organized routes:
- Accounts, Transactions, Receipts, Tax, Invoices, Contacts, Rules, Reconciliation, Reports
- `GET /health` is unauthenticated for service discovery

### Authentication

Bearer token middleware checks `Authorization: Bearer <key>` header (constant-time comparison), or validates an OIDC JWT when `AEQUI_OIDC_CONFIG` is set. The `/health` endpoint is exempt. The server refuses to start without either one unless it is bound to a loopback address (`AEQUI_BIND_ADDR`) or `AEQUI_INSECURE_NO_AUTH=1` is set explicitly; a config variable that is set but does not parse is a startup error rather than silently disabling auth.

### Containerization

Multi-stage Dockerfile: Rust builder stage → minimal Debian runtime. The database is stored in a `/data` volume. Port 8060 is exposed.

## Consequences

- **Pros:**
  - Same storage layer as Tauri app — no logic duplication
  - Docker deployment is a single `docker run` command
  - API key auth is simple and sufficient for single-user scenarios
  - CORS enabled for cross-origin browser access

- **Cons:**
  - OIDC authorization is limited to optional subject/email allowlists; without one, any account from the issuer has full access
  - No rate limiting in v1
  - SQLite limits concurrent write throughput (acceptable for single-user)

## References
- Axum web framework: https://github.com/tokio-rs/axum
- Docker multi-stage builds: https://docs.docker.com/build/building/multi-stage/
