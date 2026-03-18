import { useEffect, useState } from "react";
import {
  getAuditLog,
  getSetting,
  setSetting,
  exportBeancount,
  exportQif,
  checkForUpdates,
  type AuditLogRecord,
  type UpdateStatus,
} from "../lib/api";
import { useToast } from "../components/Toast";

export function SettingsPage() {
  const [mcpEnabled, setMcpEnabled] = useState(false);
  const [readOnly, setReadOnly] = useState(false);
  const [auditLog, setAuditLog] = useState<AuditLogRecord[]>([]);
  const [businessName, setBusinessName] = useState("");
  const [businessEin, setBusinessEin] = useState("");
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus | null>(null);
  const { toast } = useToast();

  useEffect(() => {
    getSetting("mcp_enabled").then((v) => setMcpEnabled(v === "true")).catch(() => {});
    getSetting("mcp_read_only").then((v) => setReadOnly(v === "true")).catch(() => {});
    getSetting("business_name").then((v) => v && setBusinessName(v)).catch(() => {});
    getSetting("business_ein").then((v) => v && setBusinessEin(v)).catch(() => {});
    getAuditLog(50).then(setAuditLog).catch(() => {});
  }, []);

  const toggleMcp = async () => {
    const next = !mcpEnabled;
    await setSetting("mcp_enabled", String(next));
    setMcpEnabled(next);
  };

  const toggleReadOnly = async () => {
    const next = !readOnly;
    await setSetting("mcp_read_only", String(next));
    setReadOnly(next);
  };

  const saveBusinessName = async () => {
    await setSetting("business_name", businessName);
    toast("success", "Business name saved");
  };

  const saveBusinessEin = async () => {
    await setSetting("business_ein", businessEin);
    toast("success", "Business EIN saved");
  };

  async function handleExport(format: "beancount" | "qif") {
    try {
      const content = format === "beancount" ? await exportBeancount() : await exportQif();
      const ext = format === "beancount" ? "beancount" : "qif";
      const blob = new Blob([content], { type: "text/plain" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `aequi-export.${ext}`;
      a.click();
      URL.revokeObjectURL(url);
      toast("success", `Exported as ${format}`);
    } catch (e) {
      toast("error", String(e));
    }
  }

  async function handleCheckUpdates() {
    try {
      const status = await checkForUpdates();
      setUpdateStatus(status);
      if (status.update_available) {
        toast("info", `Update available: ${status.latest_version}`);
      } else {
        toast("success", "You're on the latest version");
      }
    } catch (e) {
      toast("error", String(e));
    }
  }

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
                className="px-3 py-1.5 text-sm bg-primary text-white rounded-md hover:bg-primary-hover"
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
                className="px-3 py-1.5 text-sm bg-primary text-white rounded-md hover:bg-primary-hover"
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
            className="px-4 py-2 text-sm font-medium border border-border rounded-md hover:bg-bg transition-colors"
          >
            Export Beancount
          </button>
          <button
            onClick={() => handleExport("qif")}
            className="px-4 py-2 text-sm font-medium border border-border rounded-md hover:bg-bg transition-colors"
          >
            Export QIF
          </button>
        </div>
      </section>

      {/* MCP Server */}
      <section className="space-y-3">
        <h3 className="font-medium">MCP Server</h3>
        <label htmlFor="settings-mcpEnabled" className="flex items-center gap-3 text-sm">
          <input id="settings-mcpEnabled" type="checkbox" checked={mcpEnabled} onChange={toggleMcp} className="rounded" />
          Enable MCP server
        </label>
        <label htmlFor="settings-readOnly" className="flex items-center gap-3 text-sm">
          <input id="settings-readOnly" type="checkbox" checked={readOnly} onChange={toggleReadOnly} className="rounded" />
          Read-only mode (disables all write tools)
        </label>
      </section>

      {/* Updates */}
      <section className="space-y-3">
        <h3 className="font-medium">Updates</h3>
        <button
          onClick={handleCheckUpdates}
          className="px-4 py-2 text-sm font-medium border border-border rounded-md hover:bg-bg transition-colors"
        >
          Check for Updates
        </button>
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
