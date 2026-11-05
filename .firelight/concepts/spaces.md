---
format: 1
title: Spaces
summary: A space is a page of its own, with its own islands, top row, footer and search, for one audience.
aliases: [space, pages, audiences, spaces.yaml]
tags: [format]
related: [islands, firelight-folder]
---
Group repos and topics for one audience: `platform`, `support`, `data`.
Any repo may declare a space in `spaces.yaml`; the first declaration (in
source order) decides its title. A repo's `spaces` say where its islands and
entries go; an entry can override them. `all` shows every space but the
built-in `firelight` one.

## What it is not

Not access control: everyone who can open Firelight sees every space.
