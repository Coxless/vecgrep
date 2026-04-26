//! Embedding runtime — DESIGN.md §7.1 / §11.1 / IMPLEMENTATION.md Step 5.
//!
//! Wraps `ort` (ONNX Runtime) and `tokenizers` (HuggingFace) to produce 384-dim
//! f32 vectors from text using `intfloat/multilingual-e5-small`.
//!
//! ## Design notes
//!
//! * E5 requires `"query: "` / `"passage: "` prefixes (DESIGN §7.1).
//!   `embed_query` and `embed_passages` apply them; callers never need to.
//! * Output vectors are L2-normalised, so cosine similarity == dot product
//!   (Step 6 brute-force search relies on this).
//! * fp16 storage is deferred to Step 10; this module returns f32.
//! * Model auto-download is M3 (Step 16).  For M1, set `VSGREP_MODEL_DIR` to a
//!   directory containing `model.onnx` and `tokenizer.json`.
//! * `session.run` requires `&mut Session`, so `embed_query` / `embed_passages`
//!   take `&mut self`.  For parallel indexing (Step 13), use one `Embedder` per
//!   worker thread or wrap in a `Mutex`.

#![allow(dead_code)] // wired into the pipeline in Step 8

use std::{
    env,
    path::{Path, PathBuf},
    time::Instant,
};

use ndarray::{Array2, ArrayView1, ArrayView2, Axis};
use ort::{session::Session, value::TensorRef};
use thiserror::Error;
use tokenizers::{
    PaddingDirection, PaddingParams, PaddingStrategy, Tokenizer, TruncationDirection,
    TruncationParams, TruncationStrategy,
};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Expected output dimensionality of the default model.  Asserted in `Embedder::new`.
pub const E5_DIM: usize = 384;

const DEFAULT_MAX_LEN: usize = 512;
const DEFAULT_BATCH_SIZE: usize = 64;
const ENV_MODEL_DIR: &str = "VSGREP_MODEL_DIR";
const MODEL_FILENAME: &str = "model.onnx";
const TOKENIZER_FILENAME: &str = "tokenizer.json";

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EmbedderConfig {
    pub onnx_path: PathBuf,
    pub tokenizer_path: PathBuf,
    /// Maximum token sequence length (default 512, DESIGN §7.1).
    pub max_len: usize,
    /// Batch size for `embed_passages` (default 64).
    pub batch_size: usize,
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            onnx_path: PathBuf::new(),
            tokenizer_path: PathBuf::new(),
            max_len: DEFAULT_MAX_LEN,
            batch_size: DEFAULT_BATCH_SIZE,
        }
    }
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("failed to load ONNX model `{path}`: {source}")]
    ModelLoad {
        path: PathBuf,
        #[source]
        source: ort::Error,
    },

    #[error("failed to load tokenizer `{path}`: {source}")]
    TokenizerLoad {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("tokenizer error: {0}")]
    Tokenize(String),

    #[error("ONNX inference failed: {0}")]
    Inference(#[from] ort::Error),

    #[error("expected output dim {expected}, got {actual}")]
    DimMismatch { expected: usize, actual: usize },

    #[error(
        "VSGREP_MODEL_DIR is not set; download the model and run:\n  \
         export VSGREP_MODEL_DIR=/path/to/model-dir\n  \
         (see tests/fixtures/README.md for instructions)"
    )]
    ModelDirNotSet,

    #[error("model directory `{dir}` is missing `{file}`")]
    ModelFileMissing { dir: PathBuf, file: &'static str },
}

// ── Embedder ──────────────────────────────────────────────────────────────────

pub struct Embedder {
    session: Session,
    tokenizer: Tokenizer,
    dim: usize,
    batch_size: usize,
    needs_token_type_ids: bool,
}

