# ADR-011: MCP Server with Tool Registry and Permission System

## Status
Accepted

## Context

Phase 7 implements a Model Context Protocol (MCP) server so AI agents (Claude Desktop, custom agents) can query and operate the accounting system. Design decisions:

1. **Transport** — stdio vs HTTP vs WebSocket
2. **Tool organization** — flat list vs registry pattern
3. **Security** — how to control which tools an agent can access
4. **Audit** — tracking what AI agents do

## Decision

### Stdio transport

The MCP server reads newline-delimited JSON-RPC 2.0 from stdin and writes responses to stdout. This matches the MCP specification and works with Claude Desktop's sidecar model. The server handles three methods: `initialize`, `tools/list`, and `tools/call`.

### Tool registry pattern

`ToolRegistry` uses a generic `register()` method accepting async closures stored as trait objects. Each tool entry has a `ToolDefinition` (name, description, JSON Schema for inputs), an `is_write` flag, and a handler function. Tools are organized by domain in separate modules:

- `accounts` (2), `transactions` (3), `receipts` (4), `tax` (3), `invoices` (3), `rules` (3), `import` (3), `reconciliation` (3) = 24 tools total

### Permission system

`Permissions` struct with:
- `read_only: bool` — blocks all write tools when enabled
- `disabled_tools: HashSet<String>` — per-tool blocklist

Permissions are checked before every tool call. The Settings UI in the frontend exposes these toggles.

### Audit logging

Every tool call is logged to the `audit_log` table with:
- Tool name, timestamp, outcome (success/error)
- SHA-256 hash of the input parameters (not raw inputs, for privacy)

The audit log is viewable in the Settings page.

## Consequences

- **Pros:**
  - Registry pattern makes adding new tools trivial (one `register()` call)
  - Permission system prevents accidental writes from AI agents
  - Audit log provides accountability for agent actions
  - Stdio transport requires no network configuration

- **Cons:**
  - Stdio transport means only one client at a time
  - No streaming/SSE support in v1
  - Input hashing means audit log can't replay exact parameters

## References
- Model Context Protocol specification: https://modelcontextprotocol.io
- JSON-RPC 2.0: https://www.jsonrpc.org/specification
