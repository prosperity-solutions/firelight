---
format: 1
title: The .firelight/ folder
summary: The small folder at a repo's root that describes the repo to Firelight; git is where it lives.
aliases: [firelight folder, .firelight, format]
tags: [format]
related: [check-a-repo]
---
A repo describes itself in `.firelight/`: `repo.yaml` (who it is and how it
relates to other repos), `links.yaml`, `islands/`, `concepts/`, `guides/`,
`golden.yaml` and `spaces.yaml`. Every YAML file starts with `format: 1`.

## What it is not

It is not a wiki. Nothing is edited in Firelight; changes are pull requests
to the repo, reviewed like code.