impl Embedder {
    /// Load the ONNX model and tokenizer, assert output dim == [`E5_DIM`].
    ///
    /// Instruments the `model.load` span (DESIGN §11.1).
    pub fn new(cfg: EmbedderConfig) -> Result<Self, EmbedError> {
        let _span = tracing::info_span!(
            "model.load",
            onnx = %cfg.onnx_path.display(),
        )
        .entered();
        let load_start = Instant::now();

        let session = Session::builder()
            .map_err(|e| EmbedError::ModelLoad {
                path: cfg.onnx_path.clone(),
                source: e,
            })?
            .commit_from_file(&cfg.onnx_path)
            .map_err(|e| EmbedError::ModelLoad {
                path: cfg.onnx_path.clone(),
                source: e,
            })?;

        let needs_token_type_ids = session.inputs.iter().any(|i| i.name == "token_type_ids");

        let mut tokenizer =
            Tokenizer::from_file(&cfg.tokenizer_path).map_err(|e| EmbedError::TokenizerLoad {
                path: cfg.tokenizer_path.clone(),
                source: e,
            })?;

        let pad_id = tokenizer.token_to_id("<pad>").unwrap_or(1);
        tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            direction: PaddingDirection::Right,
            pad_id,
            pad_type_id: 0,
            pad_token: "<pad>".to_string(),
            pad_to_multiple_of: None,
        }));
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: cfg.max_len,
                strategy: TruncationStrategy::LongestFirst,
                stride: 0,
                direction: TruncationDirection::Right,
            }))
            .map_err(|e| EmbedError::TokenizerLoad {
                path: cfg.tokenizer_path.clone(),
                source: Box::new(std::io::Error::other(e.to_string())),
            })?;

        let mut embedder = Embedder {
            session,
            tokenizer,
            dim: E5_DIM,
            batch_size: cfg.batch_size,
            needs_token_type_ids,
        };

        // Warmup inference confirms actual dim (catches wrong-model mistakes).
        let warmup = embedder.run_batch(&[prefixed_query("warmup").as_str()])?;
        let actual_dim = warmup.first().map(Vec::len).unwrap_or(0);
        if actual_dim != E5_DIM {
            return Err(EmbedError::DimMismatch {
                expected: E5_DIM,
                actual: actual_dim,
            });
        }
        embedder.dim = actual_dim;

        tracing::info!(wall_ms = load_start.elapsed().as_millis(), dim = actual_dim);
        Ok(embedder)
    }

    /// Output vector dimensionality (always 384 after `new` succeeds).
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Embed a single query string.  Prefixes `"query: "`, mean-pools, L2-normalises.
    ///
    /// Instruments `query.tokenize` and `query.embed` spans (DESIGN §11.1).
    pub fn embed_query(&mut self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let prefixed = prefixed_query(text);

        let enc = {
            let _span = tracing::info_span!("query.tokenize").entered();
            self.tokenizer
                .encode(prefixed, true)
                .map_err(|e| EmbedError::Tokenize(e.to_string()))?
        };

        let seq_len = enc.len();
        let _span = tracing::info_span!("query.embed", seq_len).entered();
        let start = Instant::now();

        let ids_data: Vec<i64> = enc.get_ids().iter().map(|&t| t as i64).collect();
        let mask_data: Vec<i64> = enc.get_attention_mask().iter().map(|&t| t as i64).collect();

        let ids = Array2::from_shape_vec((1, seq_len), ids_data)
            .map_err(|e| EmbedError::Tokenize(e.to_string()))?;
        let mask = Array2::from_shape_vec((1, seq_len), mask_data)
            .map_err(|e| EmbedError::Tokenize(e.to_string()))?;

        let mut result = self.infer(&ids, &mask)?;
        tracing::info!(wall_ms = start.elapsed().as_millis(), seq_len);

        result
            .pop()
            .ok_or_else(|| EmbedError::Tokenize("empty inference output".into()))
    }

    /// Embed a batch of passage strings.  Prefixes `"passage: "`, batches at
    /// `cfg.batch_size`, mean-pools, L2-normalises.
    ///
    /// Instruments `tokenize.batch` and `embed.batch` spans (DESIGN §11.1).
    pub fn embed_passages(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let prefixed: Vec<String> = texts.iter().map(|t| prefixed_passage(t)).collect();
        let mut results = Vec::with_capacity(texts.len());

        for chunk in prefixed.chunks(self.batch_size) {
            let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
            let batch = self.run_batch(&refs)?;
            results.extend(batch);
        }

        Ok(results)
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Tokenize a pre-prefixed batch and run ONNX inference.
    ///
    /// Instruments `tokenize.batch` and `embed.batch` spans (DESIGN §11.1).
    fn run_batch(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let n = texts.len();
        if n == 0 {
            return Ok(vec![]);
        }

        let encodings = {
            let _span = tracing::info_span!("tokenize.batch", items = n).entered();
            self.tokenizer
                .encode_batch(texts.to_vec(), true)
                .map_err(|e| EmbedError::Tokenize(e.to_string()))?
        };

        let s = encodings.first().map(|e| e.len()).unwrap_or(0);
        if s == 0 {
            return Ok(vec![vec![0.0; self.dim]; n]);
        }

        let mut ids_flat = Vec::with_capacity(n * s);
        let mut mask_flat = Vec::with_capacity(n * s);
        for enc in &encodings {
            ids_flat.extend(enc.get_ids().iter().map(|&t| t as i64));
            mask_flat.extend(enc.get_attention_mask().iter().map(|&t| t as i64));
        }

        let ids = Array2::from_shape_vec((n, s), ids_flat)
            .map_err(|e| EmbedError::Tokenize(e.to_string()))?;
        let mask = Array2::from_shape_vec((n, s), mask_flat)
            .map_err(|e| EmbedError::Tokenize(e.to_string()))?;

        let span = tracing::info_span!("embed.batch", items = n);
        let _guard = span.enter();
        let start = Instant::now();

        let result = self.infer(&ids, &mask)?;

        let wall_s = start.elapsed().as_secs_f64();
        let chunks_per_sec = if wall_s > 1e-9 {
            n as f64 / wall_s
        } else {
            0.0
        };
        tracing::info!(
            items = n,
            wall_ms = (wall_s * 1000.0) as u64,
            chunks_per_sec
        );

        Ok(result)
    }

    /// Run the ONNX session and return mean-pooled, L2-normalised vectors.
    fn infer(
        &mut self,
        ids: &Array2<i64>,
        mask: &Array2<i64>,
    ) -> Result<Vec<Vec<f32>>, EmbedError> {
        let (b, s) = (ids.nrows(), ids.ncols());

        // TensorRef borrows the ndarray data without copying.
        let ids_ref = TensorRef::<i64>::from_array_view(ids.view())?;
        let mask_ref = TensorRef::<i64>::from_array_view(mask.view())?;

        let outputs = if self.needs_token_type_ids {
            let type_ids = Array2::<i64>::zeros((b, s));
            let type_ids_ref = TensorRef::<i64>::from_array_view(type_ids.view())?;
            self.session.run(ort::inputs![
                "input_ids"      => ids_ref,
                "attention_mask" => mask_ref,
                "token_type_ids" => type_ids_ref,
            ])?
        } else {
            self.session.run(ort::inputs![
                "input_ids"      => ids_ref,
                "attention_mask" => mask_ref,
            ])?
        };

        // last_hidden_state: [B, S, D]
        let hidden = outputs[0].try_extract_array::<f32>()?;
        let hidden3 = hidden
            .into_dimensionality::<ndarray::Ix3>()
            .map_err(|e| EmbedError::Tokenize(format!("unexpected output rank: {e}")))?;

        let d = hidden3.shape()[2];

        let mut embeddings = Vec::with_capacity(b);
        for i in 0..b {
            let row: ArrayView2<f32> = hidden3.index_axis(Axis(0), i);
            let mask_row: ArrayView1<i64> = mask.index_axis(Axis(0), i);
            let mut vec = mean_pool(row, mask_row, d);
            l2_normalize(&mut vec);
            embeddings.push(vec);
        }

        Ok(embeddings)
    }
}

