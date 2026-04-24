# vsgrep 実装工程ドキュメント

> DESIGN.md が「何を作るか」を定義する憲法だとすれば、本ドキュメントは「どの順で、どの粒度で作っていくか」を定める工程表。
> 各ステップは「プラン作成 → 実装 → 受け入れ基準クリア」の 1 ループで閉じる。

---

## 0. 本ドキュメントの位置付け

- **変わりにくい前提 (制約)** は DESIGN.md を参照。本ドキュメントで DESIGN を上書きしない。
- **変わりやすい工程 (実行計画)** は本ドキュメント。実装しながら学んだことは随時反映する。
- ここには「各ステップで詰めるべき観点」と「受け入れ基準」までを書く。**詳細なプランはステップ着手時に別途作成する** (長大化を避けるため)。

---

## 1. 進め方の原則

1. **1 ステップ = 1 プラン = 1 PR** を基本単位とする。マージ単位が小さいほど回帰原因の特定が速い。
2. **ステップ着手時にプラン合意 → 実装 → 受け入れ基準で確認**。プラン合意前に実装に入らない。
3. **計測は M1 の最初から**。DESIGN §11 のルール (5% 以上を占めるフェーズのみ最適化) を適用できる状態を初日から作る。
4. **実装はトップダウン (CLI → ダミー実装 → 中身差し替え)**。動く骨格を最優先で通し、部品を順次本物に差し替える。
5. **依存クレートの API は着手前に素振り**。公式サンプルを最小コードで動かしてからプロダクトに統合する (`ort`, `hnsw_rs`, `tokenizers` は特に)。
6. **性能目標 (DESIGN §10) は検証を M2 末で最初に行う**。M1 中は正しさを優先し、ベンチは回帰検出の下地作りに留める。

### ステップ着手時のプラン・テンプレ

```
## Step N: <題名>
### 目的
<このステップで何を達成するか>
### スコープ / 含めるもの
- ...
### 非スコープ / 今はやらないもの
- ...
### 設計判断が必要な論点
- 論点 A: 選択肢 / 推奨 / 影響
- ...
### タスク分解
- [ ] ...
### 受け入れ基準
- [ ] ...
### 想定リスクと対策
- ...
```

---

## 2. マイルストーンとステップ対応

| Phase | ステップ範囲 | 出荷目標 |
|-------|--------------|---------|
| M1 Walking Skeleton | Step 1〜9 | `vsgrep "query" .` が動き、`--timings` で内訳が見える |
| M2 永続インデックス + HNSW | Step 10〜15 | ripgrep 並みの起動 + 永続インデックス + 差分更新 |
| M3 モデル管理 + 配布 | Step 16〜20 | `cargo install vsgrep` / GitHub Releases で 1 コマンド導入 |
| M4 拡張 | Step 21〜 | reranker / ハイブリッド / AST チャンク / watch |

---

## 3. Phase M1 — Walking Skeleton

### Step 1: プロジェクト初期化と CI 雛形
- **目的**: 以降のすべてのステップが乗る土台を作る。
- **スコープ**: `cargo new --bin vsgrep` / `Cargo.toml` メタデータ (MIT, edition) / `rust-toolchain.toml` / `rustfmt.toml` / `clippy.toml` / `.gitignore` / GitHub Actions (fmt, clippy, test) / `CHANGELOG.md` 雛形。
- **非スコープ**: リリースワークフロー、バイナリ配布 (M3)。
- **プラン時に詰める論点**: ワークスペース化するか単一クレートか (将来 `vsgrep-core` と `vsgrep-cli` を分ける可能性)。MSRV の決定。
- **受け入れ基準**:
  - `cargo build` / `cargo test` / `cargo clippy -- -D warnings` が通る。
  - CI がこれら 3 つを PR ごとに実行。
  - `cargo run -- --version` が動く。
- **備考**:
  - Rustはmiseで管理する。プロジェクトルートに、.mise.tomlを起き、バージョンは1.93.0とする

