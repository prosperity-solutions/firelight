//! A table test per startup refusal (docs/HANDOVER.md §13.1, "Config").

use super::*;

fn inputs(env: &[(&str, &str)], files: &[(&str, &str)]) -> Inputs {
    let files: BTreeMap<PathBuf, String> = files
        .iter()
        .map(|(p, c)| (PathBuf::from(p), (*c).to_string()))
        .collect();
    let files2 = files.clone();
    Inputs {
        env: env
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        read: Box::new(move |p| {
            files.get(p).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory")
            })
        }),
        exists: Box::new(move |p| files2.contains_key(p) || p == Path::new("/srv/docs")),
    }
}

const OK: &[(&str, &str)] = &[
    (
        "FIRELIGHT_SERVER_PUBLIC_URL",
        "https://firelight.acme.example",
    ),
    ("FIRELIGHT_AUTH_UI", "none"),
    ("FIRELIGHT_AUTH_MCP", "none"),
];

fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    let mut v: Vec<_> = OK.to_vec();
    for (k, val) in extra {
        v.retain(|(k2, _)| k2 != k);
        if !val.is_empty() {
            v.push((k, val));
        }
    }
    v
}

fn problems(env: &[(&str, &str)], files: &[(&str, &str)], purpose: Purpose) -> Vec<String> {
    match load(&inputs(env, files), purpose) {
        Ok(_) => vec![],
        Err(p) => p.0.iter().map(ToString::to_string).collect(),
    }
}

#[test]
fn a_minimal_config_loads() {
    let c = load(&inputs(OK, &[]), Purpose::Serve).unwrap_or_else(|p| panic!("{p}"));
    assert_eq!(c.listen.to_string(), "127.0.0.1:8080");
    assert_eq!(c.poll_interval, Duration::from_secs(120));
    assert_eq!(c.embedder, "candle");
    assert!(c.self_docs);
    assert_eq!(c.ui_auth, UiAuth::None);
}

