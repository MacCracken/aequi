import { NavLink, Outlet } from "react-router-dom";

const links = [
  { to: "/accounts", label: "Accounts" },
  { to: "/transactions", label: "Transactions" },
  { to: "/receipts", label: "Receipts" },
];

export function AppShell() {
  return (
    <div className="min-h-screen flex flex-col">
      <header className="bg-surface border-b border-border px-4 py-3 flex items-center gap-6">
        <h1 className="text-lg font-semibold tracking-tight">Aequi</h1>
        <nav className="flex gap-1">
          {links.map((link) => (
            <NavLink
              key={link.to}
              to={link.to}
              className={({ isActive }) =>
                `px-3 py-1.5 rounded-md text-sm font-medium transition-colors ${
                  isActive
                    ? "bg-primary text-white"
                    : "text-text-muted hover:bg-border/50"
                }`
              }
            >
              {link.label}
            </NavLink>
          ))}
        </nav>
      </header>
      <main className="flex-1 p-4 md:p-6 max-w-6xl w-full mx-auto">
        <Outlet />
      </main>
    </div>
  );
}
