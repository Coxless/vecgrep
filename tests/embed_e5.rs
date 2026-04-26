//! Integration tests for the embedding runtime.
//!
//! All tests are `#[ignore]` — they require a real ONNX model on disk.
//!
//! Setup:
//!   export VSGREP_MODEL_DIR=tests/fixtures/models/multilingual-e5-small
//!   cargo test --test embed_e5 -- --ignored
//!
//! See tests/fixtures/README.md for model download instructions.

use vsgrep::embed::{E5_DIM, Embedder, EmbedderConfig, resolve_model_paths};

fn make_embedder() -> Embedder {
    let (onnx, tok) = resolve_model_paths()
        .expect("VSGREP_MODEL_DIR must point to a directory with model.onnx and tokenizer.json");
    Embedder::new(EmbedderConfig {
        onnx_path: onnx,
        tokenizer_path: tok,
        ..EmbedderConfig::default()
    })
    .expect("Embedder::new failed")
}

/// `Embedder::new` succeeds and dim == 384 (acceptance criterion from IMPLEMENTATION.md Step 5).
#[test]
#[ignore]
fn embedder_dim_is_384() {
    let e = make_embedder();
    assert_eq!(e.dim(), E5_DIM, "output dimension must be {E5_DIM}");
}

/// Every query embedding is a unit vector (L2 norm ≈ 1.0).
#[test]
#[ignore]
fn embed_query_is_unit_norm() {
    let mut e = make_embedder();
    let v = e.embed_query("semantic search in Rust").unwrap();
    assert_eq!(v.len(), E5_DIM);
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-4, "expected unit norm, got {norm}");
}

/// Batch output matches single-item output up to float noise (proves padding doesn't
/// perturb the unpadded row).
#[test]
#[ignore]
fn batch_matches_single() {
    let mut e = make_embedder();

    let texts = ["hello world", "semantic search"];
    let batch = e.embed_passages(&texts).unwrap();
    let single0 = e.embed_passages(&[texts[0]]).unwrap();
    let single1 = e.embed_passages(&[texts[1]]).unwrap();

    assert_eq!(batch.len(), 2);
    for (b, s) in batch[0].iter().zip(&single0[0]) {
        assert!(
            (b - s).abs() < 1e-4,
            "batch[0] vs single[0] mismatch: {b} vs {s}"
        );
    }
    for (b, s) in batch[1].iter().zip(&single1[0]) {
        assert!(
            (b - s).abs() < 1e-4,
            "batch[1] vs single[1] mismatch: {b} vs {s}"
        );
    }
}

/// Batch throughput at size 64 must exceed per-item throughput (linear-to-sub-linear
/// scaling acceptance criterion from IMPLEMENTATION.md Step 5).
#[test]
#[ignore]
fn batch_throughput_scales() {
    let mut e = make_embedder();

    let texts: Vec<&str> = (0..64).map(|_| "the quick brown fox jumps").collect();

    // Single-item baseline
    let t0 = std::time::Instant::now();
    for &t in &texts {
        e.embed_passages(&[t]).unwrap();
    }
    let single_ms = t0.elapsed().as_millis();

    // Full batch
    let t1 = std::time::Instant::now();
    e.embed_passages(&texts).unwrap();
    let batch_ms = t1.elapsed().as_millis().max(1);

    assert!(
        batch_ms < single_ms,
        "batch ({batch_ms} ms) should be faster than 64 single calls ({single_ms} ms)"
    );
}
