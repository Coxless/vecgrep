# vsgrep — 基本構想ドキュメント

> ripgrep ライクな使用感で、ローカルファイルに対して超高速なセマンティック(ベクトル)検索を行う Rust 製 CLI。

---

## 1. プロダクト・ビジョン

- **ripgrep を置き換えるのではなく補完する**。grep できないもの (言い換え・概念検索・多言語) を grep と同じテンポで引けることを目指す。
- `vsgrep "TCP 再送制御を行う箇所" .` のように **意味で検索**できる。
- 初回以外はすべてローカル処理。ネットワーク不要、秘密情報を外部に送らない。
- **「インストールして即使える」** を最重要 UX とする。

---

## 2. 設計原則 (優先度順)

| # | 原則 | 含意 |
|---|------|------|
| 1 | **高速性** | クエリのエンドツーエンドで < 50ms (10万チャンク規模、ウォームキャッシュ) を目標。 |
| 2 | **計測可能性** | 全フェーズの所要時間を常時収集。`--timings` で内訳を即可視化。最適化判断はベンチ数値に基づく。 |
| 3 | **単一バイナリ** | 動的リンク依存を最小化。`cargo install vsgrep` / GitHub Releases の1ファイルで動く。 |
| 4 | **grep 互換の体感** | 引数順・出力形式・終了コード・パイプ親和性を grep/ripgrep に寄せる。 |
| 5 | **ゼロ設定で動く** | 設定ファイル必須にしない。デフォルトで賢く振る舞う。 |
| 6 | **差し替え可能** | モデル・距離関数・チャンク戦略は全て差し替え可能なプラガブル構造。 |

---

## 3. ユーザ確定事項 (本構想の前提)

| 項目 | 決定 |
|------|------|
| 検索対象 | コード / 自然言語 **両方を汎用的に扱う** (ファイル拡張子で自動判定) |
| 既定モデル | **`intfloat/multilingual-e5-small`** (384 次元)。`--model` で差し替え可能。 |
| インデックス | **永続インデックス** (`.vsgrep/` ディレクトリにキャッシュ)。`--no-cache` 併設。 |
| `.gitignore` 連携 | **初回 `index` 時に `.vsgrep/` を自動追記** (git リポジトリ内のみ、既存行があればスキップ)。`--no-gitignore` で抑止可。 |
| 検索アルゴリズム | **HNSW** (近似最近傍)。小規模時は自動でブルートフォースへフォールバック。 |
| CLI エイリアス | **`vg` を同梱**。バイナリのハードリンク or シンボリックリンクとしてインストーラで配置。 |
| ライセンス | **MIT**。 |
| テレメトリ | **無し** (常時オフ、ネットワーク送信なし)。 |

---

## 4. CLI UX

### 4.1 基本形 (grep/ripgrep に寄せる)

```bash
# 基本: カレント以下を意味検索
vsgrep "再試行ロジック"
vg "再試行ロジック"            # 同梱エイリアス (vsgrep と等価)

# パス指定
vsgrep "user authentication flow" src/

# 複数パス
vsgrep "database connection pool" src/ lib/

# 件数制限 (デフォルト 10 件)
vsgrep -k 20 "error handling"

# スコア閾値
vsgrep --threshold 0.75 "parse JSON"

# ファイルタイプ絞り込み (ripgrep 互換)
vsgrep -t rust "lock-free queue"
vsgrep -g '!**/*.test.ts' "feature flag"

# grep と組み合わせ (ハイブリッド検索の原点)
vsgrep "認証処理" | rg -F "TODO"
```

### 4.2 サブコマンド

```bash
vsgrep index [PATH]       # インデックスの明示的な構築・更新
vsgrep index --watch      # fs 変更を検知してインクリメンタル更新
vsgrep status             # 現在のインデックスの統計情報
vsgrep clean              # .vsgrep/ を削除
vsgrep model list         # 利用可能なモデル一覧
vsgrep model use <NAME>   # デフォルトモデルを切替
vsgrep model add <ONNX>   # 任意の ONNX モデルを登録
```

