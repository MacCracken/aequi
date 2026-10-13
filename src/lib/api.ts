import { invoke } from "@tauri-apps/api/core";

/** Shape of every rejected `invoke` (see the backend's command error type). */
export interface ApiError {
  code: string;
  message: string;
}

/** Human-readable message for anything thrown by an API call or helper. */
export function errorMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "object" && e !== null && "message" in e) {
    const { message } = e as { message: unknown };
    if (typeof message === "string") return message;
  }
  if (typeof e === "string") return e;
  return String(e);
}

export interface Account {
  code: string;
  name: string;
  account_type: string;
}

export interface TransactionLineInput {
  account_code: string;
  debit_cents: number;
  credit_cents: number;
  memo?: string;
}

export interface TransactionInput {
  date: string;
  description: string;
  lines: TransactionLineInput[];
  memo?: string;
  /** Payee or payer. Contractor payments linked here count toward 1099-NEC. */
  contact_id?: number;
}

export interface TransactionOutput {
  id: number;
  date: string;
  description: string;
  balanced_total: string;
  memo: string | null;
  created_at: string;
  contact_id: number | null;
  contact_name: string | null;
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
  /** Fields were filled in by an AI agent (not OCR); they still need approval. */
  filled_by_agent: boolean;
  created_at: string;
}

export function getAccounts(): Promise<Account[]> {
  return invoke("get_accounts");
}

export function createTransaction(
  input: TransactionInput,
): Promise<TransactionOutput> {
  return invoke("create_transaction", { input });
}

export function getTransactions(
  startDate?: string,
  endDate?: string,
): Promise<TransactionOutput[]> {
  return invoke("get_transactions", { startDate, endDate });
}

export function getProfitLoss(
  startDate?: string,
  endDate?: string,
): Promise<ProfitLossEntry[]> {
  return invoke("get_profit_loss", { startDate, endDate });
}

export function ingestReceipt(filePath: string): Promise<ReceiptOutput> {
  return invoke("ingest_receipt", { filePath });
}

export function getPendingReceipts(): Promise<ReceiptOutput[]> {
  return invoke("get_pending_receipts");
}

export function approveReceipt(
  receiptId: number,
  transactionId?: number,
): Promise<void> {
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
  /** Year-to-date net profit projected to a full year. */
  annualized_net_profit_cents: number;
  standard_deduction_cents: number;
  /** Qualified business income deduction, taken after the standard deduction. */
  qbi_deduction_cents: number;
  /** Income taxed by the brackets (after the standard and QBI deductions). */
  taxable_income_cents: number;
  se_tax_cents: number;
  se_tax_deduction_cents: number;
  income_tax_cents: number;
  additional_medicare_tax_cents: number;
  /** Projected full-year tax. */
  total_tax_cents: number;
  /** The smaller of 90% of the projected tax and the prior-year requirement. */
  safe_harbor_cents: number;
  /**
   * Share of last year's tax the prior-year safe harbor requires, as a decimal
   * string: "1.00", or "1.10" when last year's AGI was over the threshold.
   */
  safe_harbor_prior_year_percentage: string;
  /**
   * Last year's tax the safe harbor is based on (the filed return's figure,
   * else the app's own projection); null when neither is known.
   */
  prior_year_tax_cents: number | null;
  /** One quarter of the projected full-year tax. */
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
  quarter?: number,
): Promise<QuarterlyEstimateOutput> {
  return invoke("estimate_quarterly_tax", { year, quarter });
}

export function getScheduleCPreview(
  year?: number,
): Promise<ScheduleCPreviewOutput> {
  return invoke("get_schedule_c_preview", { year });
}

/** Figures from a filed return, in cents; null = unknown. */
export interface TaxReturnFigures {
  /** Form 1040 line 24, "total tax". */
  total_tax_cents: number | null;
  /** Form 1040 line 11, adjusted gross income. */
  agi_cents: number | null;
}

export function getTaxReturn({ year }: { year: number }): Promise<TaxReturnFigures> {
  return invoke("get_tax_return", { year });
}

