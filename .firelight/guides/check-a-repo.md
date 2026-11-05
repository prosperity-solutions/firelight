---
format: 1
title: Check a repo's .firelight/ folder
summary: Run firelight check before pushing, so a broken ref or an unknown key never reaches the hub.
aliases: [validate, lint docs, firelight check]
tags: [format, ci]
related: [firelight-folder]
---
1. Run `firelight check` in the repo.
2. Fix every `path:line:col: error:` it prints.
3. Several repos at once resolve their refs to each other:
   `firelight check ../infra-repo .`

## In CI

`firelight check --strict` also fails on refs into repos it can't see.
