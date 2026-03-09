import { useEffect, useState } from "react";
import { getAccounts, type Account } from "../lib/api";

export function AccountsPage() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getAccounts().then(setAccounts).catch((e) => setError(String(e)));
  }, []);

  if (error) {
    return <p className="text-danger">Error loading accounts: {error}</p>;
  }

  const grouped = accounts.reduce<Record<string, Account[]>>((acc, a) => {
    (acc[a.account_type] ??= []).push(a);
    return acc;
  }, {});

  return (
    <div className="space-y-6">
      <h2 className="text-xl font-semibold">Chart of Accounts</h2>
      {Object.entries(grouped).map(([type, accts]) => (
        <section key={type} className="bg-surface rounded-lg border border-border">
          <h3 className="px-4 py-2 text-sm font-medium text-text-muted border-b border-border uppercase tracking-wide">
            {type}
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
      ))}
    </div>
  );
}
