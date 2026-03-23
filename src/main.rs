mod cli;
mod config;
mod error;
mod fs_ops;
mod language;
mod mapping;
mod obfuscator;
mod ollama;
mod tui;

use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use cli::{Args, Mode};
use config::ConfigPaths;
use error::{AppError, AppResult};
use mapping::{detect_terms, enrich_with_random, load_manual, load_mapping, save_mapping};
use ollama::{OllamaConfig, suggest_mapping};

const PROGRESS_BAR_WIDTH: usize = 30;

#[derive(Debug, Clone)]
pub(crate) struct RunRequest {
    mode: Mode,
    source: PathBuf,
    target: PathBuf,
    mapping: Option<PathBuf>,
    output_mapping: Option<PathBuf>,
    deep: bool,
    ollama_url: Option<String>,
    ollama_model: Option<String>,
    ollama_top_n: usize,
    seed: Option<u64>,
}

#[derive(Debug, Clone)]
struct MappingSelection {
    path: Option<PathBuf>,
    values: BTreeMap<String, String>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let args = cli::parse();
    let config = ConfigPaths::discover()?;
    let use_tui = args.tui || !args.is_non_interactive();
    let request = if use_tui {
        tui::prompt(&config)?
    } else {
        build_non_interactive_request(args)?
    };

    execute(&request, &config)
}

fn build_non_interactive_request(args: Args) -> AppResult<RunRequest> {
    if !args.is_non_interactive() {
        return err("either pass non-interactive flags (--mode/--source/--target) or use --tui");
    }

    Ok(RunRequest {
        mode: args.mode.ok_or_else(|| {
            AppError::InvalidArg("--mode is required unless --tui is used".into())
        })?,
        source: args.source.ok_or_else(|| {
            AppError::InvalidArg("--source is required unless --tui is used".into())
        })?,
        target: args.target.ok_or_else(|| {
            AppError::InvalidArg("--target is required unless --tui is used".into())
        })?,
        mapping: args.mapping,
        output_mapping: args.output_mapping,
        deep: args.deep,
        ollama_url: args.ollama_url,
        ollama_model: args.ollama_model,
        ollama_top_n: args.ollama_top_n,
        seed: args.seed,
    })
}

fn execute(request: &RunRequest, config: &ConfigPaths) -> AppResult<()> {
    validate(request, config)?;
    std::fs::create_dir_all(&request.target)?;
    match request.mode {
        Mode::Forward => forward(request, config),
        Mode::Reverse => reverse(request),
    }
}

fn validate(request: &RunRequest, config: &ConfigPaths) -> AppResult<()> {
    if !request.source.exists() {
        return err("source directory does not exist");
    }
    if !request.source.is_dir() {
        return err("source must be a directory");
    }
    if matches!(request.mode, Mode::Forward)
        && !request.deep
        && resolve_forward_mapping_path(request, config).is_none()
    {
        return err("mapping is required in forward mode unless --deep is set");
    }
    Ok(())
}

fn err<T>(msg: &str) -> AppResult<T> {
    Err(AppError::InvalidArg(msg.into()))
}

fn forward(request: &RunRequest, config: &ConfigPaths) -> AppResult<()> {
    let files = fs_ops::read_text_tree(&request.source)?;
    let mapping_selection = load_forward_mapping(request, config)?;

    if !request.deep {
        return apply_and_save(
            request,
            files,
            mapping_selection.values,
            mapping_selection.path,
        );
    }

    let mut map = mapping_selection.values;
    let terms = detect_terms(&files)?;
    merge_ai(request, &terms, &mut map)?;
    enrich_with_random(&mut map, &terms, &files, request.seed);
    apply_and_save(request, files, map, mapping_selection.path)
}

fn load_forward_mapping(request: &RunRequest, config: &ConfigPaths) -> AppResult<MappingSelection> {
    if let Some(path) = request.mapping.as_ref() {
        let values = load_manual(Some(path))?;
        config.persist_default_mapping(&values)?;
        return Ok(MappingSelection {
            path: Some(path.clone()),
            values,
        });
    }

    if let Some(path) = config.default_mapping_path_if_exists() {
        return Ok(MappingSelection {
            values: config.load_default_mapping()?,
            path: Some(path),
        });
    }

    Ok(MappingSelection {
        path: None,
        values: BTreeMap::new(),
    })
}

fn merge_ai(
    request: &RunRequest,
    terms: &std::collections::BTreeSet<String>,
    map: &mut BTreeMap<String, String>,
) -> AppResult<()> {
    let Some(cfg) = ollama_cfg(request) else {
        return Ok(());
    };
    let ai = suggest_mapping(&cfg, &terms.iter().cloned().collect::<Vec<_>>())?;
    for (k, v) in ai {
        map.entry(k).or_insert(v);
    }
    Ok(())
}