// ── Model path resolution ─────────────────────────────────────────────────────

/// Resolve `(onnx_path, tokenizer_path)` from `VSGREP_MODEL_DIR`.
///
/// Set `VSGREP_MODEL_DIR` to a directory containing `model.onnx` and
/// `tokenizer.json`.  See `tests/fixtures/README.md` for download instructions.
pub fn resolve_model_paths() -> Result<(PathBuf, PathBuf), EmbedError> {
    let dir = env::var(ENV_MODEL_DIR).map_err(|_| EmbedError::ModelDirNotSet)?;
    let dir = PathBuf::from(dir);
    check_model_file(&dir, MODEL_FILENAME)?;
    check_model_file(&dir, TOKENIZER_FILENAME)?;
    Ok((dir.join(MODEL_FILENAME), dir.join(TOKENIZER_FILENAME)))
}

fn check_model_file(dir: &Path, file: &'static str) -> Result<(), EmbedError> {
    if !dir.join(file).exists() {
        return Err(EmbedError::ModelFileMissing {
            dir: dir.to_owned(),
            file,
        });
    }
    Ok(())
}

// ── Pure helpers ──────────────────────────────────────────────────────────────

pub(crate) fn prefixed_query(s: &str) -> String {
    format!("query: {s}")
}

pub(crate) fn prefixed_passage(s: &str) -> String {
    format!("passage: {s}")
}

