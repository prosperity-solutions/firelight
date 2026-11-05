---
format: 1
title: Golden questions
summary: Questions people really type, with the entries that must answer them; they keep search honest.
aliases: [golden.yaml, golden set, search tests]
tags: [search, ci]
related: [search]
---
`golden.yaml` lists questions and the entries that must rank within the top
`within` results. `needs: meaning` marks questions only meaning search can
answer; with the embedder off they are skipped, never failed. `firelight
preview --check` runs them in a repo's CI; the server runs each repo's after
every re-index and warns. Firelight's own are its readiness check.
