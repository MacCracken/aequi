import { useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import {
  createInvoice,
  updateInvoice,
  errorMessage,
  type ContactRecord,
  type InvoiceDetail,
  type NewDiscount,
  type NewInvoice,
  type NewInvoiceLine,
  type NewInvoiceTaxLine,
} from "../lib/api";
import {
  centsToInputText,
  formatCents,
  formatHundredths,
  parseDollarsToCents,
  parseLocalDate,
  parsePercentToBps,
  parseQuantityToHundredths,
  todayLocal,
} from "../lib/format";
import { computeInvoiceTotals } from "../lib/invoiceMath";
import { useMountedRef } from "../lib/hooks";

/** 100% in basis points. */
const MAX_BPS = 10_000;
const DEFAULT_TAX_LABEL = "Sales tax";

interface LineDraft {
  key: number;
  description: string;
  /** Raw text as typed; parsed only for totals and submit. */
  quantityText: string;
  rateText: string;
  taxable: boolean;
}

interface TaxDraft {
  key: number;
  label: string;
  rateText: string;
}

type DiscountMode = "none" | "percentage" | "flat";

let nextDraftKey = 0;

function emptyLine(): LineDraft {
  return { key: ++nextDraftKey, description: "", quantityText: "1", rateText: "", taxable: false };
}

function initialLines(detail: InvoiceDetail | null): LineDraft[] {
  if (!detail || detail.lines.length === 0) return [emptyLine()];
  return [...detail.lines]
    .sort((a, b) => a.sort_order - b.sort_order)
    .map((l) => ({
      key: ++nextDraftKey,
      description: l.description,
      quantityText: formatHundredths(l.quantity_hundredths),
      rateText: centsToInputText(l.unit_rate_cents),
      taxable: l.taxable,
    }));
}

function initialTaxLines(detail: InvoiceDetail | null): TaxDraft[] {
  if (!detail || detail.tax_lines.length === 0) {
    return [{ key: ++nextDraftKey, label: DEFAULT_TAX_LABEL, rateText: "" }];
  }
  return detail.tax_lines.map((t) => ({
    key: ++nextDraftKey,
    label: t.label,
    rateText: formatHundredths(t.rate_bps),
  }));
}

function initialDiscount(detail: InvoiceDetail | null): { mode: DiscountMode; text: string } {
  if (detail?.discount_value != null) {
    if (detail.discount_type === "Percentage") {
      return { mode: "percentage", text: formatHundredths(detail.discount_value) };
    }
    if (detail.discount_type === "Flat") {
      return { mode: "flat", text: centsToInputText(detail.discount_value) };
    }
  }
  return { mode: "none", text: "" };
}

const inputClass =
  "w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary";

function numberInputClass(invalid: boolean, width: string): string {
  return `${width} px-2 py-1.5 text-sm text-right font-mono border rounded-md bg-bg focus:outline-none ${
    invalid ? "border-danger" : "border-border focus:border-primary"
  }`;
}

/**
 * Create a new invoice, or (with `initial`) edit a Draft. Amounts are kept as
 * raw text and parsed to integer cents / hundredths / basis points.
 */
export function InvoiceForm({
  contacts,
  initial = null,
  onSaved,
  onCancel,
  onError,
}: {
  contacts: ContactRecord[];
  initial?: InvoiceDetail | null;
  onSaved: (invoice: InvoiceDetail) => void;
  onCancel: () => void;
  onError: (msg: string) => void;
}) {
  const editing = initial !== null;
  const [invoiceNumber, setInvoiceNumber] = useState(initial?.invoice_number ?? "");
  const [contactId, setContactId] = useState<number | "">(initial?.contact_id ?? "");
  const [issueDate, setIssueDate] = useState(() => initial?.issue_date ?? todayLocal());
  const [dueDate, setDueDate] = useState(initial?.due_date ?? "");
  const [notes, setNotes] = useState(initial?.notes ?? "");
  const [terms, setTerms] = useState(initial?.terms ?? "");
  const [lines, setLines] = useState<LineDraft[]>(() => initialLines(initial));
  const [taxLines, setTaxLines] = useState<TaxDraft[]>(() => initialTaxLines(initial));
  const [discountMode, setDiscountMode] = useState<DiscountMode>(() => initialDiscount(initial).mode);
  const [discountText, setDiscountText] = useState(() => initialDiscount(initial).text);
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);
  const mountedRef = useMountedRef();

  const parsedLines = useMemo(
    () =>
      lines.map((l) => ({
        qty: parseQuantityToHundredths(l.quantityText),
        rate: parseDollarsToCents(l.rateText),
      })),
    [lines],
  );
  const parsedTax = useMemo(() => taxLines.map((t) => parsePercentToBps(t.rateText)), [taxLines]);
  const discountValue =
    discountMode === "percentage"
      ? parsePercentToBps(discountText)
      : discountMode === "flat"
        ? parseDollarsToCents(discountText)
        : 0;

  const lineInvalid = parsedLines.map((p) => ({ qty: p.qty === null, rate: p.rate === null }));
  const taxInvalid = parsedTax.map((bps) => bps === null || bps > MAX_BPS);
  const discountInvalid =
    discountValue === null || (discountMode === "percentage" && discountValue > MAX_BPS);
  const hasInvalidAmount =
    lineInvalid.some((l) => l.qty || l.rate) || taxInvalid.some(Boolean) || discountInvalid;

  /** Tax lines that apply (valid, non-zero rate), for preview and submit. */
  const activeTax: NewInvoiceTaxLine[] = taxLines.flatMap((t, i) => {
    const bps = parsedTax[i];
    if (bps === null || bps === 0 || bps > MAX_BPS) return [];
    return [{ label: t.label.trim() || DEFAULT_TAX_LABEL, rate_bps: bps }];
  });

  let discount: NewDiscount | null = null;
  if (!discountInvalid && discountValue !== null && discountValue > 0) {
    discount =
      discountMode === "percentage"
        ? { type: "percentage", bps: discountValue }
        : discountMode === "flat"
          ? { type: "flat", cents: discountValue }
          : null;
  }

  const preview = computeInvoiceTotals(
    lines.map((l, i) => ({
      quantity_hundredths: parsedLines[i].qty ?? 0,
      unit_rate_cents: parsedLines[i].rate ?? 0,
      taxable: l.taxable,
    })),
    activeTax.map((t) => t.rate_bps),
    discount,
  );

  function updateLine(idx: number, patch: Partial<Omit<LineDraft, "key">>) {
    setLines((prev) => prev.map((l, i) => (i === idx ? { ...l, ...patch } : l)));
  }

  function addLine() {
    setLines((prev) => [...prev, emptyLine()]);
  }

  function removeLine(idx: number) {
    setLines((prev) => (prev.length <= 1 ? prev : prev.filter((_, i) => i !== idx)));
  }

  function updateTax(idx: number, patch: Partial<Omit<TaxDraft, "key">>) {
    setTaxLines((prev) => prev.map((t, i) => (i === idx ? { ...t, ...patch } : t)));
  }

  function removeTax(idx: number) {
    setTaxLines((prev) => (prev.length <= 1 ? prev : prev.filter((_, i) => i !== idx)));
  }

  /** The request payload, or a message telling the user what to fix. */
  function buildInput(): { ok: true; input: NewInvoice } | { ok: false; error: string } {
    const fail = (error: string) => ({ ok: false as const, error });
    if (!invoiceNumber.trim()) return fail("Invoice number is required");
    if (contactId === "") return fail("Please select a contact");
    if (!parseLocalDate(issueDate)) return fail("Issue date is required");
    if (!parseLocalDate(dueDate)) return fail("Due date is required");
    if (dueDate < issueDate) return fail("Due date must be on or after issue date");
    if (lines.length === 0) return fail("Add at least one line item");

    const outLines: NewInvoiceLine[] = [];
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      const { qty, rate } = parsedLines[i];
      const n = i + 1;
      if (!line.description.trim()) return fail(`Line ${n}: enter a description`);
      if (qty === null) {
        return fail(`Line ${n}: enter a quantity like 1.5 (at most two decimal places)`);
      }
      if (qty <= 0) return fail(`Line ${n}: quantity must be greater than 0`);
      if (!line.rateText.trim()) return fail(`Line ${n}: enter a rate`);
      if (rate === null) {
        return fail(`Line ${n}: enter a rate like 150.00 (at most two decimal places)`);
      }
      outLines.push({
        description: line.description.trim(),
        quantity_hundredths: qty,
        unit_rate_cents: rate,
        taxable: line.taxable,
      });
    }

    for (const bps of parsedTax) {
      if (bps === null) return fail("Sales tax: enter a rate like 8.25 (at most two decimal places)");
      if (bps > MAX_BPS) return fail("Sales tax rate must be between 0% and 100%");
    }
    if (activeTax.length > 0 && !lines.some((l) => l.taxable)) {
      return fail("Sales tax only applies to taxable lines: tick Taxable on a line or clear the rate");
    }

    if (discountMode === "percentage") {
      if (discountValue === null) {
        return fail("Discount: enter a percentage like 10 (at most two decimal places)");
      }
      if (discountValue > MAX_BPS) return fail("Discount percentage must be between 0% and 100%");
    } else if (discountMode === "flat") {
      if (discountValue === null) {
        return fail("Discount: enter an amount like 50.00 (at most two decimal places)");
      }
      if (discountValue > preview.subtotal_cents) {
        return fail(
          `Discount (${formatCents(discountValue)}) can't exceed the subtotal (${formatCents(preview.subtotal_cents)})`,
        );
      }
    }

    return {
      ok: true,
      input: {
        invoice_number: invoiceNumber.trim(),
        contact_id: contactId,
        issue_date: issueDate,
        due_date: dueDate,
        lines: outLines,
        tax_lines: activeTax,
        discount,
        notes: notes.trim() || undefined,
        terms: terms.trim() || undefined,
      },
    };
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (submittingRef.current) return;
    const built = buildInput();
    if (!built.ok) {
      onError(built.error);
      return;
    }

    submittingRef.current = true;
    setSubmitting(true);
    try {
      const saved = initial
        ? await updateInvoice(initial.id, built.input)
        : await createInvoice(built.input);
      if (mountedRef.current) onSaved(saved);
    } catch (err) {
      if (mountedRef.current) onError(errorMessage(err));
    } finally {
      submittingRef.current = false;
      if (mountedRef.current) setSubmitting(false);
    }
  }

  const taxLabel = activeTax
    .map((t) => `${t.label} ${formatHundredths(t.rate_bps)}%`)
    .join(", ");

  return (
    <form
      onSubmit={handleSubmit}
      className="bg-surface rounded-lg border border-border p-4 space-y-4 mb-4"
      aria-label={editing ? `Edit invoice ${initial.invoice_number}` : "New invoice"}
    >
      {editing && (
        <h3 className="font-medium">Edit invoice {initial.invoice_number}</h3>
      )}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <div>
          <label htmlFor="inv-invoiceNumber" className="block text-xs text-text-muted mb-1">Invoice Number *</label>
          <input
            id="inv-invoiceNumber"
            type="text"
            value={invoiceNumber}
            onChange={(e) => setInvoiceNumber(e.target.value)}
            required
            placeholder="e.g. INV-001"
            className={inputClass}
          />
        </div>
        <div>
          <label htmlFor="inv-contactId" className="block text-xs text-text-muted mb-1">Contact *</label>
          <select
            id="inv-contactId"
            value={contactId}
            onChange={(e) => setContactId(e.target.value ? Number(e.target.value) : "")}
            required
            className={inputClass}
          >
            <option value="">Select contact</option>
            {contacts.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
          {contacts.length === 0 && (
            <p className="text-xs text-text-muted mt-1">
              No contacts yet.{" "}
              <Link to="/contacts" className="text-primary hover:underline">
                Add a client first
              </Link>
              .
            </p>
          )}
        </div>
        <div>
          <label htmlFor="inv-issueDate" className="block text-xs text-text-muted mb-1">Issue Date *</label>
          <input
            id="inv-issueDate"
            type="date"
            value={issueDate}
            onChange={(e) => setIssueDate(e.target.value)}
            required
            className={inputClass}
          />
        </div>
        <div>
          <label htmlFor="inv-dueDate" className="block text-xs text-text-muted mb-1">Due Date *</label>
          <input
            id="inv-dueDate"
            type="date"
            value={dueDate}
            onChange={(e) => setDueDate(e.target.value)}
            required
            min={issueDate}
            className={inputClass}
          />
        </div>
      </div>

      {/* Line items */}
      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-xs text-text-muted font-medium">Line items *</span>
          <button type="button" onClick={addLine} className="text-xs text-primary hover:underline">
            + Add line
          </button>
        </div>
        <div className="hidden md:flex gap-2 text-xs text-text-muted" aria-hidden="true">
          <span className="flex-1">Description</span>
          <span className="w-20 text-right">Qty</span>
          <span className="w-28 text-right">Rate</span>
          <span className="w-16 text-center">Taxable</span>
          <span className="w-28 text-right">Amount</span>
          <span className="w-5" />
        </div>
        {lines.map((line, idx) => {
          const n = idx + 1;
          const invalid = lineInvalid[idx];
          return (
            <div
              key={line.key}
              className="flex flex-wrap md:flex-nowrap gap-2 items-center pb-2 border-b border-border/50 md:pb-0 md:border-0"
            >
              <label htmlFor={`inv-line-${line.key}-desc`} className="sr-only">Line {n} description</label>
              <input
                id={`inv-line-${line.key}-desc`}
                type="text"
                value={line.description}
                onChange={(e) => updateLine(idx, { description: e.target.value })}
                placeholder="Description"
                className="w-full md:w-auto md:flex-1 min-w-0 px-2 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
              />
              <label htmlFor={`inv-line-${line.key}-qty`} className="sr-only">Line {n} quantity</label>
              <input
                id={`inv-line-${line.key}-qty`}
                type="text"
                inputMode="decimal"
                autoComplete="off"
                placeholder="Qty"
                value={line.quantityText}
                onChange={(e) => updateLine(idx, { quantityText: e.target.value })}
                aria-invalid={invalid.qty || undefined}
                className={numberInputClass(invalid.qty, "w-20")}
              />
              <label htmlFor={`inv-line-${line.key}-rate`} className="sr-only">Line {n} rate</label>
              <input
                id={`inv-line-${line.key}-rate`}
                type="text"
                inputMode="decimal"
                autoComplete="off"
                placeholder="Rate"
                value={line.rateText}
                onChange={(e) => updateLine(idx, { rateText: e.target.value })}
                aria-invalid={invalid.rate || undefined}
                className={numberInputClass(invalid.rate, "w-28")}
              />
              <label className="flex items-center gap-1 text-xs text-text-muted md:w-16 md:justify-center">
                <input
                  type="checkbox"
                  checked={line.taxable}
                  onChange={(e) => updateLine(idx, { taxable: e.target.checked })}
                  aria-label={`Line ${n} taxable`}
                />
                <span className="md:sr-only">Taxable</span>
              </label>
              <span className="ml-auto md:ml-0 w-28 text-right text-sm font-mono">
                {invalid.qty || invalid.rate ? "—" : formatCents(preview.line_cents[idx])}
              </span>
              {lines.length > 1 ? (
                <button
                  type="button"
                  onClick={() => removeLine(idx)}
                  aria-label={`Remove line ${n}`}
                  className="w-5 text-danger text-sm hover:underline"
                >
                  X
                </button>
              ) : (
                <span className="hidden md:block w-5" />
              )}
            </div>
          );
        })}
      </div>

      {/* Tax, discount, and live totals */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <div className="space-y-3">
          {taxLines.map((tax, idx) => {
            const invalid = taxInvalid[idx];
            return (
              <div key={tax.key} className="flex gap-2 items-end">
                <div className="flex-1 min-w-0">
                  <label htmlFor={`inv-tax-${tax.key}-label`} className="block text-xs text-text-muted mb-1">
                    Tax label
                  </label>
                  <input
                    id={`inv-tax-${tax.key}-label`}
                    type="text"
                    value={tax.label}
                    onChange={(e) => updateTax(idx, { label: e.target.value })}
                    placeholder={DEFAULT_TAX_LABEL}
                    className={inputClass}
                  />
                </div>
                <div>
                  <label htmlFor={`inv-tax-${tax.key}-rate`} className="block text-xs text-text-muted mb-1">
                    Rate %
                  </label>
                  <input
                    id={`inv-tax-${tax.key}-rate`}
                    type="text"
                    inputMode="decimal"
                    autoComplete="off"
                    placeholder="e.g. 8.25"
                    value={tax.rateText}
                    onChange={(e) => updateTax(idx, { rateText: e.target.value })}
                    aria-invalid={invalid || undefined}
                    className={numberInputClass(invalid, "w-24")}
                  />
                </div>
                {taxLines.length > 1 && (
                  <button
                    type="button"
                    onClick={() => removeTax(idx)}
                    aria-label={`Remove ${tax.label || "tax"}`}
                    className="pb-2 text-danger text-sm hover:underline"
                  >
                    X
                  </button>
                )}
              </div>
            );
          })}
          <p className="text-xs text-text-muted">
            Applied to lines marked taxable. Leave the rate blank for no sales tax.
          </p>
          <div>
            <label htmlFor="inv-discountMode" className="block text-xs text-text-muted mb-1">Discount</label>
            <div className="flex gap-2 items-center">
              <select
                id="inv-discountMode"
                value={discountMode}
                onChange={(e) => setDiscountMode(e.target.value as DiscountMode)}
                className="px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
              >
                <option value="none">None</option>
                <option value="percentage">Percent of subtotal</option>
                <option value="flat">Flat amount</option>
              </select>
              {discountMode !== "none" && (
                <>
                  <label htmlFor="inv-discountValue" className="sr-only">
                    {discountMode === "percentage" ? "Discount percent" : "Discount amount"}
                  </label>
                  <input
                    id="inv-discountValue"
                    type="text"
                    inputMode="decimal"
                    autoComplete="off"
                    placeholder={discountMode === "percentage" ? "10" : "50.00"}
                    value={discountText}
                    onChange={(e) => setDiscountText(e.target.value)}
                    aria-invalid={discountInvalid || undefined}
                    className={numberInputClass(discountInvalid, "w-28")}
                  />
                  {discountMode === "percentage" && <span className="text-sm text-text-muted">%</span>}
                </>
              )}
            </div>
          </div>
        </div>

        <dl
          className="bg-bg rounded-md border border-border p-3 text-sm space-y-1 self-start"
          aria-live="polite"
          aria-label="Invoice totals"
        >
          <TotalRow label="Subtotal" value={formatCents(preview.subtotal_cents)} />
          {preview.discount_cents > 0 && (
            <TotalRow
              label={
                discount?.type === "percentage"
                  ? `Discount (${formatHundredths(discount.bps)}%)`
                  : "Discount"
              }
              value={`−${formatCents(preview.discount_cents)}`}
            />
          )}
          {activeTax.length > 0 && (
            <TotalRow label={taxLabel} value={formatCents(preview.tax_cents)} />
          )}
          <div className="border-t border-border pt-1">
            <TotalRow label="Total" value={formatCents(preview.total_cents)} strong />
          </div>
          {hasInvalidAmount && <p className="text-xs text-danger">Invalid amount</p>}
        </dl>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <div>
          <label htmlFor="inv-terms" className="block text-xs text-text-muted mb-1">Terms</label>
          <input
            id="inv-terms"
            type="text"
            value={terms}
            onChange={(e) => setTerms(e.target.value)}
            placeholder="e.g. Net 30"
            className={inputClass}
          />
        </div>
        <div>
          <label htmlFor="inv-notes" className="block text-xs text-text-muted mb-1">Notes</label>
          <input
            id="inv-notes"
            type="text"
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            className={inputClass}
          />
        </div>
      </div>

      <div className="flex gap-2">
        <button
          type="submit"
          disabled={submitting}
          className="px-4 py-2 text-sm font-medium bg-primary text-white rounded-md hover:bg-primary-hover transition-colors disabled:opacity-50"
        >
          {editing
            ? submitting ? "Saving..." : "Save Changes"
            : submitting ? "Creating..." : "Create Invoice"}
        </button>
        <button
          type="button"
          onClick={onCancel}
          disabled={submitting}
          className="px-4 py-2 text-sm font-medium border border-border rounded-md hover:bg-border/50 transition-colors disabled:opacity-50"
        >
          Cancel
        </button>
      </div>
    </form>
  );
}

function TotalRow({ label, value, strong = false }: { label: string; value: string; strong?: boolean }) {
  return (
    <div className={`flex justify-between gap-4 ${strong ? "font-semibold" : ""}`}>
      <dt className={strong ? "" : "text-text-muted"}>{label}</dt>
      <dd className="font-mono">{value}</dd>
    </div>
  );
}
