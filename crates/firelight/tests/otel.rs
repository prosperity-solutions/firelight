//! OpenTelemetry (§17.2): with OTEL_EXPORTER_OTLP_ENDPOINT set, `serve`
//! exports traces and metrics over OTLP/HTTP; the search span carries the
//! query's length, never its text unless asked.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// (path, body) of every POST the receiver got.
type Got = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// A tiny OTLP/HTTP receiver: records (path, body) of every POST.
fn receiver() -> (u16, Got) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("{e}"));
    let port = l.local_addr().map(|a| a.port()).unwrap_or(0);
    let got = Arc::new(Mutex::new(Vec::new()));
    let g = got.clone();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let g = g.clone();
            std::thread::spawn(move || serve_one(s, &g));
        }
    });
    (port, got)
}

fn serve_one(mut s: TcpStream, got: &Mutex<Vec<(String, Vec<u8>)>>) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 65536];
    loop {
        let Ok(n) = s.read(&mut tmp) else { return };
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&tmp[..n]);
        let text = String::from_utf8_lossy(&buf).to_string();
        if let Some(h) = text.find("\r\n\r\n") {
            let head = &text[..h];
            let len: usize = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap_or(0))
                })
                .unwrap_or(0);
            if buf.len() >= h + 4 + len {
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                if let Ok(mut g) = got.lock() {
                    g.push((path, buf[h + 4..h + 4 + len].to_vec()));
                }
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                );
                return;
            }
        }
    }
}

fn get(port: u16, path: &str) -> String {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else {
        return String::new();
    };
    let _ = write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    let mut b = String::new();
    let _ = s.read_to_string(&mut b);
    b
}

/// Kills the server even when an assertion fails.
struct Child(std::process::Child, PathBuf);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

/// Run serve, search once, and return everything the OTLP receiver got once
/// the request's own span and the search metric have arrived.
fn exported(record_queries: bool) -> Vec<(String, String)> {
    let (otlp, got) = receiver();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap_or(root);
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(0);
    let data = std::env::temp_dir().join(format!("firelight-otel-{}-{port}", std::process::id()));
    let child = Command::new(env!("CARGO_BIN_EXE_firelight"))
        .arg("serve")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", &data)
        .env("FIRELIGHT_CONFIG", "")
        .env("FIRELIGHT_SERVER_LISTEN", format!("127.0.0.1:{port}"))
        .env(
            "FIRELIGHT_SERVER_PUBLIC_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .env("FIRELIGHT_DATA_DIR", &data)
        .env(
            "FIRELIGHT_SOURCES",
            format!(
                r#"[{{"id":"acme-infra","kind":"dir","path":"{}"}}]"#,
                root.join("fixtures/acme-infra").display()
            ),
        )
        .env("FIRELIGHT_EMBEDDER", "off")
        .env("FIRELIGHT_AUTH_UI", "none")
        .env("FIRELIGHT_AUTH_MCP", "none")
        .env(
            "FIRELIGHT_OTEL_RECORD_QUERIES",
            if record_queries { "true" } else { "false" },
        )
        .env(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            format!("http://127.0.0.1:{otlp}"),
        )
        .env("OTEL_BSP_SCHEDULE_DELAY", "200")
        .env("OTEL_METRIC_EXPORT_INTERVAL", "500")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("{e}"));
    let _child = Child(child, data);
    let start = Instant::now();
    while !get(port, "/readyz").starts_with("HTTP/1.1 200") {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "serve never got ready"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = get(port, "/api/v1/search?q=zebra-secret-query");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let all: Vec<(String, String)> = got
            .lock()
            .map(|g| {
                g.iter()
                    .map(|(p, b)| (p.clone(), String::from_utf8_lossy(b).to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let request_span = all.iter().any(|(p, b)| {
            p == "/v1/traces" && b.contains("http.request") && b.contains("/api/v1/search")
        });
        let search_span = all
            .iter()
            .any(|(p, b)| p == "/v1/traces" && b.contains("query.length"));
        let metric = all
            .iter()
            .any(|(p, b)| p == "/v1/metrics" && b.contains("firelight.search.zero_results"));
        if request_span && search_span && metric {
            return all;
        }
        assert!(
            Instant::now() < deadline,
            "missing: request span {request_span}, search span {search_span}, metric {metric}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn serve_exports_traces_and_metrics_without_the_query() {
    let all = exported(false);
    assert!(
        !all.iter().any(|(_, b)| b.contains("zebra-secret-query")),
        "the query text was exported without otel.record_queries"
    );
}

#[test]
fn record_queries_adds_the_query_text() {
    let all = exported(true);
    assert!(
        all.iter()
            .any(|(p, b)| p == "/v1/traces" && b.contains("zebra-secret-query")),
        "otel.record_queries should put the query on the search span"
    );
}
