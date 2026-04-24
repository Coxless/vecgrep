//! Source-tree walker for `vsgrep`.
//!
//! Wraps the `ignore` crate (ripgrep's walker) to enumerate candidate files
//! while honoring `.gitignore`, `-t/--type`, `-g/--glob`, and skipping hidden
//! files, symlinks, and binary files. Step 3 of IMPLEMENTATION.md.

// Wired into the search/index path in Step 8; until then the API is
// exercised by unit tests only.
#![allow(dead_code)]

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use ignore::overrides::OverrideBuilder;
use ignore::types::TypesBuilder;
use ignore::{Walk, WalkBuilder};

const BINARY_PROBE_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct WalkConfig {
    pub paths: Vec<PathBuf>,
    pub types: Vec<String>,
    pub globs: Vec<String>,
    pub follow_symlinks: bool,
    pub hidden: bool,
}

impl WalkConfig {
    pub fn from_search_args(args: &crate::cli::SearchArgs) -> Self {
        let paths = if args.paths.is_empty() {
            vec![PathBuf::from(".")]
        } else {
            args.paths.clone()
        };
        Self {
            paths,
            types: args.types.clone(),
            globs: args.globs.clone(),
            follow_symlinks: false,
            hidden: false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WalkError {
    #[error("unknown file type: {0}")]
    UnknownType(String),
    #[error("invalid glob `{glob}`: {source}")]
    InvalidGlob {
        glob: String,
        #[source]
        source: ignore::Error,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Enumerate candidate files for indexing/searching.
///
/// The iterator yields regular files only; directories, hidden files
/// (when `hidden = false`), `.gitignore`-matched paths, and binary files
/// (NUL byte detected in the first 8 KiB) are excluded. Walk-level errors
/// are reported on stderr in ripgrep's style and skipped.
pub fn walk(config: &WalkConfig) -> Result<impl Iterator<Item = PathBuf>, WalkError> {
    let walker = build_walker(config)?;
    Ok(walker.filter_map(filter_entry))
}

fn filter_entry(result: Result<ignore::DirEntry, ignore::Error>) -> Option<PathBuf> {
    match result {
        Ok(entry) => {
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                return None;
            }
            let path = entry.path();
            match is_probably_binary(path) {
                Ok(true) => None,
                Ok(false) => Some(entry.into_path()),
                Err(err) => {
                    eprintln!("vsgrep: {}: {}", path.display(), err);
                    None
                }
            }
        }
        Err(err) => {
            eprintln!("vsgrep: {err}");
            None
        }
    }
}

fn build_walker(config: &WalkConfig) -> Result<Walk, WalkError> {
    let (first, rest) = config
        .paths
        .split_first()
        .expect("WalkConfig::paths must not be empty (guaranteed by from_search_args)");

    let mut builder = WalkBuilder::new(first);
    for path in rest {
        builder.add(path);
    }
    builder.hidden(!config.hidden);
    builder.follow_links(config.follow_symlinks);

    if let Some(types) = build_types(&config.types)? {
        builder.types(types);
    }

    if let Some(overrides) = build_overrides(first, &config.globs)? {
        builder.overrides(overrides);
    }

    Ok(builder.build())
}

fn build_types(names: &[String]) -> Result<Option<ignore::types::Types>, WalkError> {
    if names.is_empty() {
        return Ok(None);
    }
    let mut tb = TypesBuilder::new();
    tb.add_defaults();

    let known: std::collections::HashSet<String> = tb
        .definitions()
        .iter()
        .map(|d| d.name().to_string())
        .collect();
    for name in names {
        if name != "all" && !known.contains(name) {
            return Err(WalkError::UnknownType(name.clone()));
        }
    }
    for name in names {
        tb.select(name);
    }
    let types = tb.build().map_err(|err| match err {
        ignore::Error::UnrecognizedFileType(name) => WalkError::UnknownType(name),
        other => WalkError::InvalidGlob {
            glob: String::new(),
            source: other,
        },
    })?;
    Ok(Some(types))
}

fn build_overrides(
    base: &Path,
    globs: &[String],
) -> Result<Option<ignore::overrides::Override>, WalkError> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut builder = OverrideBuilder::new(base);
    for glob in globs {
        builder.add(glob).map_err(|source| WalkError::InvalidGlob {
            glob: glob.clone(),
            source,
        })?;
    }
    let overrides = builder.build().map_err(|source| WalkError::InvalidGlob {
        glob: String::new(),
        source,
    })?;
    Ok(Some(overrides))
}

/// Heuristically detect whether `path` looks like a binary file.
///
/// Reads up to the first 8 KiB and returns `true` if any NUL (`0x00`) byte
/// is found. Mirrors ripgrep's default content-based detection. Empty files
/// are treated as text.
pub fn is_probably_binary(path: &Path) -> std::io::Result<bool> {
    let mut file = File::open(path)?;
    let mut buf = [0u8; BINARY_PROBE_BYTES];
    let n = file.read(&mut buf)?;
    Ok(buf[..n].contains(&0u8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    use clap::Parser;

    fn parse(args: &[&str]) -> crate::cli::Cli {
        crate::cli::Cli::try_parse_from(args).expect("parse should succeed")
    }

    #[test]
    fn from_search_args_defaults_to_dot() {
        let cli = parse(&["vsgrep", "q"]);
        let cfg = WalkConfig::from_search_args(&cli.search);
        assert_eq!(cfg.paths, vec![PathBuf::from(".")]);
        assert!(cfg.types.is_empty());
        assert!(cfg.globs.is_empty());
        assert!(!cfg.follow_symlinks);
        assert!(!cfg.hidden);
    }

    #[test]
    fn from_search_args_carries_paths_types_globs() {
        let cli = parse(&["vsgrep", "-t", "rust", "-g", "!*.md", "q", "src/", "lib/"]);
        let cfg = WalkConfig::from_search_args(&cli.search);
        assert_eq!(
            cfg.paths,
            vec![PathBuf::from("src/"), PathBuf::from("lib/")]
        );
        assert_eq!(cfg.types, vec!["rust".to_string()]);
        assert_eq!(cfg.globs, vec!["!*.md".to_string()]);
    }

    #[test]
    fn unknown_type_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = WalkConfig {
            paths: vec![dir.path().to_path_buf()],
            types: vec!["this-type-does-not-exist".to_string()],
            globs: vec![],
            follow_symlinks: false,
            hidden: false,
        };
        match walk(&cfg) {
            Err(WalkError::UnknownType(name)) => assert_eq!(name, "this-type-does-not-exist"),
            Err(other) => panic!("expected UnknownType, got {other:?}"),
            Ok(_) => panic!("expected UnknownType, got Ok"),
        }
    }

    #[test]
    fn invalid_glob_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = WalkConfig {
            paths: vec![dir.path().to_path_buf()],
            types: vec![],
            globs: vec!["[".to_string()],
            follow_symlinks: false,
            hidden: false,
        };
        match walk(&cfg) {
            Err(WalkError::InvalidGlob { glob, .. }) => assert_eq!(glob, "["),
            Err(other) => panic!("expected InvalidGlob, got {other:?}"),
            Ok(_) => panic!("expected InvalidGlob, got Ok"),
        }
    }

    #[test]
    fn binary_text_file_is_not_binary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::write(&path, "hello, world\n").unwrap();
        assert!(!is_probably_binary(&path).unwrap());
    }

    #[test]
    fn binary_with_nul_byte_is_binary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\n\x00\x00\x00rest").unwrap();
        assert!(is_probably_binary(&path).unwrap());
    }

    #[test]
    fn empty_file_is_not_binary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty");
        std::fs::write(&path, b"").unwrap();
        assert!(!is_probably_binary(&path).unwrap());
    }

