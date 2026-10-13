import { useEffect, useState, useMemo, useRef, useCallback } from "react";
import {
  getTransactions,
  getAccounts,
  createTransaction,
  errorMessage,
  type TransactionOutput,
  type Account,
} from "../lib/api";
import { formatCents, formatDate, parseDollarsToCents, todayLocal } from "../lib/format";
import { TableSkeleton } from "../components/Skeleton";
import { useToast } from "../components/Toast";

const PAGE_SIZE = 50;

export function TransactionsPage() {
  const [transactions, setTransactions] = useState<TransactionOutput[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);
  const [search, setSearch] = useState("");
  const [dateFrom, setDateFrom] = useState("");
  const [dateTo, setDateTo] = useState("");
  const [page, setPage] = useState(0);
  const { toast } = useToast();
  // Only the latest request may update state (date filters can change faster
  // than responses arrive); bumped on unmount so late responses are dropped.
  const requestIdRef = useRef(0);

  const refresh = useCallback(() => {
    const id = ++requestIdRef.current;
    setLoading(true);
    // Each bound is optional on its own; never send "" for an empty field.
    const start = dateFrom || undefined;
    const end = dateTo || undefined;
    Promise.all([getTransactions(start, end), getAccounts()])
      .then(([txs, accts]) => {
        if (id !== requestIdRef.current) return;
        setTransactions(txs);
        setAccounts(accts);
        setPage(0);
      })
      .catch((e) => {
        if (id === requestIdRef.current) toast("error", errorMessage(e));
      })
      .finally(() => {
        if (id === requestIdRef.current) setLoading(false);
      });
  }, [dateFrom, dateTo, toast]);

  useEffect(() => {
    refresh();
    const ref = requestIdRef;
    return () => { ref.current++; };
  }, [refresh]);

  const filtered = useMemo(() => {
    if (!search) return transactions;
    const q = search.toLowerCase();
    return transactions.filter(
      (tx) =>
        tx.description.toLowerCase().includes(q) ||
        tx.date.includes(q) ||
        String(tx.id).includes(q)
    );
  }, [transactions, search]);

  const totalPages = Math.ceil(filtered.length / PAGE_SIZE);
  const paginated = filtered.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE);

  if (loading && transactions.length === 0) {
    return (
      <div className="space-y-4">
        <h2 className="text-xl font-semibold">Transactions</h2>
        <TableSkeleton rows={8} cols={3} />
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between flex-wrap gap-2">
        <h2 className="text-xl font-semibold">Transactions</h2>
        <button
          onClick={() => setShowForm(!showForm)}
          className="px-4 py-2 text-sm font-medium bg-primary text-white rounded-md hover:bg-primary-hover transition-colors"
        >
          {showForm ? "Cancel" : "New Transaction"}
        </button>
      </div>

      {showForm && (
        <TransactionForm
          accounts={accounts}
          onCreated={() => {
            setShowForm(false);
            refresh();
            toast("success", "Transaction created");
          }}
          onError={(msg) => toast("error", msg)}
        />
      )}

      {/* Filters */}
      <div className="flex flex-wrap gap-3">
        <input
          type="text"
          placeholder="Search transactions..."
          value={search}
          onChange={(e) => {
            setSearch(e.target.value);
            setPage(0);
          }}
          className="px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary flex-1 min-w-[200px]"
        />
        <div className="flex items-center gap-2">
          <input
            type="date"
            value={dateFrom}
            onChange={(e) => setDateFrom(e.target.value)}
            className="px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          />
          <span className="text-text-muted text-sm">to</span>
          <input
            type="date"
            value={dateTo}
            onChange={(e) => setDateTo(e.target.value)}
            className="px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          />
        </div>
      </div>

      {filtered.length === 0 ? (
        <p className="text-text-muted">
          {transactions.length === 0 ? "No transactions yet." : "No transactions match your filters."}
        </p>
      ) : (
        <>
          <div className="bg-surface rounded-lg border border-border overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-border text-left text-text-muted">
                  <th className="px-4 py-2 font-medium">Date</th>
                  <th className="px-4 py-2 font-medium">Description</th>
                  <th className="px-4 py-2 font-medium text-right">Amount</th>
                </tr>
              </thead>
              <tbody>
                {paginated.map((tx) => (
                  <tr key={tx.id} className="border-b border-border last:border-b-0">
                    <td className="px-4 py-2 font-mono">{formatDate(tx.date)}</td>
                    <td className="px-4 py-2">{tx.description}</td>
                    <td className="px-4 py-2 text-right font-mono whitespace-nowrap">{tx.balanced_total}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {totalPages > 1 && (
            <div className="flex items-center justify-between text-sm">
              <span className="text-text-muted">
                Showing {page * PAGE_SIZE + 1}–{Math.min((page + 1) * PAGE_SIZE, filtered.length)} of{" "}
                {filtered.length}
              </span>
              <div className="flex gap-1">
                <button
                  onClick={() => setPage(Math.max(0, page - 1))}
                  disabled={page === 0}
                  className="px-3 py-1 rounded border border-border disabled:opacity-40"
                >
                  Prev
                </button>
                <button
                  onClick={() => setPage(Math.min(totalPages - 1, page + 1))}
                  disabled={page >= totalPages - 1}
                  className="px-3 py-1 rounded border border-border disabled:opacity-40"
                >
                  Next
                </button>
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}

interface LineDraft {
  key: number;
  account_code: string;
  /** Raw text as typed; parsed to cents only for totals and submit. */
  debitText: string;
  creditText: string;
}

let nextLineKey = 0;

function emptyLine(): LineDraft {
  return { key: ++nextLineKey, account_code: "", debitText: "", creditText: "" };
}

function TransactionForm({
  accounts,
  onCreated,
  onError,
}: {
  accounts: Account[];
  onCreated: () => void;
  onError: (msg: string) => void;
}) {
  const [date, setDate] = useState(todayLocal);
  const [description, setDescription] = useState("");
  const [lines, setLines] = useState<LineDraft[]>(() => [emptyLine(), emptyLine()]);
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);

  const parsed = useMemo(
    () =>
      lines.map((l) => ({
        debit: parseDollarsToCents(l.debitText),
        credit: parseDollarsToCents(l.creditText),
      })),
    [lines]
  );
  const totalDebits = parsed.reduce((sum, p) => sum + (p.debit ?? 0), 0);
  const totalCredits = parsed.reduce((sum, p) => sum + (p.credit ?? 0), 0);
  const hasInvalidAmount = parsed.some((p) => p.debit === null || p.credit === null);
  const balanced = !hasInvalidAmount && totalDebits > 0 && totalDebits === totalCredits;

  function updateLine(idx: number, patch: Partial<Omit<LineDraft, "key">>) {
    setLines((prev) => prev.map((l, i) => (i === idx ? { ...l, ...patch } : l)));
  }

  function addLine() {
    setLines((prev) => [...prev, emptyLine()]);
  }

  function removeLine(idx: number) {
    setLines((prev) => (prev.length <= 2 ? prev : prev.filter((_, i) => i !== idx)));
  }

  /** Validated line payloads, or an error message for the user. */
  function buildLines():
    | { ok: true; lines: { account_code: string; debit_cents: number; credit_cents: number }[] }
    | { ok: false; error: string } {
    const out: { account_code: string; debit_cents: number; credit_cents: number }[] = [];
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      const { debit, credit } = parsed[i];
      const n = i + 1;
      if (debit === null || credit === null) {
        return { ok: false, error: `Line ${n}: enter amounts like 1234.56 (at most two decimal places)` };
      }
      if (debit === 0 && credit === 0) continue; // blank line (account alone is ignored)
      if (!line.account_code) {
        return { ok: false, error: `Line ${n}: select an account` };
      }
      if (debit > 0 && credit > 0) {
        return { ok: false, error: `Line ${n}: enter either a debit or a credit, not both` };
      }
      out.push({ account_code: line.account_code, debit_cents: debit, credit_cents: credit });
    }
    if (out.length < 2) {
      return { ok: false, error: "At least two lines with amounts are required" };
    }
    if (totalDebits !== totalCredits) {
      return {
        ok: false,
        error: `Debits (${formatCents(totalDebits)}) must equal credits (${formatCents(totalCredits)})`,
      };
    }
    return { ok: true, lines: out };
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (submittingRef.current) return;
    if (!description.trim()) {
      onError("Description is required");
      return;
    }
    const built = buildLines();
    if (!built.ok) {
      onError(built.error);
      return;
    }

    submittingRef.current = true;
    setSubmitting(true);
    try {
      await createTransaction({
        date,
        description: description.trim(),
        lines: built.lines,
      });
      onCreated();
    } catch (err) {
      onError(errorMessage(err));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  const amountClass =
    "w-28 px-2 py-1.5 text-sm text-right font-mono border rounded-md bg-bg focus:outline-none";

  return (
    <form onSubmit={handleSubmit} className="bg-surface rounded-lg border border-border p-4 space-y-4">
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <div>
          <label htmlFor="tx-date" className="block text-xs text-text-muted mb-1">Date</label>
          <input
            id="tx-date"
            type="date"
            value={date}
            onChange={(e) => setDate(e.target.value)}
            required
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          />
        </div>
        <div>
          <label htmlFor="tx-description" className="block text-xs text-text-muted mb-1">Description</label>
          <input
            id="tx-description"
            type="text"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            required
            placeholder="e.g. Client payment received"
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          />
        </div>
      </div>

      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-xs text-text-muted font-medium">Lines</span>
          <button
            type="button"
            onClick={addLine}
            className="text-xs text-primary hover:underline"
          >
            + Add line
          </button>
        </div>
        {lines.map((line, idx) => {
          const debitInvalid = parsed[idx].debit === null;
          const creditInvalid = parsed[idx].credit === null;
          return (
            <div key={line.key} className="flex gap-2 items-center">
              <label htmlFor={`line-${line.key}-account`} className="sr-only">Line {idx + 1} account</label>
              <select
                id={`line-${line.key}-account`}
                value={line.account_code}
                onChange={(e) => updateLine(idx, { account_code: e.target.value })}
                className="flex-1 px-2 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
              >
                <option value="">Select account</option>
                {accounts.map((a) => (
                  <option key={a.code} value={a.code}>
                    {a.code} — {a.name}
                  </option>
                ))}
              </select>
              <label htmlFor={`line-${line.key}-debit`} className="sr-only">Line {idx + 1} debit</label>
              <input
                id={`line-${line.key}-debit`}
                type="text"
                inputMode="decimal"
                autoComplete="off"
                placeholder="Debit"
                value={line.debitText}
                onChange={(e) => updateLine(idx, { debitText: e.target.value })}
                aria-invalid={debitInvalid || undefined}
                className={`${amountClass} ${debitInvalid ? "border-danger" : "border-border focus:border-primary"}`}
              />
              <label htmlFor={`line-${line.key}-credit`} className="sr-only">Line {idx + 1} credit</label>
              <input
                id={`line-${line.key}-credit`}
                type="text"
                inputMode="decimal"
                autoComplete="off"
                placeholder="Credit"
                value={line.creditText}
                onChange={(e) => updateLine(idx, { creditText: e.target.value })}
                aria-invalid={creditInvalid || undefined}
                className={`${amountClass} ${creditInvalid ? "border-danger" : "border-border focus:border-primary"}`}
              />
              {lines.length > 2 && (
                <button
                  type="button"
                  onClick={() => removeLine(idx)}
                  aria-label={`Remove line ${idx + 1}`}
                  className="text-danger text-sm hover:underline"
                >
                  X
                </button>
              )}
            </div>
          );
        })}
        <div className="flex justify-end gap-4 text-xs font-mono" aria-live="polite">
          <span>Debits {formatCents(totalDebits)}</span>
          <span>Credits {formatCents(totalCredits)}</span>
          {hasInvalidAmount ? (
            <span className="text-danger">Invalid amount</span>
          ) : balanced ? (
            <span className="text-success">Balanced</span>
          ) : totalDebits !== totalCredits ? (
            <span className="text-danger">
              Off by {formatCents(Math.abs(totalDebits - totalCredits))}
            </span>
          ) : null}
        </div>
      </div>

      <button
        type="submit"
        disabled={submitting}
        className="px-4 py-2 text-sm font-medium bg-primary text-white rounded-md hover:bg-primary-hover transition-colors disabled:opacity-50"
      >
        {submitting ? "Creating..." : "Create Transaction"}
      </button>
    </form>
  );
}
