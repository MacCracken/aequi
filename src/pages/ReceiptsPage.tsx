import { useEffect, useState, useCallback, useRef, useMemo } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  getPendingReceipts,
  getTransactions,
  approveReceipt,
  rejectReceipt,
  errorMessage,
  type ReceiptOutput,
  type TransactionOutput,
} from "../lib/api";
import { formatCents, formatDate, confidenceLabel, confidenceColor } from "../lib/format";
import { CameraCapture } from "../components/CameraCapture";
import { ingestCapturedFile } from "../lib/capture";
import { useToast } from "../components/Toast";

export function ReceiptsPage() {
  const [receipts, setReceipts] = useState<ReceiptOutput[]>([]);
  const [transactions, setTransactions] = useState<TransactionOutput[]>([]);
  const [selected, setSelected] = useState<ReceiptOutput | null>(null);
  const [capturing, setCapturing] = useState(false);
  const [actionPending, setActionPending] = useState(false);
  const { toast } = useToast();
  // Latest refresh wins; bumped on unmount so late responses are dropped.
  const refreshIdRef = useRef(0);
  const actionPendingRef = useRef(false);

  const refresh = useCallback(() => {
    const id = ++refreshIdRef.current;
    getPendingReceipts()
      .then((r) => { if (id === refreshIdRef.current) setReceipts(r); })
      .catch((e) => { if (id === refreshIdRef.current) toast("error", errorMessage(e)); });
    getTransactions()
      .then((t) => { if (id === refreshIdRef.current) setTransactions(t); })
      .catch((e) => { if (id === refreshIdRef.current) toast("error", errorMessage(e)); });
  }, [toast]);

  useEffect(() => {
    refresh();
    const ref = refreshIdRef;
    return () => { ref.current++; };
  }, [refresh]);

  async function runAction(action: () => Promise<void>, successText: string) {
    if (actionPendingRef.current) return;
    actionPendingRef.current = true;
    setActionPending(true);
    try {
      await action();
      setSelected(null);
      refresh();
      toast("success", successText);
    } catch (e) {
      toast("error", errorMessage(e));
    } finally {
      actionPendingRef.current = false;
      setActionPending(false);
    }
  }

  function handleApprove(id: number, transactionId?: number) {
    return runAction(() => approveReceipt(id, transactionId), "Receipt approved");
  }

  function handleReject(id: number) {
    return runAction(() => rejectReceipt(id), "Receipt rejected");
  }

  async function handleCapture(file: File) {
    if (capturing) return;
    setCapturing(true);
    try {
      await ingestCapturedFile(file);
      refresh();
      toast("success", "Receipt captured");
    } catch (e) {
      toast("error", errorMessage(e));
    } finally {
      setCapturing(false);
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-semibold">Receipt Review Queue</h2>
        <CameraCapture onCapture={handleCapture} busy={capturing} />
      </div>
      <p className="text-sm text-text-muted">
        Vendor, date and amounts are filled in by OCR or by an AI agent connected over MCP.
        Receipts stay here until you check them and approve or reject them — only you can
        approve a receipt.
      </p>

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
                {r.filled_by_agent && (
                  <span className="inline-block mt-1 px-2 py-0.5 rounded-full text-xs font-medium bg-primary/10 text-primary">
                    Filled by agent — needs your review
                  </span>
                )}
              </button>
            ))}
          </div>

          {/* Detail panel */}
          <div className="lg:col-span-2">
            {selected ? (
              <ReceiptDetail
                receipt={selected}
                transactions={transactions}
                busy={actionPending}
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
  busy,
  onApprove,
  onReject,
}: {
  receipt: ReceiptOutput;
  transactions: TransactionOutput[];
  busy: boolean;
  onApprove: (id: number, transactionId?: number) => void;
  onReject: (id: number) => void;
}) {
  const [linkedTxId, setLinkedTxId] = useState<number | undefined>(undefined);
  const [search, setSearch] = useState("");
  const [showPicker, setShowPicker] = useState(false);
  const [imageFailed, setImageFailed] = useState(false);

  // Reset per-receipt state when receipt changes
  useEffect(() => {
    setLinkedTxId(undefined);
    setSearch("");
    setShowPicker(false);
    setImageFailed(false);
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

  const isImage = /\.(png|jpe?g|gif|webp|bmp)$/i.test(receipt.attachment_path);
  const fileName = receipt.attachment_path.split(/[\\/]/).pop();

  return (
    <div className="bg-surface rounded-lg border border-border overflow-hidden">
      {/* Attachment viewer */}
      <div className="bg-bg border-b border-border p-4 flex items-center justify-center min-h-[200px] md:min-h-[300px]">
        {isImage && !imageFailed ? (
          <img
            src={convertFileSrc(receipt.attachment_path)}
            alt="Receipt"
            onError={() => setImageFailed(true)}
            className="max-h-[400px] object-contain rounded"
          />
        ) : (
          <div className="text-text-muted text-sm">
            {imageFailed ? "Could not load image: " : "Attachment: "}
            {fileName}
          </div>
        )}
      </div>

      {/* Extracted fields */}
      <div className="p-4 space-y-3">
        <div>
          <h3 className="font-medium">Review the extracted details</h3>
          <p className="text-xs text-text-muted mt-0.5">
            {receipt.filled_by_agent
              ? "An AI agent read these fields from the receipt image. Check them against the image, then approve or reject the receipt."
              : "Check these fields against the receipt image, then approve or reject the receipt."}
          </p>
        </div>

        <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 bg-bg border border-border rounded-md p-3">
          <Field label="Vendor" value={receipt.vendor} prominent />
          <Field label="Date" value={receipt.receipt_date ? formatDate(receipt.receipt_date) : null} prominent />
          <Field label="Total" value={receipt.total_cents != null ? formatCents(receipt.total_cents) : null} prominent />
        </div>

        <div className="grid grid-cols-3 gap-3 text-sm">
          <Field label="Subtotal" value={receipt.subtotal_cents != null ? formatCents(receipt.subtotal_cents) : null} />
          <Field label="Tax" value={receipt.tax_cents != null ? formatCents(receipt.tax_cents) : null} />
          <Field label="Payment" value={receipt.payment_method} />
        </div>

        <div className="flex items-center gap-2 text-sm flex-wrap">
          <span className="text-text-muted">Confidence:</span>
          <span className={`font-medium ${confidenceColor(receipt.confidence)}`}>
            {(receipt.confidence * 100).toFixed(0)}% — {confidenceLabel(receipt.confidence)}
          </span>
          {receipt.filled_by_agent && (
            <span className="text-xs text-text-muted">(reported by the agent)</span>
          )}
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
            disabled={busy}
            className="px-4 py-2 rounded-md text-sm font-medium text-white bg-success hover:bg-success/90 transition-colors disabled:opacity-50"
          >
            {linkedTxId != null ? "Approve & Link" : "Approve"}
          </button>
          <button
            onClick={() => onReject(receipt.id)}
            disabled={busy}
            className="px-4 py-2 rounded-md text-sm font-medium text-white bg-danger hover:bg-danger/90 transition-colors disabled:opacity-50"
          >
            Reject
          </button>
        </div>
      </div>
    </div>
  );
}

function Field({
  label,
  value,
  prominent = false,
}: {
  label: string;
  value: string | null;
  prominent?: boolean;
}) {
  return (
    <div className="min-w-0">
      <span className="text-text-muted text-xs">{label}</span>
      <p className={`break-words ${prominent ? "text-lg font-semibold" : "font-medium"}`}>
        {value ?? "—"}
      </p>
    </div>
  );
}