/// Mean-pool `hidden` ([S, D]) weighted by binary `mask` ([S]).
fn mean_pool(hidden: ArrayView2<f32>, mask: ArrayView1<i64>, d: usize) -> Vec<f32> {
    let s = hidden.shape()[0];
    let mut sum = vec![0.0f32; d];
    let mut count = 0.0f32;
    for t in 0..s {
        if mask[t] != 0 {
            let row = hidden.index_axis(Axis(0), t);
            for j in 0..d {
                sum[j] += row[j];
            }
            count += 1.0;
        }
    }
    if count > 1e-9 {
        sum.iter_mut().for_each(|x| *x /= count);
    }
    sum
}

/// In-place L2 normalisation.  Zero vectors are left unchanged.
pub(crate) fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use ndarray::{arr1, arr2};

    use super::*;

    // ── l2_normalize ──────────────────────────────────────────────────────────

    #[test]
    fn l2_normalize_unit_vector_unchanged() {
        let mut v = vec![1.0f32, 0.0, 0.0];
        l2_normalize(&mut v);
        assert!((v[0] - 1.0).abs() < 1e-6);
        assert!(v[1].abs() < 1e-6);
    }

    #[test]
    fn l2_normalize_produces_unit_norm() {
        let mut v = vec![3.0f32, 4.0];
        l2_normalize(&mut v);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "norm={norm}");
    }

    #[test]
    fn l2_normalize_zero_vector_unchanged() {
        let mut v = vec![0.0f32, 0.0, 0.0];
        l2_normalize(&mut v);
        assert!(v.iter().all(|&x| x == 0.0));
    }

    // ── mean_pool ─────────────────────────────────────────────────────────────

    #[test]
    fn mean_pool_masks_correctly() {
        // rows 0-1 unmasked, rows 2-3 masked: mean = average of rows 0-1
        let hidden = arr2(&[[1.0f32, 2.0], [3.0, 4.0], [99.0, 99.0], [99.0, 99.0]]);
        let mask = arr1(&[1i64, 1, 0, 0]);
        let result = mean_pool(hidden.view(), mask.view(), 2);
        assert!(
            (result[0] - 2.0).abs() < 1e-6,
            "expected 2.0, got {}",
            result[0]
        );
        assert!(
            (result[1] - 3.0).abs() < 1e-6,
            "expected 3.0, got {}",
            result[1]
        );
    }

    #[test]
    fn mean_pool_all_masked_returns_zeros() {
        let hidden = arr2(&[[1.0f32, 2.0], [3.0, 4.0]]);
        let mask = arr1(&[0i64, 0]);
        let result = mean_pool(hidden.view(), mask.view(), 2);
        assert!(result.iter().all(|&x| x == 0.0));
    }

    // ── prefix functions ──────────────────────────────────────────────────────

    #[test]
    fn prefix_query() {
        assert_eq!(prefixed_query("foo"), "query: foo");
    }

    #[test]
    fn prefix_passage() {
        assert_eq!(prefixed_passage("bar"), "passage: bar");
    }

    // ── EmbedderConfig defaults ───────────────────────────────────────────────

    #[test]
    fn embedder_config_default_values() {
        let cfg = EmbedderConfig::default();
        assert_eq!(cfg.max_len, DEFAULT_MAX_LEN);
        assert_eq!(cfg.batch_size, DEFAULT_BATCH_SIZE);
    }

    // ── resolve_model_paths ───────────────────────────────────────────────────

    #[test]
    fn resolve_model_paths_fails_when_env_unset() {
        let saved = env::var(ENV_MODEL_DIR).ok();
        // Safety: test binary is single-threaded by default (cargo test --test-threads=1
        // is not required, but env mutation in tests is accepted in this project for M1).
        unsafe {
            env::remove_var(ENV_MODEL_DIR);
        }
        let err = resolve_model_paths().unwrap_err();
        assert!(matches!(err, EmbedError::ModelDirNotSet));
        unsafe {
            if let Some(v) = saved {
                env::set_var(ENV_MODEL_DIR, v);
            }
        }
    }

    #[test]
    fn resolve_model_paths_fails_on_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            env::set_var(ENV_MODEL_DIR, dir.path());
        }
        let err = resolve_model_paths().unwrap_err();
        assert!(matches!(err, EmbedError::ModelFileMissing { .. }));
        unsafe {
            env::remove_var(ENV_MODEL_DIR);
        }
    }
}
