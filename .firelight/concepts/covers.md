---
format: 1
title: Maybe stale (covers)
summary: An entry's covers name the code it describes; when that code changes and the entry doesn't, the entry is marked maybe stale.
aliases: [covers, stale, staleness, outdated docs, Firelight-Covers, covers trailer, firelight-covers-checked]
tags: [format, ci]
related: [add-the-action]
---
`covers: [terraform/access/**]` in an entry's front matter. Firelight
compares git history only: the newest commit touching a covered path vs the
newest touching the entry. In a pull request, `firelight covers --base
origin/main` lists the entries to update; the GitHub Action fails the PR
until each is updated, or a commit message carries the trailer
`Firelight-Covers: checked <repo>/<entry>` (`checked` alone for all), or the
PR has the `firelight-covers-checked` label. `firelight covers --report`
shows the top-level paths no entry covers.

## What it is not
Not a content check: it never reads the covered files, only git history.
