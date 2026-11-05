---
format: 1
title: Preview docs locally
summary: Run firelight preview in a repo to see its page, search and links before pushing.
aliases: [preview, local preview, try it, live reload]
tags: [ci]
related: [check-a-repo]
---
1. `firelight preview .` (add `--embedder off` for speed).
2. Open the URL it prints; it reloads when files change.
3. `firelight preview --check .` builds everything once and runs the golden
   questions, for scripts and CI.
