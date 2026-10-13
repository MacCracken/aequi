import { useCallback, useEffect, useRef, useState } from "react";
import {
  getInvoice,
  recordInvoicePayment,
  sendInvoice,
  voidInvoice,
  errorMessage,
  type Account,
  type InvoiceDetail,
} from "../lib/api";
import {
  centsToInputText,
  daysUntil,
  formatCents,
  formatDate,
  formatHundredths,
  parseDollarsToCents,
  parseLocalDate,
  todayLocal,
} from "../lib/format";
import { lineAmountCents } from "../lib/invoiceMath";
import { useMountedRef } from "../lib/hooks";
import { useToast } from "./Toast";
import { InvoiceStatusBadge } from "./InvoiceStatusBadge";

const PAYABLE_STATUSES = ["Sent", "Viewed", "PartiallyPaid"];
const VOIDABLE_STATUSES = ["Draft", "Sent", "Viewed"];
const SENDABLE_STATUSES = ["Draft", "Sent", "Viewed", "PartiallyPaid"];
const DEFAULT_DEPOSIT_ACCOUNT = "1000";
const PAYMENT_METHODS = ["Bank transfer", "Check", "Cash", "Card", "Payment app", "Other"];

const inputClass =
  "w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary";
const secondaryButton =
  "px-3 py-1.5 text-sm font-medium border border-border rounded-md hover:bg-border/50 transition-colors disabled:opacity-50";
const primaryButton =
  "px-3 py-1.5 text-sm font-medium bg-primary text-white rounded-md hover:bg-primary-hover transition-colors disabled:opacity-50";

/** The reason stored in a Void invoice's status data, if any. */
function voidReason(statusData: string | null): string | null {
  if (!statusData) return null;
  try {
    const data: unknown = JSON.parse(statusData);
    if (typeof data === "object" && data !== null && "reason" in data) {
      const { reason } = data as { reason: unknown };
      if (typeof reason === "string" && reason.trim()) return reason;
    }
  } catch {
    // Older rows may hold a bare string; nothing to show.
  }
  return null;
}

/**
 * One invoice with its line items, totals and payments, plus the actions its
 * status allows (edit, send, record payment, void).
 */
