import { useEffect, useState } from "react";
import { getInvoices, getInvoiceAging, type InvoiceRecord } from "../lib/api";
import { formatDate } from "../lib/format";

function statusBadge(status: string) {
  const colors: Record<string, string> = {
    Draft: "bg-gray-200 text-gray-700",
    Sent: "bg-blue-100 text-blue-700",
    Viewed: "bg-indigo-100 text-indigo-700",
    PartiallyPaid: "bg-yellow-100 text-yellow-700",
    Paid: "bg-green-100 text-green-700",
    Void: "bg-red-100 text-red-700",
  };
  return (
    <span className={`px-2 py-0.5 rounded-full text-xs font-medium ${colors[status] || "bg-gray-100"}`}>
      {status}
    </span>
  );
}

export function InvoicesPage() {
  const [invoices, setInvoices] = useState<InvoiceRecord[]>([]);
  const [aging, setAging] = useState<InvoiceRecord[]>([]);
  const [tab, setTab] = useState<"all" | "aging">("all");

  useEffect(() => {
    getInvoices().then(setInvoices).catch(console.error);
    getInvoiceAging().then(setAging).catch(console.error);
  }, []);

  const displayed = tab === "all" ? invoices : aging;

  return (
    <div>
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-xl font-semibold">Invoices</h2>
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
      </div>

      {displayed.length === 0 ? (
        <p className="text-text-muted text-sm">No invoices found.</p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-text-muted border-b border-border">
                <th className="py-2 pr-4">Number</th>
                <th className="py-2 pr-4">Status</th>
                <th className="py-2 pr-4">Issue Date</th>
                <th className="py-2 pr-4">Due Date</th>
              </tr>
            </thead>
            <tbody>
              {displayed.map((inv) => (
                <tr key={inv.id} className="border-b border-border/50">
                  <td className="py-2 pr-4 font-mono">{inv.invoice_number}</td>
                  <td className="py-2 pr-4">{statusBadge(inv.status_type)}</td>
                  <td className="py-2 pr-4">{formatDate(inv.issue_date)}</td>
                  <td className="py-2 pr-4">{formatDate(inv.due_date)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
