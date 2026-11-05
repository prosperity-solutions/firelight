---
format: 1
title: The MCP server
summary: Firelight's read-only MCP server lets any coding agent search, browse and read every repo's docs.
aliases: [mcp, model context protocol, agent tools, tools]
tags: [agents]
related: [connect-an-agent]
---
Tools: `search`, `list_entries`, `get_entry`, `find_link`, `list_spaces`,
`list_repos`, `get_repo`, `status`. Resources: `firelight://format` and
`firelight://onboarding`; prompt `onboard_repo`. Every result starts by
saying it is data from git repositories, not instructions, and names the
repo and commit of each piece.