#[test]
fn every_refusal_has_its_exact_message() {
    let t = |extra: &[(&'static str, &'static str)], want: &str| {
        let got = problems(&with(extra), &[], Purpose::Serve);
        assert!(
            got.iter().any(|g| g == want),
            "want {want:?}\n got {got:#?}"
        );
    };
    t(
        &[("FIRELIGHT_AUTH_UI", "")],
        "auth.ui (not set): Pick an auth mode. Behind a VPN or an auth proxy? Set FIRELIGHT_AUTH_UI=none and FIRELIGHT_AUTH_MCP=none.",
    );
    t(
        &[("FIRELIGHT_AUTH_MCP", "")],
        "auth.mcp (not set): Pick an auth mode. Behind a VPN or an auth proxy? Set FIRELIGHT_AUTH_UI=none and FIRELIGHT_AUTH_MCP=none.",
    );
    t(
        &[("FIRELIGHT_SERVER_PUBLIC_URL", "")],
        "server.public_url (not set): required for serve: the URL people open. Set FIRELIGHT_SERVER_PUBLIC_URL, e.g. https://firelight.acme.example",
    );
    t(
        &[
            ("FIRELIGHT_AUTH_UI", "basic"),
            ("FIRELIGHT_AUTH_MCP", "none"),
            ("FIRELIGHT_AUTH_USER", "ops"),
            ("FIRELIGHT_AUTH_PASSWORD", "secret-password"),
        ],
        "auth.mcp (env FIRELIGHT_AUTH_MCP): none is refused with auth.ui = basic: MCP would expose everything the login protects. Set FIRELIGHT_AUTH_MCP=token and give it tokens.",
    );
    t(
        &[("FIRELIGHT_AUTH_UI", "basic")],
        "auth.user (not set): required with auth.ui = basic. Set FIRELIGHT_AUTH_USER.",
    );
    t(
        &[("FIRELIGHT_AUTH_UI", "basic")],
        "auth.password (not set): required with auth.ui = basic. Set FIRELIGHT_AUTH_PASSWORD_FILE to a mounted secret (or FIRELIGHT_AUTH_PASSWORD).",
    );
    t(
        &[("FIRELIGHT_AUTH_MCP", "token")],
        "auth.mcp_tokens (not set): required with auth.mcp = token: at least one token. Set FIRELIGHT_AUTH_MCP_TOKENS_FILE to a file with one token per line.",
    );
    t(
        &[("FIRELIGHT_AUTH_UI", "sometimes")],
        "auth.ui (env FIRELIGHT_AUTH_UI): \"sometimes\" is not one of basic, none.",
    );
    t(
        &[("FIRELIGHT_SERVER_LISTEN", "everywhere")],
        "server.listen (env FIRELIGHT_SERVER_LISTEN): \"everywhere\" is not an address and port. Write e.g. \"0.0.0.0:8080\".",
    );
    t(
        &[("FIRELIGHT_POLL_INTERVAL", "3s")],
        "poll.interval (env FIRELIGHT_POLL_INTERVAL): 3s is too short; the minimum is 10s.",
    );
    t(
        &[("FIRELIGHT_POLL_INTERVAL", "soon")],
        "poll.interval (env FIRELIGHT_POLL_INTERVAL): \"soon\" is not a duration. Write e.g. \"120s\", \"5m\" or \"1h\".",
    );
    t(
        &[("FIRELIGHT_SELF_DOCS", "maybe")],
        "self_docs (env FIRELIGHT_SELF_DOCS): \"maybe\" is not true or false.",
    );
    t(
        &[("FIRELIGHT_LIMITS_MAX_FILES", "0")],
        "limits.max_files (env FIRELIGHT_LIMITS_MAX_FILES): 0 is out of range (1..=1000000).",
    );
    t(
        &[("FIRELIGHT_EMBED_REVISION", "main")],
        "embed.revision (env FIRELIGHT_EMBED_REVISION): \"main\" is not a 40-hex commit. Branch names and tags are refused, because they move: copy the commit from the model's \"Files and versions\" page.",
    );
    t(
        &[("FIRELIGHT_EMBED_MODEL", "acme/other-model")],
        "embed.revision (not set): required for a model other than the default (Qwen/Qwen3-Embedding-0.6B): the model's 40-hex commit on Hugging Face. Set FIRELIGHT_EMBED_REVISION.",
    );
    t(
        &[("FIRELIGHT_EMBEDDER", "openai-compatible")],
        "embed.base_url (not set): required with embedder = openai-compatible. Set FIRELIGHT_EMBED_BASE_URL, e.g. http://ollama:11434/v1",
    );
    t(
        &[("FIRELIGHT_UI_ACCENT", "orange")],
        "ui.accent (env FIRELIGHT_UI_ACCENT): \"orange\" is not a colour. Write it as #rrggbb, e.g. #d9541e.",
    );
    t(
        &[("FIRELIGHT_GITHUB_APP_ID", "12")],
        "github_app.private_key (not set): required with github_app.id. Set FIRELIGHT_GITHUB_APP_PRIVATE_KEY_FILE to the mounted PEM.",
    );
    t(
        &[("FIRELIGHT_AUTH_PASSWORD_FILE", "/run/secrets/nope")],
        "auth.password (env FIRELIGHT_AUTH_PASSWORD_FILE): cannot read the file /run/secrets/nope: No such file or directory. Fix the path or mount the secret there.",
    );
    t(
        &[("FIRELIGHT_SERVER_PUBLIC_URLL", "x")],
        "FIRELIGHT_SERVER_PUBLIC_URLL (env FIRELIGHT_SERVER_PUBLIC_URLL): unknown environment variable. Did you mean FIRELIGHT_SERVER_PUBLIC_URL? Every key and its env var: docs/config.md.",
    );
    t(
        &[("FIRELIGHT_SOURCES", "app-repo")],
        "sources (env FIRELIGHT_SOURCES): expected a JSON array in FIRELIGHT_SOURCES, e.g. [{\"id\":\"app-repo\",\"kind\":\"github\",\"url\":\"https://github.com/acme/app-repo\"}]",
    );
}

