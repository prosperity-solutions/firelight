---
format: 1
title: Search
summary: Search ranks by words and by meaning, in any language, and opens a hit at the right section.
aliases: [meaning search, embeddings, semantic search, find]
tags: [search]
related: [golden-questions]
---
Words: SQLite FTS5 with bm25 over title, aliases, summary and body. Meaning:
an embedding model (by default Qwen3-Embedding-0.6B, in-process on the CPU)
compares the question with every section. The two lists are fused, and an
exact title or alias match comes first. `FIRELIGHT_EMBEDDER=off` keeps words
only, which is fully supported.
