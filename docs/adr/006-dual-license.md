# ADR-006: Dual License (MIT + AGPL-3.0)

## Status
Accepted

## Context
Need a license that:
- Encourages open contribution
- Prevents proprietary SaaS derivatives
- Allows core library reuse in other tools

## Decision
Dual licensing:
- **AGPL-3.0** for the full application (prevents closed-source forks)
- **MIT** for the `core` crate (allows embedding in other tools)

## Consequences
- **Pros:**
  - Community benefits from open source application
  - Core library can be reused in other projects
  - Clear distinction between app and library
  
- **Cons:**
  - More complex licensing for contributors
  - AGPL may deter some commercial users
