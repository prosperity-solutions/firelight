---
format: 1
title: Maybe stale (covers)
summary: An entry's covers name the code it describes; when that code changes and the entry doesn't, the entry is marked maybe stale.
aliases: [covers, stale, staleness, outdated docs]
tags: [format, ci]
related: [add-the-action]
---
`covers: [terraform/access/**]` in an entry's front matter. Firelight
compares git history only: the newest commit touching a covered path vs the
newest touching the entry. In a pull request, `firelight covers --base
origin/main` lists the entries to update.
