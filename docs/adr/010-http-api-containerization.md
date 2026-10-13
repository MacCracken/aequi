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
- Accounts, Transactions, Receipts, Tax, Invoices, Contacts, Rules, Reconciliation, Reports, Imports
- Imported bank transactions (CSV/OFX imports and `POST /plaid/sync`) wait in a review queue and reach the ledger only through `POST /imports/{id}/post` (or are dismissed with `POST /imports/{id}/ignore`); `GET /imports/pending` lists the queue
- `GET /health` is unauthenticated for service discovery
- `GET /tax/returns/{year}` / `PUT /tax/returns/{year}` read and replace the figures from a filed return (`{"total_tax_cents": …, "agi_cents": …}`; an omitted field is cleared). The next year's quarterly estimate uses them for the prior-year safe harbor (100%, or 110% above the AGI threshold); `GET /tax/quarterly-estimate` reports `qbi_deduction_cents` and `safe_harbor_prior_year_percentage` (`"1.00"` or `"1.10"`)

### Integrations

- **Plaid** (`AEQUI_PLAID_CONFIG`): `POST /plaid/sync` (no request body) calls Plaid's cursor-based `/transactions/sync`, following `has_more` and restarting the loop from its starting cursor if Plaid reports `TRANSACTIONS_SYNC_MUTATION_DURING_PAGINATION`. The cursor is stored per Item (settings key `plaid_cursor_<item_id>`) in the same database transaction as the review-queue changes, so a failed sync saves nothing and the next one replays the same changes. Added transactions are queued (pending-at-the-bank, non-USD and zero ones are skipped); modified ones refresh queue entries still awaiting review; removed ones drop them. Entries already posted, ignored or matched are never changed: the response counts them as `transactions_modified_after_review` / `transactions_removed_after_review` so the ledger entry can be checked by hand.
- **Stripe** (`STRIPE_WEBHOOK_SECRET`): `POST /stripe/webhook` sits outside the auth middleware and verifies Stripe's signature instead. It books `charge.succeeded` (Undeposited Funds / Services Revenue), `charge.refunded` and `payout.paid`, once per event id. With `AEQUI_STRIPE_API_KEY` (a restricted key with read access to balance transactions only), the processing fee is read from the charge's balance transaction and booked to Bank Fees in the same entry. For asynchronous payment methods the balance transaction appears later; the charge is booked at gross and the fee is booked as a separate entry (memo `Stripe fee: <charge id>`, at most once) from `charge.updated`. A failed or non-USD fee lookup never blocks booking the charge; it is logged.

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