#[test]
fn all_problems_come_at_once() {
    let got = problems(
        &[
            ("FIRELIGHT_POLL_INTERVAL", "3s"),
            ("FIRELIGHT_UI_ACCENT", "red"),
            ("FIRELIGHT_NOPE", "1"),
        ],
        &[],
        Purpose::Serve,
    );
    // public_url, auth.ui, auth.mcp, poll.interval, ui.accent, FIRELIGHT_NOPE.
    assert_eq!(got.len(), 6, "{got:#?}");
}

#[test]
fn sources_are_checked() {
    let env = with(&[(
        "FIRELIGHT_SOURCES",
        r#"[{"id":"app-repo","kind":"github","url":"https://github.com/acme/app-repo"},
            {"id":"app-repo","kind":"git","url":"https://git.acme.example/infra.git"},
            {"id":"firelight","kind":"git","url":"https://git.acme.example/f.git"},
            {"id":"Bad_Id","kind":"dir","path":"/srv/docs"},
            {"id":"creds","kind":"git","url":"https://user:hunter2@git.acme.example/x.git"},
            {"id":"lab","kind":"github","url":"https://gitlab.acme.example/x"},
            {"id":"local","kind":"dir","path":"/nowhere"},
            {"id":"t","kind":"git","url":"https://git.acme.example/t.git","token_file":"/run/secrets/missing","colour":"red"}]"#,
    )]);
    let got = problems(&env, &[], Purpose::Serve).join("\n");
    for want in [
        "sources[1].id (env FIRELIGHT_SOURCES): app-repo is used by two sources.",
        "sources[2].id (env FIRELIGHT_SOURCES): firelight is reserved for Firelight's own docs.",
        "sources[3].id (env FIRELIGHT_SOURCES): \"Bad_Id\" is not a valid id",
        "sources[4].url (env FIRELIGHT_SOURCES): a URL with a user name or password in it is refused",
        "sources[5].url (env FIRELIGHT_SOURCES): a github source needs a https://github.com/<owner>/<repo> URL.",
        "sources[6].path (env FIRELIGHT_SOURCES): /nowhere does not exist.",
        "sources[7].token_file (env FIRELIGHT_SOURCES): cannot read /run/secrets/missing.",
        "sources[7].colour (env FIRELIGHT_SOURCES): unknown key.",
    ] {
        assert!(got.contains(want), "want {want:?} in\n{got}");
    }
    // A credential is never echoed.
    assert!(!got.contains("hunter2"), "{got}");
}

#[test]
fn a_dir_source_may_replace_the_self_docs() {
    let env = with(&[(
        "FIRELIGHT_SOURCES",
        r#"[{"id":"firelight","kind":"dir","path":"/srv/docs"}]"#,
    )]);
    assert!(problems(&env, &[], Purpose::Serve).is_empty());
}

#[test]
fn the_file_then_env() {
    let file = r#"
[server]
public_url = "https://firelight.acme.example"
listen = "0.0.0.0:9000"
[auth]
ui = "none"
mcp = "none"
[poll]
interval = "5m"
[[sources]]
id = "docs"
kind = "dir"
path = "/srv/docs"
"#;
    let ins = inputs(
        &[
            ("FIRELIGHT_CONFIG", "/etc/f.toml"),
            ("FIRELIGHT_SERVER_LISTEN", "127.0.0.1:7000"),
        ],
        &[("/etc/f.toml", file)],
    );
    let c = load(&ins, Purpose::Serve).unwrap_or_else(|p| panic!("{p}"));
    assert_eq!(
        c.listen.to_string(),
        "127.0.0.1:7000",
        "env wins over the file"
    );
    assert_eq!(c.poll_interval, Duration::from_secs(300));
    assert_eq!(c.sources.len(), 1);
    let shown: BTreeMap<_, _> = c
        .shown
        .iter()
        .map(|(k, v, o)| (k.as_str(), (v.as_str(), o.clone())))
        .collect();
    assert_eq!(
        shown["server.listen"].1,
        Origin::Env("FIRELIGHT_SERVER_LISTEN".into())
    );
    assert_eq!(shown["poll.interval"].1, Origin::File("/etc/f.toml".into()));
}

