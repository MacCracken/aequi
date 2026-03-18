import { useEffect, useState, useMemo } from "react";
import { getAccounts, type Account } from "../lib/api";
import { TableSkeleton } from "../components/Skeleton";
import { useToast } from "../components/Toast";

export function AccountsPage() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [typeFilter, setTypeFilter] = useState("");
  const { toast } = useToast();

  useEffect(() => {
    getAccounts()
      .then(setAccounts)
      .catch((e) => toast("error", String(e)))
      .finally(() => setLoading(false));
  }, [toast]);

  const filtered = useMemo(() => {
    let result = accounts;
    if (search) {
      const q = search.toLowerCase();
      result = result.filter(
        (a) =>
          a.code.toLowerCase().includes(q) ||
          a.name.toLowerCase().includes(q)
      );
    }
    if (typeFilter) {
      result = result.filter((a) => a.account_type === typeFilter);
    }
    return result;
  }, [accounts, search, typeFilter]);

  const grouped = filtered.reduce<Record<string, Account[]>>((acc, a) => {
    (acc[a.account_type] ??= []).push(a);
    return acc;
  }, {});

  const accountTypes = [...new Set(accounts.map((a) => a.account_type))];

  if (loading) {
    return (
      <div className="space-y-6">
        <h2 className="text-xl font-semibold">Chart of Accounts</h2>
        <TableSkeleton rows={10} cols={2} />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <h2 className="text-xl font-semibold">Chart of Accounts</h2>

      {/* Filters */}
      <div className="flex gap-3">
        <input
          type="text"
          placeholder="Search accounts..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary flex-1 min-w-[200px]"
        />
        <select
          value={typeFilter}
          onChange={(e) => setTypeFilter(e.target.value)}
          className="px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
        >
          <option value="">All types</option>
          {accountTypes.map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </select>
      </div>

      {Object.keys(grouped).length === 0 ? (
        <p className="text-text-muted text-sm">No accounts match your filters.</p>
      ) : (
        Object.entries(grouped).map(([type, accts]) => (
          <section key={type} className="bg-surface rounded-lg border border-border">
            <h3 className="px-4 py-2 text-sm font-medium text-text-muted border-b border-border uppercase tracking-wide">
              {type} ({accts.length})
            </h3>
            <ul>
              {accts.map((a) => (
                <li
                  key={a.code}
                  className="px-4 py-2.5 flex justify-between border-b border-border last:border-b-0"
                >
                  <span className="font-mono text-sm text-text-muted">{a.code}</span>
                  <span className="font-medium">{a.name}</span>
                </li>
              ))}
            </ul>
          </section>
        ))
      )}
    </div>
  );
}