### Step 2: CLI スケルトン (clap derive)
- **目的**: DESIGN §4 の CLI 表面を型で定義する。中身はダミーで OK。
- **スコープ**: `clap` v4 derive で以下を受けられる最小形。
  - 位置引数: `query`, `paths...`
  - オプション: `-k/--top-k`, `--threshold`, `-t/--type`, `-g/--glob`, `--score`, `--json`, `-C/--context`, `--heading`, `--timings`, `--trace`, `--no-cache`, `--model`, `--ef`, `--chunk-size`, `--chunk-overlap`, `-H/--hybrid` (Phase 2 でもフラグだけ用意)
  - サブコマンド: `index`, `status`, `clean`, `model {list,use,add}`
- **非スコープ**: 実処理の実装 (ダミーで println! 可)。
- **プラン時に詰める論点**: フラグ既定値、short オプションの競合 (grep 互換との齟齬)。サブコマンドとフラグの優先順位 (例: `--json` と `vsgrep index` の組合せ)。
- **受け入れ基準**:
  - `vsgrep --help` が DESIGN §4 の CLI を網羅して表示。
  - 全サブコマンドが認識され、未実装箇所は "not implemented" を stderr 出力。
  - CLI パースの単体テスト (`clap::Command::debug_assert` を使用) が CI で通る。
- **備考**:
  - vsgrepは名前がほかのプロダクトと被っていたので、vector semanticの略でvsgrepにプロダクトの名前を変更したい。DESIGN.mdとIMPLEMENTATION.mdなど必要な箇所でvsgrepをvsgrepに置換する。エイリアスはvgで引き続き。

### Step 3: ファイル走査 (`ignore` クレート)
- **目的**: 対象ファイル列挙の基盤を作る。ripgrep の挙動を踏襲。
- **スコープ**: `ignore::WalkBuilder` でパス列挙 / `-t/--type` と `-g/--glob` を `ignore` の仕組みに接続 / バイナリ検出スキップ / シンボリックリンクの扱い既定値設定。
- **非スコープ**: チャンク分割 (Step 4) / 埋め込み (Step 5)。
- **プラン時に詰める論点**: バイナリ検出の閾値 / 隠しファイルの扱い / 既定の並列度 / 標準入力からの読込をサポートするか (M1 ではしない方針だが CLI 的に)。
- **受け入れ基準**:
  - 任意のリポジトリで `ignore` 同等のファイル列挙ができる。
  - `.gitignore` を尊重することの結合テスト。
  - バイナリファイル (例: PNG) がスキップされることのテスト。

### Step 4: チャンク分割 (スライディングウィンドウ)
- **目的**: DESIGN §8.1 の MVP チャンク戦略を実装する。
- **スコープ**: 行ベースの窓 (既定 20 行 / オーバーラップ 5 行) / 空白大量スキップ / `(path, start_line, end_line, text)` 構造体生成 / `--chunk-size`・`--chunk-overlap` 反映。
- **非スコープ**: AST ベース分割 (M4) / 段落ベース分割。
- **プラン時に詰める論点**: 改行コードの正規化 (CRLF / LF 混在) / 巨大ファイルのストリーム処理 / マルチバイト境界。空白スキップの閾値定義。
- **受け入れ基準**:
  - 単体テストで既知の入力に対し期待どおりのチャンク境界になる。
  - オーバーラップが正しく機能する。
  - `chunk` フェーズの tracing スパンで chunks/sec が取れる。

### Step 5: 埋め込みランタイム (ort + tokenizers)
- **目的**: ONNX セッションを立ち上げ、文字列 → ベクトルの 1 本道を通す。
- **スコープ**: `ort` セッション初期化 / `tokenizers` で `tokenizer.json` ロード / E5 系のプレフィックス (`query:` / `passage:`) 扱い / 単一文字列の埋め込み関数 / バッチ埋め込み関数 (既定 64) / fp32 出力で一旦確定 (fp16 は Step 10 で導入)。
- **非スコープ**: モデル自動 DL (M3) — ここでは固定パスを環境変数または既定で読む。
- **プラン時に詰める論点**: モデルファイルの配置場所 (開発時は `tests/fixtures/` or `$VSGREP_MODEL`)。`ort` の `load-dynamic` / `bundled` どちらで開発を進めるか (開発は system、リリースは bundled)。`session.run` の入力 shape と attention mask の作り方。
- **受け入れ基準**:
  - 1 文の埋め込みが `query.embed` span で計測可能。
  - ベクトル次元が 384 であることを起動時にアサート。
  - バッチサイズを変えて chunks/sec が線形〜サブリニアに伸びるのを確認。

