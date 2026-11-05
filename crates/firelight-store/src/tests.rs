use super::*;

fn snap() -> Snapshot {
    Snapshot {
        commit: "c".repeat(40),
        committed_at: Some("2026-11-01T12:00:00Z".into()),
        files: vec![
            InputFile {
                path: "guides/l.md".into(),
                kind: FileKind::Symlink,
                contents: None,
                size: 0,
            },
            InputFile::regular("repo.yaml", b"format: 1\nid: x\n".to_vec()),
        ],
        history: vec![EntryHistory {
            entry_path: ".firelight/guides/a.md".into(),
            entry_changed: Some("2026-10-01T00:00:00Z".into()),
            covers_changed: None,
        }],
    }
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fl-store-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d.join("firelight.db")
}

#[test]
fn snapshots_round_trip() {
    let mut s = Store::memory().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(s.load_snapshot("x").ok().flatten(), None);
    s.save_snapshot("x", &snap())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(s.load_snapshot("x").ok().flatten(), Some(snap()));
    // A newer snapshot replaces the old one entirely.
    let mut n = snap();
    n.commit = "d".repeat(40);
    n.files.remove(0);
    s.save_snapshot("x", &n).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(s.load_snapshot("x").ok().flatten(), Some(n));
}

#[test]
fn pulls_and_retention() {
    let mut s = Store::memory().unwrap_or_else(|e| panic!("{e}"));
    s.save_snapshot("a", &snap())
        .unwrap_or_else(|e| panic!("{e}"));
    let st = SourceState {
        source_id: "a".into(),
        newest_commit: Some("e".repeat(40)),
        last_pull_ms: Some(12),
        last_pull_error: Some("boom".into()),
        ..Default::default()
    };
    s.record_pull(&st).unwrap_or_else(|e| panic!("{e}"));
    let all = s.source_states().unwrap_or_default();
    assert_eq!(all.len(), 1);
    assert_eq!(
        all[0].served_commit.as_deref(),
        Some("c".repeat(40).as_str()),
        "a pull keeps the served commit"
    );
    assert_eq!(all[0].last_pull_error.as_deref(), Some("boom"));
    s.save_snapshot("b", &snap())
        .unwrap_or_else(|e| panic!("{e}"));
    s.retain_sources(&["b"]).unwrap_or_else(|e| panic!("{e}"));
    assert!(s.load_snapshot("a").ok().flatten().is_none());
    assert!(s.load_snapshot("b").ok().flatten().is_some());
}

#[test]
fn deleting_the_file_loses_nothing_that_git_has() {
    // The cache is rebuilt from git: after a delete the store is empty and
    // works; the next pull fills it again (tested end to end in M3's source tests).
    let p = tmp("delete");
    {
        let mut s = Store::open(&p).unwrap_or_else(|e| panic!("{e}"));
        s.save_snapshot("x", &snap())
            .unwrap_or_else(|e| panic!("{e}"));
    }
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", p.display()));
    }
    let s = Store::open(&p).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(s.schema_version().unwrap_or(0), latest_version());
    assert!(s.load_snapshot("x").ok().flatten().is_none());
}

