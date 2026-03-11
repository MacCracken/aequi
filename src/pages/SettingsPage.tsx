import { useEffect, useState } from "react";
import { getAuditLog, getSetting, setSetting, type AuditLogRecord } from "../lib/api";

export function SettingsPage() {
  const [mcpEnabled, setMcpEnabled] = useState(false);
  const [readOnly, setReadOnly] = useState(false);
  const [auditLog, setAuditLog] = useState<AuditLogRecord[]>([]);

  useEffect(() => {
    getSetting("mcp_enabled").then((v) => setMcpEnabled(v === "true")).catch(() => {});
    getSetting("mcp_read_only").then((v) => setReadOnly(v === "true")).catch(() => {});
    getAuditLog(50).then(setAuditLog).catch(console.error);
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

  return (
    <div className="space-y-6">
      <h2 className="text-xl font-semibold">Settings</h2>

      <section className="space-y-3">
        <h3 className="font-medium">MCP Server</h3>
        <label className="flex items-center gap-3 text-sm">
          <input type="checkbox" checked={mcpEnabled} onChange={toggleMcp} className="rounded" />
          Enable MCP server
        </label>
        <label className="flex items-center gap-3 text-sm">
          <input type="checkbox" checked={readOnly} onChange={toggleReadOnly} className="rounded" />
          Read-only mode (disables all write tools)
        </label>
      </section>

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
                      <span className={entry.outcome === "success" ? "text-green-600" : "text-red-600"}>
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