### Step 6: ブルートフォース検索
- **目的**: HNSW 導入前に「クエリ 1 本 → top-k」の end-to-end を閉じる。
- **スコープ**: インメモリ `Vec<Vector>` に対する cosine 類似度の計算 / 上位 k 抽出 / SIMD は `simsimd` でも自前でも可 (M1 では素朴実装で OK、ベンチ対象として残す)。
- **非スコープ**: 永続化 / HNSW (共に M2)。
- **プラン時に詰める論点**: ベクトルは正規化済みとして扱うか / dot product と cosine のどちらで持つか / ソートアルゴリズム (k 個の min-heap か全ソートか)。
- **受け入れ基準**:
  - 小規模データセット (1k 件程度) で top-k が正しい。
  - `ann.search` span が 5,000 チャンクで p50 < 10ms (DESIGN §10 の目標) の素地があるかの初期計測。

### Step 7: 出力フォーマッタ (grep 互換)
- **目的**: DESIGN §4.3 の既定出力と主要フラグを実装する。
- **スコープ**: 既定の `path:line:content` / `--score` / `--json` / `-C/--context` / `--heading` / 色付け (tty 判定)。
- **非スコープ**: `--timings` 出力 (Step 9)。
- **プラン時に詰める論点**: JSON Lines のスキーマ (キー名、スコアの型) / 色付け有効化判定 (TTY + `NO_COLOR` 尊重) / コンテキスト行取得の I/O 回数 (ヒットチャンク分を 1 ファイルずつまとめるか)。
- **受け入れ基準**:
  - 結合テストで主要オプションの組合せ出力がスナップショット (`insta`) で安定する。
  - パイプ時に色が自動オフになる。

### Step 8: M1 結合 (エンドツーエンド)
- **目的**: Step 2〜7 をワイヤリングし `vsgrep "query" .` を動かす。
- **スコープ**: インデックスは in-memory (毎回再構築) / 検索パスのオーケストレーション / エラーハンドリング (モデル未配置時の親切メッセージ)。
- **受け入れ基準**:
  - 任意のディレクトリで `vsgrep "retry logic" src/` が結果を返す。
  - 終了コードが grep 互換 (ヒット 0、未ヒット 1、エラー 2)。
  - スモーク E2E テスト (小コーパス + クエリ → 期待ヒット) が CI で通る。

### Step 9: 計測基盤 (tracing + --timings + criterion)
- **目的**: DESIGN §11 の観測性を M1 卒業要件として整える。
- **スコープ**:
  - `tracing` スパンで DESIGN §11.1 のフェーズ名を付与。
  - `TimingCollector` サブスクライバを実装し、プロセス終了直前に階層ツリー + % を stderr 出力 (`--timings`)。
  - `--timings=json` / `VSGREP_TIMINGS=1` 対応。
  - `--trace <FILE>` で `tracing-chrome` を配線。
  - `benches/` に各フェーズの `criterion` 雛形 (`query_embed`, `bruteforce_search`, `chunk`, `tokenize_batch`)。
  - `hyperfine` E2E スクリプトを `bench/` に置く (CI では任意実行)。
- **受け入れ基準**:
  - `--timings` の出力が DESIGN §11.3 の例と同等の形で出る。
  - `other/overhead` が全体の 20% 未満 (計測漏れがない水準)。
  - `criterion` が 4 本走り、JSON がアーティファクト化される。
- **M1 出荷判定**: ここまでクリアで M1 完了。README に `--timings` 出力の実測スクショを貼る。

---

## 4. Phase M2 — 永続インデックス + HNSW

