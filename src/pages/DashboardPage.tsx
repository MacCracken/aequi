import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { getDashboardSummary, type DashboardSummary } from "../lib/api";
import { formatCents, formatDate } from "../lib/format";
import { DashboardSkeleton } from "../components/Skeleton";

export function DashboardPage() {
  const [data, setData] = useState<DashboardSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const navigate = useNavigate();

  useEffect(() => {
    let cancelled = false;
    getDashboardSummary()
      .then((d) => { if (!cancelled) setData(d); })
      .catch((e) => { if (!cancelled) setError(String(e)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, []);

  if (loading) return <DashboardSkeleton />;
  if (error || !data) {
    return (
      <div className="space-y-4 text-center py-12">
        <p className="text-danger">{error || "Failed to load dashboard"}</p>
        <button
          onClick={() => window.location.reload()}
          className="px-4 py-2 text-sm font-medium bg-primary text-white rounded-md hover:bg-primary-hover"
        >
          Retry
        </button>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <h2 className="text-xl font-semibold">Dashboard</h2>

      {/* Key metrics */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
        <MetricCard
          label="YTD Income"
          value={formatCents(data.ytd_income_cents)}
          color="text-success"
        />
        <MetricCard
          label="YTD Expenses"
          value={formatCents(data.ytd_expenses_cents)}
          color="text-danger"
        />
        <MetricCard
          label="Net Profit"
          value={formatCents(data.ytd_net_profit_cents)}
          color={data.ytd_net_profit_cents >= 0 ? "text-success" : "text-danger"}
        />
        {data.quarterly_tax_due_cents != null ? (
          <MetricCard
            label="Quarterly Tax Due"
            value={formatCents(data.quarterly_tax_due_cents)}
            subtitle={data.next_tax_due_date ? `Due ${formatDate(data.next_tax_due_date)}` : undefined}
            color="text-warning"
          />
        ) : (
          <MetricCard label="Quarterly Tax" value="--" color="text-text-muted" />
        )}
      </div>

      {/* Status cards */}
      <div className="grid grid-cols-2 md:grid-cols-3 gap-4">
        <StatusCard
          label="Outstanding Invoices"
          count={data.outstanding_invoices}
          alert={data.overdue_invoices}
          alertLabel="overdue"
          onClick={() => navigate("/invoices")}
        />
        <StatusCard
          label="Pending Receipts"
          count={data.pending_receipts}
          onClick={() => navigate("/receipts")}
        />
        <StatusCard
          label="Transactions"
          count={data.total_transactions}
          onClick={() => navigate("/transactions")}
        />
      </div>

      {/* Recent transactions */}
      <section>
        <div className="flex items-center justify-between mb-3">
          <h3 className="font-medium">Recent Transactions</h3>
          <button
            onClick={() => navigate("/transactions")}
            className="text-sm text-primary hover:underline"
          >
            View all
          </button>
        </div>
        {data.recent_transactions.length === 0 ? (
          <p className="text-text-muted text-sm">No transactions yet. Create one to get started.</p>
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
                {data.recent_transactions.map((tx) => (
                  <tr key={tx.id} className="border-b border-border last:border-b-0">
                    <td className="px-4 py-2 font-mono">{formatDate(tx.date)}</td>
                    <td className="px-4 py-2">{tx.description}</td>
                    <td className="px-4 py-2 text-right font-mono">{tx.balanced_total}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </div>
  );
}

function MetricCard({
  label,
  value,
  subtitle,
  color = "",
}: {
  label: string;
  value: string;
  subtitle?: string;
  color?: string;
}) {
  return (
    <div className="bg-surface rounded-lg border border-border p-4">
      <p className="text-xs text-text-muted">{label}</p>
      <p className={`text-xl font-bold mt-1 ${color}`}>{value}</p>
      {subtitle && <p className="text-xs text-text-muted mt-0.5">{subtitle}</p>}
    </div>
  );
}

function StatusCard({
  label,
  count,
  alert,
  alertLabel,
  onClick,
}: {
  label: string;
  count: number;
  alert?: number;
  alertLabel?: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="bg-surface rounded-lg border border-border p-4 text-left hover:border-primary/40 transition-colors"
    >
      <p className="text-xs text-text-muted">{label}</p>
      <p className="text-2xl font-bold mt-1">{count}</p>
      {alert != null && alert > 0 && (
        <p className="text-xs text-danger mt-0.5 font-medium">
          {alert} {alertLabel}
        </p>
      )}
    </button>
  );
}
