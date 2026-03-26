use std::backtrace::Backtrace;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::panic::{self, PanicHookInfo};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

const DEFAULT_LOG_DIR: &str = "logs";
const DEFAULT_LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;
const DEFAULT_LOG_MAX_FILES: usize = 10;
const REDACTED_DEFAULT: &str = "<omitted in default mode>";

#[derive(Clone)]
pub struct McpLogger {
    inner: Arc<Mutex<LoggerInner>>,
}

struct LoggerInner {
    log_file_path: PathBuf,
    file: File,
    max_bytes: u64,
    max_files: usize,
    console_enabled: bool,
    mode: LogMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogMode {
    Deep,
    Default,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConsoleTarget {
    Stdout,
    Stderr,
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
        let console_enabled = env::var("MCP_LOG_STDOUT")
            .ok()
            .map(|v| parse_bool(&v))
            .unwrap_or(true);
        let mode = env::var("MCP_LOG_MODE")
            .ok()
            .and_then(|v| LogMode::parse(&v))
            .unwrap_or(LogMode::Default);

        Self::new(
            PathBuf::from(log_dir),
            max_bytes,
            max_files,
            console_enabled,
            mode,
        )
    }

    fn new(
        log_dir: PathBuf,
        max_bytes: u64,
        max_files: usize,
        console_enabled: bool,
        mode: LogMode,
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
                console_enabled,
                mode,
            })),
        })
    }

    pub fn install_panic_hook(&self) {
        let logger = self.clone();
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            logger.log_backtrace(
                LogEvent {
                    level: "error",
                    transport: "system",
                    direction: "lifecycle",
                    request_id: None,
                    jsonrpc_id: None,
                    method: None,
                    path: None,
                    status: Some("panic"),
                    duration_ms: None,
                    payload: None,
                },
                &panic_message(info),
            );
            previous(info);
        }));
    }

    pub fn log(&self, event: LogEvent<'_>) {
        let rendered = {
            let guard = match self.inner.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            render_event(&event, guard.mode)
        };

        let Some(rendered) = rendered else {
            return;
        };

        self.write_rendered(console_target(event.transport), &rendered);
    }

    pub fn log_backtrace(&self, event: LogEvent<'_>, message: &str) {
        let payload = json!({
            "error": message,
            "backtrace": Backtrace::force_capture().to_string(),
        });
        self.log(LogEvent {
            level: "error",
            transport: event.transport,
            direction: event.direction,
            request_id: event.request_id,
            jsonrpc_id: event.jsonrpc_id,
            method: event.method,
            path: event.path,
            status: event.status,
            duration_ms: event.duration_ms,
            payload: Some(&payload),
        });
    }

    fn write_rendered(&self, console_target: ConsoleTarget, rendered: &str) {
        let mut guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };

        let incoming_len = rendered.len() as u64 + 2;
        if rotate_if_needed(&mut guard, incoming_len).is_err() {
            return;
        }

        if guard.file.write_all(rendered.as_bytes()).is_err() {
            return;
        }
        if guard.file.write_all(b"\n\n").is_err() {
            return;
        }
        let _ = guard.file.flush();

        if guard.console_enabled {
            match console_target {
                ConsoleTarget::Stdout => {
                    let mut out = io::stdout().lock();
                    let _ = out.write_all(rendered.as_bytes());
                    let _ = out.write_all(b"\n\n");
                    let _ = out.flush();
                }
                ConsoleTarget::Stderr => {
                    let mut err = io::stderr().lock();
                    let _ = err.write_all(rendered.as_bytes());
                    let _ = err.write_all(b"\n\n");
                    let _ = err.flush();
                }
            }
        }
    }
}

impl LogMode {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "deep" => Some(Self::Deep),
            "default" => Some(Self::Default),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

fn render_event(event: &LogEvent<'_>, mode: LogMode) -> Option<String> {
    if !should_emit_event(event, mode) {
        return None;
    }

    let mut lines = Vec::new();
    lines.push(event_title(event).to_string());
    push_meta(&mut lines, "ts", &now_epoch_ms().to_string());
    push_meta(&mut lines, "level", event.level);
    push_meta(&mut lines, "transport", event.transport);
    push_meta(&mut lines, "direction", event.direction);
    push_meta_opt(&mut lines, "method", event.method);
    push_meta_opt(&mut lines, "path", event.path);
    push_meta_opt(&mut lines, "status", event.status);
    push_meta_opt(&mut lines, "request_id", event.request_id);
    push_meta_value(&mut lines, "jsonrpc_id", event.jsonrpc_id);
    if let Some(duration_ms) = event.duration_ms {
        push_meta(&mut lines, "duration_ms", &duration_ms.to_string());
    }

    if let Some(body) = render_body_value(event, mode) {
        lines.push("body:".to_string());
        render_body_lines(&mut lines, &body, 1);
    }

    Some(lines.join("\n"))
}

fn should_emit_event(event: &LogEvent<'_>, mode: LogMode) -> bool {
    match mode {
        LogMode::Deep | LogMode::Default => true,
        LogMode::System => {
            event.level == "error"
                || event.level == "warn"
                || event.direction == "lifecycle"
                || event.path == Some("/health")
        }
    }
}

fn render_body_value(event: &LogEvent<'_>, mode: LogMode) -> Option<Value> {
    let payload = event.payload?;
    if mode == LogMode::System
        && event.level != "error"
        && event.level != "warn"
        && event.direction != "lifecycle"
        && event.path != Some("/health")
    {
        return None;
    }
    Some(sanitize_value(None, payload, mode))
}

fn sanitize_value(key: Option<&str>, value: &Value, mode: LogMode) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (child_key, child_value) in map {
                out.insert(
                    child_key.clone(),
                    sanitize_value(Some(child_key), child_value, mode),
                );
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| sanitize_value(None, item, mode))
                .collect(),
        ),
        Value::String(text) => sanitize_string_value(key, text, mode),
        _ => value.clone(),
    }
}

