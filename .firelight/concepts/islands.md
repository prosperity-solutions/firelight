---
format: 1
title: Islands
summary: The blocks of a page; cards of links, a plain list of links, or a template that builds links from input.
aliases: [island, cards, cards island, links island, card]
tags: [format]
related: [template-islands, spaces]
---
An island is a file in `islands/` with a `type`: `cards` (cards with
sections of links), `links` (one list), `template` (links built from input),
or `cards-contribution` (adds cards to another repo's island). Links pick
their place with `at:`. A link whose island doesn't exist goes into the
Unsorted card, so nothing disappears.