#[test]
fn file_problems_name_the_file() {
    let ins = inputs(
        &[("FIRELIGHT_CONFIG", "/etc/f.toml")],
        &[(
            "/etc/f.toml",
            "[server]\npublic_ulr = \"x\"\n[auth]\nui = \"none\"\nmcp = \"none\"\n",
        )],
    );
    let got = load(&ins, Purpose::Serve)
        .err()
        .map(|p| p.to_string())
        .unwrap_or_default();
    assert!(
        got.contains(
            "server.public_ulr (file /etc/f.toml): unknown key. Did you mean server.public_url?"
        ),
        "{got}"
    );
    let ins = inputs(&[("FIRELIGHT_CONFIG", "/etc/missing.toml")], &[]);
    let got = load(&ins, Purpose::Serve)
        .err()
        .map(|p| p.to_string())
        .unwrap_or_default();
    assert!(
        got.contains(
            "FIRELIGHT_CONFIG (env FIRELIGHT_CONFIG): cannot read the config file /etc/missing.toml"
        ),
        "{got}"
    );
    let ins = inputs(
        &[("FIRELIGHT_CONFIG", "/etc/f.toml")],
        &[("/etc/f.toml", "[server\n")],
    );
    let got = load(&ins, Purpose::Serve)
        .err()
        .map(|p| p.to_string())
        .unwrap_or_default();
    assert!(got.contains("not valid TOML"), "{got}");
}

#[test]
fn kubernetes_service_links_are_ignored() {
    let env = with(&[
        ("FIRELIGHT_PORT", "tcp://10.0.0.1:80"),
        ("FIRELIGHT_PORT_80_TCP", "tcp://10.0.0.1:80"),
        ("FIRELIGHT_PORT_80_TCP_ADDR", "10.0.0.1"),
        ("FIRELIGHT_SERVICE_HOST", "10.0.0.1"),
        ("FIRELIGHT_SERVICE_PORT", "80"),
        ("FIRELIGHT_SERVICE_PORT_HTTP", "80"),
    ]);
    assert!(problems(&env, &[], Purpose::Serve).is_empty());
}

#[test]
fn secrets_from_files_and_never_shown() {
    let env = with(&[
        ("FIRELIGHT_AUTH_UI", "basic"),
        ("FIRELIGHT_AUTH_MCP", "token"),
        ("FIRELIGHT_AUTH_USER", "ops"),
        ("FIRELIGHT_AUTH_PASSWORD_FILE", "/run/secrets/password"),
        ("FIRELIGHT_AUTH_MCP_TOKENS_FILE", "/run/secrets/tokens"),
    ]);
    let ins = inputs(
        &env,
        &[
            ("/run/secrets/password", "correct-horse-battery\n"),
            (
                "/run/secrets/tokens",
                "aaaaaaaaaaaaaaaaaaaaaaaa\n\nbbbbbbbbbbbbbbbbbbbbbbbb\n",
            ),
        ],
    );
    let c = load(&ins, Purpose::Serve).unwrap_or_else(|p| panic!("{p}"));
    assert_eq!(
        c.basic.as_ref().map(|b| b.1.0.as_str()),
        Some("correct-horse-battery")
    );
    assert_eq!(c.mcp_tokens.len(), 2);
    let all = format!("{c:?} {:?}", c.shown);
    assert!(
        !all.contains("correct-horse") && !all.contains("aaaaaaaa"),
        "a secret leaked: {all}"
    );
}

