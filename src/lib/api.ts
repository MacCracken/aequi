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

// ── Tax commands ─────────────────────────────────────────────────────────────

export interface ScheduleCLineOutput {
  line: string;
  label: string;
  amount_cents: number;
  is_income: boolean;
}

export interface QuarterlyEstimateOutput {
  year: number;
  quarter: string;
  ytd_gross_income_cents: number;
  ytd_total_expenses_cents: number;
  ytd_net_profit_cents: number;
  se_tax_cents: number;
  se_tax_deduction_cents: number;
  income_tax_cents: number;
  total_tax_cents: number;
  safe_harbor_cents: number;
  quarterly_payment_cents: number;
  payment_due_date: string;
  schedule_c_lines: ScheduleCLineOutput[];
}

export interface ScheduleCPreviewOutput {
  year: number;
  gross_income_cents: number;
  total_expenses_cents: number;
  net_profit_cents: number;
  lines: ScheduleCLineOutput[];
}

export function estimateQuarterlyTax(
  year?: number,
  quarter?: number
): Promise<QuarterlyEstimateOutput> {
  return invoke("estimate_quarterly_tax", { year, quarter });
}

export function getScheduleCPreview(year?: number): Promise<ScheduleCPreviewOutput> {
  return invoke("get_schedule_c_preview", { year });
}

// ── Contact commands ─────────────────────────────────────────────────────────

export interface ContactRecord {
  id: number;
  name: string;
  email: string | null;
  phone: string | null;
  address: string | null;
  contact_type: string;
  is_contractor: boolean;
  tax_id: string | null;
  notes: string | null;
  created_at: string;
}

export interface ContactInput {
  name: string;
  email?: string;
  phone?: string;
  address?: string;
  contact_type: string;
  is_contractor: boolean;
  tax_id?: string;
  notes?: string;
}

export function getContacts(): Promise<ContactRecord[]> {
  return invoke("get_contacts");
}

export function createContact(input: ContactInput): Promise<ContactRecord> {
  return invoke("create_contact", { input });
}

// ── Invoice commands ─────────────────────────────────────────────────────────

export interface InvoiceRecord {
  id: number;
  invoice_number: string;
  contact_id: number;
  status_type: string;
  status_data: string | null;
  issue_date: string;
  due_date: string;
  discount_type: string | null;
  discount_value: number | null;
  notes: string | null;
  terms: string | null;
  created_at: string;
  updated_at: string;
}

export interface InvoiceInput {
  invoice_number: string;
  contact_id: number;
  issue_date: string;
  due_date: string;
  notes?: string;
  terms?: string;
}

export interface PaymentInput {
  invoice_id: number;
  amount_cents: number;
  date: string;
  method?: string;
}

export interface NecSummaryEntry {
  contact_id: number;
  contact_name: string;
  ytd_cents: number;
  over_threshold: boolean;
}

export function getInvoices(): Promise<InvoiceRecord[]> {
  return invoke("get_invoices");
}

export function createInvoice(input: InvoiceInput): Promise<InvoiceRecord> {
  return invoke("create_invoice", { input });
}

export function getInvoiceAging(): Promise<InvoiceRecord[]> {
  return invoke("get_invoice_aging");
}

export function recordInvoicePayment(input: PaymentInput): Promise<number> {
  return invoke("record_invoice_payment", { input });
}

export function get1099Summary(year?: number): Promise<NecSummaryEntry[]> {
  return invoke("get_1099_summary", { year });
}

export interface SendInvoiceInput {
  invoice_id: number;
  subject?: string;
}

export interface DeliveryResult {
  recipient: string;
  invoice_number: string;
  backend: string;
}

export function sendInvoice(input: SendInvoiceInput): Promise<DeliveryResult> {
  return invoke("send_invoice", { input });
}

// ── Export commands ──────────────────────────────────────────────────────────

export function exportBeancount(): Promise<string> {
  return invoke("export_beancount");
}

export function exportQif(): Promise<string> {
  return invoke("export_qif");
}

// ── Settings commands ───────────────────────────────────────────────────────

export function getSetting(key: string): Promise<string | null> {
  return invoke("get_setting", { key });
}

export function setSetting(key: string, value: string): Promise<void> {
  return invoke("set_setting", { key, value });
}

// ── Audit log ───────────────────────────────────────────────────────────────

export interface AuditLogRecord {
  id: number;
  timestamp: string;
  tool_name: string;
  input_hash: string | null;
  outcome: string;
  details: string | null;
}

export function getAuditLog(limit?: number): Promise<AuditLogRecord[]> {
  return invoke("get_audit_log", { limit });
}

// ── Update commands ────────────────────────────────────────────────────────

export interface UpdateStatus {
  update_available: boolean;
  current_version: string;
  latest_version: string | null;
}

export function checkForUpdates(): Promise<UpdateStatus> {
  return invoke("check_for_updates");
}
