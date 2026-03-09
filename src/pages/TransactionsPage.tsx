import { useEffect, useState } from "react";
import { getTransactions, type TransactionOutput } from "../lib/api";
import { formatCents, formatDate } from "../lib/format";

export function TransactionsPage() {
  const [transactions, setTransactions] = useState<TransactionOutput[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getTransactions().then(setTransactions).catch((e) => setError(String(e)));
  }, []);

  if (error) {
    return <p className="text-danger">Error: {error}</p>;
  }

  return (
    <div className="space-y-4">
      <h2 className="text-xl font-semibold">Transactions</h2>
      {transactions.length === 0 ? (
        <p className="text-text-muted">No transactions yet.</p>
      ) : (
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
              {transactions.map((tx) => {
                const debitTotal = tx.entries
                  .filter((e) => e.amount_cents > 0)
                  .reduce((s, e) => s + e.amount_cents, 0);
                return (
                  <tr key={tx.id} className="border-b border-border last:border-b-0">
                    <td className="px-4 py-2 font-mono">{formatDate(tx.date)}</td>
                    <td className="px-4 py-2">{tx.description}</td>
                    <td className="px-4 py-2 text-right font-mono">
                      {formatCents(debitTotal)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
