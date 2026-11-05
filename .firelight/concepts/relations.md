---
format: 1
title: Relations
summary: Rules between repos in repo.yaml, like "when you change the schema here, also update the client there".
aliases: [relations, depends on, related repos]
tags: [format]
related: [firelight-folder]
---
`relations: [{ repo: app-repo, when: "you change the API", then: "update the client" }]`.
Agents act on them, so write only the ones that really hold.
