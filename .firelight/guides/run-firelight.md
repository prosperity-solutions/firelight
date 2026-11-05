---
format: 1
title: Run a Firelight server
summary: Run the container or binary with your repos as sources, an auth mode and a public URL.
aliases: [deploy, self-host, install server, kubernetes, helm, docker]
tags: [operations]
related: [last-good, mcp-server]
---
1. Pick sources (`FIRELIGHT_SOURCES`), `FIRELIGHT_SERVER_PUBLIC_URL`, and
   auth for the page and MCP (`FIRELIGHT_AUTH_UI`, `FIRELIGHT_AUTH_MCP`).
2. Run the image or `firelight serve`; the Helm chart sets the rest.
3. `firelight config check` lists every problem before you start.
4. `firelight skills config` lists every setting.
