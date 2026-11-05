//! `openai-compatible` (§8.1): `POST {base}/embeddings` on Ollama, vLLM,
//! OpenAI, Azure and others.
//!
//! Chunk text leaves the process (and usually the cluster) on this backend.
//! The key is sent only over https or to a loopback host, redirects are never
//! followed, responses are capped, and no error ever carries the key or a
//! response body.

use std::time::Duration;

use crate::{Embedder, Result, err, normalize};

/// The biggest response read (64 vectors × 4096 dims as JSON is ~5 MB).
pub const MAX_RESPONSE_BYTES: usize = 32 << 20;
const BATCH: usize = 64;

pub struct OpenAi {
    base: String,
    model: String,
    key: Option<String>,
    dims: usize,
    client: reqwest::Client,
    rt: tokio::runtime::Handle,
}

/// Is `url` http(s) and, when it carries a key over plain http, loopback?
pub fn check_base(base: &str, has_key: bool) -> Result<()> {
    let u = reqwest::Url::parse(base).map_err(|e| err(format!("embed.base_url: {e}")))?;
    match u.scheme() {
        "https" => {}
        "http" => {
            let loopback = match u.host() {
                Some(url::Host::Ipv4(a)) => a.is_loopback(),
                Some(url::Host::Ipv6(a)) => a.is_loopback(),
                Some(url::Host::Domain(d)) => d == "localhost",
                None => false,
            };
            if has_key && !loopback {
                return Err(err(
                    "embed.api_key over plain http would leak the key: use https, or a loopback base_url",
                ));
            }
        }
        _ => return Err(err("embed.base_url must be an http(s) URL")),
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err(err(
            "embed.base_url must not carry credentials; use embed.api_key",
        ));
    }
    Ok(())
}

impl OpenAi {
    /// Connects once to learn the dimensions. `rt` runs the requests; call the
    /// embedder only from blocking threads (`spawn_blocking`).
    pub fn new(
        base: &str,
        model: &str,
        key: Option<String>,
        rt: tokio::runtime::Handle,
    ) -> Result<OpenAi> {
        check_base(base, key.is_some())?;
        // reqwest is built without a default TLS provider (§17.2: ring only).
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| err(format!("HTTP client: {e}")))?;
        let mut me = OpenAi {
            base: base.trim_end_matches('/').to_string(),
            model: model.to_string(),
            key,
            dims: 0,
            client,
            rt,
        };
        let probe = me.call(&["dimension probe"])?;
        me.dims = probe.first().map_or(0, Vec::len);
        if me.dims == 0 {
            return Err(err("the embeddings endpoint returned an empty vector"));
        }
        Ok(me)
    }

    fn call(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/embeddings", self.base);
        let body = serde_json::json!({ "model": self.model, "input": texts });
        let fut = async {
            let mut req = self.client.post(&url).json(&body);
            if let Some(k) = &self.key {
                req = req.bearer_auth(k);
            }
            let mut res = req
                .send()
                .await
                .map_err(|e| err(format!("POST {url}: {}", e.without_url())))?;
            let status = res.status();
            if !status.is_success() {
                return Err(err(format!("POST {url}: HTTP {status}")));
            }
            let mut buf = Vec::new();
            while let Some(c) = res
                .chunk()
                .await
                .map_err(|e| err(format!("POST {url}: {}", e.without_url())))?
            {
                buf.extend_from_slice(&c);
                if buf.len() > MAX_RESPONSE_BYTES {
                    return Err(err(format!("POST {url}: the response is too big")));
                }
            }
            Ok(buf)
        };
        let buf = self.rt.block_on(fut)?;
        parse(&buf, texts.len(), self.dims)
    }
}

