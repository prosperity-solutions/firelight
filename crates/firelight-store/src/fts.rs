//! The word index (§9.2): an FTS5 table in its own in-memory database, built
//! with each catalog and never shared with the cache's connection.

use std::sync::Mutex;

use rusqlite::{Connection, params};

use crate::{Result, StoreError};

/// The fields of one chunk, weighted 5, 4, 2, 1 by bm25.
#[derive(Debug, Clone, Default)]
pub struct FtsRow<'a> {
    pub title: &'a str,
    pub aliases: &'a str,
    pub summary: &'a str,
    pub body: &'a str,
}

pub struct FtsIndex {
    conn: Mutex<Connection>,
}

fn e(source: rusqlite::Error) -> StoreError {
    StoreError::Sqlite {
        path: "word index".into(),
        source,
    }
}

impl FtsIndex {
    /// Index `rows`; row `i` gets id `i`.
    pub fn build(rows: &[FtsRow<'_>]) -> Result<FtsIndex> {
        let mut conn = Connection::open_in_memory().map_err(e)?;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE chunks USING fts5(title, aliases, summary, body, \
             tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3');",
        )
        .map_err(e)?;
        let tx = conn.transaction().map_err(e)?;
        {
            let mut st = tx
                .prepare("INSERT INTO chunks (rowid, title, aliases, summary, body) VALUES (?1, ?2, ?3, ?4, ?5)")
                .map_err(e)?;
            for (i, r) in rows.iter().enumerate() {
                let id = i64::try_from(i).unwrap_or(i64::MAX);
                st.execute(params![id, r.title, r.aliases, r.summary, r.body])
                    .map_err(e)?;
            }
        }
        tx.commit().map_err(e)?;
        Ok(FtsIndex {
            conn: Mutex::new(conn),
        })
    }

    /// Rows matching an FTS5 expression, best first, as (row, bm25) where a
    /// lower bm25 is better. Build `expr` with [`match_expr`]: user text never
    /// reaches FTS5 syntax unquoted.
    pub fn query(&self, expr: &str, limit: usize) -> Result<Vec<(usize, f64)>> {
        if expr.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| e(rusqlite::Error::InvalidQuery))?;
        let mut st = conn
            .prepare_cached(
                "SELECT rowid, bm25(chunks, 5.0, 4.0, 2.0, 1.0) AS r FROM chunks \
                 WHERE chunks MATCH ?1 ORDER BY r LIMIT ?2",
            )
            .map_err(e)?;
        let lim = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = st
            .query_map(params![expr, lim], |r| {
                Ok((
                    usize::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                    r.get::<_, f64>(1)?,
                ))
            })
            .map_err(e)?;
        rows.collect::<std::result::Result<_, _>>().map_err(e)
    }
}

/// An FTS5 expression for `words`: each one a quoted string (so `NEAR`,
/// `col:`, `^`, `*` and quotes stay literal), the last also as a prefix,
/// joined with OR so bm25 ranks rows by how many words they hold.
pub fn match_expr(words: &[String]) -> String {
    let q = |w: &str| format!("\"{}\"", w.replace('"', "\"\""));
    let mut parts: Vec<String> = words
        .iter()
        .filter(|w| !w.is_empty())
        .map(|w| q(w))
        .collect();
    if let Some(last) = words.iter().rev().find(|w| !w.is_empty()) {
        parts.push(format!("{}*", q(last)));
    }
    parts.join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx() -> FtsIndex {
        FtsIndex::build(&[
            FtsRow {
                title: "Request access rights",
                aliases: "admin rights",
                summary: "How to get admin rights.",
                body: "Open the portal.",
            },
            FtsRow {
                title: "Argo CD",
                aliases: "",
                summary: "Deployments.",
                body: "NEAR the cluster; col: x",
            },
            FtsRow {
                title: "Café",
                aliases: "",
                summary: "",
                body: "über",
            },
        ])
        .unwrap_or_else(|err| panic!("{err}"))
    }

    fn w(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_lowercase).collect()
    }

    #[test]
    fn ranks_and_prefixes() {
        let i = idx();
        let r = i
            .query(&match_expr(&w("how do I get admin rights")), 10)
            .unwrap_or_default();
        assert_eq!(
            r.first().map(|x| x.0),
            Some(0),
            "OR keeps the row that has most words"
        );
        let r = i.query(&match_expr(&w("depl")), 10).unwrap_or_default();
        assert_eq!(
            r.iter().map(|x| x.0).collect::<Vec<_>>(),
            [1],
            "the last word is a prefix"
        );
        let r = i
            .query(&match_expr(&w("cafe uber")), 10)
            .unwrap_or_default();
        assert_eq!(r.first().map(|x| x.0), Some(2), "diacritics fold");
    }

    #[test]
    fn syntax_stays_literal() {
        let i = idx();
        for q in [
            "NEAR(",
            "col:",
            "^argo",
            "\"",
            "a AND",
            "*",
            "body:open",
            "(",
            "-x",
        ] {
            assert!(i.query(&match_expr(&w(q)), 10).is_ok(), "{q}");
        }
        assert!(i.query(&match_expr(&[]), 10).unwrap_or_default().is_empty());
    }
}
