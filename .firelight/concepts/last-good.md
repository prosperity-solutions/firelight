---
format: 1
title: Last good commit
summary: A commit whose .firelight/ doesn't pass its checks is never served; the last good one stays, and the status page says why.
aliases: [last good, broken commit, served commit]
tags: [operations]
related: [run-firelight]
---
Firelight pulls each source, validates the newest commit, and only then
serves it. A broken commit keeps the previous good one online; the status
page names the file, line and problem.
