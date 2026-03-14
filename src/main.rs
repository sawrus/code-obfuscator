mod cli;
mod error;
mod fs_ops;
mod language;
mod mapping;
mod obfuscator;
mod ollama;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cli::{Args, Mode};
use error::{AppError, AppResult};
use mapping::{detect_terms, enrich_with_random, load_manual, load_mapping, save_mapping};
use ollama::{OllamaConfig, suggest_mapping};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let args = cli::parse();
    validate(&args)?;
    std::fs::create_dir_all(&args.target)?;
    match args.mode {
        Mode::Forward => forward(&args),
        Mode::Reverse => reverse(&args),
    }
}

fn validate(args: &Args) -> AppResult<()> {
    if !args.source.exists() {
        return err("source directory does not exist");
    }
    if !args.source.is_dir() {
        return err("source must be a directory");
    }
    if matches!(args.mode, Mode::Forward) && !args.deep && args.mapping.is_none() {
        return err("mapping is required in forward mode unless --deep is set");
    }
    Ok(())
}

fn err(msg: &str) -> AppResult<()> {
    Err(AppError::InvalidArg(msg.into()))
}

fn forward(args: &Args) -> AppResult<()> {
    let files = fs_ops::read_text_tree(&args.source)?;
    if !args.deep {
        let map = load_manual(args.mapping.as_deref())?;
        return apply_and_save(args, files, map);
    }

    let mut map = load_manual(args.mapping.as_deref())?;
    let terms = detect_terms(&files)?;
    merge_ai(args, &terms, &mut map)?;
    enrich_with_random(&mut map, &terms, &files, args.seed);
    apply_and_save(args, files, map)
}

fn merge_ai(
    args: &Args,
    terms: &std::collections::BTreeSet<String>,
    map: &mut BTreeMap<String, String>,
) -> AppResult<()> {
    let Some(cfg) = ollama_cfg(args) else {
        return Ok(());
    };
    let ai = suggest_mapping(&cfg, &terms.iter().cloned().collect::<Vec<_>>())?;
    for (k, v) in ai {
        map.entry(k).or_insert(v);
    }
    Ok(())
}

fn ollama_cfg(args: &Args) -> Option<OllamaConfig> {
    Some(OllamaConfig {
        url: args.ollama_url.clone()?,
        model: args.ollama_model.clone()?,
        top_n: args.ollama_top_n,
    })
}

fn apply_and_save(
    args: &Args,
    files: Vec<fs_ops::FileEntry>,
    map: BTreeMap<String, String>,
) -> AppResult<()> {
    let transformed = if args.deep {
        obfuscator::transform_files(&files, &map)?
    } else {
        obfuscator::transform_files_global(&files, &map)?
    };
    fs_ops::write_text_tree(&args.target, &transformed)?;
    let path = output_map_path(args);
    save_mapping(&path, &map)?;
    print_stats(files.len(), &map, &path);
    Ok(())
}

fn reverse(args: &Args) -> AppResult<()> {
    let path = input_map_path(args)?;
    let map_file = load_mapping(&path)?;
    let files = fs_ops::read_text_tree(&args.source)?;
    let transformed = if args.deep {
        obfuscator::transform_files(&files, &map_file.reverse)?
    } else {
        obfuscator::transform_files_global(&files, &map_file.reverse)?
    };
    fs_ops::write_text_tree(&args.target, &transformed)?;
    print_stats(files.len(), &map_file.reverse, &path);
    Ok(())
}

fn output_map_path(args: &Args) -> PathBuf {
    args.output_mapping
        .clone()
        .unwrap_or_else(|| args.target.join("mapping.generated.json"))
}

fn input_map_path(args: &Args) -> AppResult<PathBuf> {
    if let Some(p) = args.mapping.clone() {
        return Ok(p);
    }
    let default = args.source.join("mapping.generated.json");
    if default.exists() {
        return Ok(default);
    }
    Err(AppError::InvalidArg(
        "mapping is required in reverse mode".into(),
    ))
}

fn print_stats(files: usize, map: &BTreeMap<String, String>, path: &Path) {
    println!("processed_files={files}");
    println!("mapping_entries={}", map.len());
    println!("mapping_path={}", path.display());
}
