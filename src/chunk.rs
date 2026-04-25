//! Sliding-window line-based chunker — DESIGN.md §8.1 / IMPLEMENTATION.md Step 4.
//!
//! A "chunk" is a contiguous window of lines from a source file, described by
//! `(path, start_line, end_line, text)`.  Windows overlap so semantic context
//! is preserved across chunk boundaries.
//!
//! ## Design decisions recorded here
//!
//! * **Whitespace-skip thresholds** (`MIN_NON_WS_RATIO`, `MIN_NON_WS_LINES`) are
//!   intentionally loose MVP values.  Step 21 (AST chunker) will revisit them.
//! * **10 MiB file cap** (`DEFAULT_MAX_FILE_BYTES`): DESIGN does not specify a
//!   limit.  For M1 whole-file reading is sufficient; real streaming can be added
//!   when profiling shows it is necessary (DESIGN §11 ≥5% rule).
//! * **`IndexArgs` lacks `chunk_size`/`chunk_overlap`**: deferred to Step 13
//!   when `vsgrep index` is fully wired.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use thiserror::Error;

// ── Whitespace-skip constants ────────────────────────────────────────────────

/// Minimum ratio of non-whitespace characters for a window to be emitted.
const MIN_NON_WS_RATIO: f32 = 0.10;

/// Minimum number of lines containing at least one non-whitespace character.
const MIN_NON_WS_LINES: usize = 2;

/// Default soft cap on file size.  Files above this limit return `TooLarge`.
const DEFAULT_MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

// ── Public types ─────────────────────────────────────────────────────────────

/// Configuration for the sliding-window chunker.
#[derive(Debug, Clone)]
pub struct ChunkConfig {
    /// Lines per window (default 20).
    pub size: usize,
    /// Overlap between consecutive windows (default 5).
    pub overlap: usize,
    /// Files larger than this are rejected with `ChunkError::TooLarge`.
    pub max_file_bytes: u64,
    /// Windows with a non-whitespace character ratio below this are skipped.
    pub min_non_ws_ratio: f32,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            size: 20,
            overlap: 5,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            min_non_ws_ratio: MIN_NON_WS_RATIO,
        }
    }
}

impl ChunkConfig {
    /// Returns an error if the config parameters are self-contradictory.
    pub fn validate(&self) -> Result<(), ChunkError> {
        if self.size == 0 || self.overlap >= self.size {
            return Err(ChunkError::InvalidConfig {
                size: self.size,
                overlap: self.overlap,
            });
        }
        Ok(())
    }
}

/// A single chunk produced by the chunker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub path: PathBuf,
    /// First line of the window, 1-based inclusive.
    pub start_line: usize,
    /// Last line of the window, 1-based inclusive.
    pub end_line: usize,
    /// Window text with CRLF normalised to LF (lines joined by `'\n'`).
    pub text: String,
}

#[derive(Debug, Error)]
pub enum ChunkError {
    #[error("invalid chunk config: size={size}, overlap={overlap}")]
    InvalidConfig { size: usize, overlap: usize },

    #[error("file `{path}` is not valid UTF-8")]
    NotUtf8 { path: PathBuf },

