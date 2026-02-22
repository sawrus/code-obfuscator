use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, ValueEnum)]
pub enum Mode {
    Forward,
    Reverse,
}

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Args {
    #[arg(long, value_enum)]
    pub mode: Mode,
    #[arg(long)]
    pub source: PathBuf,
    #[arg(long)]
    pub target: PathBuf,
    #[arg(long)]
    pub mapping: Option<PathBuf>,
    #[arg(long)]
    pub output_mapping: Option<PathBuf>,
    #[arg(long)]
    pub ollama_url: Option<String>,
    #[arg(long)]
    pub ollama_model: Option<String>,
    #[arg(long, default_value_t = 25)]
    pub ollama_top_n: usize,
    #[arg(long)]
    pub seed: Option<u64>,
}

pub fn parse() -> Args {
    Args::parse()
}