### Step 10: `.vsgrep/` レイアウトと永続化 (ベクトル / メタ)
- **目的**: インデックスをディスクに置き、起動時に mmap で即ロードできるようにする。
- **スコープ**: `meta.bin` (model_id, model_version, dim, chunk 数, 作成日時) / `chunks.bin` (path, span, hash) / `vectors.bin` (fp16 に切替、mmap) / アトミック書き出し (tmp → rename) / バージョン番号付与。
- **プラン時に詰める論点**: シリアライズは `rkyv` か `bincode` か (起動時間と保守性のトレードオフ)。fp16 変換で精度が落ちないことの計測。マルチプロセス同時書込みのロック戦略。
- **受け入れ基準**:
  - 再起動してもインデックスが再利用される。
  - `index.load` span が DESIGN §11 目標 (< 5ms) に収まる。
  - 不正なバージョンを親切に拒否 (再構築を促すメッセージ)。

### Step 11: HNSW 統合 (`hnsw_rs`)
- **目的**: 10 万チャンク規模で p99 < 50ms を射程に入れる。
- **スコープ**: `hnsw_rs` で構築・検索 / `hnsw.bin` mmap / `--ef` 反映 / 小規模 (< 5,000) 自動ブルートフォース・フォールバック / ブルート実装は Step 6 のものを再利用。
- **プラン時に詰める論点**: `M`, `ef_construction` の既定値 (DESIGN §15.4 の宿題を実測で解決)。`hnsw_rs` と `instant-distance` の最終比較 (構築時間 / クエリ時間 / ファイルサイズ)。
- **受け入れ基準**:
  - 10 万ダミーチャンクで p50 < 20ms / p99 < 50ms を実測し README に表を掲載。
  - ブルートとの recall@10 差が合意水準 (例: ≥ 0.98) を満たす。

### Step 12: 差分インデックス更新
- **目的**: ファイル 1 つの変更で全再構築を避ける。
- **スコープ**: `blake3` で hash / `file_index.bin` (ファイル → チャンク ID 逆引き) / 変更検知で対象チャンクだけ再埋め込み / 削除は tombstone / 一定割合超過で自動再構築。
- **プラン時に詰める論点**: mtime を先にチェックしてから hash するかどうか (I/O 削減)。tombstone の閾値。再構築中のクエリ挙動 (古いインデックスで応答するか、エラーか)。
- **受け入れ基準**:
  - 1 ファイル変更の差分更新が DESIGN 目標 < 200ms に収まる。
  - tombstone 比率の status 表示で可視化。

### Step 13: `vsgrep index` / `status` / `clean` サブコマンド
- **目的**: DESIGN §4.2 の運用コマンドを満たす。
- **スコープ**: 明示的インデックス構築 / 統計表示 (チャンク数、モデル、更新時刻、tombstone 率、サイズ) / `.vsgrep/` 削除。初回検索時の自動インデックス構築 (進捗バー in `indicatif`, stderr)。
- **受け入れ基準**:
  - `status` が人間可読 + `--json` の両方を持つ。
  - 自動インデックス構築時に stdout を汚さない (パイプ安全)。

### Step 14: `.gitignore` 自動追記
- **目的**: DESIGN §3 と §4.2 の ".vsgrep/ を自動追記" ポリシーを満たす。
- **スコープ**: 初回 `index` 時のみ追記 / 既存行があればスキップ / `--no-gitignore` / git リポジトリ外では無動作 / 追記時は stderr に 1 行通知。
- **受け入れ基準**:
  - 単体テスト: 既に行がある / 無い / git 外 / フラグ抑止の 4 ケース。
  - CRLF 既存ファイルを壊さない。

### Step 15: M2 結合とベンチ
- **目的**: M2 出荷判定。
- **スコープ**: README 更新 (性能表、クイックスタート) / `hyperfine` で ripgrep との起動時間比較 / CI の性能回帰ゲート (DESIGN §11.4, p50 > 10% 悪化で PR fail) を導入。
- **受け入れ基準**:
  - 性能目標 (DESIGN §10) のうちインデックス / 検索 / メモリ常駐を満たす。
  - 回帰ゲートが前コミット比で動作する (意図的に遅くした PR を落とせる)。

