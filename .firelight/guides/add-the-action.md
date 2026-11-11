---
format: 1
title: Check docs in CI with the GitHub Action
summary: The Firelight action runs check, preview --check and covers on every pull request.
aliases: [github action, ci, workflow, continuous integration]
tags: [ci]
related: [check-a-repo, covers]
---
1. Run `firelight skills ci` for the workflow file.
2. Check out with `fetch-depth: 0` so `covers` sees the base.
3. Pin the action and `version` to a release.
4. `covers` is strict by default: a flagged entry fails the PR until it is
   updated or acknowledged with `Firelight-Covers: checked <repo>/<entry>` in
   a commit message. `strict-covers: "false"` only warns.
