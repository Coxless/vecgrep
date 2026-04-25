use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "vsgrep",
    version,
    about = "ripgrep-like local semantic (vector) search CLI",
    long_about = None,
    disable_help_subcommand = true,
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(flatten)]
    pub search: SearchArgs,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Args)]
pub struct GlobalArgs {
    #[arg(long, global = true)]
    pub json: bool,

    #[arg(long, global = true)]
    pub timings: Option<Option<String>>,

    #[arg(long, value_name = "FILE", global = true)]
    pub trace: Option<PathBuf>,

    #[arg(long, global = true)]
    pub no_cache: bool,

    #[arg(long, value_name = "MODEL", global = true)]
    pub model: Option<String>,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    #[arg(value_name = "QUERY")]
    pub query: Option<String>,

    #[arg(value_name = "PATH")]
    pub paths: Vec<PathBuf>,

    #[arg(short = 'k', long = "top-k", value_name = "N", default_value_t = 10)]
    pub top_k: usize,

    #[arg(long, value_name = "FLOAT")]
    pub threshold: Option<f32>,

    #[arg(short = 't', long = "type", value_name = "TYPE")]
    pub types: Vec<String>,

    #[arg(short = 'g', long = "glob", value_name = "GLOB")]
    pub globs: Vec<String>,

    #[arg(long)]
    pub score: bool,

    #[arg(short = 'C', long = "context", value_name = "N", default_value_t = 0)]
    pub context: usize,

    #[arg(long)]
    pub heading: bool,

    #[arg(long, value_name = "N")]
    pub ef: Option<usize>,

    #[arg(long, value_name = "N", default_value_t = 20)]
    pub chunk_size: usize,

    #[arg(long, value_name = "N", default_value_t = 5)]
    pub chunk_overlap: usize,

    #[arg(short = 'H', long = "hybrid")]
    pub hybrid: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Build or update the persistent index.
    Index(IndexArgs),

    /// Show statistics about the current index.
    Status(StatusArgs),

    /// Remove the `.vsgrep/` directory.
    Clean,

    /// Manage embedding models.
    Model {
        #[command(subcommand)]
        action: ModelCmd,
    },
}

#[derive(Debug, Args)]
pub struct IndexArgs {
    #[arg(value_name = "PATH")]
    pub paths: Vec<PathBuf>,

    #[arg(long)]
    pub watch: bool,

    #[arg(long)]
    pub no_gitignore: bool,
}

#[derive(Debug, Args)]
pub struct StatusArgs {}

#[derive(Debug, Subcommand)]
pub enum ModelCmd {
    /// List available embedding models.
    List,

    /// Switch the default embedding model (requires reindex).
    Use {
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Register a local ONNX model.
    Add {
        #[arg(value_name = "ONNX")]
        onnx: PathBuf,

        #[arg(long, value_name = "TOKENIZER")]
        tokenizer: Option<PathBuf>,
    },
}

#[allow(dead_code)]
impl SearchArgs {
    /// Validate chunk parameters that clap cannot express as type constraints.
    ///
    /// Called from main in Step 8 when the search pipeline is wired up.
    pub fn validate_chunk_args(&self) -> Result<(), String> {
        if self.chunk_size == 0 {
            return Err("--chunk-size must be > 0".into());
        }
        if self.chunk_overlap >= self.chunk_size {
            return Err(format!(
                "--chunk-overlap ({}) must be less than --chunk-size ({})",
                self.chunk_overlap, self.chunk_size
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Cli::command().debug_assert();
    }

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("parse should succeed")
    }

    #[test]
    fn bare_query_runs_search_mode() {
        let cli = parse(&["vsgrep", "retry logic"]);
        assert!(cli.command.is_none());
        assert_eq!(cli.search.query.as_deref(), Some("retry logic"));
        assert!(cli.search.paths.is_empty());
        assert_eq!(cli.search.top_k, 10);
    }

    #[test]
    fn query_with_paths_and_top_k() {
        let cli = parse(&[
            "vsgrep",
            "q",
            "src/",
            "lib/",
            "-k",
            "20",
            "--threshold",
            "0.7",
        ]);
        assert_eq!(cli.search.query.as_deref(), Some("q"));
        assert_eq!(
            cli.search.paths,
            vec![PathBuf::from("src/"), PathBuf::from("lib/")]
        );
        assert_eq!(cli.search.top_k, 20);
        assert_eq!(cli.search.threshold, Some(0.7));
    }

    #[test]
    fn type_and_glob_accept_multiple() {
        let cli = parse(&[
            "vsgrep",
            "-t",
            "rust",
            "-t",
            "ts",
            "-g",
            "!**/*.test.ts",
            "x",
        ]);
        assert_eq!(cli.search.types, vec!["rust", "ts"]);
        assert_eq!(cli.search.globs, vec!["!**/*.test.ts"]);
        assert_eq!(cli.search.query.as_deref(), Some("x"));
    }

    #[test]
    fn index_watch_subcommand() {
        let cli = parse(&["vsgrep", "index", "--watch"]);
        match cli.command {
            Some(Command::Index(args)) => {
                assert!(args.watch);
                assert!(args.paths.is_empty());
            }
            other => panic!("expected Index, got {other:?}"),
        }
    }

    #[test]
    fn model_use_subcommand() {
        let cli = parse(&["vsgrep", "model", "use", "e5-small"]);
        match cli.command {
            Some(Command::Model {
                action: ModelCmd::Use { name },
            }) => {
                assert_eq!(name, "e5-small");
            }
            other => panic!("expected Model::Use, got {other:?}"),
        }
    }

    #[test]
    fn json_flag_works_with_subcommand() {
        let cli = parse(&["vsgrep", "--json", "status"]);
        assert!(cli.global.json);
        assert!(matches!(cli.command, Some(Command::Status(_))));
    }

    #[test]
    fn json_flag_after_subcommand_also_works() {
        let cli = parse(&["vsgrep", "status", "--json"]);
        assert!(cli.global.json);
        assert!(matches!(cli.command, Some(Command::Status(_))));
    }

    #[test]
    fn chunk_defaults() {
        let cli = parse(&["vsgrep", "q"]);
        assert_eq!(cli.search.chunk_size, 20);
        assert_eq!(cli.search.chunk_overlap, 5);
        assert_eq!(cli.search.context, 0);
    }

    #[test]
    fn hybrid_short_flag() {
        let cli = parse(&["vsgrep", "-H", "q"]);
        assert!(cli.search.hybrid);
    }

    #[test]
    fn model_add_with_tokenizer() {
        let cli = parse(&[
            "vsgrep",
            "model",
            "add",
            "./m.onnx",
            "--tokenizer",
            "./tok.json",
        ]);
        match cli.command {
            Some(Command::Model {
                action: ModelCmd::Add { onnx, tokenizer },
            }) => {
                assert_eq!(onnx, PathBuf::from("./m.onnx"));
                assert_eq!(tokenizer, Some(PathBuf::from("./tok.json")));
            }
            other => panic!("expected Model::Add, got {other:?}"),
        }
    }

    #[test]
    fn chunk_validates_overlap_lt_size() {
        // Parse succeeds; semantic validation catches the error.
        let cli = parse(&["vsgrep", "--chunk-size", "5", "--chunk-overlap", "5", "q"]);
        assert!(cli.search.validate_chunk_args().is_err());
    }

    #[test]
    fn chunk_validates_size_nonzero() {
        let cli = parse(&["vsgrep", "--chunk-size", "0", "q"]);
        assert!(cli.search.validate_chunk_args().is_err());
    }
}
