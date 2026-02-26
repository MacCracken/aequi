# ADR-002: Local-First SQLite Storage

## Status
Accepted

## Context
Need data storage that:
- Works offline without mandatory cloud
- Supports double-entry bookkeeping schema
- Provides ACID guarantees for financial data

## Decision
We will use **SQLite via sqlx** with:
- WAL mode enabled
- Foreign keys enforced
- Data stored in `~/.aequi/ledger.db`

## Consequences
- **Pros:**
  - Zero configuration, single file
  - ACID transactions for financial integrity
  - WAL mode provides crash safety
  - sqlx provides compile-time query verification
  
- **Cons:**
  - Single-user only (acceptable for v1)
  - Large files can have performance issues (mitigated with WAL + 32MB cache)

## References
- [SQLite WAL Mode](https://www.sqlite.org/wal.html)
- [sqlx Documentation](https://docs.rs/sqlx)