fn ollama_cfg(request: &RunRequest) -> Option<OllamaConfig> {
    Some(OllamaConfig {
        url: request.ollama_url.clone()?,
        model: request.ollama_model.clone()?,
        top_n: request.ollama_top_n,
    })
}

fn apply_and_save(
    request: &RunRequest,
    files: Vec<fs_ops::FileEntry>,
    map: BTreeMap<String, String>,
    input_mapping_path: Option<PathBuf>,
) -> AppResult<()> {
    let transformed = transform_with_progress(&files, &map, request.deep, "Obfuscating")?;
    fs_ops::write_text_tree(&request.target, &transformed)?;
    let path = output_map_path(request);
    save_mapping(&path, &map)?;
    print_stats(files.len(), &map, input_mapping_path.as_deref(), &path);
    Ok(())
}

fn reverse(request: &RunRequest) -> AppResult<()> {
    let path = input_map_path(request)?;
    let map_file = load_mapping(&path)?;
    let files = fs_ops::read_text_tree(&request.source)?;
    let transformed =
        transform_with_progress(&files, &map_file.reverse, request.deep, "Deobfuscating")?;
    fs_ops::write_text_tree(&request.target, &transformed)?;
    print_stats(files.len(), &map_file.reverse, Some(&path), &path);
    Ok(())
}

fn transform_with_progress(
    files: &[fs_ops::FileEntry],
    map: &BTreeMap<String, String>,
    deep: bool,
    stage: &'static str,
) -> AppResult<Vec<(PathBuf, String)>> {
    if !io::stdout().is_terminal() || files.is_empty() {
        return if deep {
            obfuscator::transform_files(files, map)
        } else {
            obfuscator::transform_files_global(files, map)
        };
    }

    let mut progress = ProgressPrinter::new(stage, files.len());
    let transformed = if deep {
        obfuscator::transform_files_with_progress(files, map, |done, total| {
            progress.update(done, total);
        })?
    } else {
        obfuscator::transform_files_global_with_progress(files, map, |done, total| {
            progress.update(done, total);
        })?
    };
    progress.finish();
    Ok(transformed)
}

struct ProgressPrinter {
    enabled: bool,
    stage: &'static str,
    total: usize,
    update_step: usize,
    last_reported: usize,
}

impl ProgressPrinter {
    fn new(stage: &'static str, total: usize) -> Self {
        let enabled = io::stdout().is_terminal() && total > 0;
        let update_step = (total / 100).max(1);
        let progress = Self {
            enabled,
            stage,
            total,
            update_step,
            last_reported: 0,
        };
        if progress.enabled {
            progress.render(0);
        }
        progress
    }

    fn update(&mut self, done: usize, total: usize) {
        if !self.enabled {
            return;
        }

        if total != self.total {
            self.total = total;
            self.update_step = (self.total / 100).max(1);
        }

        let done = done.min(self.total);
        if done < self.total && done.saturating_sub(self.last_reported) < self.update_step {
            return;
        }
        self.last_reported = done;
        self.render(done);
    }

    fn finish(&mut self) {
        if !self.enabled {
            return;
        }
        self.render(self.total);
        println!();
    }

    fn render(&self, done: usize) {
        let safe_total = self.total.max(1);
        let done = done.min(self.total);
        let filled = done.saturating_mul(PROGRESS_BAR_WIDTH) / safe_total;
        let empty = PROGRESS_BAR_WIDTH.saturating_sub(filled);
        let percent = done.saturating_mul(100) / safe_total;
        print!(
            "\r{stage}: [{filled_bar}{empty_bar}] {done}/{total} ({percent}%)",
            stage = self.stage,
            filled_bar = "=".repeat(filled),
            empty_bar = "-".repeat(empty),
            total = self.total
        );
        let _ = io::stdout().flush();
    }
}

fn resolve_forward_mapping_path(request: &RunRequest, config: &ConfigPaths) -> Option<PathBuf> {
    request
        .mapping
        .clone()
        .or_else(|| config.default_mapping_path_if_exists())
}

fn output_map_path(request: &RunRequest) -> PathBuf {
    request
        .output_mapping
        .clone()
        .unwrap_or_else(|| request.target.join("mapping.generated.json"))
}

fn input_map_path(request: &RunRequest) -> AppResult<PathBuf> {
    if let Some(p) = request.mapping.clone() {
        return Ok(p);
    }
    let default = request.source.join("mapping.generated.json");
    if default.exists() {
        return Ok(default);
    }
    Err(AppError::InvalidArg(
        "mapping is required in reverse mode".into(),
    ))
}

fn print_stats(
    files: usize,
    map: &BTreeMap<String, String>,
    input_mapping_path: Option<&Path>,
    path: &Path,
) {
    println!("processed_files={files}");
    println!("mapping_entries={}", map.len());
    if let Some(input_mapping_path) = input_mapping_path {
        println!("mapping_input_path={}", input_mapping_path.display());
    }
    println!("mapping_path={}", path.display());
}
