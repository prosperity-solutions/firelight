---
format: 1
title: Template islands
summary: Islands that build links from what a reader types, like a PR number, checked so a value can never change where a link points.
aliases: [template, preview links, PR previews, inputs]
tags: [format]
related: [islands]
---
Each input has a `name` and an anchored `pattern` (`^[0-9]{1,6}$`). A
placeholder may sit only in the host's first label, as one whole path
segment, or as one query value, so a value can't change the scheme or the
registered domain. The island works without JavaScript.

## What it is not

Not a way to reach arbitrary URLs: every link is fixed in git except the
checked placeholders.
