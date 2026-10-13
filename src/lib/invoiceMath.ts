import type { NewDiscount } from "./api";

/**
 * Invoice totals in integer cents, computed with the same rules as the backend
 * (which stays authoritative). BigInt keeps the intermediate products exact.
 */

export interface LineAmountInput {
  quantity_hundredths: number;
  unit_rate_cents: number;
  taxable: boolean;
}

export interface InvoiceTotalsPreview {
  /** Amount of each line, in input order. */
  line_cents: number[];
  subtotal_cents: number;
  discount_cents: number;
  /** Taxable lines minus their proportional share of the discount. */
  taxable_base_cents: number;
  tax_cents: number;
  total_cents: number;
}

/** n / d rounded to the nearest integer, halves away from zero (d > 0). */
function divRoundHalfAway(n: bigint, d: bigint): bigint {
  const negative = n < 0n;
  const abs = negative ? -n : n;
  const q = (abs * 2n + d) / (d * 2n);
  return negative ? -q : q;
}

/** round_half_away_from_zero(quantity_hundredths × unit_rate_cents / 100). */
export function lineAmountCents(quantityHundredths: number, unitRateCents: number): number {
  return Number(divRoundHalfAway(BigInt(quantityHundredths) * BigInt(unitRateCents), 100n));
}

export function computeInvoiceTotals(
  lines: LineAmountInput[],
  taxRatesBps: number[],
  discount: NewDiscount | null,
): InvoiceTotalsPreview {
  const lineCents = lines.map((l) =>
    divRoundHalfAway(BigInt(l.quantity_hundredths) * BigInt(l.unit_rate_cents), 100n),
  );
  const subtotal = lineCents.reduce((a, b) => a + b, 0n);
  const taxable = lineCents.reduce((a, b, i) => (lines[i].taxable ? a + b : a), 0n);

  // Discount, clamped to 0..subtotal.
  let rawDiscount = 0n;
  if (discount?.type === "percentage") {
    rawDiscount = divRoundHalfAway(subtotal * BigInt(discount.bps), 10_000n);
  } else if (discount?.type === "flat") {
    rawDiscount = BigInt(discount.cents);
  }
  const discountCents =
    rawDiscount < 0n || subtotal < 0n ? 0n : rawDiscount > subtotal ? subtotal : rawDiscount;

  // Taxable base = taxable subtotal minus its proportional share of the discount.
  const base =
    subtotal === 0n ? 0n : taxable - divRoundHalfAway(taxable * discountCents, subtotal);

  const tax = taxRatesBps.reduce(
    (sum, bps) => sum + divRoundHalfAway(base * BigInt(bps), 10_000n),
    0n,
  );

  return {
    line_cents: lineCents.map(Number),
    subtotal_cents: Number(subtotal),
    discount_cents: Number(discountCents),
    taxable_base_cents: Number(base),
    tax_cents: Number(tax),
    total_cents: Number(subtotal - discountCents + tax),
  };
}
