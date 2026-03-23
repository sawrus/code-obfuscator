use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use crate::RunRequest;
use crate::cli::Mode;
use crate::config::ConfigPaths;
use crate::error::{AppError, AppResult};

pub fn prompt(config: &ConfigPaths) -> AppResult<RunRequest> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    prompt_with_io(&mut reader, &mut writer, config)
}

fn prompt_with_io<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    config: &ConfigPaths,
) -> AppResult<RunRequest> {
    writeln!(writer, "code-obfuscator interactive mode")?;
    writeln!(
        writer,
        "default config mapping: {}",
        config.mapping_file.display()
    )?;
    writeln!(
        writer,
        "default config mapping status: {}",
        if config.mapping_file.exists() {
            "found"
        } else {
            "not found"
        }
    )?;

    let mode = prompt_mode(reader, writer)?;
    let source = prompt_required_path(reader, writer, "Source directory")?;
    let target = prompt_required_path(reader, writer, "Target directory")?;
    let deep = prompt_bool(reader, writer, "Enable deep mode?", false)?;

    let mut mapping = None;
    let mut output_mapping = None;

    match mode {
        Mode::Forward => {
            mapping = prompt_forward_mapping(reader, writer, config, deep)?;
            output_mapping = prompt_optional_path(
                reader,
                writer,
                "Custom output mapping path (leave blank for target/mapping.generated.json)",
            )?;
        }
        Mode::Reverse => {
            if prompt_bool(
                reader,
                writer,
                "Provide explicit mapping path instead of auto-detecting source/mapping.generated.json?",
                false,
            )? {
                mapping = Some(prompt_required_path(reader, writer, "Mapping path")?);
            }
        }
    }

    let mut ollama_url = None;
    let mut ollama_model = None;
    let mut ollama_top_n = 25;

    if matches!(mode, Mode::Forward)
        && deep
        && prompt_bool(reader, writer, "Use Ollama suggestions?", false)?
    {
        ollama_url = prompt_optional_text(reader, writer, "Ollama URL")?;
        ollama_model = prompt_optional_text(reader, writer, "Ollama model")?;
        ollama_top_n = prompt_usize(reader, writer, "Ollama top_n", 25)?;
    }

    let seed = if prompt_bool(reader, writer, "Set a deterministic seed?", false)? {
        Some(prompt_u64(reader, writer, "Seed")?)
    } else {
        None
    };

    Ok(RunRequest {
        mode,
        source,
        target,
        mapping,
        output_mapping,
        deep,
        ollama_url,
        ollama_model,
        ollama_top_n,
        seed,
    })
}

fn prompt_forward_mapping<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    config: &ConfigPaths,
    deep: bool,
) -> AppResult<Option<PathBuf>> {
    let config_exists = config.mapping_file.exists();
    if !deep {
        if config_exists && prompt_bool(reader, writer, "Use default config mapping?", true)? {
            return Ok(Some(config.mapping_file.clone()));
        }
        return Ok(Some(prompt_required_path(reader, writer, "Mapping path")?));
    }

    if config_exists
        && prompt_bool(
            reader,
            writer,
            "Use default config mapping as a base mapping?",
            true,
        )?
    {
        return Ok(Some(config.mapping_file.clone()));
    }

    if prompt_bool(reader, writer, "Provide an explicit mapping path?", false)? {
        return Ok(Some(prompt_required_path(reader, writer, "Mapping path")?));
    }

    Ok(None)
}

fn prompt_mode<R: BufRead, W: Write>(reader: &mut R, writer: &mut W) -> AppResult<Mode> {
    loop {
        writeln!(writer, "Select mode: [1] forward (default), [2] reverse")?;
        let value = read_line(reader, writer, "Mode [1]")?;
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "1" | "forward" => return Ok(Mode::Forward),
            "2" | "reverse" => return Ok(Mode::Reverse),
            _ => writeln!(writer, "Please enter 1/forward or 2/reverse.")?,
        }
    }
}

fn prompt_required_path<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
) -> AppResult<PathBuf> {
    loop {
        let value = read_line(reader, writer, label)?;
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
        writeln!(writer, "Value is required.")?;
    }
}

