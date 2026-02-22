use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::AppResult;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub rel: PathBuf,
    pub text: String,
}

pub fn read_text_tree(root: &Path) -> AppResult<Vec<FileEntry>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root).into_iter().flatten() {
        maybe_push(root, entry.path(), &mut files)?;
    }
    Ok(files)
}

fn maybe_push(root: &Path, path: &Path, files: &mut Vec<FileEntry>) -> AppResult<()> {
    if !path.is_file() || is_mapping(path) {
        return Ok(());
    }
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(());
    };
    let rel = path.strip_prefix(root).unwrap_or(path).to_path_buf();
    files.push(FileEntry { rel, text });
    Ok(())
}

fn is_mapping(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or_default();
    matches!(name, "mapping.json" | "mapping.generated.json")
}

pub fn write_text_tree(target: &Path, files: &[(PathBuf, String)]) -> AppResult<()> {
    for (rel, text) in files {
        write_one(target, rel, text)?;
    }
    Ok(())
}

fn write_one(target: &Path, rel: &Path, text: &str) -> AppResult<()> {
    let out = target.join(rel);
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out, text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn reads_only_utf8_files() {
        let dir = tempdir().expect("tmp");
        fs::write(dir.path().join("a.rs"), "let x = 1;").expect("write");
        let files = read_text_tree(dir.path()).expect("read");
        assert_eq!(files.len(), 1);
    }
}
