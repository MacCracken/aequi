import { invoke } from "@tauri-apps/api/core";

export interface Account {
  code: string;
  name: string;
  account_type: string;
}

export interface TransactionInput {
  date: string;
  description: string;
  entries: { account_code: string; amount_cents: number }[];
}

export interface TransactionOutput {
  id: number;
  date: string;
  description: string;
  entries: { account_code: string; amount_cents: number }[];
}

export interface ProfitLossEntry {
  account_code: string;
  account_name: string;
  total_cents: number;
}

export interface ReceiptOutput {
  id: number;
  file_hash: string;
  vendor: string | null;
  receipt_date: string | null;
  total_cents: number | null;
  subtotal_cents: number | null;
  tax_cents: number | null;
  payment_method: string | null;
  confidence: number;
  status: string;
  transaction_id: number | null;
  attachment_path: string;
  needs_review: boolean;
  created_at: string;
}

export function getAccounts(): Promise<Account[]> {
  return invoke("get_accounts");
}

export function createTransaction(input: TransactionInput): Promise<TransactionOutput> {
  return invoke("create_transaction", { input });
}

export function getTransactions(startDate?: string, endDate?: string): Promise<TransactionOutput[]> {
  return invoke("get_transactions", { startDate, endDate });
}

export function getProfitLoss(startDate?: string, endDate?: string): Promise<ProfitLossEntry[]> {
  return invoke("get_profit_loss", { startDate, endDate });
}

export function ingestReceipt(filePath: string): Promise<ReceiptOutput> {
  return invoke("ingest_receipt", { filePath });
}

export function getPendingReceipts(): Promise<ReceiptOutput[]> {
  return invoke("get_pending_receipts");
}

export function approveReceipt(receiptId: number, transactionId?: number): Promise<void> {
  return invoke("approve_receipt", { receiptId, transactionId });
}

export function rejectReceipt(receiptId: number): Promise<void> {
  return invoke("reject_receipt", { receiptId });
}