#[test]
fn short_tokens_are_refused() {
    let env = with(&[
        ("FIRELIGHT_AUTH_MCP", "token"),
        ("FIRELIGHT_AUTH_MCP_TOKENS", r#"["short"]"#),
    ]);
    let got = problems(&env, &[], Purpose::Serve);
    assert!(
        got.iter().any(|g| g.contains("at least 16 characters")),
        "{got:?}"
    );
    // The token itself is not echoed.
    assert!(got.iter().all(|g| !g.contains("short\"")), "{got:?}");
}

#[test]
fn preview_needs_no_auth_or_url() {
    assert!(problems(&[], &[], Purpose::Preview).is_empty());
}

#[test]
fn every_key_has_a_unique_env_and_parses_its_default() {
    let mut envs = BTreeSet::new();
    for k in KEYS {
        assert!(envs.insert(k.env()), "two keys map to {}", k.env());
        if let Some(d) = k.default {
            let v = parse_default(k, d);
            assert!(!v.is_null(), "{}: default {d} doesn't parse", k.path);
        }
    }
}

#[test]
fn docs_config_md_is_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/config.md");
    let want = reference_markdown();
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        let _ = std::fs::write(&path, &want);
    }
    let got = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        got == want,
        "docs/config.md is stale. Run: just schema (or UPDATE_SNAPSHOTS=1 cargo test --locked -p firelight)"
    );
}

#[test]
fn empty_values_mean_unset() {
    // As the chart renders them before the operator states auth.
    let ins = inputs(
        &[
            ("FIRELIGHT_CONFIG", "/etc/f.toml"),
            ("FIRELIGHT_EMBED_MODEL", ""),
        ],
        &[(
            "/etc/f.toml",
            "sources = []\n[server]\npublic_url = \"https://firelight.acme.example\"\n[auth]\nui = \"\"\nmcp = \"\"\n",
        )],
    );
    let got = load(&ins, Purpose::Serve)
        .err()
        .map(|p| p.to_string())
        .unwrap_or_default();
    assert!(
        got.contains("auth.ui (not set): Pick an auth mode."),
        "{got}"
    );
    assert!(!got.contains("embed.model"), "{got}");
}

#[test]
fn toml_errors_never_echo_the_line() {
    let ins = inputs(
        &[("FIRELIGHT_CONFIG", "/etc/f.toml")],
        &[("/etc/f.toml", "[auth]\npassword = hunter2-secret\n")],
    );
    let got = load(&ins, Purpose::Serve)
        .err()
        .map(|p| p.to_string())
        .unwrap_or_default();
    assert!(got.contains("not valid TOML at line 2"), "{got}");
    assert!(!got.contains("hunter2"), "a secret leaked: {got}");
}

