-- 002: the embedding cache (docs/HANDOVER.md §7.1).
-- One vector per (embedder, dims, chunk text hash): a changed chunk or a
-- changed model re-embeds only what's missing. `model` is the embedder's id
-- (backend, model, revision or base URL); `vec` is little-endian f32.
-- Keep this table in every later migration, so upgrades don't re-embed.
CREATE TABLE embeddings (
    model TEXT NOT NULL,
    dims  INTEGER NOT NULL CHECK (dims > 0),
    hash  BLOB NOT NULL,
    vec   BLOB NOT NULL CHECK (length(vec) = dims * 4),
    PRIMARY KEY (model, dims, hash)
) STRICT;