fn sanitize_string_value(key: Option<&str>, text: &str, mode: LogMode) -> Value {
    if should_expand_embedded_json(key)
        && let Ok(parsed) = serde_json::from_str::<Value>(text)
    {
        return sanitize_value(key, &parsed, mode);
    }

    if mode == LogMode::Default && key.is_some_and(is_sensitive_key) {
        return Value::String(REDACTED_DEFAULT.to_string());
    }

    Value::String(text.to_string())
}

fn should_expand_embedded_json(key: Option<&str>) -> bool {
    matches!(key, Some("content" | "text" | "raw"))
}

fn is_sensitive_key(key: &str) -> bool {
    matches!(key, "content" | "text" | "raw")
}

fn render_value_lines(lines: &mut Vec<String>, key: Option<&str>, value: &Value, indent: usize) {
    let prefix = "  ".repeat(indent);
    match value {
        Value::Object(map) => {
            if let Some(key) = key {
                lines.push(format!("{prefix}{key}:"));
            }
            if map.is_empty() {
                lines.push(format!("{prefix}  <empty object>"));
                return;
            }
            for (child_key, child_value) in map {
                render_value_lines(lines, Some(child_key), child_value, indent + 1);
            }
        }
        Value::Array(items) => {
            if let Some(key) = key {
                lines.push(format!("{prefix}{key}:"));
            }
            if items.is_empty() {
                lines.push(format!("{prefix}  []"));
                return;
            }
            for (idx, item) in items.iter().enumerate() {
                render_value_lines(lines, Some(&format!("[{idx}]")), item, indent + 1);
            }
        }
        Value::String(text) => render_string_lines(lines, key, text, indent),
        Value::Null => {
            if let Some(key) = key {
                lines.push(format!("{prefix}{key}: null"));
            } else {
                lines.push(format!("{prefix}null"));
            }
        }
        other => {
            if let Some(key) = key {
                lines.push(format!("{prefix}{key}: {other}"));
            } else {
                lines.push(format!("{prefix}{other}"));
            }
        }
    }
}

fn render_body_lines(lines: &mut Vec<String>, value: &Value, indent: usize) {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                lines.push(format!("{}<empty object>", "  ".repeat(indent)));
                return;
            }
            for (child_key, child_value) in map {
                render_value_lines(lines, Some(child_key), child_value, indent);
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                lines.push(format!("{}[]", "  ".repeat(indent)));
                return;
            }
            for (idx, item) in items.iter().enumerate() {
                render_value_lines(lines, Some(&format!("[{idx}]")), item, indent);
            }
        }
        _ => render_value_lines(lines, None, value, indent),
    }
}

fn render_string_lines(lines: &mut Vec<String>, key: Option<&str>, text: &str, indent: usize) {
    let prefix = "  ".repeat(indent);
    if text.contains('\n') {
        if let Some(key) = key {
            lines.push(format!("{prefix}{key}:"));
        }
        for line in text.lines() {
            lines.push(format!("{prefix}  {line}"));
        }
        if text.ends_with('\n') {
            lines.push(format!("{prefix}  "));
        }
    } else if let Some(key) = key {
        lines.push(format!("{prefix}{key}: {text}"));
    } else {
        lines.push(format!("{prefix}{text}"));
    }
}

fn event_title(event: &LogEvent<'_>) -> &'static str {
    if event.level == "error" {
        "ERROR"
    } else {
        match event.direction {
            "request" => "REQUEST",
            "response" => "RESPONSE",
            "lifecycle" => "SYSTEM",
            _ => "EVENT",
        }
    }
}

fn push_meta(lines: &mut Vec<String>, key: &str, value: &str) {
    lines.push(format!("{key}: {value}"));
}

fn push_meta_opt(lines: &mut Vec<String>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        push_meta(lines, key, value);
    }
}