検索時に `.vsgrep/` が存在しなければ **自動で初回インデックスを構築** (対話なし、進捗バー表示)。

初回 `index` 時、カレントが git リポジトリ内であれば `.gitignore` に `.vsgrep/` 行を自動追記する (既に存在する場合はスキップ)。追記時は stderr に 1 行通知し、`--no-gitignore` で抑止可能。

### 4.3 出力フォーマット

デフォルトは grep 互換:

```
src/retry.rs:42:    fn exponential_backoff(attempt: u32) -> Duration {
src/net/mod.rs:118: pub async fn retry_with_jitter<F>(f: F) -> Result<T>
```

オプション:
- `--score` : スコア列を付加 `file:line:score:content`
- `--json`  : JSON Lines 出力 (エージェント・スクリプト用途)
- `--context N` / `-C N` : 周辺行を同時出力 (grep 互換)
- `--heading` : ファイル名をヘッダとして表示 (ripgrep 互換)
- `--timings` : 処理フェーズ別の所要時間を stderr にサマリ出力 (詳細は §11)
- `--trace <FILE>` : Chrome Tracing 形式の JSON を書き出し (Perfetto / chrome://tracing で可視化)

---

## 5. アーキテクチャ

```
┌────────────────────────────────────────────────────────────────┐
│                         vsgrep CLI                            │
└────────────────────────────────────────────────────────────────┘
           │                              │
           ▼                              ▼
  ┌──────────────────┐          ┌──────────────────┐
  │  Indexer         │          │  Query Engine    │
  │  ──────────────  │          │  ──────────────  │
  │  ① walk (ignore) │          │  ① embed query   │
  │  ② chunk         │          │  ② HNSW search   │
  │  ③ embed (batch) │          │  ③ rerank (opt.) │
  │  ④ persist       │          │  ④ format/print  │
  └──────────────────┘          └──────────────────┘
           │                              │
           └──────────┬───────────────────┘
                      ▼
           ┌──────────────────────┐
           │   .vsgrep/ (store)  │
           │   ├── meta.bin       │  マニフェスト (モデル ID、次元、チャンク数、...)
           │   ├── chunks.bin     │  チャンクメタ (path, span, hash)
           │   ├── vectors.bin    │  密ベクトル (f16 or f32, mmap)
           │   ├── hnsw.bin       │  HNSW グラフ (mmap)
           │   └── file_index.bin │  ファイル→チャンク ID 逆引き (差分更新用)
           └──────────────────────┘
```

### 5.1 差分インデックス更新

- ファイルのハッシュ (blake3 を想定) と mtime で変更検知。
- 削除/変更されたファイルの旧チャンクは tombstone マーク → 再構築時に圧縮。
- HNSW は挿入のみサポート。削除は tombstone とし、一定割合を超えたら再構築。

### 5.2 並列化

- インデックス構築: `rayon` でファイル並列、モデル推論はバッチ (例 64) でまとめて実行。
- 検索: クエリ 1 本であれば HNSW 1 スレッドで十分。複数クエリやバッチ時に並列化。

---

## 6. 技術スタック (採用候補)

| 領域 | クレート | 理由 |
|------|----------|------|
| ONNX 推論 | **`ort`** (onnxruntime-rs) | 最速。静的リンク可。CUDA/CoreML/DirectML への将来拡張あり。 |
| トークナイザ | **`tokenizers`** (HF) | Rust 実装で高速。`tokenizer.json` をそのまま読める。 |
| HNSW | **`hnsw_rs`** | 純 Rust、mmap 対応、実績あり。`instant-distance` も選択肢。 |
| ファイル走査 | **`ignore`** | ripgrep 由来。`.gitignore` / `.ignore` を自動尊重。 |
| SIMD 類似度 | **`simsimd`** or 自前 | ブルートフォース / reranker 用。AVX2/AVX-512/NEON 対応。 |
| ハッシュ | **`blake3`** | 高速。SIMD。差分検知用。 |
| mmap | **`memmap2`** | ベクトル配列・HNSW グラフの zero-copy ロード。 |
| シリアライズ | **`rkyv`** (ゼロコピー) or `bincode` | 起動時間重視なら rkyv。互換性重視なら bincode。 |
| CLI | **`clap` v4** | 事実上の標準。derive API。 |
| 進捗表示 | **`indicatif`** | grep 互換出力を壊さないよう stderr に出す。 |
| ログ / 計器 | **`tracing`** + 自前の軽量 `TimingCollector` | `--verbose` ログと `--timings` 計測を同一スパンから取得。 |
| トレース出力 | **`tracing-chrome`** | `--trace <FILE>` で Chrome Tracing / Perfetto 形式を生成。 |
| ベンチマーク | **`criterion`** + `hyperfine` (E2E) | フェーズ単位とエンドツーエンド両方で回帰検出。 |
| fs 監視 | **`notify`** | `index --watch` 用。 |

> **ONNX ランタイムの注意**: `ort` はデフォルトで onnxruntime の共有ライブラリを要求するが、`load-dynamic` を外し `bundled` ビルドを使うことで単一バイナリ化可能。リリースバイナリでは bundled、開発時は system を既定とする。

---

## 7. 埋め込み (Embedding)

### 7.1 デフォルトモデル

- **`intfloat/multilingual-e5-small`** (384 次元、約 118MB、多言語・コード両対応)
- ONNX 変換済みバージョンを Hugging Face から自動 DL。
- ベクトル格納は **fp16** (半精度) を既定 — サイズ半減、HNSW 距離計算には十分。
- 長文は `max_len` (例 512 トークン) で分割。

### 7.2 モデル取得

1. 初回検索 / `vsgrep index` で `~/.cache/vsgrep/models/<model-id>/` が無ければ自動 DL。
2. DL 元: Hugging Face (ONNX 変換済みレポジトリを優先)。
3. `--model <HF_ID|LOCAL_PATH>` で任意モデル指定。
4. `vsgrep model add ./my_model.onnx --tokenizer ./tokenizer.json` でローカル登録。

### 7.3 モデル・インデックスの整合性

- `.vsgrep/meta.bin` に `(model_id, model_version, dim)` を記録。
- モデルを切り替えたら **インデックス再構築を要求** (自動で検知し、確認プロンプト)。

---

## 8. チャンク戦略

### 8.1 Phase 1 (MVP): 言語非依存スライディングウィンドウ

- 行ベースの窓 (既定: **20 行**、オーバーラップ **5 行**)。
- 窓サイズ・オーバーラップは `--chunk-size` / `--chunk-overlap` で上書き可。
- 空白大量の窓はスキップ。
- バイナリファイル自動スキップ (ripgrep と同ロジック)。

### 8.2 Phase 2: 拡張子別戦略

- `.md` / `.txt`: 段落・見出し単位で分割。
- `.rs` / `.py` / `.ts` 等: tree-sitter で関数・クラス単位の AST ベース分割 (`--chunk-mode ast`)。
- コード行がメタデータ (コンテナ関数名など) として埋め込みベクトルとは別に保存され、出力時に活用。

---

## 9. 検索アルゴリズム

### 9.1 既定フロー

```
query ─► tokenize ─► embed ─► HNSW.search(k=40) ─► rerank ─► top-k
```

- HNSW の `ef_search` は既定 64。`--ef` で上書き可。
- 小規模 (< 5,000 チャンク) の場合は自動でブルートフォース + SIMD に切替 — HNSW の構築コストを回避。

### 9.2 reranker (オプション)

- Phase 2: cross-encoder の軽量モデル (`BAAI/bge-reranker-v2-m3-small` 等) を `--rerank` で選択。
- デフォルトでは無効 (レイテンシ優先)。

### 9.3 ハイブリッド検索 (Phase 2)

- `vsgrep -H "retry logic"` で BM25 (tantivy) + ベクトルのハイブリッドスコア。
- Reciprocal Rank Fusion を既定。

---

## 10. パフォーマンス目標

| シナリオ | 規模 | 目標 |
|----------|------|------|
| 初回インデックス | Rust プロジェクト (10万行) | < 30 秒 (CPU、4 コア) |
| 差分更新 | 1 ファイル変更 | < 200ms |
| 検索 (HNSW) | 10 万チャンク | p50 < 20ms / p99 < 50ms |
| 検索 (ブルート) | 5,000 チャンク | p50 < 10ms |
| メモリ常駐 | 10 万チャンク | < 200MB (mmap 活用) |
| バイナリサイズ | — | < 30MB (モデル別ダウンロードのため) |

---

## 11. 計測と可観測性 (Observability & Profiling)

> 「速度が最優先」は「計測が最優先」と同義。推測で最適化しない。
> 全フェーズの所要時間を常時収集し、いつでも内訳を出せる状態を MVP から維持する。

### 11.1 計測対象フェーズ

検索パスと索引構築パスの両方を、以下の粒度でタイミング収集する。

**検索パス** (`vsgrep "query"`):

| フェーズ | 内容 | 期待レンジ (10万チャンク) |
|---------|------|--------------------------|
| `startup`       | プロセス起動〜`clap` パース完了 | < 5ms |
| `meta.load`     | `.vsgrep/meta.bin` 読み込み | < 1ms |
| `index.load`    | vectors / hnsw の mmap | < 5ms (lazy) |
| `model.load`    | ONNX セッション初期化 (ウォーム時は 0) | 初回 50〜200ms |
| `query.tokenize`| クエリのトークナイズ | < 1ms |
| `query.embed`   | 埋め込み推論 (1 文) | 5〜15ms |
| `ann.search`    | HNSW 探索 (ef_search 依存) | 1〜10ms |
| `rerank`        | (任意) reranker 推論 | 5〜30ms |
| `io.extract`    | ヒットしたチャンクの原文取得 | < 5ms |
| `format.print`  | 標準出力へ整形 | < 2ms |

**索引構築パス** (`vsgrep index`):

| フェーズ | 内容 |
|---------|------|
| `walk`           | `ignore` によるディレクトリ列挙 |
| `hash.diff`      | blake3 ハッシュで差分検出 |
| `read`           | ファイル読込 (rayon 並列) |
| `chunk`          | 行/AST チャンク分割 |
| `tokenize.batch` | バッチトークナイズ |
| `embed.batch`    | ONNX 推論バッチ (スループット計測: chunks/sec) |
| `hnsw.insert`    | HNSW への挿入 |
| `persist`        | vectors / hnsw / meta の書き出し |
| `gitignore`      | `.gitignore` 追記 (初回のみ) |

各フェーズに対し **wall-clock / CPU-time / 処理アイテム数 / スループット** を同時収集する。

### 11.2 実装方式

- **計器**: `tracing` のスパン (`#[tracing::instrument]`) でフェーズを囲む。常時有効、オーバーヘッドは無視できる (< 100ns/スパン)。
- **収集**: 起動時に軽量な `TimingCollector` サブスクライバを登録。スパンの enter/exit を原子カウンタに加算。
- **出力**:
  - `--timings` : プロセス終了直前に階層ツリー + 累積時間 + 実行回数 + % を stderr に出力。
  - `--timings=json` : 同じ内容を機械可読 JSON で出力 (CI でのトレンド監視用)。
  - `--trace <FILE>` : Chrome Tracing 形式 (Perfetto で開ける)。詳細な呼び出しチェインを可視化。
  - 環境変数 `VSGREP_TIMINGS=1` でも有効化 (フラグを付け忘れてもトラブルシュート可能)。

### 11.3 出力例 (`--timings`)

```
vsgrep timings (total 42.3 ms, 12,438 chunks scanned)
├─ startup ............................ 3.1 ms   ( 7.3%)
├─ meta.load .......................... 0.4 ms   ( 0.9%)
├─ index.load (mmap) .................. 1.2 ms   ( 2.8%)
├─ model.load ......................... 0.0 ms   ( 0.0%)   [cached]
├─ query.tokenize ..................... 0.3 ms   ( 0.7%)
├─ query.embed ........................ 11.8 ms  (27.9%)   [onnx-cpu, 1×seq=18]
├─ ann.search (ef=64, k=40) ........... 4.6 ms   (10.9%)
├─ rerank ............................. 0.0 ms   ( 0.0%)   [disabled]
├─ io.extract (40 spans) .............. 2.9 ms   ( 6.9%)
└─ format.print ....................... 0.5 ms   ( 1.2%)
other/overhead ......................... 17.5 ms (41.4%)
```

- **`other/overhead`** が大きければ計測漏れ → スパン追加の合図。
- スループット系 (`embed.batch: 3,412 chunks/sec`) はインデックス時に表示。

### 11.4 ベンチマークと回帰検出

- **`criterion`** クレートでマイクロベンチ (`benches/*.rs`):
  - `bench_query_embed`, `bench_hnsw_search`, `bench_chunk`, `bench_tokenize_batch` など各フェーズ単位。
- **エンドツーエンドベンチ** は固定コーパス (固定シードで生成、CI にチェックイン) で `hyperfine` を使い median レイテンシを測る。
- CI で前コミットとの比較を行い、**各フェーズの p50 が > 10% 悪化したら PR を失敗**させるゲート (GitHub Actions)。
- 同じ仕組みをリリース前ベンチマークにも使用し、README に性能表を掲載。

### 11.5 プロファイラ連携

- `cargo flamegraph` / `samply` で CPU プロファイルを取れるよう、リリースビルドに `debug = 1` (行情報のみ) を保持。
- `--trace` 出力は tracing span と CPU プロファイルを突き合わせるための時間軸として使える。
- ONNX Runtime 側のプロファイラ (`session.end_profiling()`) も `--trace-onnx` で有効化。推論ノード単位のホットスポットを抽出可能。

### 11.6 意思決定ルール

- 「最適化する」と決める前に、対象フェーズの占有率が **全体時間の 5% 以上** であることをタイミングログで確認する。
- 改善 PR には「before / after」のタイミングログ (同一コーパス) を添付することを必須化 (PR テンプレートに欄を用意)。

---

## 12. 非目標 (初期バージョン)

- **リモート/分散インデックス** — ローカル単一マシン前提。
- **リアルタイム GPU 推論** — CPU ONNX で十分な性能を出す。GPU 対応は後続。
- **サーバモード/デーモン** — 毎回プロセス起動でも十分速く作れる想定。
- **GUI** — CLI のみ。

---

## 13. マイルストーン

### M1: Walking Skeleton (2〜3週)
- `cargo` プロジェクト骨格、`clap` CLI、`ignore` でファイル列挙
- tokenizer + ort で埋め込み (固定モデルハードコード)
- ブルートフォース検索のみ、インデックス in-memory
- grep 互換出力
- **計測基盤を MVP 時点で導入**: `tracing` スパン + `--timings` フラグ + `criterion` ベンチ雛形
- **出荷目標**: `vsgrep "query" .` が動き、`--timings` で全フェーズの内訳が見える

### M2: 永続インデックス + HNSW
- `.vsgrep/` 書き出し / ロード (mmap)
- HNSW 構築 + 検索
- 差分更新 (ハッシュベース)
- **出荷目標**: ripgrep 並みの起動速度 + 意味検索

### M3: モデル管理 + 配布
- `model` サブコマンド、HF 自動 DL
- GitHub Releases で Linux/macOS/Windows バイナリ配布
- `cargo-binstall` 対応
- **出荷目標**: `cargo install vsgrep` で 1 コマンド導入

### M4: 拡張 (reranker / ハイブリッド / AST チャンク / watch)
- tree-sitter ベース AST チャンク
- tantivy 併用のハイブリッド検索
- `index --watch` による常時同期

---

## 14. リスクと対応

| リスク | 影響 | 対応 |
|--------|------|------|
| ONNX Runtime のリンクトラブル (Windows / musl) | 配布体験が崩れる | `bundled` ビルド固定、musl は別マトリクスで検証。必要なら `tract` フォールバックを検討。 |
| 埋め込み精度が grep ユーザの期待に届かない | 定着率低下 | デフォルトモデル選定を実データで評価。reranker を早期に用意。 |
| インデックスとコードの乖離 | 誤検知 | `--watch` / pre-commit hook テンプレ提供。staleness 警告。 |
| モデル DL のネットワーク制約 (企業内環境等) | 初回失敗 | オフラインブートストラップ手順、`model add` でローカル登録、プロキシ環境変数尊重。 |
| 単一バイナリサイズ | 体感悪化 | モデルは別 DL、バイナリは < 30MB を維持。UPX 等の圧縮は採用しない (起動コスト)。 |

---

## 15. プロジェクト・メタ

### 15.1 ライセンス
- **MIT ライセンス** で配布。
- リポジトリ直下に `LICENSE` ファイル、`Cargo.toml` に `license = "MIT"` を記載。
- 依存クレート (e.g., `ort`, `tokenizers`, `hnsw_rs`, `ignore` 等) はそれぞれ MIT / Apache-2.0 / BSD などを想定。将来 GPL 系の依存が入りそうになったら当該 PR で協議する運用。
- 埋め込みモデル自体のライセンスは **モデル側に従う**。`multilingual-e5-small` は MIT。`vsgrep` 本体には同梱しない (初回 DL) ため再配布条項は発生しない。

### 15.2 テレメトリ
- **常時オフ**。ビルドオプションでも有効化しない。
- 唯一の外向き通信は「Hugging Face からのモデル DL」と「明示的に指定された場合のユーザ側モデル URL」のみ。
- プライバシポリシー: ファイル内容・クエリ・パス等を一切外部に送信しない旨を README に明記。

### 15.3 エイリアス `vg` の配置
- `cargo install` 経由: `build.rs` or `install.sh` で `$CARGO_HOME/bin/vg` を `vsgrep` にシンボリックリンク (Windows はハードリンクにフォールバック)。
- GitHub Releases バイナリ: tarball / zip に `vsgrep` と `vg` (単なるリンク) を同梱。
- 名前衝突時は警告を出してスキップし、`vsgrep` のみ有効にする。

### 15.4 残された意思決定 (将来スプリント)
- [ ] HNSW パラメータ (`M`, `ef_construction`) のデフォルト値を実測して決定。
- [ ] ベクトル量子化 (int8 / PQ) を段階導入するかのベンチマーク。
- [ ] `reranker` 既定を「無し」のままとするか、小型クロスエンコーダを同梱するか。
- [ ] Windows での fs 監視 (`ReadDirectoryChangesW`) の信頼性検証。

---

## 16. 参考にするプロジェクト

- **ripgrep** — CLI UX、`ignore` クレート、file-type 管理、出力フォーマット。
- **fastembed-rs** — ONNX + tokenizers の実装パターン。
- **qdrant** / **lancedb** — HNSW 永続化・差分更新のアイデア (ただし依存はしない)。
- **nomic-embed / e5 family** — 埋め込みモデルのベースライン。
