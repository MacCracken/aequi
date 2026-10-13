import { useEffect, useState, useMemo, useRef, useCallback } from "react";
import {
  getInvoices,
  getContacts,
  getAccounts,
  errorMessage,
  type Account,
  type ContactRecord,
  type InvoiceDetail,
  type InvoiceSummary,
} from "../lib/api";
import { daysUntil, formatCents, formatDate } from "../lib/format";
import { TableSkeleton } from "../components/Skeleton";
import { useToast } from "../components/Toast";
import { InvoiceForm } from "../components/InvoiceForm";
import { InvoiceDetailPanel } from "../components/InvoiceDetailPanel";
import { InvoiceStatusBadge } from "../components/InvoiceStatusBadge";

/** What occupies the panel above the list. */
type Panel =
  | { kind: "none" }
  | { kind: "create" }
  | { kind: "detail"; id: number }
  | { kind: "edit"; invoice: InvoiceDetail };

/** Billed invoices with money still owed (drafts are not receivables yet). */
function isReceivable(inv: InvoiceSummary): boolean {
  return inv.balance_cents > 0 && inv.status_type !== "Draft" && inv.status_type !== "Void";
}

export function InvoicesPage() {
  const [invoices, setInvoices] = useState<InvoiceSummary[]>([]);
  const [contacts, setContacts] = useState<ContactRecord[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [loading, setLoading] = useState(true);
  const [tab, setTab] = useState<"all" | "aging">("all");
  const [panel, setPanel] = useState<Panel>({ kind: "none" });
  const [search, setSearch] = useState("");
  const [statusFilter, setStatusFilter] = useState("");
  const { toast } = useToast();
  // Only the latest request may update state; bumped on unmount.
  const requestIdRef = useRef(0);

  const refresh = useCallback(() => {
    const id = ++requestIdRef.current;
    setLoading(true);
    Promise.all([getInvoices(), getContacts(), getAccounts()])
      .then(([inv, cont, accts]) => {
        if (id !== requestIdRef.current) return;
        setInvoices(inv);
        setContacts(cont);
        setAccounts(accts);
      })
      .catch((e) => {
        if (id === requestIdRef.current) toast("error", errorMessage(e));
      })
      .finally(() => {
        if (id === requestIdRef.current) setLoading(false);
      });
  }, [toast]);

  useEffect(() => {
    refresh();
    const ref = requestIdRef;
    return () => { ref.current++; };
  }, [refresh]);

  const contactNames = useMemo(() => new Map(contacts.map((c) => [c.id, c.name])), [contacts]);
  const contactName = useCallback(
    (contactId: number) => contactNames.get(contactId) ?? `Contact #${contactId}`,
    [contactNames],
  );

  // Aging: outstanding receivables, oldest due date first.
  const aging = useMemo(
    () => invoices.filter(isReceivable).sort((a, b) => a.due_date.localeCompare(b.due_date)),
    [invoices],
  );

  const displayed = tab === "all" ? invoices : aging;

  const filtered = useMemo(() => {
    let result = displayed;
    if (search) {
      const q = search.toLowerCase();
      result = result.filter(
        (inv) =>
          inv.invoice_number.toLowerCase().includes(q) ||
          contactName(inv.contact_id).toLowerCase().includes(q)
      );
    }
    if (statusFilter) {
      result = result.filter((inv) => inv.status_type === statusFilter);
    }
    return result;
  }, [displayed, search, statusFilter, contactName]);

  const agingTotal = aging.reduce((sum, inv) => sum + inv.balance_cents, 0);
  const selectedId = panel.kind === "detail" ? panel.id : panel.kind === "edit" ? panel.invoice.id : null;

  if (loading && invoices.length === 0) {
    return (
      <div>
        <h2 className="text-xl font-semibold mb-4">Invoices</h2>
        <TableSkeleton rows={5} cols={4} />
      </div>
    );
  }

  return (
    <div>
      <div className="flex items-center justify-between mb-4 flex-wrap gap-2">
        <h2 className="text-xl font-semibold">Invoices</h2>
        <div className="flex gap-2">
          <div className="flex gap-1">
            <button
              onClick={() => setTab("all")}
              className={`px-3 py-1 rounded text-sm ${tab === "all" ? "bg-primary text-white" : "bg-border/50"}`}
            >
              All
            </button>
            <button
              onClick={() => setTab("aging")}
              className={`px-3 py-1 rounded text-sm ${tab === "aging" ? "bg-primary text-white" : "bg-border/50"}`}
            >
              Aging ({aging.length})
            </button>
          </div>
          <button
            onClick={() => setPanel(panel.kind === "create" ? { kind: "none" } : { kind: "create" })}
            className="px-4 py-2 text-sm font-medium bg-primary text-white rounded-md hover:bg-primary-hover transition-colors"
          >
            {panel.kind === "create" ? "Cancel" : "New Invoice"}
          </button>
        </div>
      </div>

      {panel.kind === "create" && (
        <InvoiceForm
          key="new"
          contacts={contacts}
          onSaved={(inv) => {
            setPanel({ kind: "detail", id: inv.id });
            refresh();
            toast("success", `Invoice ${inv.invoice_number} created`);
          }}
          onCancel={() => setPanel({ kind: "none" })}
          onError={(msg) => toast("error", msg)}
        />
      )}

      {panel.kind === "edit" && (
        <InvoiceForm
          key={`edit-${panel.invoice.id}`}
          contacts={contacts}
          initial={panel.invoice}
          onSaved={(inv) => {
            setPanel({ kind: "detail", id: inv.id });
            refresh();
            toast("success", `Invoice ${inv.invoice_number} updated`);
          }}
          onCancel={() => setPanel({ kind: "detail", id: panel.invoice.id })}
          onError={(msg) => toast("error", msg)}
        />
      )}

      {panel.kind === "detail" && (
        <InvoiceDetailPanel
          key={panel.id}
          invoiceId={panel.id}
          contactName={contactName}
          accounts={accounts}
          onClose={() => setPanel({ kind: "none" })}
          onEdit={(invoice) => setPanel({ kind: "edit", invoice })}
          onChanged={refresh}
        />
      )}

      {tab === "aging" && aging.length > 0 && (
        <p className="text-sm text-text-muted mb-3">
          Outstanding receivables:{" "}
          <span className="font-semibold text-text">{formatCents(agingTotal)}</span>
        </p>
      )}

      {/* Filters */}
      <div className="flex flex-wrap gap-3 mb-4">
        <input
          type="text"
          placeholder="Search invoices..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary flex-1 min-w-[200px]"
        />
        <select
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value)}
          aria-label="Filter by status"
          className="px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
        >
          <option value="">All statuses</option>
          <option value="Draft">Draft</option>
          <option value="Sent">Sent</option>
          <option value="Viewed">Viewed</option>
          <option value="PartiallyPaid">Partially Paid</option>
          <option value="Paid">Paid</option>
          <option value="Void">Void</option>
        </select>
      </div>

      {filtered.length === 0 ? (
        <p className="text-text-muted text-sm">
          {invoices.length === 0
            ? "No invoices yet. Create one to get started."
            : tab === "aging" && aging.length === 0
              ? "No outstanding invoices."
              : "No invoices match your filters."}
        </p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-text-muted border-b border-border">
                <th className="py-2 pr-4">Number</th>
                <th className="py-2 pr-4">Contact</th>
                <th className="py-2 pr-4 hidden md:table-cell">Status</th>
                <th className="py-2 pr-4 hidden md:table-cell">Issue Date</th>
                <th className="py-2 pr-4">Due Date</th>
                <th className="py-2 pr-4 text-right hidden md:table-cell">Total</th>
                <th className="py-2 pr-4 text-right hidden md:table-cell">Paid</th>
                <th className="py-2 text-right">Balance</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((inv) => {
                const dueIn = daysUntil(inv.due_date);
                const overdue = isReceivable(inv) && dueIn !== null && dueIn < 0;
                const isVoid = inv.status_type === "Void";
                const selected = inv.id === selectedId;
                return (
                  <tr
                    key={inv.id}
                    onClick={() => setPanel({ kind: "detail", id: inv.id })}
                    className={`border-b border-border/50 cursor-pointer hover:bg-border/30 ${selected ? "bg-primary/5" : ""}`}
                  >
                    <td className="py-2 pr-4 font-mono whitespace-nowrap">
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          setPanel({ kind: "detail", id: inv.id });
                        }}
                        aria-current={selected || undefined}
                        className="text-primary hover:underline"
                      >
                        {inv.invoice_number}
                      </button>
                      <span className="block mt-0.5 md:hidden">
                        <InvoiceStatusBadge status={inv.status_type} />
                      </span>
                    </td>
                    <td className="py-2 pr-4">{contactName(inv.contact_id)}</td>
                    <td className="py-2 pr-4 hidden md:table-cell"><InvoiceStatusBadge status={inv.status_type} /></td>
                    <td className="py-2 pr-4 whitespace-nowrap hidden md:table-cell">{formatDate(inv.issue_date)}</td>
                    <td className="py-2 pr-4 whitespace-nowrap">
                      {formatDate(inv.due_date)}
                      {overdue && dueIn !== null && (
                        <span className="block text-xs text-danger font-medium">
                          {-dueIn} {dueIn === -1 ? "day" : "days"} overdue
                        </span>
                      )}
                    </td>
                    <td className={`py-2 pr-4 text-right font-mono whitespace-nowrap hidden md:table-cell ${isVoid ? "line-through text-text-muted" : ""}`}>
                      {formatCents(inv.total_cents)}
                    </td>
                    <td className="py-2 pr-4 text-right font-mono whitespace-nowrap hidden md:table-cell">
                      {formatCents(inv.paid_cents)}
                    </td>
                    <td className="py-2 text-right font-mono whitespace-nowrap">
                      {isVoid ? "—" : formatCents(inv.balance_cents)}
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
