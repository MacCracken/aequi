import { NavLink, Outlet } from "react-router-dom";

const links = [
  { to: "/accounts", label: "Accounts", icon: "M3 6h18M3 12h18M3 18h18" },
  { to: "/transactions", label: "Transactions", icon: "M12 8c-1.657 0-3 .895-3 2s1.343 2 3 2 3 .895 3 2-1.343 2-3 2m0-8c1.11 0 2.08.402 2.599 1M12 8V7m0 1v8m0 0v1m0-1c-1.11 0-2.08-.402-2.599-1" },
  { to: "/receipts", label: "Receipts", icon: "M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" },
];

function NavIcon({ d }: { d: string }) {
  return (
    <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
      <path strokeLinecap="round" strokeLinejoin="round" d={d} />
    </svg>
  );
}

export function AppShell() {
  return (
    <div className="min-h-screen flex flex-col">
      {/* Desktop top nav */}
      <header className="hidden md:flex bg-surface border-b border-border px-4 py-3 items-center gap-6">
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

      <main className="flex-1 p-4 md:p-6 max-w-6xl w-full mx-auto pb-20 md:pb-6">
        <Outlet />
      </main>

      {/* Mobile bottom nav */}
      <nav className="md:hidden fixed bottom-0 inset-x-0 bg-surface border-t border-border flex justify-around py-2 safe-bottom z-50">
        {links.map((link) => (
          <NavLink
            key={link.to}
            to={link.to}
            className={({ isActive }) =>
              `flex flex-col items-center gap-0.5 px-3 py-1 rounded-lg text-xs font-medium transition-colors min-w-[64px] ${
                isActive ? "text-primary" : "text-text-muted"
              }`
            }
          >
            <NavIcon d={link.icon} />
            {link.label}
          </NavLink>
        ))}
      </nav>
    </div>
  );
}