fn push_meta_value(lines: &mut Vec<String>, key: &str, value: Option<&Value>) {
    if let Some(value) = value {
        push_meta(lines, key, &json_value_to_string(value));
    }
}

fn json_value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_string(),
        _ => value.to_string(),
    }
}

fn console_target(transport: &str) -> ConsoleTarget {
    if transport.starts_with("http") {
        ConsoleTarget::Stdout
    } else {
        ConsoleTarget::Stderr
    }
}

fn panic_message(info: &PanicHookInfo<'_>) -> String {
    let location = info
        .location()
        .map(|location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        })
        .unwrap_or_else(|| "<unknown>".to_string());

    let payload = if let Some(message) = info.payload().downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = info.payload().downcast_ref::<String>() {
        message.clone()
    } else {
        "panic without string payload".to_string()
    };

    format!("panic at {location}: {payload}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_request_in_key_value_style() {
        let payload = json!({"jsonrpc":"2.0","method":"tools/list"});
        let rendered = render_event(
            &LogEvent {
                level: "info",
                transport: "http-mcp",
                direction: "request",
                request_id: Some("req-1"),
                jsonrpc_id: Some(&json!(1)),
                method: Some("tools/list"),
                path: Some("/mcp"),
                status: Some("received"),
                duration_ms: None,
                payload: Some(&payload),
            },
            LogMode::Default,
        )
        .expect("rendered");

        assert!(rendered.contains("REQUEST"), "{rendered}");
        assert!(rendered.contains("method: tools/list"), "{rendered}");
        assert!(rendered.contains("body:"), "{rendered}");
        assert!(rendered.contains("jsonrpc: 2.0"), "{rendered}");
    }

    #[test]
    fn default_mode_redacts_file_contents_but_keeps_structure() {
        let payload = json!({
            "result": {
                "content": [{
                    "type": "text",
                    "text": "{\"files\":[{\"path\":\"a.py\",\"content\":\"print(1)\"}]}"
                }]
            }
        });

        let rendered = render_event(
            &LogEvent {
                level: "info",
                transport: "stdio",
                direction: "response",
                request_id: Some("req-2"),
                jsonrpc_id: Some(&json!(2)),
                method: Some("tools/call"),
                path: None,
                status: Some("ok"),
                duration_ms: Some(4),
                payload: Some(&payload),
            },
            LogMode::Default,
        )
        .expect("rendered");

        assert!(rendered.contains("files:"), "{rendered}");
        assert!(rendered.contains("path: a.py"), "{rendered}");
        assert!(rendered.contains(REDACTED_DEFAULT), "{rendered}");
        assert!(!rendered.contains("print(1)"), "{rendered}");
    }

    #[test]
    fn deep_mode_keeps_file_contents() {
        let payload = json!({
            "result": {
                "content": [{
                    "type": "text",
                    "text": "{\"files\":[{\"path\":\"a.py\",\"content\":\"print(1)\"}]}"
                }]
            }
        });

        let rendered = render_event(
            &LogEvent {
                level: "info",
                transport: "stdio",
                direction: "response",
                request_id: Some("req-3"),
                jsonrpc_id: Some(&json!(3)),
                method: Some("tools/call"),
                path: None,
                status: Some("ok"),
                duration_ms: Some(5),
                payload: Some(&payload),
            },
            LogMode::Deep,
        )
        .expect("rendered");

        assert!(rendered.contains("print(1)"), "{rendered}");
    }

    #[test]
    fn system_mode_suppresses_regular_request_bodies() {
        let payload = json!({"jsonrpc":"2.0","method":"tools/list"});
        let rendered = render_event(
            &LogEvent {
                level: "info",
                transport: "http-mcp",
                direction: "request",
                request_id: Some("req-4"),
                jsonrpc_id: Some(&json!(4)),
                method: Some("tools/list"),
                path: Some("/mcp"),
                status: Some("received"),
                duration_ms: None,
                payload: Some(&payload),
            },
            LogMode::System,
        );

        assert!(rendered.is_none(), "{rendered:?}");
    }

    #[test]
    fn system_mode_keeps_error_body_with_backtrace() {
        let payload = json!({"error":"boom","backtrace":"stack line 1\nstack line 2"});
        let rendered = render_event(
            &LogEvent {
                level: "error",
                transport: "http",
                direction: "response",
                request_id: Some("req-5"),
                jsonrpc_id: Some(&json!(5)),
                method: Some("tools/call"),
                path: Some("/mcp"),
                status: Some("error"),
                duration_ms: Some(7),
                payload: Some(&payload),
            },
            LogMode::System,
        )
        .expect("rendered");

        assert!(rendered.contains("ERROR"), "{rendered}");
        assert!(rendered.contains("backtrace:"), "{rendered}");
        assert!(rendered.contains("stack line 1"), "{rendered}");
    }
}
