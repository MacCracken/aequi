import { useEffect, useState } from "react";
import {
  estimateQuarterlyTax,
  getScheduleCPreview,
  errorMessage,
  type QuarterlyEstimateOutput,
  type ScheduleCPreviewOutput,
} from "../lib/api";
import { daysUntil, formatCents, formatDate } from "../lib/format";

export function TaxPage() {
  const [estimate, setEstimate] = useState<QuarterlyEstimateOutput | null>(null);
  const [preview, setPreview] = useState<ScheduleCPreviewOutput | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<"estimate" | "schedule_c">("estimate");

  useEffect(() => {
    let cancelled = false;
    Promise.all([estimateQuarterlyTax(), getScheduleCPreview()])
      .then(([est, prev]) => {
        if (cancelled) return;
        setEstimate(est);
        setPreview(prev);
      })
      .catch((e) => { if (!cancelled) setError(errorMessage(e)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, []);

  if (loading) {
    return <p className="text-text-muted">Computing tax estimates...</p>;
  }

  if (error) {
    return <p className="text-danger">{error}</p>;
  }

  return (
    <div className="space-y-6">
      <h2 className="text-xl font-semibold">Tax Center</h2>

      {/* Tab switcher */}
      <div className="flex gap-1 border-b border-border">
        <button
          onClick={() => setActiveTab("estimate")}
          className={`px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
            activeTab === "estimate"
              ? "border-primary text-primary"
              : "border-transparent text-text-muted hover:text-foreground"
          }`}
        >
          Quarterly Estimate
        </button>
        <button
          onClick={() => setActiveTab("schedule_c")}
          className={`px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
            activeTab === "schedule_c"
              ? "border-primary text-primary"
              : "border-transparent text-text-muted hover:text-foreground"
          }`}
        >
          Schedule C Preview
        </button>
      </div>

      {activeTab === "estimate" && estimate && (
        <QuarterlyEstimateCard estimate={estimate} />
      )}

      {activeTab === "schedule_c" && preview && (
        <ScheduleCCard preview={preview} />
      )}
    </div>
  );
}

function dueLabel(days: number | null) {
  if (days === null) return null;
  if (days > 0) return <span className="ml-1">({days} {days === 1 ? "day" : "days"})</span>;
  if (days === 0) return <span className="ml-1 text-warning">(due today)</span>;
  return <span className="ml-1 text-danger">(overdue)</span>;
}

function QuarterlyEstimateCard({ estimate }: { estimate: QuarterlyEstimateOutput }) {
  const daysUntilDue = daysUntil(estimate.payment_due_date);

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-baseline justify-between">
        <h3 className="text-lg font-medium">
          {estimate.year} {estimate.quarter} Estimate
        </h3>
        <div className="text-sm text-text-muted">
          Due {formatDate(estimate.payment_due_date)}
          {dueLabel(daysUntilDue)}
        </div>
      </div>

      {/* Quarterly payment callout */}
      <div className="bg-primary/10 border border-primary/20 rounded-lg p-4 text-center">
        <p className="text-sm text-text-muted mb-1">Estimated Quarterly Payment</p>
        <p className="text-3xl font-bold">{formatCents(estimate.quarterly_payment_cents)}</p>
        <p className="text-xs text-text-muted mt-1">
          One quarter of the projected {estimate.year} tax of {formatCents(estimate.total_tax_cents)}
        </p>
        <p className="text-xs text-text-muted mt-1">
          Safe harbor: {formatCents(estimate.safe_harbor_cents)}
        </p>
      </div>

      {/* Year to date */}
      <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
        <SummaryCard label="YTD Gross Income" cents={estimate.ytd_gross_income_cents} />
        <SummaryCard label="YTD Expenses" cents={estimate.ytd_total_expenses_cents} />
        <SummaryCard label="YTD Net Profit" cents={estimate.ytd_net_profit_cents} />
      </div>

      {/* Full-year projection */}
      <div className="space-y-2">
        <h4 className="text-sm font-medium text-text-muted">Projected {estimate.year}</h4>
        <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
          <SummaryCard label="Annualized Net Profit" cents={estimate.annualized_net_profit_cents} />
          <SummaryCard label="Standard Deduction" cents={estimate.standard_deduction_cents} />
          <SummaryCard label="Taxable Income" cents={estimate.taxable_income_cents} />
          <SummaryCard label="SE Tax" cents={estimate.se_tax_cents} />
          <SummaryCard label="Income Tax" cents={estimate.income_tax_cents} />
          <SummaryCard label="Additional Medicare Tax" cents={estimate.additional_medicare_tax_cents} />
          <SummaryCard label="Total Projected Tax" cents={estimate.total_tax_cents} />
        </div>
      </div>

      {/* Method notes */}
      <div className="text-xs text-text-muted bg-surface rounded-lg p-3 border border-border">
        <p>
          Year-to-date net profit is annualized to project full-year income; taxes are
          computed on that projection and the quarterly payment is one quarter of the total.
        </p>
        <p className="mt-1">SE tax deduction (50%): {formatCents(estimate.se_tax_deduction_cents)}</p>
        <p className="mt-1">
          Estimates are informational only. Consult a CPA for your specific situation.
        </p>
      </div>
    </div>
  );
}

function ScheduleCCard({ preview }: { preview: ScheduleCPreviewOutput }) {
  const incomeLines = preview.lines.filter((l) => l.is_income);
  const expenseLines = preview.lines.filter((l) => !l.is_income);

  return (
    <div className="space-y-4">
      <h3 className="text-lg font-medium">{preview.year} Schedule C Preview</h3>

      {/* Summary */}
      <div className="grid grid-cols-3 gap-3">
        <SummaryCard label="Gross Income" cents={preview.gross_income_cents} />
        <SummaryCard label="Total Expenses" cents={preview.total_expenses_cents} />
        <SummaryCard label="Net Profit" cents={preview.net_profit_cents} />
      </div>

      {/* Income lines */}
      {incomeLines.length > 0 && (
        <LineTable title="Income" lines={incomeLines} />
      )}

      {/* Expense lines */}
      {expenseLines.length > 0 && (
        <LineTable title="Expenses" lines={expenseLines} />
      )}

      <p className="text-xs text-text-muted">
        Meals (Line 24b) shown at 50% deductible amount. This is a preview — not a filed return.
      </p>
    </div>
  );
}

function SummaryCard({ label, cents }: { label: string; cents: number }) {
  return (
    <div className="bg-surface rounded-lg p-3 border border-border">
      <p className="text-xs text-text-muted">{label}</p>
      <p className="text-lg font-semibold mt-0.5">{formatCents(cents)}</p>
    </div>
  );
}

function LineTable({
  title,
  lines,
}: {
  title: string;
  lines: { label: string; amount_cents: number }[];
}) {
  return (
    <div>
      <h4 className="text-sm font-medium text-text-muted mb-2">{title}</h4>
      <div className="border border-border rounded-lg overflow-hidden">
        <table className="w-full text-sm">
          <tbody>
            {lines.map((line) => (
              <tr key={line.label} className="border-b border-border last:border-0">
                <td className="px-3 py-2">{line.label}</td>
                <td className="px-3 py-2 text-right font-mono">
                  {formatCents(line.amount_cents)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
