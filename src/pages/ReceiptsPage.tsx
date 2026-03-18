import { useEffect, useState, useCallback, useMemo } from "react";
import {
  getPendingReceipts,
  getTransactions,
  approveReceipt,
  rejectReceipt,
  ingestReceipt,
  type ReceiptOutput,
  type TransactionOutput,
} from "../lib/api";
import { formatCents, formatDate, confidenceLabel, confidenceColor } from "../lib/format";
import { CameraCapture } from "../components/CameraCapture";
import { writeCapturedFile } from "../lib/capture";

export function ReceiptsPage() {
  const [receipts, setReceipts] = useState<ReceiptOutput[]>([]);
  const [transactions, setTransactions] = useState<TransactionOutput[]>([]);
  const [selected, setSelected] = useState<ReceiptOutput | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    getPendingReceipts()
      .then(setReceipts)
      .catch((e) => setError(String(e)));
    getTransactions()
      .then(setTransactions)
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function handleApprove(id: number, transactionId?: number) {
    try {
      await approveReceipt(id, transactionId);
      setSelected(null);
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleReject(id: number) {
    try {
      await rejectReceipt(id);
      setSelected(null);
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleCapture(file: File) {
    try {
      const filePath = await writeCapturedFile(file);
      await ingestReceipt(filePath);
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  if (error) {
    return <p className="text-danger">Error: {error}</p>;
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-semibold">Receipt Review Queue</h2>
        <CameraCapture onCapture={handleCapture} />
      </div>

      {receipts.length === 0 ? (
        <p className="text-text-muted">No pending receipts.</p>
      ) : (
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
          {/* Receipt list */}
          <div className="lg:col-span-1 space-y-2">
            {receipts.map((r) => (
              <button
                key={r.id}
                onClick={() => setSelected(r)}
                className={`w-full text-left p-3 rounded-lg border transition-colors ${
                  selected?.id === r.id
                    ? "border-primary bg-primary/5"
                    : "border-border bg-surface hover:border-primary/40"
                }`}
              >
                <div className="flex justify-between items-start">
                  <span className="font-medium truncate">
                    {r.vendor ?? "Unknown vendor"}
                  </span>
                  <span className={`text-xs font-medium ${confidenceColor(r.confidence)}`}>
                    {confidenceLabel(r.confidence)}
                  </span>
                </div>
                <div className="flex justify-between mt-1 text-sm text-text-muted">
                  <span>{r.receipt_date ? formatDate(r.receipt_date) : "No date"}</span>
                  <span>{r.total_cents != null ? formatCents(r.total_cents) : "—"}</span>
                </div>
              </button>
            ))}
          </div>

          {/* Detail panel */}
          <div className="lg:col-span-2">
            {selected ? (
              <ReceiptDetail
                receipt={selected}
                transactions={transactions}
                onApprove={handleApprove}
                onReject={handleReject}
              />
            ) : (
              <div className="bg-surface rounded-lg border border-border p-8 text-center text-text-muted">
                Select a receipt to review
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function ReceiptDetail({
  receipt,
  transactions,
  onApprove,
  onReject,
}: {
  receipt: ReceiptOutput;
  transactions: TransactionOutput[];
  onApprove: (id: number, transactionId?: number) => void;
  onReject: (id: number) => void;
}) {
  const [linkedTxId, setLinkedTxId] = useState<number | undefined>(undefined);
  const [search, setSearch] = useState("");
  const [showPicker, setShowPicker] = useState(false);

  // Reset link state when receipt changes
  useEffect(() => {
    setLinkedTxId(undefined);
    setSearch("");
    setShowPicker(false);
  }, [receipt.id]);

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

  const linkedTx = linkedTxId != null
    ? transactions.find((tx) => tx.id === linkedTxId)
    : undefined;

  const isImage = receipt.attachment_path.match(/\.(png|jpg|jpeg|gif|webp)$/i);

  return (
    <div className="bg-surface rounded-lg border border-border overflow-hidden">
      {/* Attachment viewer */}
      <div className="bg-bg border-b border-border p-4 flex items-center justify-center min-h-[200px] md:min-h-[300px]">
        {isImage ? (
          <img
            src={`asset://localhost/${receipt.attachment_path}`}
            alt="Receipt"
            className="max-h-[400px] object-contain rounded"
          />
        ) : (
          <div className="text-text-muted text-sm">
            Attachment: {receipt.attachment_path.split("/").pop()}
          </div>
        )}
      </div>

      {/* Extracted fields */}
      <div className="p-4 space-y-3">
        <div className="grid grid-cols-2 gap-3 text-sm">
          <Field label="Vendor" value={receipt.vendor} />
          <Field label="Date" value={receipt.receipt_date ? formatDate(receipt.receipt_date) : null} />
          <Field label="Total" value={receipt.total_cents != null ? formatCents(receipt.total_cents) : null} />
          <Field label="Subtotal" value={receipt.subtotal_cents != null ? formatCents(receipt.subtotal_cents) : null} />
          <Field label="Tax" value={receipt.tax_cents != null ? formatCents(receipt.tax_cents) : null} />
          <Field label="Payment" value={receipt.payment_method} />
        </div>

        <div className="flex items-center gap-2 text-sm">
          <span className="text-text-muted">Confidence:</span>
          <span className={`font-medium ${confidenceColor(receipt.confidence)}`}>
            {(receipt.confidence * 100).toFixed(0)}% — {confidenceLabel(receipt.confidence)}
          </span>
        </div>

        {/* Transaction linking */}
        <div className="border-t border-border pt-3 space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-sm font-medium">Link to Transaction</span>
            {linkedTx ? (
              <button
                onClick={() => { setLinkedTxId(undefined); setShowPicker(false); }}
                className="text-xs text-danger hover:underline"
              >
                Unlink
              </button>
            ) : (
              <button
                onClick={() => setShowPicker(!showPicker)}
                className="text-xs text-primary hover:underline"
              >
                {showPicker ? "Cancel" : "Select transaction"}
              </button>
            )}
          </div>

          {linkedTx && (
            <div className="text-sm bg-primary/5 border border-primary/20 rounded-md px-3 py-2 flex justify-between">
              <span>
                <span className="font-mono text-text-muted">{formatDate(linkedTx.date)}</span>
                {" "}{linkedTx.description}
              </span>
              <span className="font-mono">{linkedTx.balanced_total}</span>
            </div>
          )}

          {showPicker && !linkedTx && (
            <div className="space-y-2">
              <input
                type="text"
                placeholder="Search by description, date, or ID..."
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
              />
              <div className="max-h-48 overflow-y-auto border border-border rounded-md divide-y divide-border">
                {filtered.length === 0 ? (
                  <div className="px-3 py-2 text-sm text-text-muted">No transactions found</div>
                ) : (
                  filtered.slice(0, 50).map((tx) => (
                      <button
                        key={tx.id}
                        onClick={() => { setLinkedTxId(tx.id); setShowPicker(false); setSearch(""); }}
                        className="w-full text-left px-3 py-2 text-sm hover:bg-bg transition-colors flex justify-between"
                      >
                        <span className="truncate">
                          <span className="font-mono text-text-muted">{formatDate(tx.date)}</span>
                          {" "}{tx.description}
                        </span>
                        <span className="font-mono shrink-0 ml-2">{tx.balanced_total}</span>
                      </button>
                    ))
                )}
              </div>
            </div>
          )}
        </div>

        {/* Actions */}
        <div className="flex gap-2 pt-2">
          <button
            onClick={() => onApprove(receipt.id, linkedTxId)}
            className="px-4 py-2 rounded-md text-sm font-medium text-white bg-success hover:bg-success/90 transition-colors"
          >
            {linkedTxId != null ? "Approve & Link" : "Approve"}
          </button>
          <button
            onClick={() => onReject(receipt.id)}
            className="px-4 py-2 rounded-md text-sm font-medium text-white bg-danger hover:bg-danger/90 transition-colors"
          >
            Reject
          </button>
        </div>
      </div>
    </div>
  );
}

function Field({ label, value }: { label: string; value: string | null }) {
  return (
    <div>
      <span className="text-text-muted text-xs">{label}</span>
      <p className="font-medium">{value ?? "—"}</p>
    </div>
  );
}
