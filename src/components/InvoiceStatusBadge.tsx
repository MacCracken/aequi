const STATUS_COLORS: Record<string, string> = {
  Draft: "bg-gray-200 text-gray-800",
  Sent: "bg-blue-100 text-blue-800",
  Viewed: "bg-indigo-100 text-indigo-800",
  PartiallyPaid: "bg-yellow-100 text-yellow-900",
  Paid: "bg-green-100 text-green-800",
  Void: "bg-red-100 text-red-800",
};

const STATUS_LABELS: Record<string, string> = {
  PartiallyPaid: "Partially Paid",
};

export function InvoiceStatusBadge({ status }: { status: string }) {
  return (
    <span
      className={`px-2 py-0.5 rounded-full text-xs font-medium whitespace-nowrap ${STATUS_COLORS[status] || "bg-gray-100"}`}
    >
      {STATUS_LABELS[status] ?? status}
    </span>
  );
}
