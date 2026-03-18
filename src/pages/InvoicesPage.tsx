import { useEffect, useState, useMemo } from "react";
import {
  getInvoices,
  getInvoiceAging,
  getContacts,
  createInvoice,
  type InvoiceRecord,
  type ContactRecord,
} from "../lib/api";
import { formatDate } from "../lib/format";
import { TableSkeleton } from "../components/Skeleton";
import { useToast } from "../components/Toast";

function statusBadge(status: string) {
  const colors: Record<string, string> = {
    Draft: "bg-gray-200 text-gray-800",
    Sent: "bg-blue-100 text-blue-800",
    Viewed: "bg-indigo-100 text-indigo-800",
    PartiallyPaid: "bg-yellow-100 text-yellow-900",
    Paid: "bg-green-100 text-green-800",
    Void: "bg-red-100 text-red-800",
  };
  return (
    <span
      className={`px-2 py-0.5 rounded-full text-xs font-medium ${colors[status] || "bg-gray-100"}`}
    >
      {status}
    </span>
  );
}

export function InvoicesPage() {
  const [invoices, setInvoices] = useState<InvoiceRecord[]>([]);
  const [aging, setAging] = useState<InvoiceRecord[]>([]);
  const [contacts, setContacts] = useState<ContactRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [tab, setTab] = useState<"all" | "aging">("all");
  const [showForm, setShowForm] = useState(false);
  const [search, setSearch] = useState("");
  const [statusFilter, setStatusFilter] = useState("");
  const { toast } = useToast();

  function refresh() {
    setLoading(true);
    Promise.all([getInvoices(), getInvoiceAging(), getContacts()])
      .then(([inv, ag, cont]) => {
        setInvoices(inv);
        setAging(ag);
        setContacts(cont);
      })
      .catch((e) => toast("error", String(e)))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    refresh();
  }, []);

  const displayed = tab === "all" ? invoices : aging;

  const filtered = useMemo(() => {
    let result = displayed;
    if (search) {
      const q = search.toLowerCase();
      result = result.filter(
        (inv) =>
          inv.invoice_number.toLowerCase().includes(q) ||
          String(inv.contact_id).includes(q)
      );
    }
    if (statusFilter) {
      result = result.filter((inv) => inv.status_type === statusFilter);
    }
    return result;
  }, [displayed, search, statusFilter]);

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
            onClick={() => setShowForm(!showForm)}
            className="px-4 py-2 text-sm font-medium bg-primary text-white rounded-md hover:bg-primary-hover transition-colors"
          >
            {showForm ? "Cancel" : "New Invoice"}
          </button>
        </div>
      </div>

      {showForm && (
        <InvoiceForm
          contacts={contacts}
          onCreated={() => {
            setShowForm(false);
            refresh();
            toast("success", "Invoice created");
          }}
          onError={(msg) => toast("error", msg)}
        />
      )}

      {/* Filters */}
      <div className="flex gap-3 mb-4">
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
          {invoices.length === 0 ? "No invoices yet. Create one to get started." : "No invoices match your filters."}
        </p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-text-muted border-b border-border">
                <th className="py-2 pr-4">Number</th>
                <th className="py-2 pr-4">Contact</th>
                <th className="py-2 pr-4">Status</th>
                <th className="py-2 pr-4">Issue Date</th>
                <th className="py-2 pr-4">Due Date</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((inv) => {
                const contact = contacts.find((c) => c.id === inv.contact_id);
                return (
                  <tr key={inv.id} className="border-b border-border/50">
                    <td className="py-2 pr-4 font-mono">{inv.invoice_number}</td>
                    <td className="py-2 pr-4">{contact?.name || `#${inv.contact_id}`}</td>
                    <td className="py-2 pr-4">{statusBadge(inv.status_type)}</td>
                    <td className="py-2 pr-4">{formatDate(inv.issue_date)}</td>
                    <td className="py-2 pr-4">{formatDate(inv.due_date)}</td>
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

function InvoiceForm({
  contacts,
  onCreated,
  onError,
}: {
  contacts: ContactRecord[];
  onCreated: () => void;
  onError: (msg: string) => void;
}) {
  const today = new Date().toISOString().split("T")[0];
  const [invoiceNumber, setInvoiceNumber] = useState("");
  const [contactId, setContactId] = useState<number | "">("");
  const [issueDate, setIssueDate] = useState(today);
  const [dueDate, setDueDate] = useState("");
  const [notes, setNotes] = useState("");
  const [terms, setTerms] = useState("");
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!invoiceNumber.trim()) {
      onError("Invoice number is required");
      return;
    }
    if (!contactId) {
      onError("Please select a contact");
      return;
    }
    if (!dueDate) {
      onError("Due date is required");
      return;
    }
    if (dueDate < issueDate) {
      onError("Due date must be on or after issue date");
      return;
    }

    setSubmitting(true);
    try {
      await createInvoice({
        invoice_number: invoiceNumber.trim(),
        contact_id: contactId as number,
        issue_date: issueDate,
        due_date: dueDate,
        notes: notes.trim() || undefined,
        terms: terms.trim() || undefined,
      });
      onCreated();
    } catch (err) {
      onError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form onSubmit={handleSubmit} className="bg-surface rounded-lg border border-border p-4 space-y-4 mb-4">
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
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          />
        </div>
        <div>
          <label htmlFor="inv-contactId" className="block text-xs text-text-muted mb-1">Contact *</label>
          <select
            id="inv-contactId"
            value={contactId}
            onChange={(e) => setContactId(e.target.value ? Number(e.target.value) : "")}
            required
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          >
            <option value="">Select contact</option>
            {contacts.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label htmlFor="inv-issueDate" className="block text-xs text-text-muted mb-1">Issue Date</label>
          <input
            id="inv-issueDate"
            type="date"
            value={issueDate}
            onChange={(e) => setIssueDate(e.target.value)}
            required
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
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
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          />
        </div>
        <div>
          <label htmlFor="inv-terms" className="block text-xs text-text-muted mb-1">Terms</label>
          <input
            id="inv-terms"
            type="text"
            value={terms}
            onChange={(e) => setTerms(e.target.value)}
            placeholder="e.g. Net 30"
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          />
        </div>
        <div>
          <label htmlFor="inv-notes" className="block text-xs text-text-muted mb-1">Notes</label>
          <input
            id="inv-notes"
            type="text"
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          />
        </div>
      </div>
      <button
        type="submit"
        disabled={submitting}
        className="px-4 py-2 text-sm font-medium bg-primary text-white rounded-md hover:bg-primary-hover transition-colors disabled:opacity-50"
      >
        {submitting ? "Creating..." : "Create Invoice"}
      </button>
    </form>
  );
}
