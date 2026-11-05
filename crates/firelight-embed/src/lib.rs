//! Embeddings (§8): the [`Embedder`] trait and its backends.
//!
//! - [`candle::Candle`]: the model in-process on CPU (the default).
//! - [`openai::OpenAi`]: `POST {base}/embeddings` on any OpenAI-compatible server.
//! - `off`: no embedder at all (`Option<Arc<dyn Embedder>>` is `None`); word search only.
//! - [`Fake`]: deterministic, for tests in every crate (§13.1).
//!
//! [`model`] downloads and verifies model files (§8.3).

use std::sync::Arc;

pub mod candle;
pub mod model;
pub mod openai;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Msg(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn err(s: impl Into<String>) -> Error {
    Error::Msg(s.into())
}

/// Turns text into L2-normalised vectors. Synchronous: callers run it on a
/// blocking thread.
pub trait Embedder: Send + Sync {
    /// Stable identity of the vectors this produces (backend, model, revision,
    /// prefix scheme): the cache key. Vectors from different ids never mix.
    fn id(&self) -> String;
    fn dims(&self) -> usize;
    /// The longest input in tokens; longer chunks are split before embedding.
    fn max_tokens(&self) -> usize;
    /// Tokens in `text`, for splitting chunks under [`Embedder::max_tokens`].
    fn count_tokens(&self, text: &str) -> usize {
        approx_tokens(text)
    }
    fn embed_documents(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    fn embed_query(&self, query: &str) -> Result<Vec<f32>>;
}

pub type Shared = Arc<dyn Embedder>;

/// A cautious token estimate when no tokenizer is at hand: words × 1.5,
/// plus one per non-ASCII-letter run, which overcounts rather than under.
pub fn approx_tokens(text: &str) -> usize {
    let words = text.split_whitespace().count();
    let wide = text.chars().filter(|c| !c.is_ascii()).count();
    words + words / 2 + wide / 2
}

/// Scale `v` to length 1 (a zero vector stays zero).
pub fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        for x in v.iter_mut() {
            *x /= n;
        }
    }
}

/// Dot product; for normalised vectors, the cosine.
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// A deterministic embedder for tests: every lowercased word lands in one of
/// `dims` buckets by a fixed hash. `synonyms` map a word onto another, so a
/// test can model "meaning" (a German word finding an English entry).
#[derive(Debug, Clone)]
pub struct Fake {
    pub dims: usize,
    pub synonyms: Vec<(String, String)>,
}

impl Default for Fake {
    fn default() -> Self {
        Fake {
            dims: 64,
            synonyms: Vec::new(),
        }
    }
}

impl Fake {
    pub fn with_synonyms(pairs: &[(&str, &str)]) -> Self {
        Fake {
            synonyms: pairs
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            ..Fake::default()
        }
    }

    fn vec(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0; self.dims];
        let lower = text.to_lowercase();
        for w in lower.split(|c: char| !c.is_alphanumeric()) {
            if w.is_empty() {
                continue;
            }
            let w = self
                .synonyms
                .iter()
                .find(|(a, _)| a == w)
                .map_or(w, |(_, b)| b.as_str());
            for part in w.split(' ') {
                // FNV-1a: stable across runs and platforms.
                let mut h: u64 = 0xcbf2_9ce4_8422_2325;
                for b in part.bytes() {
                    h ^= u64::from(b);
                    h = h.wrapping_mul(0x0100_0000_01b3);
                }
                let i = usize::try_from(h % self.dims as u64).unwrap_or(0);
                v[i] += 1.0;
            }
        }
        normalize(&mut v);
        v
    }
}

impl Embedder for Fake {
    fn id(&self) -> String {
        format!("fake-{}", self.dims)
    }
    fn dims(&self) -> usize {
        self.dims
    }
    fn max_tokens(&self) -> usize {
        512
    }
    fn embed_documents(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.vec(t)).collect())
    }
    fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        Ok(self.vec(query))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_is_deterministic_and_normalised() {
        let f = Fake::with_synonyms(&[("adminrechte", "admin rights")]);
        let a = f.embed_query("Admin rights").unwrap_or_default();
        let b = f.embed_query("adminrechte").unwrap_or_default();
        assert_eq!(a, f.embed_query("admin rights").unwrap_or_default());
        assert!((dot(&a, &a) - 1.0).abs() < 1e-5);
        assert!(dot(&a, &b) > 0.99, "a synonym lands on the same buckets");
        let c = f.embed_query("terraform").unwrap_or_default();
        assert!(dot(&a, &c) < 0.5);
    }

    #[test]
    fn normalize_keeps_zero() {
        let mut v = vec![0.0; 3];
        normalize(&mut v);
        assert_eq!(v, [0.0, 0.0, 0.0]);
    }
}