/** Store the figures from the `year` return; returns what is now stored. */
export function setTaxReturn({
  year,
  figures,
}: {
  year: number;
  figures: TaxReturnFigures;
}): Promise<TaxReturnFigures> {
  return invoke("set_tax_return", { year, figures });
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

/** Stored invoice status (`status_type`). */
export type InvoiceStatus = "Draft" | "Sent" | "Viewed" | "PartiallyPaid" | "Paid" | "Void";

export interface InvoiceRecord {
  id: number;
  invoice_number: string;
  contact_id: number;
  status_type: string;
  /** JSON with status details, e.g. `{"voided_at": …, "reason": …}`. */
  status_data: string | null;
  issue_date: string;
  due_date: string;
  /** "Percentage" (value in basis points) or "Flat" (value in cents). */
  discount_type: string | null;
  discount_value: number | null;
  notes: string | null;
  terms: string | null;
  created_at: string;
  updated_at: string;
}

/** Computed money totals of an invoice, in cents (backend is authoritative). */
export interface InvoiceTotals {
  subtotal_cents: number;
  discount_cents: number;
  tax_cents: number;
  total_cents: number;
  paid_cents: number;
  balance_cents: number;
}

export interface InvoiceSummary extends InvoiceRecord, InvoiceTotals {}

export interface InvoiceLineRecord {
  id: number;
  invoice_id: number;
  description: string;
  /** 150 = 1.5 */
  quantity_hundredths: number;
  unit_rate_cents: number;
  taxable: boolean;
  sort_order: number;
}

export interface InvoiceTaxLineRecord {
  id: number;
  invoice_id: number;
  label: string;
  /** 825 = 8.25% */
  rate_bps: number;
}

export interface InvoicePaymentRecord {
  id: number;
  invoice_id: number;
  amount_cents: number;
  date: string;
  method: string | null;
  transaction_id: number | null;
  created_at: string;
}

export interface InvoiceDetail extends InvoiceRecord, InvoiceTotals {
  lines: InvoiceLineRecord[];
  tax_lines: InvoiceTaxLineRecord[];
  payments: InvoicePaymentRecord[];
}

export interface NewInvoiceLine {
  description: string;
  /** 150 = 1.5 */
  quantity_hundredths: number;
  unit_rate_cents: number;
  taxable: boolean;
}

export interface NewInvoiceTaxLine {
  label: string;
  /** 825 = 8.25% */
  rate_bps: number;
}

export type NewDiscount =
  | { type: "percentage"; bps: number }
  | { type: "flat"; cents: number };

export interface NewInvoice {
  invoice_number: string;
  contact_id: number;
  issue_date: string;
  due_date: string;
  lines: NewInvoiceLine[];
  tax_lines: NewInvoiceTaxLine[];
  discount: NewDiscount | null;
  notes?: string;
  terms?: string;
}

export interface PaymentInput {
  invoice_id: number;
  amount_cents: number;
  date: string;
  method?: string;
  /** Asset account the money went into; the backend defaults to "1000" (Checking). */
  deposit_account_code?: string;
}

export interface NecSummaryEntry {
  contact_id: number;
  contact_name: string;
  ytd_cents: number;
  over_threshold: boolean;
  threshold_cents: number;
}

export function getInvoices(): Promise<InvoiceSummary[]> {
  return invoke("get_invoices");
}

export function getInvoice(id: number): Promise<InvoiceDetail> {
  return invoke("get_invoice", { id });
}

export function createInvoice(input: NewInvoice): Promise<InvoiceDetail> {
  return invoke("create_invoice", { input });
}

/** Replace the contents of a Draft invoice. */
export function updateInvoice(id: number, input: NewInvoice): Promise<InvoiceDetail> {
  return invoke("update_invoice", { id, input });
}

export function voidInvoice(id: number, reason: string): Promise<void> {
  return invoke("void_invoice", { id, reason });
}

/** Record a payment (posts the cash-basis ledger entry); returns the payment id. */
export function recordInvoicePayment(input: PaymentInput): Promise<number> {
  return invoke("record_invoice_payment", { input });
}

/** Payments made to contractors from bank/cash accounts in `year`. */
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

// ── Dashboard commands ──────────────────────────────────────────────────────

export interface DashboardSummary {
  ytd_income_cents: number;
  ytd_expenses_cents: number;
  ytd_net_profit_cents: number;
  /** Sent/viewed/partially paid invoices (paid and void excluded). */
  outstanding_invoices: number;
  overdue_invoices: number;
  /** Balance still owed on outstanding invoices. */
  outstanding_receivables_cents: number;
  pending_receipts: number;
  total_accounts: number;
  total_transactions: number;
  recent_transactions: TransactionOutput[];
  /** This quarter's installment minus payments already recorded. */
  quarterly_tax_due_cents: number | null;
  next_tax_due_date: string | null;
}

export function getDashboardSummary(): Promise<DashboardSummary> {
  return invoke("get_dashboard_summary");
}

// ── Update contact command ──────────────────────────────────────────────────

export interface UpdateContactInput {
  id: number;
  name: string;
  email?: string;
  phone?: string;
  address?: string;
  contact_type: string;
  is_contractor: boolean;
  tax_id?: string;
  notes?: string;
}

export function updateContact(
  input: UpdateContactInput,
): Promise<ContactRecord> {
  return invoke("update_contact", { input });
}
