import { useEffect, useRef, useState } from "react";
import {
  getAuditLog,
  getSetting,
  setSetting,
  exportBeancount,
  exportQif,
  checkForUpdates,
  errorMessage,
  type AuditLogRecord,
  type UpdateStatus,
} from "../lib/api";
import { useToast } from "../components/Toast";

// Delay before releasing an export's object URL; revoking it synchronously
// after click() can cancel the download in some webviews.
const REVOKE_DELAY_MS = 60_000;

export function SettingsPage() {
  const [settingsLoaded, setSettingsLoaded] = useState(false);
  // Defaults mirror the MCP server: unset mcp_enabled means enabled,
  // unset mcp_read_only means read-write.
  const [mcpEnabled, setMcpEnabled] = useState(true);
  const [readOnly, setReadOnly] = useState(false);
  const [auditLog, setAuditLog] = useState<AuditLogRecord[]>([]);
  const [businessName, setBusinessName] = useState("");
  const [businessEin, setBusinessEin] = useState("");
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [savingKey, setSavingKey] = useState<string | null>(null);
  const savingRef = useRef(false);
  const { toast } = useToast();

  useEffect(() => {
    let cancelled = false;
    Promise.all([
      getSetting("mcp_enabled"),
      getSetting("mcp_read_only"),
      getSetting("business_name"),
      getSetting("business_ein"),
    ])
      .then(([enabled, ro, name, ein]) => {
        if (cancelled) return;
        setMcpEnabled(enabled !== "false");
        setReadOnly(ro === "true");
        setBusinessName(name ?? "");
        setBusinessEin(ein ?? "");
        setSettingsLoaded(true);
      })
      .catch((e) => {
        if (!cancelled) toast("error", `Failed to load settings: ${errorMessage(e)}`);
      });
    getAuditLog(50)
      .then((log) => { if (!cancelled) setAuditLog(log); })
      .catch((e) => {
        if (!cancelled) toast("error", `Failed to load audit log: ${errorMessage(e)}`);
      });
    return () => { cancelled = true; };
  }, [toast]);

  /** Persist one setting; resolves true on success, toasts and resolves false on failure. */
  async function saveSetting(key: string, value: string): Promise<boolean> {
    if (savingRef.current) return false;
    savingRef.current = true;
    setSavingKey(key);
    try {
      await setSetting(key, value);
      return true;
    } catch (e) {
      toast("error", `Failed to save setting: ${errorMessage(e)}`);
      return false;
    } finally {
      savingRef.current = false;
      setSavingKey(null);
    }
  }

  // The checkboxes are controlled, so they only flip once the save succeeded.
  const toggleMcp = async () => {
    const next = !mcpEnabled;
    if (await saveSetting("mcp_enabled", String(next))) setMcpEnabled(next);
  };

  const toggleReadOnly = async () => {
    const next = !readOnly;
    if (await saveSetting("mcp_read_only", String(next))) setReadOnly(next);
  };

  const saveBusinessName = async () => {
    if (await saveSetting("business_name", businessName)) {
      toast("success", "Business name saved");
    }
  };

  const saveBusinessEin = async () => {
    if (await saveSetting("business_ein", businessEin)) {
      toast("success", "Business EIN saved");
    }
  };

  async function handleExport(format: "beancount" | "qif") {
    if (exporting) return;
    setExporting(true);
    try {
      const content = format === "beancount" ? await exportBeancount() : await exportQif();
      const ext = format === "beancount" ? "beancount" : "qif";
      const blob = new Blob([content], { type: "text/plain;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `aequi-export.${ext}`;
      a.style.display = "none";
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(url), REVOKE_DELAY_MS);
      toast("success", `Exported as ${format}`);
    } catch (e) {
      toast("error", `Export failed: ${errorMessage(e)}`);
    } finally {
      setExporting(false);
    }
  }

  async function handleCheckUpdates() {
    if (checkingUpdates) return;
    setCheckingUpdates(true);
    setUpdateError(null);
    try {
      const status = await checkForUpdates();
      setUpdateStatus(status);
      if (status.update_available) {
        toast("info", `Update available: ${status.latest_version}`);
      } else {
        toast("success", "You're on the latest version");
      }
    } catch (e) {
      // A failed check says nothing about whether we're up to date.
      const msg = errorMessage(e);
      setUpdateStatus(null);
      setUpdateError(msg);
      toast("error", `Update check failed: ${msg}`);
    } finally {
      setCheckingUpdates(false);
    }
  }

  const saveDisabled = !settingsLoaded || savingKey !== null;

  return (
    <div className="space-y-6">
      <h2 className="text-xl font-semibold">Settings</h2>

      {/* Business Profile */}
      <section className="space-y-3">
        <h3 className="font-medium">Business Profile</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <div>
            <label htmlFor="settings-businessName" className="block text-xs text-text-muted mb-1">Business Name</label>
            <div className="flex gap-2">
              <input
                id="settings-businessName"
                type="text"
                value={businessName}
                onChange={(e) => setBusinessName(e.target.value)}
                placeholder="Your Business LLC"
                className="flex-1 px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
              />
              <button
                onClick={saveBusinessName}
                disabled={saveDisabled}
                className="px-3 py-1.5 text-sm bg-primary text-white rounded-md hover:bg-primary-hover disabled:opacity-50"
              >
                Save
              </button>
            </div>
          </div>
          <div>
            <label htmlFor="settings-ein" className="block text-xs text-text-muted mb-1">EIN / Tax ID</label>
            <div className="flex gap-2">
              <input
                id="settings-ein"
                type="text"
                value={businessEin}
                onChange={(e) => setBusinessEin(e.target.value)}
                placeholder="XX-XXXXXXX"
                className="flex-1 px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
              />
              <button
                onClick={saveBusinessEin}
                disabled={saveDisabled}
                className="px-3 py-1.5 text-sm bg-primary text-white rounded-md hover:bg-primary-hover disabled:opacity-50"
              >
                Save
              </button>
            </div>
          </div>
        </div>
      </section>

      {/* Export */}
      <section className="space-y-3">
        <h3 className="font-medium">Export Data</h3>
        <div className="flex gap-2">
          <button
            onClick={() => handleExport("beancount")}
            disabled={exporting}
            className="px-4 py-2 text-sm font-medium border border-border rounded-md hover:bg-bg transition-colors disabled:opacity-50"
          >
            Export Beancount
          </button>
          <button
            onClick={() => handleExport("qif")}
            disabled={exporting}
            className="px-4 py-2 text-sm font-medium border border-border rounded-md hover:bg-bg transition-colors disabled:opacity-50"
          >
            Export QIF
          </button>
        </div>
      </section>

      {/* MCP Server */}
      <section className="space-y-3">
        <h3 className="font-medium">MCP Server</h3>
        <label htmlFor="settings-mcpEnabled" className="flex items-center gap-3 text-sm">
          <input
            id="settings-mcpEnabled"
            type="checkbox"
            checked={mcpEnabled}
            onChange={toggleMcp}
            disabled={saveDisabled}
            className="rounded"
          />
          Enable MCP server
        </label>
        <label htmlFor="settings-readOnly" className="flex items-center gap-3 text-sm">
          <input
            id="settings-readOnly"
            type="checkbox"
            checked={readOnly}
            onChange={toggleReadOnly}
            disabled={saveDisabled}
            className="rounded"
          />
          Read-only mode (disables all write tools)
        </label>
        <p className="text-xs text-text-muted">
          Changes apply to the next MCP tool call; no restart needed.
        </p>
      </section>

      {/* Updates */}
      <section className="space-y-3">
        <h3 className="font-medium">Updates</h3>
        <button
          onClick={handleCheckUpdates}
          disabled={checkingUpdates}
          className="px-4 py-2 text-sm font-medium border border-border rounded-md hover:bg-bg transition-colors disabled:opacity-50"
        >
          {checkingUpdates ? "Checking..." : "Check for Updates"}
        </button>
        {updateError && (
          <p className="text-sm text-danger">Update check failed: {updateError}</p>
        )}
        {updateStatus && (
          <p className="text-sm text-text-muted">
            Current version: {updateStatus.current_version}
            {updateStatus.update_available && updateStatus.latest_version
              ? ` — Update available: ${updateStatus.latest_version}`
              : " — Up to date"}
          </p>
        )}
      </section>

      {/* Audit Log */}
      <section>
        <h3 className="font-medium mb-2">Audit Log</h3>
        {auditLog.length === 0 ? (
          <p className="text-text-muted text-sm">No audit entries.</p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-xs">
              <thead>
                <tr className="text-left text-text-muted border-b border-border">
                  <th className="py-1 pr-3">Time</th>
                  <th className="py-1 pr-3">Tool</th>
                  <th className="py-1 pr-3">Outcome</th>
                </tr>
              </thead>
              <tbody>
                {auditLog.map((entry) => (
                  <tr key={entry.id} className="border-b border-border/50">
                    <td className="py-1 pr-3 font-mono">{entry.timestamp}</td>
                    <td className="py-1 pr-3">{entry.tool_name}</td>
                    <td className="py-1 pr-3">
                      <span
                        className={
                          entry.outcome === "success" ? "text-green-600" : "text-red-600"
                        }
                      >
                        {entry.outcome}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </div>
  );
}