#[test]
fn credential_urls_are_never_echoed() {
    for url in [
        "ssh://oauth2:hunter2@git.acme.example/x.git",
        "https://oauth2:hunter2@git.acme.example/x.git",
        "https://hunter2@git.acme.example/x.git",
        "oauth2:hunter2@git.acme.example:x.git",
    ] {
        let env = with(&[(
            "FIRELIGHT_SERVER_PUBLIC_URL",
            "https://firelight.acme.example",
        )]);
        let mut env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        env.push((
            "FIRELIGHT_SOURCES".into(),
            format!(r#"[{{"id":"x","kind":"git","url":"{url}"}}]"#),
        ));
        let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let got = problems(&pairs, &[], Purpose::Serve).join("\n");
        assert!(got.contains("user name or password"), "{url}: {got}");
        assert!(!got.contains("hunter2"), "{url}: a secret leaked: {got}");
    }
}

#[test]
fn helm_style_floats_are_whole_numbers() {
    // What the chart's toToml renders from values.yaml.
    let file = "data = { dir = \"/var/lib/firelight\" }\nsources = []\n[server]\nlisten = \"0.0.0.0:8080\"\npublic_url = \"https://firelight.acme.example\"\n[auth]\nui = \"none\"\nmcp = \"none\"\n[limits]\nmax_files = 1000.0\n[poll]\ninterval = 300.0\n";
    let ins = inputs(
        &[("FIRELIGHT_CONFIG", "/etc/f.toml")],
        &[("/etc/f.toml", file)],
    );
    let c = load(&ins, Purpose::Serve).unwrap_or_else(|p| panic!("{p}"));
    assert_eq!(c.max_files, 1000);
    assert_eq!(c.poll_interval, Duration::from_secs(300));
    let bad = "[server]\npublic_url = \"https://f.acme.example\"\n[auth]\nui = \"none\"\nmcp = \"none\"\n[limits]\nmax_files = 2.5\n";
    let ins = inputs(
        &[("FIRELIGHT_CONFIG", "/etc/f.toml")],
        &[("/etc/f.toml", bad)],
    );
    assert!(load(&ins, Purpose::Serve).is_err());
}

#[test]
fn an_invalid_value_is_one_problem() {
    let got = problems(
        &with(&[
            ("FIRELIGHT_AUTH_UI", "sometimes"),
            ("FIRELIGHT_SERVER_PUBLIC_URL", "not a url"),
        ]),
        &[],
        Purpose::Serve,
    );
    assert_eq!(got.len(), 2, "{got:#?}");
}

#[test]
fn empty_unknown_tables_are_reported() {
    let ins = inputs(
        &[("FIRELIGHT_CONFIG", "/etc/f.toml")],
        &[(
            "/etc/f.toml",
            "[servr]\n[server]\npublic_url = \"https://f.acme.example\"\n[auth]\nui = \"none\"\nmcp = \"none\"\n",
        )],
    );
    let got = load(&ins, Purpose::Serve)
        .err()
        .map(|p| p.to_string())
        .unwrap_or_default();
    assert!(
        got.contains("servr (file /etc/f.toml): unknown key."),
        "{got}"
    );
}

#[test]
fn build_variables_and_empty_files_are_fine() {
    let env = with(&[
        ("FIRELIGHT_VERSION", "1.2.3"),
        ("FIRELIGHT_COMMIT", "abc"),
        ("FIRELIGHT_WEBHOOK_SECRET_FILE", ""),
    ]);
    assert!(problems(&env, &[], Purpose::Serve).is_empty());
}

#[test]
fn tokens_and_secrets_need_safe_transport_and_length() {
    let env = with(&[("FIRELIGHT_WEBHOOK_SECRET", "short")]);
    assert!(
        problems(&env, &[], Purpose::Serve)
            .iter()
            .any(|p| p.contains("webhook.secret") && p.contains("16 characters"))
    );
    let env = with(&[(
        "FIRELIGHT_SOURCES",
        r#"[{"id":"a","kind":"github","url":"http://github.com/acme/a"},
            {"id":"b","kind":"git","url":"http://git.acme.example/b.git","token_file":"/run/t"},
            {"id":"c","kind":"git","url":"http://localhost:8080/c.git","token_file":"/run/t"},
            {"id":"d","kind":"git","url":"http://git.acme.example/d.git"}]"#,
    )]);
    let got = problems(&env, &[("/run/t", "tok")], Purpose::Serve).join("\n");
    assert!(
        got.contains("sources[0].url") && got.contains("https://github.com"),
        "{got}"
    );
    assert!(
        got.contains("sources[1].url") && got.contains("clear text"),
        "{got}"
    );
    assert!(
        !got.contains("sources[2].url"),
        "loopback http is fine: {got}"
    );
    assert!(
        !got.contains("sources[3].url"),
        "anonymous http is allowed: {got}"
    );
}
