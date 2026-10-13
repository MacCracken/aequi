export function formatCents(cents: number): string {
  if (!Number.isFinite(cents)) return "—";
  const whole = Math.round(cents);
  const negative = whole < 0;
  const abs = Math.abs(whole);
  const dollars = Math.floor(abs / 100);
  const remainder = abs % 100;
  const formatted = `$${dollars.toLocaleString("en-US")}.${String(remainder).padStart(2, "0")}`;
  return negative ? `(${formatted})` : formatted;
}

const DATE_ONLY = /^(\d{4})-(\d{2})-(\d{2})$/;

/** Parse a YYYY-MM-DD string as local midnight (never UTC). */
export function parseLocalDate(iso: string): Date | null {
  const m = DATE_ONLY.exec(iso);
  if (!m) return null;
  const [year, month, day] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const date = new Date(year, month - 1, day);
  // Reject dates the Date constructor would silently roll over (e.g. Feb 31).
  if (date.getMonth() !== month - 1 || date.getDate() !== day) return null;
  return date;
}

export function formatDate(iso: string): string {
  const date = parseLocalDate(iso);
  if (!date) return iso;
  return date.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

/** Today's date on the local calendar as YYYY-MM-DD. */
export function todayLocal(): string {
  const now = new Date();
  const y = now.getFullYear();
  const m = String(now.getMonth() + 1).padStart(2, "0");
  const d = String(now.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

/**
 * Whole calendar days from today (local) until `iso`: 0 on the day itself,
 * negative once it has passed. Returns null for an unparseable date.
 */
export function daysUntil(iso: string): number | null {
  const target = parseLocalDate(iso);
  if (!target) return null;
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  // Round to absorb the 23/25-hour days around DST changes.
  return Math.round((target.getTime() - today.getTime()) / 86_400_000);
}

const PLAIN_AMOUNT = /^(\d*)(?:\.(\d*))?$/;
const GROUPED_AMOUNT = /^(\d{1,3}(?:,\d{3})+)(?:\.(\d*))?$/;
// Keeps every amount (and a sum of a few) well inside Number.MAX_SAFE_INTEGER cents.
const MAX_WHOLE_DIGITS = 12;

/**
 * Parse a non-negative dollar amount typed by the user ("12", "12.5",
 * "$1,234.56") into integer cents without floating-point math.
 * Empty input is 0. Returns null for anything else: more than two decimal
 * places, misplaced commas, signs, letters, or absurdly large values.
 */
export function parseDollarsToCents(text: string): number | null {
  let s = text.trim();
  if (s.startsWith("$")) s = s.slice(1).trimStart();
  if (s === "") return 0;

  const m = PLAIN_AMOUNT.exec(s) ?? GROUPED_AMOUNT.exec(s);
  if (!m) return null;
  const whole = (m[1] ?? "").replace(/,/g, "");
  const frac = m[2] ?? "";
  if (whole === "" && frac === "") return null; // a lone "."
  if (frac.length > 2) return null;

  const wholeDigits = whole.replace(/^0+/, "");
  if (wholeDigits.length > MAX_WHOLE_DIGITS) return null;
  return Number(wholeDigits || "0") * 100 + Number(frac.padEnd(2, "0"));
}

export function confidenceLabel(confidence: number): string {
  if (confidence >= 0.9) return "High";
  if (confidence >= 0.7) return "Medium";
  return "Low";
}

export function confidenceColor(confidence: number): string {
  if (confidence >= 0.9) return "text-success";
  if (confidence >= 0.7) return "text-warning";
  return "text-danger";
}