fn prompt_optional_path<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
) -> AppResult<Option<PathBuf>> {
    Ok(prompt_optional_text(reader, writer, label)?.map(PathBuf::from))
}

fn prompt_optional_text<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
) -> AppResult<Option<String>> {
    let value = read_line(reader, writer, label)?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

fn prompt_bool<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
    default: bool,
) -> AppResult<bool> {
    loop {
        let suffix = if default { "[Y/n]" } else { "[y/N]" };
        let value = read_line(reader, writer, &format!("{label} {suffix}"))?;
        let trimmed = value.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            return Ok(default);
        }
        match trimmed.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(writer, "Please answer yes or no.")?,
        }
    }
}

fn prompt_usize<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
    default: usize,
) -> AppResult<usize> {
    loop {
        let value = read_line(reader, writer, &format!("{label} [{default}]"))?;
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(default);
        }
        if let Ok(parsed) = trimmed.parse::<usize>() {
            return Ok(parsed);
        }
        writeln!(writer, "Please enter a valid positive integer.")?;
    }
}

fn prompt_u64<R: BufRead, W: Write>(reader: &mut R, writer: &mut W, label: &str) -> AppResult<u64> {
    loop {
        let value = read_line(reader, writer, label)?;
        if let Ok(parsed) = value.trim().parse::<u64>() {
            return Ok(parsed);
        }
        writeln!(writer, "Please enter a valid integer.")?;
    }
}

fn read_line<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
) -> AppResult<String> {
    write!(writer, "{label}: ")?;
    writer.flush()?;
    let mut line = String::new();
    let bytes = reader.read_line(&mut line)?;
    if bytes == 0 {
        return Err(AppError::InvalidArg(
            "interactive input ended before the TUI flow completed".into(),
        ));
    }
    Ok(line)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::config::ConfigPaths;

    use super::*;

    #[test]
    fn forward_flow_uses_config_mapping_when_selected() {
        let temp = tempfile::tempdir().expect("tmp");
        let config = ConfigPaths {
            dir: temp.path().to_path_buf(),
            mapping_file: temp.path().join("mapping.json"),
        };
        std::fs::write(&config.mapping_file, "{}\n").expect("write mapping");

        let input = b"1\n/tmp/src\n/tmp/out\nn\ny\n\n\nn\n";
        let mut reader = Cursor::new(&input[..]);
        let mut writer = Vec::new();
        let request = prompt_with_io(&mut reader, &mut writer, &config).expect("request");

        assert!(matches!(request.mode, Mode::Forward));
        assert_eq!(request.mapping, Some(config.mapping_file));
        assert!(!request.deep);
    }

    #[test]
    fn reverse_flow_allows_auto_mapping() {
        let temp = tempfile::tempdir().expect("tmp");
        let config = ConfigPaths {
            dir: temp.path().to_path_buf(),
            mapping_file: temp.path().join("mapping.json"),
        };

        let input = b"2\n/tmp/src\n/tmp/out\nn\nn\nn\n";
        let mut reader = Cursor::new(&input[..]);
        let mut writer = Vec::new();
        let request = prompt_with_io(&mut reader, &mut writer, &config).expect("request");

        assert!(matches!(request.mode, Mode::Reverse));
        assert_eq!(request.mapping, None);
        assert_eq!(request.seed, None);
    }

    #[test]
    fn mode_defaults_to_forward_on_empty_input() {
        let temp = tempfile::tempdir().expect("tmp");
        let config = ConfigPaths {
            dir: temp.path().to_path_buf(),
            mapping_file: temp.path().join("mapping.json"),
        };
        std::fs::write(&config.mapping_file, "{}\n").expect("write mapping");

        let input = b"\n/tmp/src\n/tmp/out\nn\ny\n\nn\n";
        let mut reader = Cursor::new(&input[..]);
        let mut writer = Vec::new();
        let request = prompt_with_io(&mut reader, &mut writer, &config).expect("request");

        assert!(matches!(request.mode, Mode::Forward));
    }
}
