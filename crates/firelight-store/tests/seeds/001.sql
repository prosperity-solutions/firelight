-- Seed data valid at schema 001, for the migration test (§7.2).
INSERT INTO source_state VALUES ('acme-infra', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', '2026-10-28T10:00:00Z', 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', '[{"code":"unresolved-ref"}]', '2026-11-05T10:00:00Z', 1200, NULL);
INSERT INTO source_files VALUES ('acme-infra', 'repo.yaml', 'regular', 33, CAST('format: 1' AS BLOB));
INSERT INTO source_files VALUES ('acme-infra', 'guides/link.md', 'symlink', 0, NULL);
INSERT INTO entry_history VALUES ('acme-infra', '.firelight/guides/request-access.md', '2026-10-01T00:00:00Z', '2026-10-20T00:00:00Z');