    #[test]
    fn large_file_without_nul_is_not_binary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        let mut f = File::create(&path).unwrap();
        let chunk = vec![b'x'; 4096];
        for _ in 0..(10 * 1024 * 1024 / chunk.len()) {
            f.write_all(&chunk).unwrap();
        }
        assert!(!is_probably_binary(&path).unwrap());
    }

    fn collect_relative(dir: &Path, cfg: &WalkConfig) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = walk(cfg)
            .unwrap()
            .map(|p| p.strip_prefix(dir).unwrap_or(&p).to_path_buf())
            .collect();
        out.sort();
        out
    }

    fn cfg_for(dir: &Path) -> WalkConfig {
        WalkConfig {
            paths: vec![dir.to_path_buf()],
            types: vec![],
            globs: vec![],
            follow_symlinks: false,
            hidden: false,
        }
    }

    #[test]
    fn gitignore_excludes_matching_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("foo.log"), "log line\n").unwrap();
        std::fs::write(dir.path().join("keep.txt"), "keep\n").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();
        // `ignore` only honors `.gitignore` inside a git tree by default; create one.
        std::fs::create_dir(dir.path().join(".git")).unwrap();

        let files = collect_relative(dir.path(), &cfg_for(dir.path()));
        assert!(files.iter().any(|p| p == Path::new("keep.txt")));
        assert!(
            !files.iter().any(|p| p == Path::new("foo.log")),
            "foo.log should be ignored, got {files:?}"
        );
    }

    #[test]
    fn binary_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("image.png"),
            b"\x89PNG\r\n\x1a\n\x00\x00\x00rest",
        )
        .unwrap();
        std::fs::write(dir.path().join("hello.txt"), "hello\n").unwrap();

        let files = collect_relative(dir.path(), &cfg_for(dir.path()));
        assert!(files.iter().any(|p| p == Path::new("hello.txt")));
        assert!(
            !files.iter().any(|p| p == Path::new("image.png")),
            "image.png should be skipped as binary, got {files:?}"
        );
    }

    #[test]
    fn hidden_files_are_skipped_by_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("visible.txt"), "v\n").unwrap();
        std::fs::write(dir.path().join(".secret"), "s\n").unwrap();

        let files = collect_relative(dir.path(), &cfg_for(dir.path()));
        assert!(files.iter().any(|p| p == Path::new("visible.txt")));
        assert!(
            !files.iter().any(|p| p == Path::new(".secret")),
            "hidden file should be skipped, got {files:?}"
        );
    }

    #[test]
    fn type_rust_filters_to_rs_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("b.py"), "print(1)\n").unwrap();

        let mut cfg = cfg_for(dir.path());
        cfg.types = vec!["rust".to_string()];

        let files = collect_relative(dir.path(), &cfg);
        assert_eq!(files, vec![PathBuf::from("a.rs")]);
    }

    #[test]
    fn negated_glob_excludes_matches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# hi\n").unwrap();

        let mut cfg = cfg_for(dir.path());
        cfg.globs = vec!["!*.md".to_string()];

        let files = collect_relative(dir.path(), &cfg);
        assert!(files.iter().any(|p| p == Path::new("a.rs")));
        assert!(
            !files.iter().any(|p| p == Path::new("README.md")),
            "README.md should be excluded, got {files:?}"
        );
    }

    #[test]
    fn multiple_paths_are_walked() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        std::fs::write(a.join("x.txt"), "x\n").unwrap();
        std::fs::write(b.join("y.txt"), "y\n").unwrap();

        let cfg = WalkConfig {
            paths: vec![a.clone(), b.clone()],
            types: vec![],
            globs: vec![],
            follow_symlinks: false,
            hidden: false,
        };
        let mut files: Vec<PathBuf> = walk(&cfg)
            .unwrap()
            .map(|p| p.strip_prefix(dir.path()).unwrap_or(&p).to_path_buf())
            .collect();
        files.sort();
        assert_eq!(
            files,
            vec![PathBuf::from("a/x.txt"), PathBuf::from("b/y.txt")]
        );
    }
}
