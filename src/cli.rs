use std::path::PathBuf;

use clap::{ArgAction, Parser, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Mode {
    Forward,
    Reverse,
}

#[derive(Debug, Parser, Clone)]
#[command(author, version, about)]
pub struct Args {
    #[arg(long, action = ArgAction::SetTrue, help = "Launch interactive terminal UI")]
    pub tui: bool,
    #[arg(long, value_enum, help = "Processing mode for non-interactive CLI")]
    pub mode: Option<Mode>,
    #[arg(long, help = "Source directory for non-interactive CLI")]
    pub source: Option<PathBuf>,
    #[arg(long, help = "Target directory for non-interactive CLI")]
    pub target: Option<PathBuf>,
    #[arg(
        long,
        help = "Explicit manual mapping JSON file (takes priority over config default)"
    )]
    pub mapping: Option<PathBuf>,
    #[arg(long, help = "Where to save generated forward+reverse mapping JSON")]
    pub output_mapping: Option<PathBuf>,
    #[arg(
        long,
        default_value_t = false,
        help = "Enable automatic identifier discovery and deep obfuscation"
    )]
    pub deep: bool,
    #[arg(long, help = "Ollama base URL for deep mode suggestions")]
    pub ollama_url: Option<String>,
    #[arg(long, help = "Ollama model name for deep mode suggestions")]
    pub ollama_model: Option<String>,
    #[arg(
        long,
        default_value_t = 25,
        help = "How many candidate terms to send to Ollama"
    )]
    pub ollama_top_n: usize,
    #[arg(long, help = "Optional randomization seed")]
    pub seed: Option<u64>,
}

impl Args {
    pub fn is_non_interactive(&self) -> bool {
        self.mode.is_some()
            || self.source.is_some()
            || self.target.is_some()
            || self.mapping.is_some()
            || self.output_mapping.is_some()
            || self.deep
            || self.ollama_url.is_some()
            || self.ollama_model.is_some()
            || self.seed.is_some()
    }
}

pub fn parse() -> Args {
    Args::parse()
}
