# tests/fixtures

Static fixtures used by vsgrep's test suite.

## Embedding model (`models/` — gitignored)

Integration tests in `tests/embed_e5.rs` require the `intfloat/multilingual-e5-small`
ONNX model and its tokenizer.  These files are **not checked in** (≈118 MB combined).

### Download with `huggingface-cli` (recommended)

```bash
pip install huggingface-hub          # if not already installed
huggingface-cli download intfloat/multilingual-e5-small \
    --include "onnx/model.onnx" "tokenizer.json" \
    --local-dir tests/fixtures/models/multilingual-e5-small
```

### Download with `curl`

```bash
MODEL_DIR=tests/fixtures/models/multilingual-e5-small
mkdir -p "$MODEL_DIR"

BASE=https://huggingface.co/intfloat/multilingual-e5-small/resolve/main
curl -L "$BASE/onnx/model.onnx"   -o "$MODEL_DIR/model.onnx"
curl -L "$BASE/tokenizer.json"    -o "$MODEL_DIR/tokenizer.json"
```

### Run the integration tests

```bash
export VSGREP_MODEL_DIR=tests/fixtures/models/multilingual-e5-small
cargo test --test embed_e5 -- --ignored
```

### Expected directory layout

```
tests/fixtures/models/multilingual-e5-small/
├── model.onnx        (~117 MB)
└── tokenizer.json    (~2 MB)
```

`VSGREP_MODEL_DIR` must point to this directory (the one containing `model.onnx`
and `tokenizer.json` directly — not a parent).
