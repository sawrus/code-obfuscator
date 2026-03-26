use std::fs;
use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use walkdir::WalkDir;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub rel: PathBuf,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct RootGitignore {
    root: PathBuf,
    matcher: Gitignore,
}

impl RootGitignore {
    pub fn from_root(root: &Path) -> AppResult<Self> {
        let mut builder = GitignoreBuilder::new(root);
        if root.join(".gitignore").is_file()
            && let Some(err) = builder.add(root.join(".gitignore"))
        {
            return Err(AppError::InvalidArg(err.to_string()));
        }
        let matcher = builder
            .build()
            .map_err(|err| AppError::InvalidArg(err.to_string()))?;
        Ok(Self {
            root: root.to_path_buf(),
            matcher,
        })
    }

    pub fn is_ignored_rel(&self, rel: &Path, is_dir: bool) -> bool {
        self.matcher
            .matched_path_or_any_parents(rel, is_dir)
            .is_ignore()
    }

    pub fn is_ignored_abs(&self, path: &Path, is_dir: bool) -> bool {
        path.strip_prefix(&self.root)
            .ok()
            .map(|rel| self.is_ignored_rel(rel, is_dir))
            .unwrap_or(false)
    }
}

pub fn read_text_tree(root: &Path) -> AppResult<Vec<FileEntry>> {
    let gitignore = RootGitignore::from_root(root)?;
    let mut files = Vec::new();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| {
            if entry.path() == root {
                return true;
            }
            !gitignore.is_ignored_abs(entry.path(), entry.file_type().is_dir())
        })
        .flatten()
    {
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

    #[test]
    fn skips_paths_ignored_by_root_gitignore() {
        let dir = tempdir().expect("tmp");
        fs::create_dir_all(dir.path().join("ignored_dir")).expect("mkdir");
        fs::write(dir.path().join(".gitignore"), "ignored_dir/\n*.secret\n").expect("write");
        fs::write(dir.path().join("ignored_dir/one.rs"), "let x = 1;").expect("write");
        fs::write(dir.path().join("two.secret"), "token").expect("write");
        fs::write(dir.path().join("three.rs"), "let y = 2;").expect("write");

        let files = read_text_tree(dir.path()).expect("read");
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.rel == Path::new(".gitignore")));
        assert!(files.iter().any(|f| f.rel == Path::new("three.rs")));
    }
}
