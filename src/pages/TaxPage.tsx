import { useEffect, useRef, useState } from "react";
import {
  estimateQuarterlyTax,
  getScheduleCPreview,
  get1099Summary,
  getTaxReturn,
  setTaxReturn,
  errorMessage,
  type NecSummaryEntry,
  type QuarterlyEstimateOutput,
  type ScheduleCPreviewOutput,
  type TaxReturnFigures,
} from "../lib/api";
import {
  centsToInputText,
  daysUntil,
  formatCents,
  formatDate,
  parseDollarsToCents,
  parseQuantityToHundredths,
} from "../lib/format";
import { useMountedRef } from "../lib/hooks";
import { useToast } from "../components/Toast";

export function TaxPage() {
  const [estimate, setEstimate] = useState<QuarterlyEstimateOutput | null>(null);
  const [preview, setPreview] = useState<ScheduleCPreviewOutput | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<"estimate" | "schedule_c">("estimate");
  // Figures from last year's filed return (relative to the estimate's year).
  const [priorReturn, setPriorReturn] = useState<TaxReturnFigures | null>(null);
  const [priorReturnError, setPriorReturnError] = useState<string | null>(null);
  const mountedRef = useMountedRef();
  const { toast } = useToast();

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

  const priorYear = estimate ? estimate.year - 1 : null;

  useEffect(() => {
    if (priorYear === null) return;
    let cancelled = false;
    getTaxReturn({ year: priorYear })
      .then((figures) => {
        if (cancelled) return;
        setPriorReturn(figures);
        setPriorReturnError(null);
      })
      .catch((e) => { if (!cancelled) setPriorReturnError(errorMessage(e)); });
    return () => { cancelled = true; };
  }, [priorYear]);

  /** Store last year's figures, then re-run the estimate so the safe harbor uses them. */
  async function savePriorReturn(year: number, figures: TaxReturnFigures) {
    const stored = await setTaxReturn({ year, figures });
    if (!mountedRef.current) return;
    setPriorReturn(stored);
    try {
      const est = await estimateQuarterlyTax();
      if (mountedRef.current) setEstimate(est);
    } catch (e) {
      if (mountedRef.current) {
        toast("error", `Saved, but the estimate could not be updated: ${errorMessage(e)}`);
      }
    }
  }

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
        <>
          <QuarterlyEstimateCard estimate={estimate} priorReturn={priorReturn} />
          <LastYearReturnCard
            year={estimate.year - 1}
            figures={priorReturn}
            loadError={priorReturnError}
            onSave={(figures) => savePriorReturn(estimate.year - 1, figures)}
          />
        </>
      )}

      {activeTab === "schedule_c" && preview && (
        <ScheduleCCard preview={preview} />
      )}

      <NecContractorsCard
        defaultYear={estimate?.year ?? preview?.year ?? new Date().getFullYear()}
      />
    </div>
  );
}