    #[error("file `{path}` exceeds max_file_bytes ({size} > {cap})")]
    TooLarge { path: PathBuf, size: u64, cap: u64 },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

// ── Core implementation ───────────────────────────────────────────────────────

/// Read `path` and split it into overlapping line windows per `cfg`.
///
/// Returns `Ok(vec![])` for empty files.  Returns `Err` for non-UTF-8 content
/// or files that exceed `cfg.max_file_bytes`.
#[tracing::instrument(skip_all, fields(path = %path.display()))]
pub fn chunk_file(path: &Path, cfg: &ChunkConfig) -> Result<Vec<Chunk>, ChunkError> {
    let meta = std::fs::metadata(path)?;
    let file_size = meta.len();
    if file_size > cfg.max_file_bytes {
        return Err(ChunkError::TooLarge {
            path: path.to_owned(),
            size: file_size,
            cap: cfg.max_file_bytes,
        });
    }

    let bytes = std::fs::read(path)?;
    let content = String::from_utf8(bytes).map_err(|_| ChunkError::NotUtf8 {
        path: path.to_owned(),
    })?;

    Ok(windows_from_str(path, &content, cfg))
}

/// Chunk an iterator of file paths, yielding `Result<Chunk, ChunkError>` per chunk.
///
/// Errors for individual files are yielded inline so callers can log-and-continue
/// or fail-fast as needed.
pub fn chunk_paths<I>(
    paths: I,
    cfg: &ChunkConfig,
) -> impl Iterator<Item = Result<Chunk, ChunkError>>
where
    I: IntoIterator<Item = PathBuf>,
{
    let _span = tracing::info_span!("chunk").entered();
    let cfg = cfg.clone();
    let mut file_count: u64 = 0;
    let mut chunk_count: u64 = 0;

    paths.into_iter().flat_map(move |path| {
        file_count += 1;
        match chunk_file(&path, &cfg) {
            Ok(chunks) => {
                chunk_count += chunks.len() as u64;
                tracing::info!(file_count, chunk_count);
                chunks.into_iter().map(Ok).collect::<Vec<_>>()
            }
            Err(e) => vec![Err(e)],
        }
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn windows_from_str(path: &Path, content: &str, cfg: &ChunkConfig) -> Vec<Chunk> {
    // Split on '\n' and strip trailing '\r' from each line (handles LF, CRLF, mixed).
    let mut lines: Vec<&str> = content
        .split('\n')
        .map(|l| l.trim_end_matches('\r'))
        .collect();

    // Drop a single trailing empty line that a trailing newline produces.
    if lines.last() == Some(&"") {
        lines.pop();
    }

    if lines.is_empty() {
        return vec![];
    }

    let n = lines.len();
    let stride = cfg.size - cfg.overlap; // validate() guarantees stride ≥ 1
    let mut chunks = Vec::new();
    let mut i = 0;

    loop {
        let start = i;
        let end = (i + cfg.size).min(n);
        let window = &lines[start..end];

        if !is_whitespace_heavy(window, cfg.min_non_ws_ratio) {
            chunks.push(Chunk {
                path: path.to_owned(),
                start_line: start + 1,
                end_line: end,
                text: window.join("\n"),
            });
        }

        if end == n {
            break;
        }
        i += stride;
    }

    chunks
}

/// Returns `true` if the window should be skipped due to being mostly whitespace.
fn is_whitespace_heavy(lines: &[&str], min_ratio: f32) -> bool {
    let non_ws_line_count = lines
        .iter()
        .filter(|l| l.chars().any(|c| !c.is_whitespace()))
        .count();
    if non_ws_line_count < MIN_NON_WS_LINES {
        return true;
    }
    let total: usize = lines.iter().map(|l| l.chars().count()).sum();
    if total == 0 {
        return true;
    }
    let non_ws: usize = lines
        .iter()
        .map(|l| l.chars().filter(|c| !c.is_whitespace()).count())
        .sum();
    (non_ws as f32 / total as f32) < min_ratio
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::NamedTempFile;

    use super::*;

    fn cfg(size: usize, overlap: usize) -> ChunkConfig {
        ChunkConfig {
            size,
            overlap,
            ..ChunkConfig::default()
        }
    }

    fn write_file(content: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content).unwrap();
        f
    }

    fn lines_n(n: usize) -> String {
        (1..=n)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    // ── boundary tests ────────────────────────────────────────────────────────

    #[test]
    fn exact_boundaries_no_overlap() {
        let f = write_file(lines_n(20).as_bytes());
        let chunks = chunk_file(f.path(), &cfg(20, 0)).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 20);
    }

    #[test]
    fn sliding_window_overlap() {
        let f = write_file(lines_n(50).as_bytes());
        let chunks = chunk_file(f.path(), &cfg(20, 5)).unwrap();
        assert_eq!(chunks.len(), 3);
        assert_eq!((chunks[0].start_line, chunks[0].end_line), (1, 20));
        assert_eq!((chunks[1].start_line, chunks[1].end_line), (16, 35));
        assert_eq!((chunks[2].start_line, chunks[2].end_line), (31, 50));
    }

    #[test]
    fn tail_partial_chunk() {
        let f = write_file(lines_n(25).as_bytes());
        let chunks = chunk_file(f.path(), &cfg(20, 5)).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!((chunks[0].start_line, chunks[0].end_line), (1, 20));
        assert_eq!((chunks[1].start_line, chunks[1].end_line), (16, 25));
    }

    #[test]
    fn single_chunk_smaller_than_size() {
        let f = write_file(lines_n(5).as_bytes());
        let chunks = chunk_file(f.path(), &cfg(20, 5)).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!((chunks[0].start_line, chunks[0].end_line), (1, 5));
    }

    // ── line-ending tests ─────────────────────────────────────────────────────

    #[test]
    fn crlf_and_lf_mixed() {
        let f = write_file(b"a\r\nb\nc\r\n");
        let chunks = chunk_file(f.path(), &cfg(20, 0)).unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(!chunks[0].text.contains('\r'), "text must not contain \\r");
        assert_eq!(chunks[0].text, "a\nb\nc");
        assert_eq!((chunks[0].start_line, chunks[0].end_line), (1, 3));
    }

    #[test]
    fn trailing_newline_does_not_shift_end_line() {
        let f = write_file(b"line1\nline2\n");
        let chunks = chunk_file(f.path(), &cfg(20, 0)).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!((chunks[0].start_line, chunks[0].end_line), (1, 2));
    }

    // ── whitespace-skip tests ─────────────────────────────────────────────────

    #[test]
    fn whitespace_heavy_skipped() {
        // 20 blank lines → non_ws_line_count = 0 < MIN_NON_WS_LINES
        let content = "\n".repeat(19); // 19 '\n' → 20 lines after split + drop trailing
        let f = write_file(content.as_bytes());
        let chunks = chunk_file(f.path(), &cfg(20, 0)).unwrap();
        assert_eq!(chunks.len(), 0);
    }

    #[test]
    fn whitespace_heavy_one_code_line() {
        // 19 blank lines + 1 code line → non_ws_line_count = 1 < MIN_NON_WS_LINES
        let content = "\n".repeat(19) + "let x = 1;";
        let f = write_file(content.as_bytes());
        let chunks = chunk_file(f.path(), &cfg(20, 0)).unwrap();
        assert_eq!(chunks.len(), 0);
    }

    // ── error tests ───────────────────────────────────────────────────────────

    #[test]
    fn invalid_utf8_errors() {
        let f = write_file(&[0xff, 0xfe, 0xfd]);
        let err = chunk_file(f.path(), &ChunkConfig::default()).unwrap_err();
        assert!(matches!(err, ChunkError::NotUtf8 { .. }));
    }

    #[test]
    fn too_large_errors() {
        let content = vec![b'a'; 2048];
        let f = write_file(&content);
        let small_cap = ChunkConfig {
            max_file_bytes: 1024,
            ..ChunkConfig::default()
        };
        let err = chunk_file(f.path(), &small_cap).unwrap_err();
        assert!(matches!(err, ChunkError::TooLarge { .. }));
    }

    #[test]
    fn empty_file_yields_no_chunks() {
        let f = write_file(b"");
        let chunks = chunk_file(f.path(), &ChunkConfig::default()).unwrap();
        assert_eq!(chunks.len(), 0);
    }

    // ── config validation tests ───────────────────────────────────────────────

    #[test]
    fn invalid_config_size_zero() {
        let err = ChunkConfig {
            size: 0,
            overlap: 0,
            ..ChunkConfig::default()
        }
        .validate();
        assert!(matches!(err, Err(ChunkError::InvalidConfig { .. })));
    }

    #[test]
    fn invalid_config_overlap_ge_size() {
        let err = ChunkConfig {
            size: 5,
            overlap: 5,
            ..ChunkConfig::default()
        }
        .validate();
        assert!(matches!(err, Err(ChunkError::InvalidConfig { .. })));
    }
}
