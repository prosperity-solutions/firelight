---
format: 1
title: Connect a coding agent
summary: Add Firelight's MCP server to Claude Code or any MCP client, so agents know every repo.
aliases: [connect claude code, mcp add, agent setup, claude mcp]
tags: [agents]
related: [mcp-server]
---
1. In Claude Code:
   `claude mcp add --transport http firelight https://<your firelight>/mcp --header "Authorization: Bearer <token>"`
2. Ask the operator for a token when MCP auth is `token`.
3. Ask a question; the agent calls `search` first.
