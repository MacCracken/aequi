import { useEffect, useState, useMemo } from "react";
import {
  getTransactions,
  getAccounts,
  createTransaction,
  type TransactionOutput,
  type Account,
} from "../lib/api";
import { formatDate } from "../lib/format";
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

  function refresh() {
    setLoading(true);
    const start = dateFrom || undefined;
    const end = dateTo || undefined;
    Promise.all([getTransactions(start, end), getAccounts()])
      .then(([txs, accts]) => {
        setTransactions(txs);
        setAccounts(accts);
      })
      .catch((e) => toast("error", String(e)))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    refresh();
  }, [dateFrom, dateTo]);

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

function TransactionForm({
  accounts,
  onCreated,
  onError,
}: {
  accounts: Account[];
  onCreated: () => void;
  onError: (msg: string) => void;
}) {
  const [date, setDate] = useState(new Date().toISOString().split("T")[0]);
  const [description, setDescription] = useState("");
  const [lines, setLines] = useState([
    { account_code: "", debit_cents: 0, credit_cents: 0, memo: "" },
    { account_code: "", debit_cents: 0, credit_cents: 0, memo: "" },
  ]);
  const [submitting, setSubmitting] = useState(false);

  function updateLine(idx: number, field: string, value: string | number) {
    setLines((prev) =>
      prev.map((l, i) => (i === idx ? { ...l, [field]: value } : l))
    );
  }

  function addLine() {
    setLines((prev) => [
      ...prev,
      { account_code: "", debit_cents: 0, credit_cents: 0, memo: "" },
    ]);
  }

  function removeLine(idx: number) {
    if (lines.length <= 2) return;
    setLines((prev) => prev.filter((_, i) => i !== idx));
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!description.trim()) {
      onError("Description is required");
      return;
    }
    const filledLines = lines.filter(
      (l) => l.account_code && (l.debit_cents > 0 || l.credit_cents > 0)
    );
    if (filledLines.length < 2) {
      onError("At least two lines with amounts are required");
      return;
    }

    setSubmitting(true);
    try {
      await createTransaction({
        date,
        description: description.trim(),
        lines: filledLines.map((l) => ({
          account_code: l.account_code,
          debit_cents: l.debit_cents,
          credit_cents: l.credit_cents,
        })),
      });
      onCreated();
    } catch (err) {
      onError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

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
          <label className="text-xs text-text-muted font-medium">Lines</label>
          <button
            type="button"
            onClick={addLine}
            className="text-xs text-primary hover:underline"
          >
            + Add line
          </button>
        </div>
        {lines.map((line, idx) => (
          <div key={idx} className="flex gap-2 items-center">
            <label htmlFor={`line-${idx}-account`} className="sr-only">Line {idx + 1} account</label>
            <select
              id={`line-${idx}-account`}
              value={line.account_code}
              onChange={(e) => updateLine(idx, "account_code", e.target.value)}
              className="flex-1 px-2 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
            >
              <option value="">Select account</option>
              {accounts.map((a) => (
                <option key={a.code} value={a.code}>
                  {a.code} — {a.name}
                </option>
              ))}
            </select>
            <label htmlFor={`line-${idx}-debit`} className="sr-only">Line {idx + 1} debit</label>
            <input
              id={`line-${idx}-debit`}
              type="number"
              step="0.01"
              min="0"
              placeholder="Debit"
              value={line.debit_cents ? (line.debit_cents / 100).toFixed(2) : ""}
              onChange={(e) =>
                updateLine(idx, "debit_cents", Math.round(Number(e.target.value) * 100))
              }
              className="w-24 px-2 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
            />
            <label htmlFor={`line-${idx}-credit`} className="sr-only">Line {idx + 1} credit</label>
            <input
              id={`line-${idx}-credit`}
              type="number"
              step="0.01"
              min="0"
              placeholder="Credit"
              value={line.credit_cents ? (line.credit_cents / 100).toFixed(2) : ""}
              onChange={(e) =>
                updateLine(idx, "credit_cents", Math.round(Number(e.target.value) * 100))
              }
              className="w-24 px-2 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
            />
            {lines.length > 2 && (
              <button
                type="button"
                onClick={() => removeLine(idx)}
                className="text-danger text-sm hover:underline"
              >
                X
              </button>
            )}
          </div>
        ))}
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
