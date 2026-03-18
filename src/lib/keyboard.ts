import { useEffect } from "react";
import { useNavigate } from "react-router-dom";

/** Global keyboard shortcuts (Ctrl+0..7 for nav, Ctrl+/ for help). */
export function useKeyboardShortcuts() {
  const navigate = useNavigate();

  useEffect(() => {
    const routes = [
      "/",
      "/accounts",
      "/transactions",
      "/receipts",
      "/tax",
      "/invoices",
      "/contacts",
      "/settings",
    ];

    function handler(e: KeyboardEvent) {
      // Ignore when typing in an input/textarea
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

      const ctrl = e.ctrlKey || e.metaKey;

      // Ctrl+0..7 navigate to pages
      if (ctrl && e.key >= "0" && e.key <= "7") {
        e.preventDefault();
        const idx = parseInt(e.key);
        navigate(routes[idx]);
        return;
      }

      // Ctrl+/ toggle shortcut help (dispatches custom event)
      if (ctrl && e.key === "/") {
        e.preventDefault();
        window.dispatchEvent(new CustomEvent("aequi:toggle-shortcuts"));
        return;
      }
    }

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [navigate]);
}

/** All registered shortcuts for display in the help overlay. */
export const SHORTCUT_LIST: { keys: string; description: string }[] = [
  { keys: "Ctrl+0", description: "Dashboard" },
  { keys: "Ctrl+1", description: "Accounts" },
  { keys: "Ctrl+2", description: "Transactions" },
  { keys: "Ctrl+3", description: "Receipts" },
  { keys: "Ctrl+4", description: "Tax" },
  { keys: "Ctrl+5", description: "Invoices" },
  { keys: "Ctrl+6", description: "Contacts" },
  { keys: "Ctrl+7", description: "Settings" },
  { keys: "Ctrl+/", description: "Toggle shortcut help" },
];
