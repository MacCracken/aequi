# ADR-009: Invoice Engine with Contact Model and Lifecycle State Machine

## Status
Accepted

## Context

Phase 5 adds invoicing for freelancers. Key design decisions:

1. **Contact model** — clients, vendors, and contractors need different treatment (1099 tracking for contractors)
2. **Invoice computation** — line items, discounts, tax lines, and totals must use the existing `Money` type
3. **Lifecycle management** — invoices move through states (Draft → Sent → Viewed → Paid) with validated transitions
4. **Payment tracking** — partial and full payments with 1099-NEC threshold monitoring

## Decision

### Contact model

`Contact` has a `ContactType` enum (Client, Vendor, Contractor). The `is_contractor` flag is auto-set when `ContactType::Contractor` is selected, enabling automatic 1099-NEC threshold tracking. Contacts live in `crates/core/src/invoice/contact.rs`.

### Invoice computation

`Invoice` contains `Vec<InvoiceLine>` with quantity × unit_rate computation, an optional `Discount` (Percentage or Flat), and `Vec<TaxLine>` with rates. Tax is applied only to lines marked `taxable`, with discounts applied proportionally to the taxable subtotal. All arithmetic uses `Money` — no floating point.

### Lifecycle state machine

`InvoiceStatus` enum with `can_transition_to()` and `transition()` methods. Valid transitions are explicitly enumerated:

- Draft → Sent, Void
- Sent → Viewed, PartiallyPaid, Paid, Void
- Viewed → PartiallyPaid, Paid, Void
- PartiallyPaid → PartiallyPaid, Paid, Void
- Paid, Void → terminal (no transitions)

Invalid transitions return `InvoiceError::InvalidTransition`.

### 1099-NEC tracking

`compute_ytd_payments()` aggregates payments to a contact within a calendar year. `check_1099_threshold()` returns true when YTD payments reach $600. Both are pure functions in `crates/core/src/invoice/compute.rs`.

## Consequences

- **Pros:**
  - Invoice computation is pure and testable without a database
  - State machine prevents invalid lifecycle transitions at the type level
  - 1099 threshold tracking is automatic for contractor contacts
  - Discount and tax computation handles both percentage and flat amounts

- **Cons:**
  - PDF generation uses plain text in v1 (Typst integration deferred)
  - No email delivery in v1 (SMTP/Resend deferred)
  - Only US 1099-NEC threshold; international contractor reporting deferred

## References
- IRS Form 1099-NEC instructions
- IRS $600 reporting threshold (26 USC § 6041A)
