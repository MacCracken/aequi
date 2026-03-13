import { useEffect, useState } from "react";
import { SHORTCUT_LIST } from "../lib/keyboard";

export function ShortcutsOverlay() {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    function toggle() {
      setOpen((v) => !v);
    }
    window.addEventListener("aequi:toggle-shortcuts", toggle);
    return () => window.removeEventListener("aequi:toggle-shortcuts", toggle);
  }, []);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 bg-black/40 flex items-center justify-center z-[100]"
      role="dialog"
      aria-modal="true"
      aria-label="Keyboard shortcuts"
      onClick={() => setOpen(false)}
      onKeyDown={(e) => {
        if (e.key === "Escape") setOpen(false);
      }}
    >
      <div
        className="bg-surface rounded-lg shadow-xl p-6 w-80 max-w-[90vw]"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="text-lg font-semibold mb-4">Keyboard Shortcuts</h2>
        <dl className="space-y-2">
          {SHORTCUT_LIST.map((s) => (
            <div key={s.keys} className="flex justify-between items-center">
              <dt className="text-text-muted text-sm">{s.description}</dt>
              <dd>
                <kbd className="px-2 py-0.5 rounded bg-border/50 text-xs font-mono">
                  {s.keys}
                </kbd>
              </dd>
            </div>
          ))}
        </dl>
        <p className="text-text-muted text-xs mt-4">Press Esc or Ctrl+/ to close</p>
      </div>
    </div>
  );
}
