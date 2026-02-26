# ADR-003: Decimal Arithmetic for Money

## Status
Accepted

## Context
Financial calculations require exact arithmetic. Floating-point (f32/f64) causes rounding errors.

## Decision
We will use **rust_decimal** for all monetary values:
- Stored as i64 (cents) in database
- `Decimal` in memory with 2 decimal places
- No f32 or f64 in the financial stack

## Consequences
- **Pros:**
  - Exact arithmetic (no floating-point errors)
  - Matches human expectations for currency
  - serde serialization support
  
- **Cons:**
  - Slightly more verbose than f64
  - Need explicit rounding for final amounts

## References
- [rust_decimal crate](https://docs.rs/rust_decimal)
