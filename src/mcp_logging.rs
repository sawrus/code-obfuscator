use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

const DEFAULT_LOG_DIR: &str = "logs";
const DEFAULT_LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;
const DEFAULT_LOG_MAX_FILES: usize = 10;

#[derive(Clone)]
pub struct McpLogger {
    inner: Arc<Mutex<LoggerInner>>,
}

struct LoggerInner {
    log_file_path: PathBuf,
    file: File,
    max_bytes: u64,
    max_files: usize,
    stdout_enabled: bool,
    disable_stdout_after_stdio: bool,
    stdout_disabled_by_stdio: bool,
}

pub struct LogEvent<'a> {
    pub level: &'a str,
    pub transport: &'a str,
    pub direction: &'a str,
    pub request_id: Option<&'a str>,
    pub jsonrpc_id: Option<&'a Value>,
    pub method: Option<&'a str>,
    pub path: Option<&'a str>,
    pub status: Option<&'a str>,
    pub duration_ms: Option<u128>,
    pub payload: Option<&'a Value>,
}

impl McpLogger {
    pub fn from_env() -> io::Result<Self> {
        let log_dir = env::var("MCP_LOG_DIR").unwrap_or_else(|_| DEFAULT_LOG_DIR.to_string());
        let max_bytes = env::var("MCP_LOG_MAX_BYTES")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_LOG_MAX_BYTES);
        let max_files = env::var("MCP_LOG_MAX_FILES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(DEFAULT_LOG_MAX_FILES);
        let stdout_enabled = env::var("MCP_LOG_STDOUT")
            .ok()
            .map(|v| parse_bool(&v))
            .unwrap_or(true);
        let disable_stdout_after_stdio = env::var("MCP_LOG_DISABLE_STDOUT_ON_STDIO")
            .ok()
            .map(|v| parse_bool(&v))
            .unwrap_or(true);

        Self::new(
            PathBuf::from(log_dir),
            max_bytes,
            max_files,
            stdout_enabled,
            disable_stdout_after_stdio,
        )
    }

    pub fn new(
        log_dir: PathBuf,
        max_bytes: u64,
        max_files: usize,
        stdout_enabled: bool,
        disable_stdout_after_stdio: bool,
    ) -> io::Result<Self> {
        fs::create_dir_all(&log_dir)?;
        let log_file_path = log_dir.join("mcp-server.log");
        let file = open_log_file(&log_file_path)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(LoggerInner {
                log_file_path,
                file,
                max_bytes,
                max_files,
                stdout_enabled,
                disable_stdout_after_stdio,
                stdout_disabled_by_stdio: false,
            })),
        })
    }

    pub fn log(&self, event: LogEvent<'_>) {
        let mut record = Map::new();
        record.insert("ts".into(), json!(now_epoch_ms()));
        record.insert("level".into(), json!(event.level));
        record.insert("transport".into(), json!(event.transport));
        record.insert("direction".into(), json!(event.direction));
        record.insert("request_id".into(), opt_json_str(event.request_id));
        record.insert(
            "jsonrpc_id".into(),
            event.jsonrpc_id.cloned().unwrap_or(Value::Null),
        );
        record.insert("method".into(), opt_json_str(event.method));
        record.insert("path".into(), opt_json_str(event.path));
        record.insert("status".into(), opt_json_str(event.status));
        record.insert(
            "duration_ms".into(),
            event
                .duration_ms
                .map(|v| Value::from(v as u64))
                .unwrap_or(Value::Null),
        );
        record.insert(
            "payload".into(),
            event.payload.cloned().unwrap_or(Value::Null),
        );

        let line = Value::Object(record).to_string();
        let disable_stdout = event.transport == "stdio";
        self.write_line(disable_stdout, &line);
    }

    fn write_line(&self, disable_stdout: bool, line: &str) {
        let mut guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };

        if disable_stdout && guard.disable_stdout_after_stdio {
            guard.stdout_disabled_by_stdio = true;
        }

        let line_len = line.len() as u64 + 1;
        if rotate_if_needed(&mut guard, line_len).is_err() {
            return;
        }

        if guard.file.write_all(line.as_bytes()).is_err() {
            return;
        }
        if guard.file.write_all(b"\n").is_err() {
            return;
        }
        let _ = guard.file.flush();

        if guard.stdout_enabled && !guard.stdout_disabled_by_stdio {
            let mut out = io::stdout().lock();
            let _ = out.write_all(line.as_bytes());
            let _ = out.write_all(b"\n");
            let _ = out.flush();
        }
    }
}

fn rotate_if_needed(inner: &mut LoggerInner, incoming_len: u64) -> io::Result<()> {
    if inner.max_bytes == 0 {
        return Ok(());
    }

    let current_size = inner.file.metadata().map(|m| m.len()).unwrap_or(0);
    if current_size.saturating_add(incoming_len) <= inner.max_bytes {
        return Ok(());
    }

    if inner.max_files == 0 {
        inner.file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&inner.log_file_path)?;
        return Ok(());
    }

    for idx in (1..=inner.max_files).rev() {
        let path = rotated_path(&inner.log_file_path, idx);
        if idx == inner.max_files {
            let _ = fs::remove_file(&path);
            continue;
        }
        let prev = rotated_path(&inner.log_file_path, idx);
        let next = rotated_path(&inner.log_file_path, idx + 1);
        if prev.exists() {
            let _ = fs::rename(prev, next);
        }
    }

    let first = rotated_path(&inner.log_file_path, 1);
    if inner.log_file_path.exists() {
        let _ = fs::rename(&inner.log_file_path, first);
    }

    inner.file = open_log_file(&inner.log_file_path)?;
    Ok(())
}

fn open_log_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn rotated_path(path: &Path, idx: usize) -> PathBuf {
    let mut s = path.to_string_lossy().to_string();
    s.push('.');
    s.push_str(&idx.to_string());
    PathBuf::from(s)
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn parse_bool(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn opt_json_str(v: Option<&str>) -> Value {
    v.map(Value::from).unwrap_or(Value::Null)
}
