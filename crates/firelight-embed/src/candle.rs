//! The in-process embedder (§8.2): candle on CPU, f32.
//!
//! The family comes from `config.json`'s `architectures` (D2):
//!
//! | architectures | example | pooling | prefixes |
//! |---|---|---|---|
//! | `Qwen3ForCausalLM`, `Qwen3Model` | Qwen/Qwen3-Embedding-0.6B (default) | last token | query: an instruction; documents: none |
//! | `BertModel` | intfloat/multilingual-e5-small | mean | `query: ` / `passage: ` |
//! | `XLMRobertaModel` | e5 variants built on XLM-R | mean | `query: ` / `passage: ` |
//! | `ModernBertModel` | ibm-granite/granite-embedding-97m-multilingual-r2 | CLS | none |
//!
//! Texts are embedded one at a time, so there is no padding to mask, and
//! inputs are cut at [`MAX_TOKENS`]. Qwen3's model keeps a KV cache inside;
//! every text runs on a fresh clone (its weights are shared, not copied), so
//! nothing carries over and calls never wait on each other.

use std::collections::HashSet;
use std::path::Path;

use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::{bert, modernbert, qwen3, xlm_roberta};
use tokenizers::Tokenizer;

use crate::{Embedder, Result, err, normalize};

/// Every chunk and query is cut at this many tokens (§9.1).
pub const MAX_TOKENS: usize = 512;

/// The query instruction from Qwen3-Embedding's config_sentence_transformers.json.
pub const QWEN3_QUERY: &str =
    "Instruct: Given a web search query, retrieve relevant passages that answer the query\nQuery:";

pub const SUPPORTED: &[&str] = &[
    "Qwen3ForCausalLM",
    "Qwen3Model",
    "BertModel",
    "XLMRobertaModel",
    "ModernBertModel",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Qwen3,
    Bert,
    XlmRoberta,
    ModernBert,
}

impl Family {
    pub fn from_architectures(config: &serde_json::Value) -> Result<Family> {
        let archs: Vec<&str> = config["architectures"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
            .unwrap_or_default();
        for a in &archs {
            match *a {
                "Qwen3ForCausalLM" | "Qwen3Model" => return Ok(Family::Qwen3),
                "BertModel" => return Ok(Family::Bert),
                "XLMRobertaModel" => return Ok(Family::XlmRoberta),
                "ModernBertModel" => return Ok(Family::ModernBert),
                _ => {}
            }
        }
        Err(err(format!(
            "the model's architectures {archs:?} aren't supported; Firelight runs {SUPPORTED:?}"
        )))
    }

    pub fn name(self) -> &'static str {
        match self {
            Family::Qwen3 => "qwen3",
            Family::Bert => "bert",
            Family::XlmRoberta => "xlm-roberta",
            Family::ModernBert => "modernbert",
        }
    }

    fn prefixes(self) -> (&'static str, &'static str) {
        match self {
            Family::Qwen3 => (QWEN3_QUERY, ""),
            Family::Bert | Family::XlmRoberta => ("query: ", "passage: "),
            Family::ModernBert => ("", ""),
        }
    }
}

enum Net {
    Qwen3(qwen3::Model),
    Bert(bert::BertModel),
    XlmRoberta(xlm_roberta::XLMRobertaModel),
    ModernBert(modernbert::ModernBert),
}

pub struct Candle {
    net: Net,
    tok: Tokenizer,
    family: Family,
    id: String,
    dims: usize,
    device: Device,
}

fn ce(e: impl std::fmt::Display) -> crate::Error {
    err(e.to_string())
}

/// Tensor names in a safetensors file, from its header.
fn tensor_names(bytes: &[u8]) -> Result<HashSet<String>> {
    let n = bytes
        .get(..8)
        .and_then(|b| <[u8; 8]>::try_from(b).ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| err("model.safetensors is too short"))?;
    let n = usize::try_from(n).map_err(ce)?;
    let header = bytes
        .get(8..8 + n)
        .ok_or_else(|| err("model.safetensors has a broken header"))?;
    let v: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(header).map_err(ce)?;
    Ok(v.into_iter()
        .map(|(k, _)| k)
        .filter(|k| k != "__metadata__")
        .collect())
}