---

## 5. Phase M3 — モデル管理 + 配布

### Step 16: モデル DL とキャッシュ
- **目的**: 初回検索時にモデル自動取得。
- **スコープ**: Hugging Face からの ONNX 取得 / `~/.cache/vsgrep/models/<model-id>/` 配置 / プロキシ環境変数尊重 / 再開可能な DL (失敗時再試行) / SHA 検証。
- **プラン時に詰める論点**: `hf-hub` クレート採用可否 / オフライン配置手順 / モデル整合性チェック (DESIGN §7.3)。

### Step 17: `vsgrep model {list,use,add}`
- **目的**: DESIGN §4.2 のモデル管理 UX。
- **スコープ**: 組込み候補モデルのカタログ / `use` でデフォルト切替 (インデックス再構築を要求) / `add` でローカル ONNX 登録 (tokenizer 必須)。
- **受け入れ基準**: モデル切替時にインデックス不整合を検知してプロンプト。

### Step 18: リリースビルド (`bundled` ort)
- **目的**: 単一バイナリ化。
- **スコープ**: `ort` を `bundled` feature で固定 / Linux gnu / musl / macOS (x86_64, aarch64) / Windows の GitHub Actions マトリクス / バイナリサイズ監視 (< 30MB) / デバッグ情報 `debug = 1` (flamegraph 用)。
- **プラン時に詰める論点**: musl と onnxruntime のリンク問題 (DESIGN §14 リスク) / Windows のコード署名どうするか (初期は未署名)。

### Step 19: リリース配布 (GitHub Releases + cargo-binstall)
- **目的**: `cargo install vsgrep` と `cargo binstall vsgrep` のどちらでも入る。
- **スコープ**: リリースワークフロー / `[package.metadata.binstall]` 設定 / tarball に `vsgrep` と `vg` を同梱 / SHA256SUMS 公開。

### Step 20: `vg` エイリアス配置
- **目的**: DESIGN §15.3。
- **スコープ**: `cargo install` 経由で `$CARGO_HOME/bin/vg` を作るための `build.rs` か postinstall 手順 / Releases tarball 側の vg 同梱 / 名前衝突時はスキップ + 警告。

---

## 6. Phase M4 — 拡張

M1〜M3 の実測結果で優先度と粒度を再見積もる。現時点の暫定ステップ:

- **Step 21: tree-sitter ベース AST チャンク (`--chunk-mode ast`)** — DESIGN §8.2。対応言語は Rust, Python, TS から。
- **Step 22: BM25 ハイブリッド (`-H`)** — `tantivy` 併用 / Reciprocal Rank Fusion。
- **Step 23: reranker (`--rerank`)** — 小型 cross-encoder を別モデルとして DL。
- **Step 24: `index --watch`** — `notify` クレート。Windows の信頼性検証 (DESIGN §15.4) を含む。
- **Step 25: 量子化 (int8 / PQ) の実験** — DESIGN §15.4 の宿題。

---

## 7. 横断タスク (各ステップで常に意識する)

- **テスト**: 単体 → 結合 → E2E の 3 層を CI で走らせる。E2E は固定シードで生成した小コーパスを `tests/fixtures/` に置く。
- **ドキュメント**: CLI フラグを追加したら `README` の該当表と `--help` テキストを必ず更新。
- **計測**: 新フェーズを追加したら必ず `tracing` スパンを付ける。`other/overhead` が 5% 超えたらスパン漏れを疑う。
- **PR テンプレ**: 性能に触れる変更では before / after タイミングログを必須添付 (DESIGN §11.6)。
- **依存追加**: 新クレートを足す PR では License の確認をチェックリストに含める (DESIGN §15.1)。

---

## 8. 最初の一歩

- **今着手するステップ**: Step 1 (プロジェクト初期化と CI 雛形)。
- **次のアクション**: Step 1 のプランを起こしてレビュー → 承認後に実装に入る。
