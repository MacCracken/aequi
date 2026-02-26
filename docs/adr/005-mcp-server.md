# ADR-005: MCP Server for AI Integration

## Status
Accepted

## Context
Need to expose accounting data and operations to AI agents for automation.

## Decision
Implement an **MCP (Model Context Protocol) server**:
- Runs as stdio subprocess (default) or Unix socket
- Shares same core and storage crates as main app
- Zero duplication of business logic
- Exposes tools: categorize, extract_receipt, draft_invoice, etc.

## Consequences
- **Pros:**
  - Standard protocol for AI-tool integration
  - Same code used by desktop app and MCP server
  - Read-only mode and per-tool permissions
  
- **Cons:**
  - Custom implementation required (no stable Rust MCP library)
  - Security considerations for local AI access

## References
- [Model Context Protocol](https://modelcontextprotocol.io)