impl Candle {
    /// Load a verified model folder (config.json, tokenizer.json,
    /// model.safetensors). `id` names the vectors (repo and revision).
    pub fn load(dir: &Path, id: &str) -> Result<Candle> {
        let read = |f: &str| {
            std::fs::read(dir.join(f))
                .map_err(|e| err(format!("cannot read {}: {e}", dir.join(f).display())))
        };
        let config: serde_json::Value =
            serde_json::from_slice(&read("config.json")?).map_err(ce)?;
        let family = Family::from_architectures(&config)?;
        let mut tok = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(ce)?;
        tok.with_truncation(Some(tokenizers::TruncationParams {
            max_length: MAX_TOKENS,
            ..Default::default()
        }))
        .map_err(ce)?;
        tok.with_padding(None);
        let weights = read("model.safetensors")?;
        let names = tensor_names(&weights)?;
        let device = Device::Cpu;
        // Checkpoints saved by sentence-transformers drop the `model.` prefix
        // candle expects (and some add one): map each asked-for name onto the
        // one the file has.
        let vb = VarBuilder::from_buffered_safetensors(weights, DType::F32, &device)
            .map_err(ce)?
            .rename_f(move |name: &str| {
                if names.contains(name) {
                    return name.to_string();
                }
                if let Some(s) = name.strip_prefix("model.")
                    && names.contains(s)
                {
                    return s.to_string();
                }
                let with = format!("model.{name}");
                if names.contains(&with) {
                    with
                } else {
                    name.to_string()
                }
            });
        let cfg = |c: &serde_json::Value| c.to_string();
        let (net, dims) = match family {
            Family::Qwen3 => {
                let c: qwen3::Config = serde_json::from_str(&cfg(&config)).map_err(ce)?;
                let d = c.hidden_size;
                (Net::Qwen3(qwen3::Model::new(&c, vb).map_err(ce)?), d)
            }
            Family::Bert => {
                let c: bert::Config = serde_json::from_str(&cfg(&config)).map_err(ce)?;
                let d = c.hidden_size;
                (Net::Bert(bert::BertModel::load(vb, &c).map_err(ce)?), d)
            }
            Family::XlmRoberta => {
                let c: xlm_roberta::Config = serde_json::from_str(&cfg(&config)).map_err(ce)?;
                let d = c.hidden_size;
                let vb = if names_has_prefix(&vb, "roberta.") {
                    vb.pp("roberta")
                } else {
                    vb
                };
                (
                    Net::XlmRoberta(xlm_roberta::XLMRobertaModel::new(&c, vb).map_err(ce)?),
                    d,
                )
            }
            Family::ModernBert => {
                let c: modernbert::Config = serde_json::from_str(&cfg(&config)).map_err(ce)?;
                let d = c.hidden_size;
                (
                    Net::ModernBert(modernbert::ModernBert::load(vb, &c).map_err(ce)?),
                    d,
                )
            }
        };
        Ok(Candle {
            net,
            tok,
            family,
            id: format!("candle:{id}:{}", family.name()),
            dims,
            device,
        })
    }

    pub fn family(&self) -> Family {
        self.family
    }

    fn one(&self, text: &str) -> Result<Vec<f32>> {
        let enc = self.tok.encode(text, true).map_err(ce)?;
        let ids = enc.get_ids();
        if ids.is_empty() {
            return Ok(vec![0.0; self.dims]);
        }
        let n = ids.len();
        let input = Tensor::new(ids, &self.device)
            .and_then(|t| t.unsqueeze(0))
            .map_err(ce)?;
        let ones = input.ones_like().map_err(ce)?;
        let hidden = match &self.net {
            Net::Qwen3(m) => m.clone().forward(&input, 0),
            Net::Bert(m) => input
                .zeros_like()
                .and_then(|tt| m.forward(&input, &tt, Some(&ones))),
            Net::XlmRoberta(m) => input
                .zeros_like()
                .and_then(|tt| m.forward(&input, &ones, &tt, None, None, None)),
            Net::ModernBert(m) => m.forward(&input, &ones),
        }
        .map_err(ce)?;
        // hidden: (1, n, dims)
        let pooled = match self.family {
            Family::Qwen3 => hidden.i((0, n - 1)),
            Family::ModernBert => hidden.i((0, 0)),
            Family::Bert | Family::XlmRoberta => hidden.i(0).and_then(|h| h.mean(0)),
        }
        .and_then(|t| t.to_dtype(DType::F32))
        .and_then(|t| t.to_vec1::<f32>())
        .map_err(ce)?;
        let mut v = pooled;
        normalize(&mut v);
        Ok(v)
    }
}

fn names_has_prefix(vb: &VarBuilder, p: &str) -> bool {
    vb.contains_tensor(&format!("{p}embeddings.word_embeddings.weight"))
}

impl Embedder for Candle {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn dims(&self) -> usize {
        self.dims
    }
    fn max_tokens(&self) -> usize {
        MAX_TOKENS
    }
    fn count_tokens(&self, text: &str) -> usize {
        self.tok
            .encode(text, false)
            .map_or_else(|_| crate::approx_tokens(text), |e| e.len())
    }
    fn embed_documents(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let (_, doc) = self.family.prefixes();
        texts
            .iter()
            .map(|t| self.one(&format!("{doc}{t}")))
            .collect()
    }
    fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        let (q, _) = self.family.prefixes();
        self.one(&format!("{q}{query}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families() {
        let f = |a: &str| Family::from_architectures(&serde_json::json!({ "architectures": [a] }));
        assert_eq!(f("Qwen3ForCausalLM").ok(), Some(Family::Qwen3));
        assert_eq!(f("BertModel").ok(), Some(Family::Bert));
        assert_eq!(f("XLMRobertaModel").ok(), Some(Family::XlmRoberta));
        assert_eq!(f("ModernBertModel").ok(), Some(Family::ModernBert));
        let e = f("LlamaForCausalLM")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            e.contains("Qwen3ForCausalLM") && e.contains("LlamaForCausalLM"),
            "{e}"
        );
    }

    #[test]
    fn header_names() {
        let header = br#"{"__metadata__":{},"embed_tokens.weight":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#;
        let mut b = (header.len() as u64).to_le_bytes().to_vec();
        b.extend_from_slice(header);
        b.extend_from_slice(&[0; 4]);
        let n = tensor_names(&b).unwrap_or_default();
        assert_eq!(n.into_iter().collect::<Vec<_>>(), ["embed_tokens.weight"]);
        assert!(tensor_names(&[1, 2]).is_err());
    }
}