#[test]
fn a_newer_database_is_rebuilt() {
    let p = tmp("newer");
    {
        let mut s = Store::open(&p).unwrap_or_else(|e| panic!("{e}"));
        s.save_snapshot("x", &snap())
            .unwrap_or_else(|e| panic!("{e}"));
        s.conn
            .pragma_update(None, "user_version", 999)
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let s = Store::open(&p).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(s.schema_version().unwrap_or(0), latest_version());
    assert!(
        s.load_snapshot("x").ok().flatten().is_none(),
        "rebuilt from scratch"
    );
}

/// §7.2: for every migration step N, a database built at N from the seeds,
/// migrated to the latest, equals a fresh latest database with the same data:
/// same schema, same contents.
#[test]
fn every_migration_step_reaches_the_same_database() {
    let seeds = |n: usize| {
        std::fs::read_to_string(format!(
            "{}/tests/seeds/{n:03}.sql",
            env!("CARGO_MANIFEST_DIR")
        ))
        .ok()
    };
    let dump = |c: &Connection| -> String {
        let mut out = String::new();
        let mut st = c.prepare("SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name").unwrap_or_else(|e| panic!("{e}"));
        let rows: Vec<(String, String, Option<String>)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap_or_else(|e| panic!("{e}"))
            .flatten()
            .collect();
        for (t, name, sql) in rows {
            out.push_str(&format!(
                "{t} {name}: {}\n",
                sql.unwrap_or_default()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
            if t == "table" {
                let mut q = c
                    .prepare(&format!("SELECT * FROM \"{name}\" ORDER BY 1, 2"))
                    .unwrap_or_else(|e| panic!("{e}"));
                let n = q.column_count();
                let rows = q
                    .query_map([], |r| {
                        Ok((0..n)
                            .map(|i| format!("{:?}", r.get_ref(i).map(|v| format!("{v:?}"))))
                            .collect::<Vec<_>>()
                            .join("|"))
                    })
                    .unwrap_or_else(|e| panic!("{e}"));
                for r in rows.flatten() {
                    out.push_str(&format!("  {r}\n"));
                }
            }
        }
        out.push_str(&format!(
            "user_version {}\n",
            c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap_or(-1)
        ));
        out
    };
    // The reference: latest schema, then every seed in order (later seeds may
    // reference newer columns, so each is applied after its own migration).
    let latest = latest_version();
    for n in 1..=latest {
        assert!(
            seeds(n).is_some(),
            "migration {n:03} needs tests/seeds/{n:03}.sql"
        );
    }
    let mut fresh = Connection::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
    migrations()
        .to_latest(&mut fresh)
        .unwrap_or_else(|e| panic!("{e}"));
    for n in 1..=latest {
        fresh
            .execute_batch(&seeds(n).unwrap_or_default())
            .unwrap_or_else(|e| panic!("seed {n} on latest: {e}"));
    }
    let want = dump(&fresh);
    for start in 1..=latest {
        let mut c = Connection::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let m = migrations();
        for n in 1..=start {
            m.to_version(&mut c, n).unwrap_or_else(|e| panic!("{e}"));
            c.execute_batch(&seeds(n).unwrap_or_default())
                .unwrap_or_else(|e| panic!("seed {n}: {e}"));
        }
        m.to_latest(&mut c).unwrap_or_else(|e| panic!("{e}"));
        for n in start + 1..=latest {
            c.execute_batch(&seeds(n).unwrap_or_default())
                .unwrap_or_else(|e| panic!("seed {n}: {e}"));
        }
        assert_eq!(
            dump(&c),
            want,
            "a database built at {start:03} and migrated differs from a fresh one"
        );
    }
}

#[test]
fn migrations_validate() {
    assert!(migrations().validate().is_ok());
}

#[test]
fn vectors_round_trip_and_prune() {
    let mut s = Store::memory().unwrap_or_else(|e| panic!("{e}"));
    let a = [1u8; 32];
    let b = [2u8; 32];
    s.put_vectors("m1", &[(a, vec![0.5, -1.0]), (b, vec![0.0, 1.0])])
        .unwrap_or_else(|e| panic!("{e}"));
    s.put_vectors("m2", &[(a, vec![1.0, 0.0, 0.0])])
        .unwrap_or_else(|e| panic!("{e}"));
    let got = s.vectors("m1", 2, &[a, [9u8; 32]]).unwrap_or_default();
    assert_eq!(got.len(), 1);
    assert_eq!(got.get(&a), Some(&vec![0.5, -1.0]));
    assert!(
        s.vectors("m1", 3, &[a]).unwrap_or_default().is_empty(),
        "dims are part of the key"
    );
    assert_eq!(s.prune_other_models("m1", 2).unwrap_or(0), 1);
    assert_eq!(s.vector_count("m1", 2).unwrap_or(0), 2);
    assert_eq!(s.vector_count("m2", 3).unwrap_or(9), 0);
}