export function InvoiceDetailPanel({
  invoiceId,
  contactName,
  accounts,
  onClose,
  onEdit,
  onChanged,
}: {
  invoiceId: number;
  contactName: (contactId: number) => string;
  accounts: Account[];
  onClose: () => void;
  onEdit: (invoice: InvoiceDetail) => void;
  /** Called after the invoice changed so the list can refresh. */
  onChanged: () => void;
}) {
  const [detail, setDetail] = useState<InvoiceDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [action, setAction] = useState<"payment" | "void" | null>(null);
  const [sending, setSending] = useState(false);
  const sendingRef = useRef(false);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const mountedRef = useMountedRef();
  const { toast } = useToast();
  // Only the latest request may update state; bumped on unmount.
  const requestIdRef = useRef(0);

  const load = useCallback(() => {
    const id = ++requestIdRef.current;
    setLoading(true);
    getInvoice(invoiceId)
      .then((d) => {
        if (id !== requestIdRef.current) return;
        setDetail(d);
        setLoadError(null);
      })
      .catch((e) => {
        if (id === requestIdRef.current) setLoadError(errorMessage(e));
      })
      .finally(() => {
        if (id === requestIdRef.current) setLoading(false);
      });
  }, [invoiceId]);

  useEffect(() => {
    load();
    const ref = requestIdRef;
    return () => { ref.current++; };
  }, [load]);

  // Bring the panel into view (it opens above a possibly long list).
  const loaded = detail !== null;
  useEffect(() => {
    if (loaded) headingRef.current?.focus();
  }, [loaded]);

  function afterChange() {
    setAction(null);
    load();
    onChanged();
  }

  async function handleSend() {
    if (!detail || sendingRef.current) return;
    sendingRef.current = true;
    setSending(true);
    try {
      const result = await sendInvoice({ invoice_id: detail.id });
      if (!mountedRef.current) return;
      toast("success", `Invoice ${result.invoice_number} emailed to ${result.recipient}`);
      afterChange();
    } catch (e) {
      if (mountedRef.current) toast("error", errorMessage(e));
    } finally {
      sendingRef.current = false;
      if (mountedRef.current) setSending(false);
    }
  }

  const header = (title: string) => (
    <div className="flex items-start justify-between gap-2">
      <h3 ref={headingRef} tabIndex={-1} className="text-lg font-semibold focus:outline-none">
        {title}
      </h3>
      <button
        type="button"
        onClick={onClose}
        aria-label="Close invoice details"
        className="px-2 text-text-muted hover:text-text text-lg leading-none"
      >
        ×
      </button>
    </div>
  );

  if (!detail) {
    return (
      <section className="bg-surface rounded-lg border border-border p-4 mb-4 space-y-2">
        {header("Invoice")}
        {loading ? (
          <p className="text-sm text-text-muted">Loading invoice...</p>
        ) : (
          <div className="space-y-2">
            <p className="text-sm text-danger">{loadError ?? "Invoice not found"}</p>
            <button type="button" onClick={load} className={secondaryButton}>
              Retry
            </button>
          </div>
        )}
      </section>
    );
  }

  const status = detail.status_type;
  const hasLines = detail.lines.length > 0;
  const canEdit = status === "Draft";
  const canSend = hasLines && SENDABLE_STATUSES.includes(status);
  const canPay = hasLines && PAYABLE_STATUSES.includes(status) && detail.balance_cents > 0;
  const canVoid = VOIDABLE_STATUSES.includes(status);
  const dueIn = daysUntil(detail.due_date);
  const overdue =
    status !== "Void" && status !== "Draft" && detail.balance_cents > 0 && dueIn !== null && dueIn < 0;
  const reason = status === "Void" ? voidReason(detail.status_data) : null;
  const taxLabel =
    detail.tax_lines.map((t) => `${t.label} ${formatHundredths(t.rate_bps)}%`).join(", ") || "Tax";
  const discountLabel =
    detail.discount_type === "Percentage" && detail.discount_value != null
      ? `Discount (${formatHundredths(detail.discount_value)}%)`
      : "Discount";

  return (
    <section
      className="bg-surface rounded-lg border border-border p-4 mb-4 space-y-4"
      aria-label={`Invoice ${detail.invoice_number}`}
    >
      {header(`Invoice ${detail.invoice_number}`)}

      <dl className="grid grid-cols-2 md:grid-cols-4 gap-3 text-sm">
        <Field label="Status">
          <InvoiceStatusBadge status={status} />
        </Field>
        <Field label="Contact">{contactName(detail.contact_id)}</Field>
        <Field label="Issued">{formatDate(detail.issue_date)}</Field>
        <Field label="Due">
          {formatDate(detail.due_date)}
          {overdue && dueIn !== null && (
            <span className="block text-xs text-danger font-medium">
              {-dueIn} {dueIn === -1 ? "day" : "days"} overdue
            </span>
          )}
        </Field>
        {detail.terms && <Field label="Terms">{detail.terms}</Field>}
        {detail.notes && (
          <div className="col-span-2 md:col-span-3">
            <Field label="Notes">{detail.notes}</Field>
          </div>
        )}
      </dl>

      {reason && (
        <p className="text-sm text-text-muted">
          <span className="font-medium text-danger">Voided:</span> {reason}
        </p>
      )}

      {!hasLines && status !== "Void" && (
        <p className="text-sm bg-warning/10 border border-warning/30 rounded-md px-3 py-2">
          This invoice has no line items, so its total is $0.00.{" "}
          {canEdit
            ? "Edit it to add line items before sending or recording a payment."
            : "Void it and create a new invoice with line items."}
        </p>
      )}

      {/* Line items */}
      {hasLines && (
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-text-muted border-b border-border">
                <th className="py-2 pr-4 font-medium">Description</th>
                <th className="py-2 pr-4 font-medium text-right hidden sm:table-cell">Qty</th>
                <th className="py-2 pr-4 font-medium text-right hidden sm:table-cell">Rate</th>
                <th className="py-2 font-medium text-right">Amount</th>
              </tr>
            </thead>
            <tbody>
              {detail.lines.map((l) => (
                <tr key={l.id} className="border-b border-border/50">
                  <td className="py-2 pr-4">
                    {l.description}
                    {l.taxable && (
                      <span className="ml-2 text-xs text-text-muted" title="Sales tax applies">
                        taxable
                      </span>
                    )}
                    <span className="block text-xs text-text-muted font-mono sm:hidden">
                      {formatHundredths(l.quantity_hundredths)} × {formatCents(l.unit_rate_cents)}
                    </span>
                  </td>
                  <td className="py-2 pr-4 text-right font-mono hidden sm:table-cell">{formatHundredths(l.quantity_hundredths)}</td>
                  <td className="py-2 pr-4 text-right font-mono whitespace-nowrap hidden sm:table-cell">{formatCents(l.unit_rate_cents)}</td>
                  <td className="py-2 text-right font-mono whitespace-nowrap">
                    {formatCents(lineAmountCents(l.quantity_hundredths, l.unit_rate_cents))}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {/* Payments */}
        <div>
          <h4 className="text-sm font-medium text-text-muted mb-2">Payments</h4>
          {detail.payments.length === 0 ? (
            <p className="text-sm text-text-muted">No payments recorded.</p>
          ) : (
            <ul className="text-sm divide-y divide-border/50 border border-border rounded-md">
              {detail.payments.map((p) => (
                <li key={p.id} className="flex justify-between gap-3 px-3 py-2">
                  <span>
                    {formatDate(p.date)}
                    {p.method && <span className="text-text-muted"> · {p.method}</span>}
                  </span>
                  <span className="font-mono">{formatCents(p.amount_cents)}</span>
                </li>
              ))}
            </ul>
          )}
        </div>

        {/* Totals */}
        <dl className="bg-bg rounded-md border border-border p-3 text-sm space-y-1 self-start">
          <TotalRow label="Subtotal" value={formatCents(detail.subtotal_cents)} />
          {detail.discount_cents > 0 && (
            <TotalRow label={discountLabel} value={`−${formatCents(detail.discount_cents)}`} />
          )}
          {(detail.tax_cents > 0 || detail.tax_lines.length > 0) && (
            <TotalRow label={taxLabel} value={formatCents(detail.tax_cents)} />
          )}
          <div className="border-t border-border pt-1">
            <TotalRow label="Total" value={formatCents(detail.total_cents)} strong />
          </div>
          <TotalRow label="Paid" value={formatCents(detail.paid_cents)} />
          {status !== "Void" && (
            <TotalRow label="Balance due" value={formatCents(detail.balance_cents)} strong />
          )}
        </dl>
      </div>

      {/* Actions */}
      {(canEdit || canSend || canPay || canVoid) && action === null && (
        <div className="flex flex-wrap gap-2">
          {canEdit && (
            <button type="button" onClick={() => onEdit(detail)} className={secondaryButton}>
              Edit
            </button>
          )}
          {canSend && (
            <button type="button" onClick={handleSend} disabled={sending} className={secondaryButton}>
              {sending ? "Sending..." : status === "Draft" ? "Send" : "Resend"}
            </button>
          )}
          {canPay && (
            <button type="button" onClick={() => setAction("payment")} className={primaryButton}>
              Record payment
            </button>
          )}
          {canVoid && (
            <button
              type="button"
              onClick={() => setAction("void")}
              className="px-3 py-1.5 text-sm font-medium border border-danger/40 text-danger rounded-md hover:bg-danger/10 transition-colors"
            >
              Void
            </button>
          )}
        </div>
      )}

      {action === "payment" && (
        <PaymentForm
          invoice={detail}
          accounts={accounts}
          onDone={() => {
            toast("success", "Payment recorded");
            afterChange();
          }}
          onCancel={() => setAction(null)}
          onError={(msg) => toast("error", msg)}
        />
      )}

      {action === "void" && (
        <VoidForm
          invoice={detail}
          onDone={() => {
            toast("success", `Invoice ${detail.invoice_number} voided`);
            afterChange();
          }}
          onCancel={() => setAction(null)}
          onError={(msg) => toast("error", msg)}
        />
      )}
    </section>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <dt className="text-xs text-text-muted">{label}</dt>
      <dd className="mt-0.5">{children}</dd>
    </div>
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

function PaymentForm({
  invoice,
  accounts,
  onDone,
  onCancel,
  onError,
}: {
  invoice: InvoiceDetail;
  accounts: Account[];
  onDone: () => void;
  onCancel: () => void;
  onError: (msg: string) => void;
}) {
  const assetAccounts = accounts.filter((a) => a.account_type === "Asset");
  const [amountText, setAmountText] = useState(() => centsToInputText(invoice.balance_cents));
  const [date, setDate] = useState(todayLocal);
  const [method, setMethod] = useState("");
  const [depositAccount, setDepositAccount] = useState(() =>
    assetAccounts.some((a) => a.code === DEFAULT_DEPOSIT_ACCOUNT)
      ? DEFAULT_DEPOSIT_ACCOUNT
      : (assetAccounts[0]?.code ?? ""),
  );
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);
  const mountedRef = useMountedRef();

  const amount = parseDollarsToCents(amountText);
  const amountInvalid = amount === null;

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (submittingRef.current) return;
    if (amount === null) {
      onError("Enter an amount like 1234.56 (at most two decimal places)");
      return;
    }
    if (amount <= 0) {
      onError("Payment amount must be greater than 0");
      return;
    }
    if (amount > invoice.balance_cents) {
      onError(`Payment can't exceed the balance due (${formatCents(invoice.balance_cents)})`);
      return;
    }
    if (!parseLocalDate(date)) {
      onError("Payment date is required");
      return;
    }
    if (!depositAccount) {
      onError("Select the account the payment was deposited to");
      return;
    }

    submittingRef.current = true;
    setSubmitting(true);
    try {
      await recordInvoicePayment({
        invoice_id: invoice.id,
        amount_cents: amount,
        date,
        method: method || undefined,
        deposit_account_code: depositAccount,
      });
      if (mountedRef.current) onDone();
    } catch (err) {
      if (mountedRef.current) onError(errorMessage(err));
    } finally {
      submittingRef.current = false;
      if (mountedRef.current) setSubmitting(false);
    }
  }

  return (
    <form
      onSubmit={handleSubmit}
      className="border border-border rounded-md p-3 space-y-3"
      aria-label="Record payment"
    >
      <h4 className="text-sm font-medium">Record payment</h4>
      <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-4 gap-3">
        <div>
          <label htmlFor="pay-amount" className="block text-xs text-text-muted mb-1">Amount *</label>
          <input
            id="pay-amount"
            type="text"
            inputMode="decimal"
            autoComplete="off"
            value={amountText}
            onChange={(e) => setAmountText(e.target.value)}
            aria-invalid={amountInvalid || undefined}
            className={`w-full px-3 py-1.5 text-sm text-right font-mono border rounded-md bg-bg focus:outline-none ${
              amountInvalid ? "border-danger" : "border-border focus:border-primary"
            }`}
          />
          <p className="text-xs text-text-muted mt-1">
            Balance due {formatCents(invoice.balance_cents)}
          </p>
        </div>
        <div>
          <label htmlFor="pay-date" className="block text-xs text-text-muted mb-1">Date *</label>
          <input
            id="pay-date"
            type="date"
            value={date}
            onChange={(e) => setDate(e.target.value)}
            required
            className={inputClass}
          />
        </div>
        <div>
          <label htmlFor="pay-method" className="block text-xs text-text-muted mb-1">Method</label>
          <select
            id="pay-method"
            value={method}
            onChange={(e) => setMethod(e.target.value)}
            className={inputClass}
          >
            <option value="">Not specified</option>
            {PAYMENT_METHODS.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label htmlFor="pay-account" className="block text-xs text-text-muted mb-1">Deposited to *</label>
          <select
            id="pay-account"
            value={depositAccount}
            onChange={(e) => setDepositAccount(e.target.value)}
            required
            className={inputClass}
          >
            {assetAccounts.length === 0 && <option value="">No asset accounts</option>}
            {assetAccounts.map((a) => (
              <option key={a.code} value={a.code}>
                {a.code} {a.name}
              </option>
            ))}
          </select>
        </div>
      </div>
      <p className="text-xs text-text-muted">
        Recording a payment posts it to the ledger as income (cash basis); any sales tax share goes
        to Taxes Payable.
      </p>
      <div className="flex gap-2">
        <button type="submit" disabled={submitting} className={primaryButton}>
          {submitting ? "Recording..." : "Record Payment"}
        </button>
        <button type="button" onClick={onCancel} disabled={submitting} className={secondaryButton}>
          Cancel
        </button>
      </div>
    </form>
  );
}

function VoidForm({
  invoice,
  onDone,
  onCancel,
  onError,
}: {
  invoice: InvoiceDetail;
  onDone: () => void;
  onCancel: () => void;
  onError: (msg: string) => void;
}) {
  const [reason, setReason] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);
  const mountedRef = useMountedRef();

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (submittingRef.current) return;
    if (!reason.trim()) {
      onError("Enter a reason for voiding the invoice");
      return;
    }

    submittingRef.current = true;
    setSubmitting(true);
    try {
      await voidInvoice(invoice.id, reason.trim());
      if (mountedRef.current) onDone();
    } catch (err) {
      if (mountedRef.current) onError(errorMessage(err));
    } finally {
      submittingRef.current = false;
      if (mountedRef.current) setSubmitting(false);
    }
  }

  return (
    <form
      onSubmit={handleSubmit}
      className="border border-danger/40 rounded-md p-3 space-y-3"
      aria-label="Void invoice"
    >
      <p className="text-sm">
        Void invoice <span className="font-medium">{invoice.invoice_number}</span>? It stays on
        record but can no longer be sent, paid, or edited. This can't be undone.
      </p>
      <div>
        <label htmlFor="void-reason" className="block text-xs text-text-muted mb-1">Reason *</label>
        <input
          id="void-reason"
          type="text"
          value={reason}
          onChange={(e) => setReason(e.target.value)}
          required
          placeholder="e.g. Issued in error"
          className={inputClass}
        />
      </div>
      <div className="flex gap-2">
        <button
          type="submit"
          disabled={submitting}
          className="px-3 py-1.5 text-sm font-medium bg-danger text-white rounded-md hover:opacity-90 transition-opacity disabled:opacity-50"
        >
          {submitting ? "Voiding..." : "Confirm Void"}
        </button>
        <button type="button" onClick={onCancel} disabled={submitting} className={secondaryButton}>
          Keep Invoice
        </button>
      </div>
    </form>
  );
}