/// The vectors of an `/embeddings` response, in input order.
pub fn parse(buf: &[u8], n: usize, dims: usize) -> Result<Vec<Vec<f32>>> {
    let v: serde_json::Value =
        serde_json::from_slice(buf).map_err(|_| err("the embeddings response is not JSON"))?;
    let data = v["data"]
        .as_array()
        .ok_or_else(|| err("the embeddings response has no data list"))?;
    if data.len() != n {
        return Err(err(format!("asked for {n} embeddings, got {}", data.len())));
    }
    let mut out: Vec<Option<Vec<f32>>> = vec![None; n];
    for (pos, d) in data.iter().enumerate() {
        let i = d["index"]
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .unwrap_or(pos);
        let mut vec: Vec<f32> = d["embedding"]
            .as_array()
            .ok_or_else(|| err("an embedding is not a list of numbers"))?
            .iter()
            .map(|x| x.as_f64().map(|f| f as f32))
            .collect::<Option<_>>()
            .ok_or_else(|| err("an embedding is not a list of numbers"))?;
        if dims != 0 && vec.len() != dims {
            return Err(err(format!(
                "the endpoint returned {} dimensions, not {dims}: did the model change?",
                vec.len()
            )));
        }
        normalize(&mut vec);
        let slot = out
            .get_mut(i)
            .ok_or_else(|| err("an embedding index is out of range"))?;
        if slot.replace(vec).is_some() {
            return Err(err("an embedding index appears twice"));
        }
    }
    out.into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| err("an embedding is missing"))
}

impl Embedder for OpenAi {
    fn id(&self) -> String {
        format!("openai:{}:{}:{}", self.base, self.model, self.dims)
    }
    fn dims(&self) -> usize {
        self.dims
    }
    fn max_tokens(&self) -> usize {
        crate::candle::MAX_TOKENS
    }
    fn embed_documents(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for batch in texts.chunks(BATCH) {
            out.extend(self.call(batch)?);
        }
        Ok(out)
    }
    fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        self.call(&[query])?
            .pop()
            .ok_or_else(|| err("no embedding returned"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_follows_index() {
        let body = br#"{"data":[{"index":1,"embedding":[0,2]},{"index":0,"embedding":[3,0]}]}"#;
        let v = parse(body, 2, 2).unwrap_or_default();
        assert_eq!(v, [vec![1.0, 0.0], vec![0.0, 1.0]]);
    }

    #[test]
    fn refusals() {
        assert!(
            parse(br#"{"data":[{"index":0,"embedding":[1,2,3]}]}"#, 1, 2).is_err(),
            "dims"
        );
        assert!(parse(br#"{"data":[]}"#, 1, 2).is_err(), "count");
        assert!(
            parse(
                br#"{"data":[{"index":0,"embedding":[1,0]},{"index":0,"embedding":[1,0]}]}"#,
                2,
                2
            )
            .is_err(),
            "dup"
        );
        assert!(parse(b"<html>", 1, 2).is_err());
    }

    #[test]
    fn bases() {
        assert!(check_base("https://llm.acme.example/v1", true).is_ok());
        assert!(check_base("http://127.0.0.1:11434/v1", true).is_ok());
        assert!(check_base("http://localhost:11434/v1", true).is_ok());
        assert!(check_base("http://llm.acme.example/v1", false).is_ok());
        assert!(check_base("http://llm.acme.example/v1", true).is_err());
        assert!(check_base("https://u:p@llm.acme.example/v1", false).is_err());
        assert!(check_base("ftp://llm.acme.example", false).is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn errors_never_carry_the_key() {
        use axum::routing::post;
        let app = axum::Router::new().route(
            "/v1/embeddings",
            post(|| async {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    "bad key sk-secret-123",
                )
            }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let addr = l.local_addr().unwrap_or_else(|e| panic!("{e}"));
        tokio::spawn(async move { axum::serve(l, app).await });
        let rt = tokio::runtime::Handle::current();
        let base = format!("http://{addr}/v1");
        let r = tokio::task::spawn_blocking(move || {
            OpenAi::new(&base, "m", Some("sk-secret-123".into()), rt)
                .err()
                .map(|e| e.to_string())
        })
        .await
        .unwrap_or_default()
        .unwrap_or_default();
        assert!(r.contains("401"), "{r}");
        assert!(!r.contains("sk-secret"), "{r}");
    }
}
