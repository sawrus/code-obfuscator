use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

pub const APP_DIR_NAME: &str = "code-obfuscator";
pub const DEFAULT_MAPPING_FILE: &str = "mapping.json";

#[derive(Debug, Clone)]
pub struct ConfigPaths {
    pub dir: PathBuf,
    pub mapping_file: PathBuf,
}

impl ConfigPaths {
    pub fn discover() -> AppResult<Self> {
        let dir = detect_config_dir()?;
        Ok(Self {
            mapping_file: dir.join(DEFAULT_MAPPING_FILE),
            dir,
        })
    }

    pub fn default_mapping_path_if_exists(&self) -> Option<PathBuf> {
        self.mapping_file
            .exists()
            .then(|| self.mapping_file.clone())
    }

    pub fn ensure_exists(&self) -> AppResult<()> {
        fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    pub fn load_default_mapping(&self) -> AppResult<BTreeMap<String, String>> {
        if !self.mapping_file.exists() {
            return Ok(BTreeMap::new());
        }
        load_plain_mapping(&self.mapping_file)
    }

    pub fn persist_default_mapping(&self, mapping: &BTreeMap<String, String>) -> AppResult<()> {
        self.ensure_exists()?;
        let text = serde_json::to_string_pretty(mapping)?;
        fs::write(&self.mapping_file, text)?;
        Ok(())
    }
}

pub fn detect_config_dir() -> AppResult<PathBuf> {
    let home = home_dir()?;
    let xdg = env::var_os("XDG_CONFIG_HOME");
    let appdata = env::var_os("APPDATA");
    resolve_config_dir(
        std::env::consts::OS,
        &home,
        xdg.as_deref(),
        appdata.as_deref(),
    )
    .ok_or_else(|| AppError::InvalidArg("unable to determine user config directory".into()))
}

pub fn resolve_config_dir(
    os: &str,
    home_dir: &Path,
    xdg_config_home: Option<&std::ffi::OsStr>,
    appdata: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    if let Some(xdg) = xdg_config_home.filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(xdg).join(APP_DIR_NAME));
    }

    match os {
        "windows" => appdata
            .map(PathBuf::from)
            .or_else(|| Some(home_dir.join("AppData").join("Roaming")))
            .map(|base| base.join(APP_DIR_NAME)),
        "macos" => Some(
            home_dir
                .join("Library")
                .join("Application Support")
                .join(APP_DIR_NAME),
        ),
        _ => Some(home_dir.join(".config").join(APP_DIR_NAME)),
    }
}

fn home_dir() -> AppResult<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| AppError::InvalidArg("unable to determine user home directory".into()))
}

fn load_plain_mapping(path: &Path) -> AppResult<BTreeMap<String, String>> {
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_prefers_xdg_config_home() {
        let path = resolve_config_dir(
            "linux",
            Path::new("/home/alice"),
            Some(std::ffi::OsStr::new("/tmp/xdg")),
            None,
        )
        .expect("path");
        assert_eq!(path, PathBuf::from("/tmp/xdg/code-obfuscator"));
    }

    #[test]
    fn linux_falls_back_to_dot_config() {
        let path = resolve_config_dir("linux", Path::new("/home/alice"), None, None).expect("path");
        assert_eq!(path, PathBuf::from("/home/alice/.config/code-obfuscator"));
    }

    #[test]
    fn macos_uses_application_support() {
        let path =
            resolve_config_dir("macos", Path::new("/Users/alice"), None, None).expect("path");
        assert_eq!(
            path,
            PathBuf::from("/Users/alice/Library/Application Support/code-obfuscator")
        );
    }

    #[test]
    fn windows_prefers_appdata() {
        let path = resolve_config_dir(
            "windows",
            Path::new("C:/Users/alice"),
            None,
            Some(std::ffi::OsStr::new("C:/Users/alice/AppData/Roaming")),
        )
        .expect("path");
        assert_eq!(
            path,
            PathBuf::from("C:/Users/alice/AppData/Roaming/code-obfuscator")
        );
    }

    #[test]
    fn macos_respects_xdg_config_home_override() {
        let path = resolve_config_dir(
            "macos",
            Path::new("/Users/alice"),
            Some(std::ffi::OsStr::new("/tmp/xdg")),
            None,
        )
        .expect("path");
        assert_eq!(path, PathBuf::from("/tmp/xdg/code-obfuscator"));
    }
}