/** Contractors paid during a year, against the 1099-NEC filing threshold. */
function NecContractorsCard({ defaultYear }: { defaultYear: number }) {
  const [year, setYear] = useState(defaultYear);
  const [entries, setEntries] = useState<NecSummaryEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  // 1099s for last year are filed in January, so offer the prior years too.
  const years = [defaultYear, defaultYear - 1, defaultYear - 2];

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    get1099Summary(year)
      .then((rows) => {
        if (cancelled) return;
        setEntries(rows);
        setError(null);
      })
      .catch((e) => { if (!cancelled) setError(errorMessage(e)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [year]);

  const threshold = entries?.[0]?.threshold_cents;

  return (
    <section className="bg-surface rounded-lg border border-border p-4 space-y-3" aria-labelledby="nec-heading">
      <div className="flex items-center justify-between gap-2 flex-wrap">
        <h3 id="nec-heading" className="font-medium">1099-NEC contractors</h3>
        <div className="flex items-center gap-2">
          <label htmlFor="nec-year" className="text-xs text-text-muted">Year</label>
          <select
            id="nec-year"
            value={year}
            onChange={(e) => setYear(Number(e.target.value))}
            className="px-2 py-1 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          >
            {years.map((y) => (
              <option key={y} value={y}>{y}</option>
            ))}
          </select>
        </div>
      </div>

      {loading && entries === null ? (
        <p className="text-sm text-text-muted">Loading contractor payments...</p>
      ) : error ? (
        <p className="text-sm text-danger">{error}</p>
      ) : !entries || entries.length === 0 ? (
        <p className="text-sm text-text-muted">
          No contractor payments recorded for {year}. Choose a contractor as the payee when you
          record a payment on the Transactions page.
        </p>
      ) : (
        <div className="overflow-x-auto" aria-busy={loading || undefined}>
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-text-muted border-b border-border">
                <th className="py-2 pr-4 font-medium">Contractor</th>
                <th className="py-2 pr-4 font-medium text-right">Paid {year}</th>
                <th className="py-2 font-medium text-right hidden sm:table-cell">Threshold</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((entry) => (
                <tr key={entry.contact_id} className="border-b border-border/50 last:border-0">
                  <td className="py-2 pr-4">
                    {entry.contact_name}
                    {entry.over_threshold && (
                      <span className="ml-2 px-2 py-0.5 rounded-full text-xs font-medium bg-yellow-100 text-yellow-900 whitespace-nowrap">
                        File 1099-NEC
                      </span>
                    )}
                  </td>
                  <td className="py-2 pr-4 text-right font-mono whitespace-nowrap">{formatCents(entry.ytd_cents)}</td>
                  <td className="py-2 text-right font-mono whitespace-nowrap text-text-muted hidden sm:table-cell">
                    {formatCents(entry.threshold_cents)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <p className="text-xs text-text-muted">
        {threshold != null && <>A 1099-NEC is required at {formatCents(threshold)} or more. </>}
        Only payments from bank or cash accounts count; card and payment-app payments are
        reported by the processor on Form 1099-K.
      </p>
    </section>
  );
}

function dueLabel(days: number | null) {
  if (days === null) return null;
  if (days > 0) return <span className="ml-1">({days} {days === 1 ? "day" : "days"})</span>;
  if (days === 0) return <span className="ml-1 text-warning">(due today)</span>;
  return <span className="ml-1 text-danger">(overdue)</span>;
}

const QBI_NOTE =
  "20% qualified business income deduction; phases out above $201,750 of taxable income for service businesses";

function QuarterlyEstimateCard({
  estimate,
  priorReturn,
}: {
  estimate: QuarterlyEstimateOutput;
  priorReturn: TaxReturnFigures | null;
}) {
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
      </div>

      <SafeHarborSummary estimate={estimate} priorReturn={priorReturn} />

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
          <SummaryCard label="QBI Deduction" cents={estimate.qbi_deduction_cents} note={QBI_NOTE} />
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

/**
 * The safe harbor amount and the rule behind it: the smaller of 90% of this
 * year's projected tax and 100% (110% above $150,000 of AGI) of last year's.
 */
function SafeHarborSummary({
  estimate,
  priorReturn,
}: {
  estimate: QuarterlyEstimateOutput;
  priorReturn: TaxReturnFigures | null;
}) {
  const lastYear = estimate.year - 1;
  // "1.10" → 110 (percent), without floating-point math.
  const percent = parseQuantityToHundredths(estimate.safe_harbor_prior_year_percentage) ?? 100;
  const priorRule =
    percent > 100
      ? `${percent}% of last year's tax (last year's AGI over $150,000)`
      : "100% of last year's tax";
  const prior = estimate.prior_year_tax_cents;
  // Integer comparison with 90% of the projected tax.
  const priorRuleApplied =
    prior !== null && estimate.safe_harbor_cents * 10 < estimate.total_tax_cents * 9;
  const fromReturn = priorReturn?.total_tax_cents != null;

  return (
    <div className="bg-surface rounded-lg p-3 border border-border space-y-1">
      <div className="flex items-baseline justify-between gap-2">
        <p className="text-sm font-medium">Safe harbor for {estimate.year}</p>
        <p className="text-lg font-semibold">{formatCents(estimate.safe_harbor_cents)}</p>
      </div>
      <p className="text-sm">
        Rule applied:{" "}
        <span className="font-medium">
          {priorRuleApplied ? priorRule : "90% of this year's projected tax"}
        </span>
      </p>
      <p className="text-xs text-text-muted">
        Paying at least the smaller of 90% of this year's projected tax and {priorRule} over
        the year avoids an underpayment penalty.
      </p>
      <p className="text-xs text-text-muted">
        {prior === null ? (
          <>No {lastYear} tax figure yet, so only the 90% rule can be used.</>
        ) : (
          <>
            Last year's tax: {formatCents(prior)}
            {fromReturn ? ` from your ${lastYear} return.` : ` (this app's ${lastYear} projection).`}
            {priorReturn?.agi_cents == null && " Last year's AGI isn't entered, so 100% is assumed."}
          </>
        )}
      </p>
    </div>
  );
}

/** Optional dollar amount: empty is unknown (null); undefined means invalid. */
function parseOptionalCents(text: string): number | null | undefined {
  if (text.trim() === "") return null;
  return parseDollarsToCents(text) ?? undefined;
}

function optionalCentsText(cents: number | null | undefined): string {
  return cents == null ? "" : centsToInputText(cents);
}

/** Figures from last year's Form 1040, which make the safe harbor exact. */
function LastYearReturnCard({
  year,
  figures,
  loadError,
  onSave,
}: {
  year: number;
  figures: TaxReturnFigures | null;
  loadError: string | null;
  onSave: (figures: TaxReturnFigures) => Promise<void>;
}) {
  const [totalTaxText, setTotalTaxText] = useState("");
  const [agiText, setAgiText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const mountedRef = useMountedRef();
  const { toast } = useToast();

  // Show what is stored whenever it is (re)loaded or saved.
  useEffect(() => {
    setTotalTaxText(optionalCentsText(figures?.total_tax_cents));
    setAgiText(optionalCentsText(figures?.agi_cents));
  }, [figures]);

  async function save(next: TaxReturnFigures, successText: string) {
    if (savingRef.current) return;
    savingRef.current = true;
    setSaving(true);
    setError(null);
    try {
      await onSave(next);
      if (mountedRef.current) toast("success", successText);
    } catch (e) {
      if (mountedRef.current) setError(errorMessage(e));
    } finally {
      savingRef.current = false;
      if (mountedRef.current) setSaving(false);
    }
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const totalTax = parseOptionalCents(totalTaxText);
    const agi = parseOptionalCents(agiText);
    if (totalTax === undefined) {
      setError("Enter the total tax as an amount like 12345.00, or leave it empty");
      return;
    }
    if (agi === undefined) {
      setError("Enter the AGI as an amount like 85000.00 (0 for a loss), or leave it empty");
      return;
    }
    void save({ total_tax_cents: totalTax, agi_cents: agi }, `Saved your ${year} return figures`);
  }

  const hasStored = figures?.total_tax_cents != null || figures?.agi_cents != null;
  const disabled = saving || figures === null;

  return (
    <section
      className="bg-surface rounded-lg border border-border p-4 space-y-3"
      aria-labelledby="last-year-return-heading"
    >
      <div>
        <h3 id="last-year-return-heading" className="font-medium">Last year's return ({year})</h3>
        <p className="text-xs text-text-muted mt-1">
          The safe harbor is exact only with last year's actual figures. Without them it uses this
          app's own {year} projection and assumes 100%. Leave a field empty if you don't know it.
        </p>
      </div>

      {loadError ? (
        <p className="text-sm text-danger">{loadError}</p>
      ) : (
        <form onSubmit={handleSubmit} className="space-y-3" aria-busy={saving || undefined}>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <div>
              <label htmlFor="last-year-total-tax" className="block text-xs text-text-muted mb-1">
                Total tax (Form 1040 line 24)
              </label>
              <input
                id="last-year-total-tax"
                type="text"
                inputMode="decimal"
                value={totalTaxText}
                onChange={(e) => setTotalTaxText(e.target.value)}
                placeholder="Unknown"
                disabled={disabled}
                className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary disabled:opacity-50"
              />
            </div>
            <div>
              <label htmlFor="last-year-agi" className="block text-xs text-text-muted mb-1">
                Adjusted gross income (line 11)
              </label>
              <input
                id="last-year-agi"
                type="text"
                inputMode="decimal"
                value={agiText}
                onChange={(e) => setAgiText(e.target.value)}
                placeholder="Unknown"
                disabled={disabled}
                className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary disabled:opacity-50"
              />
            </div>
          </div>

          {error && <p className="text-sm text-danger" role="alert">{error}</p>}

          <div className="flex gap-2">
            <button
              type="submit"
              disabled={disabled}
              className="px-3 py-1.5 text-sm bg-primary text-white rounded-md hover:bg-primary-hover disabled:opacity-50"
            >
              {saving ? "Saving..." : "Save"}
            </button>
            {hasStored && (
              <button
                type="button"
                onClick={() =>
                  void save({ total_tax_cents: null, agi_cents: null }, `Cleared your ${year} return figures`)
                }
                disabled={disabled}
                className="px-3 py-1.5 text-sm border border-border rounded-md hover:bg-bg transition-colors disabled:opacity-50"
              >
                Clear
              </button>
            )}
          </div>
        </form>
      )}
    </section>
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

function SummaryCard({ label, cents, note }: { label: string; cents: number; note?: string }) {
  return (
    <div className="bg-surface rounded-lg p-3 border border-border">
      <p className="text-xs text-text-muted">{label}</p>
      <p className="text-lg font-semibold mt-0.5">{formatCents(cents)}</p>
      {note && <p className="text-xs text-text-muted mt-1">{note}</p>}
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
