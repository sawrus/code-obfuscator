#![allow(dead_code)]

use std::backtrace::Backtrace;
#[path = "../error.rs"]
mod error;
#[path = "../fs_ops.rs"]
mod fs_ops;
#[path = "../language.rs"]
mod language;
#[path = "../mapping.rs"]
mod mapping;
#[path = "../mcp_logging.rs"]
mod mcp_logging;
#[path = "../obfuscator.rs"]
mod obfuscator;

use std::collections::{BTreeMap, hash_map::DefaultHasher};
use std::env;
use std::fmt::Display;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use error::{AppError, AppResult};
use fs_ops::{FileEntry, RootGitignore};
use mapping::{MappingFile, detect_terms, enrich_with_random, invert};
use mcp_logging::{LogEvent, McpLogger};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use walkdir::WalkDir;

const MAX_FILES_PER_PROJECT: usize = 1_000_000;
const DEFAULT_TREE_MAX_DEPTH: usize = 6;
const DEFAULT_TREE_MAX_ENTRIES: usize = 1_000;
const MAX_TREE_MAX_DEPTH: usize = 32;
const MAX_TREE_MAX_ENTRIES: usize = 20_000;
const MAX_HTTP_BODY_BYTES: usize = 64 * 1024 * 1024;
const MAX_REQUEST_ID_LENGTH: usize = 256;
const REQUEST_MAPPING_TTL_SECONDS: u64 = 24 * 60 * 60;
const REQUEST_MAPPING_MAX_ENTRIES: usize = 256;

#[derive(Clone)]
struct AppState {
    default_mapping: MappingStore,
    request_mappings: RequestMappingStore,
    logger: McpLogger,
}

#[derive(Clone)]
struct MappingStore {
    inner: Arc<RwLock<BTreeMap<String, String>>>,
    path: Option<PathBuf>,
}

#[derive(Clone)]
struct RequestMappingStore {
    inner: Arc<RwLock<BTreeMap<String, StoredRequestMapping>>>,
    ttl_seconds: u64,
    max_entries: usize,
}

#[derive(Clone)]
struct StoredRequestMapping {
    stored_at_epoch_s: u64,
    mapping: MappingPayload,
    root_dir: Option<String>,
    workspace_dir: Option<String>,
    baseline_hashes: BTreeMap<String, u64>,
}

impl MappingStore {
    fn new(path: Option<PathBuf>) -> Self {
        let mapping = path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|raw| serde_json::from_str::<BTreeMap<String, String>>(&raw).ok())
            .unwrap_or_default();
        Self {
            inner: Arc::new(RwLock::new(mapping)),
            path,
        }
    }

    fn get(&self) -> BTreeMap<String, String> {
        self.inner.read().map(|m| m.clone()).unwrap_or_default()
    }

    fn set(&self, mapping: BTreeMap<String, String>) -> AppResult<()> {
        {
            let mut guard = self
                .inner
                .write()
                .map_err(|_| AppError::InvalidArg("mapping store lock poisoned".into()))?;
            *guard = mapping.clone();
        }
        if let Some(path) = &self.path {
            let text = serde_json::to_string_pretty(&mapping)?;
            fs::write(path, text)?;
        }
        Ok(())
    }
}

impl RequestMappingStore {
    fn new(ttl_seconds: u64, max_entries: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(BTreeMap::new())),
            ttl_seconds,
            max_entries,
        }
    }

    fn insert(&self, request_id: String, mapping: MappingPayload) -> AppResult<()> {
        let now = now_epoch_s();
        let mut guard = self
            .inner
            .write()
            .map_err(|_| AppError::InvalidArg("request mapping store lock poisoned".into()))?;
        self.cleanup_locked(&mut guard, now);
        guard.insert(
            request_id,
            StoredRequestMapping {
                stored_at_epoch_s: now,
                mapping,
                root_dir: None,
                workspace_dir: None,
                baseline_hashes: BTreeMap::new(),
            },
        );
        self.enforce_max_entries(&mut guard);
        Ok(())
    }

    fn resolve(&self, request_id: &str) -> AppResult<MappingPayload> {
        Ok(self.resolve_session(request_id)?.mapping)
    }

    fn resolve_session(&self, request_id: &str) -> AppResult<StoredRequestMapping> {
        let now = now_epoch_s();
        let mut guard = self
            .inner
            .write()
            .map_err(|_| AppError::InvalidArg("request mapping store lock poisoned".into()))?;
        self.cleanup_locked(&mut guard, now);
        guard
            .get(request_id)
            .cloned()
            .ok_or_else(|| AppError::InvalidArg(format!("unknown request_id: {request_id}")))
    }

    fn update_context(
        &self,
        request_id: &str,
        root_dir: &Path,
        workspace_dir: Option<&Path>,
        baseline_hashes: BTreeMap<String, u64>,
    ) -> AppResult<StoredRequestMapping> {
        let now = now_epoch_s();
        let mut guard = self
            .inner
            .write()
            .map_err(|_| AppError::InvalidArg("request mapping store lock poisoned".into()))?;
        self.cleanup_locked(&mut guard, now);
        let stored = guard
            .get_mut(request_id)
            .ok_or_else(|| AppError::InvalidArg(format!("unknown request_id: {request_id}")))?;
        stored.root_dir = Some(root_dir.to_string_lossy().to_string());
        if let Some(workspace_dir) = workspace_dir {
            stored.workspace_dir = Some(workspace_dir.to_string_lossy().to_string());
        }
        stored.baseline_hashes = baseline_hashes;
        Ok(stored.clone())
    }

    fn bind_workspace_dir(
        &self,
        request_id: &str,
        workspace_dir: &Path,
    ) -> AppResult<StoredRequestMapping> {
        let now = now_epoch_s();
        let mut guard = self
            .inner
            .write()
            .map_err(|_| AppError::InvalidArg("request mapping store lock poisoned".into()))?;
        self.cleanup_locked(&mut guard, now);
        let stored = guard
            .get_mut(request_id)
            .ok_or_else(|| AppError::InvalidArg(format!("unknown request_id: {request_id}")))?;
        stored.workspace_dir = Some(workspace_dir.to_string_lossy().to_string());
        Ok(stored.clone())
    }

    fn refresh_snapshot(
        &self,
        request_id: &str,
        baseline_hashes: BTreeMap<String, u64>,
    ) -> AppResult<()> {
        let now = now_epoch_s();
        let mut guard = self
            .inner
            .write()
            .map_err(|_| AppError::InvalidArg("request mapping store lock poisoned".into()))?;
        self.cleanup_locked(&mut guard, now);
        let stored = guard
            .get_mut(request_id)
            .ok_or_else(|| AppError::InvalidArg(format!("unknown request_id: {request_id}")))?;
        stored.baseline_hashes = baseline_hashes;
        Ok(())
    }

    fn cleanup_locked(&self, guard: &mut BTreeMap<String, StoredRequestMapping>, now: u64) {
        if self.ttl_seconds > 0 {
            guard.retain(|_, stored| {
                now.saturating_sub(stored.stored_at_epoch_s) <= self.ttl_seconds
            });
        }
        self.enforce_max_entries(guard);
    }

    fn enforce_max_entries(&self, guard: &mut BTreeMap<String, StoredRequestMapping>) {
        if self.max_entries == 0 {
            guard.clear();
            return;
        }

        while guard.len() > self.max_entries {
            let Some(oldest_key) = guard
                .iter()
                .min_by_key(|(request_id, stored)| {
                    (stored.stored_at_epoch_s, (*request_id).clone())
                })
                .map(|(request_id, _)| request_id.clone())
            else {
                break;
            };
            guard.remove(&oldest_key);
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcRequest {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StdioFraming {
    ContentLength,
    JsonStream,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectFile {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PullArgs {
    root_dir: String,
    #[serde(default)]
    file_paths: Vec<String>,
    #[serde(default)]
    options: ToolOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloneArgs {
    root_dir: String,
    workspace_dir: String,
    #[serde(default)]
    options: ToolOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusArgs {
    workspace_dir: String,
    #[serde(default)]
    options: ToolOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PushArgs {
    workspace_dir: String,
    #[serde(default)]
    options: ToolOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PullComputationArgs {
    project_files: Vec<ProjectFile>,
    #[serde(default)]
    options: ToolOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LsTreeArgs {
    root_dir: String,
    max_depth: Option<usize>,
    max_entries: Option<usize>,
    include_hidden: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LsFilesArgs {
    root_dir: String,
    max_entries: Option<usize>,
    include_hidden: Option<bool>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolOptions {
    request_id: Option<String>,
    stream: Option<bool>,
    enrich_detected_terms: Option<bool>,
    security: Option<SecurityOptions>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecurityOptions {
    sign_mapping: Option<bool>,
    ttl_seconds: Option<u64>,
    encrypt_mapping: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MappingPayload {
    mapping: MappingFile,
    created_at_epoch_s: u64,
    expires_at_epoch_s: Option<u64>,
    signature: Option<String>,
    encryption: Option<String>,
    #[serde(default)]
    metadata: Option<MappingMetadata>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct MappingMetadata {
    #[serde(default)]
    original_paths: Vec<String>,
    #[serde(default)]
    file_tokens: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Serialize)]
struct PullResult {
    request_id: String,
    obfuscated_files: Vec<ProjectFile>,
    stats: Stats,
    events: Vec<StageEvent>,
}

#[derive(Debug, Serialize)]
struct PushResult {
    request_id: String,
    applied_files: Vec<String>,
    deleted_files: Vec<String>,
    stats: PushStats,
    events: Vec<StageEvent>,
}

#[derive(Debug, Serialize)]
struct CloneResult {
    request_id: String,
    workspace_dir: String,
    cloned_files: Vec<String>,
    stats: Stats,
    events: Vec<StageEvent>,
}

#[derive(Debug, Serialize)]
struct PushStats {
    applied_count: usize,
    deleted_count: usize,
    mapping_entries: usize,
}

#[derive(Debug, Serialize)]
struct Stats {
    file_count: usize,
    mapping_entries: usize,
}

#[derive(Debug, Serialize)]
struct StageEvent {
    stage: &'static str,
    timestamp_epoch_s: u64,
}

#[derive(Debug, Serialize)]
struct TreeEntry {
    path: String,
    kind: &'static str,
}

#[derive(Debug, Serialize)]
struct ProjectTreeResult {
    root_dir: String,
    entries: Vec<TreeEntry>,
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct LsFilesResult {
    root_dir: String,
    files: Vec<String>,
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct StatusResult {
    request_id: String,
    workspace_dir: String,
    clean: bool,
    diff: WorkspaceDiff,
    mapping_state: MappingState,
}

#[derive(Debug, Serialize)]
struct WorkspaceDiff {
    added: Vec<String>,
    modified: Vec<String>,
    deleted: Vec<String>,
}

#[derive(Debug, Serialize)]
struct MappingState {
    stored_at_epoch_s: u64,
    expires_at_epoch_s: Option<u64>,
    mapping_entries: usize,
    tracked_files: usize,
    root_dir: String,
    workspace_dir: String,
}

fn format_fatal_error(context: &str, err: &dyn Display) -> String {
    format!(
        "{context}: {err}\nbacktrace:\n{}",
        Backtrace::force_capture()
    )
}

fn log_error_with_backtrace(logger: &McpLogger, event: LogEvent<'_>, err: &dyn Display) {
    let message = err.to_string();
    logger.log_backtrace(event, &message);
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{}", format_fatal_error("fatal mcp server error", &err));
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let logger = McpLogger::from_env()?;
    logger.install_panic_hook();
    let mapping_path = env::var("MCP_DEFAULT_MAPPING_PATH").ok().map(PathBuf::from);
    let http_addr = env::var("MCP_HTTP_ADDR")
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty());
    let disable_stdio = parse_env_bool("MCP_DISABLE_STDIO").unwrap_or(false);
    let state = AppState {
        default_mapping: MappingStore::new(mapping_path),
        request_mappings: RequestMappingStore::new(
            REQUEST_MAPPING_TTL_SECONDS,
            REQUEST_MAPPING_MAX_ENTRIES,
        ),
        logger,
    };

    if disable_stdio && http_addr.is_none() {
        return Err(AppError::InvalidArg(
            "MCP_DISABLE_STDIO requires MCP_HTTP_ADDR".into(),
        ));
    }

    if !disable_stdio {
        state.logger.log(LogEvent {
            level: "info",
            transport: "stdio",
            direction: "lifecycle",
            request_id: None,
            jsonrpc_id: None,
            method: None,
            path: None,
            status: Some("ready"),
            duration_ms: None,
            payload: None,
        });
    }

    if let Some(addr) = http_addr {
        state.logger.log(LogEvent {
            level: "info",
            transport: "http",
            direction: "lifecycle",
            request_id: None,
            jsonrpc_id: None,
            method: None,
            path: Some("/"),
            status: Some("listening"),
            duration_ms: None,
            payload: Some(&json!({ "addr": addr })),
        });

        if disable_stdio {
            let http_state = state.clone();
            let http_logger = state.logger.clone();
            return run_http_api(&addr, http_state).inspect_err(|err| {
                log_error_with_backtrace(
                    &http_logger,
                    LogEvent {
                        level: "error",
                        transport: "http",
                        direction: "lifecycle",
                        request_id: None,
                        jsonrpc_id: None,
                        method: None,
                        path: Some("/"),
                        status: Some("fatal"),
                        duration_ms: None,
                        payload: None,
                    },
                    &err,
                );
            });
        }

        let state_for_http = state.clone();
        let http_logger = state.logger.clone();
        thread::spawn(move || {
            if let Err(err) = run_http_api(&addr, state_for_http) {
                log_error_with_backtrace(
                    &http_logger,
                    LogEvent {
                        level: "error",
                        transport: "http",
                        direction: "lifecycle",
                        request_id: None,
                        jsonrpc_id: None,
                        method: None,
                        path: Some("/"),
                        status: Some("fatal"),
                        duration_ms: None,
                        payload: None,
                    },
                    &err,
                );
                eprintln!("{}", format_fatal_error("http api error", &err));
            }
        });
    }

    let stdio_state = state.clone();
    match run_stdio_mcp(state) {
        Ok(()) => Ok(()),
        Err(err) => {
            log_error_with_backtrace(
                &stdio_state.logger,
                LogEvent {
                    level: "error",
                    transport: "stdio",
                    direction: "lifecycle",
                    request_id: None,
                    jsonrpc_id: None,
                    method: None,
                    path: None,
                    status: Some("fatal"),
                    duration_ms: None,
                    payload: None,
                },
                &err,
            );
            Err(err)
        }
    }
}

fn run_stdio_mcp(state: AppState) -> AppResult<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut out = stdout.lock();
    let mut framing: Option<StdioFraming> = None;

    while let Some((req, detected_framing)) = read_message(&mut reader, framing)? {
        if framing.is_none() {
            framing = Some(detected_framing);
        }
        let started = Instant::now();
        let request_payload = serde_json::to_value(&req).ok();
        let request_id = extract_request_id(&req);
        let method = req.method.clone();
        let id_opt = req.id.clone();

        state.logger.log(LogEvent {
            level: "info",
            transport: "stdio",
            direction: "request",
            request_id: request_id.as_deref(),
            jsonrpc_id: id_opt.as_ref(),
            method: Some(&method),
            path: None,
            status: Some("received"),
            duration_ms: None,
            payload: request_payload.as_ref(),
        });

        if id_opt.is_none() {
            let status = match handle_request(req, &state) {
                Ok(_) => "notification_ok",
                Err(err) => {
                    log_error_with_backtrace(
                        &state.logger,
                        LogEvent {
                            level: "error",
                            transport: "stdio",
                            direction: "response",
                            request_id: request_id.as_deref(),
                            jsonrpc_id: None,
                            method: Some(&method),
                            path: None,
                            status: Some("notification_error"),
                            duration_ms: Some(started.elapsed().as_millis()),
                            payload: None,
                        },
                        &err,
                    );
                    "notification_error"
                }
            };
            state.logger.log(LogEvent {
                level: "info",
                transport: "stdio",
                direction: "response",
                request_id: request_id.as_deref(),
                jsonrpc_id: None,
                method: Some(&method),
                path: None,
                status: Some(status),
                duration_ms: Some(started.elapsed().as_millis()),
                payload: None,
            });
            continue;
        }

        let id = id_opt.unwrap_or(Value::Null);
        let response = match handle_request(req, &state) {
            Ok(result) => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(result),
                error: None,
            },
            Err(err) => {
                log_error_with_backtrace(
                    &state.logger,
                    LogEvent {
                        level: "error",
                        transport: "stdio",
                        direction: "response",
                        request_id: request_id.as_deref(),
                        jsonrpc_id: Some(&id),
                        method: Some(&method),
                        path: None,
                        status: Some("error"),
                        duration_ms: Some(started.elapsed().as_millis()),
                        payload: None,
                    },
                    &err,
                );
                JsonRpcResponse {
                    jsonrpc: "2.0",
                    id,
                    result: None,
                    error: Some(json!({"code": -32000, "message": err.to_string()})),
                }
            }
        };

        let response_payload = serde_json::to_value(&response).ok();
        let status = if response.error.is_some() {
            "error"
        } else {
            "ok"
        };
        state.logger.log(LogEvent {
            level: "info",
            transport: "stdio",
            direction: "response",
            request_id: request_id.as_deref(),
            jsonrpc_id: Some(&response.id),
            method: Some(&method),
            path: None,
            status: Some(status),
            duration_ms: Some(started.elapsed().as_millis()),
            payload: response_payload.as_ref(),
        });

        write_message(
            &mut out,
            &response,
            framing.unwrap_or(StdioFraming::ContentLength),
        )?;
    }

    state.logger.log(LogEvent {
        level: "info",
        transport: "stdio",
        direction: "lifecycle",
        request_id: None,
        jsonrpc_id: None,
        method: None,
        path: None,
        status: Some("shutdown"),
        duration_ms: None,
        payload: None,
    });

    Ok(())
}

fn run_http_api(addr: &str, state: AppState) -> AppResult<()> {
    let listener = TcpListener::bind(addr)?;
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(err) => {
                log_error_with_backtrace(
                    &state.logger,
                    LogEvent {
                        level: "error",
                        transport: "http",
                        direction: "lifecycle",
                        request_id: None,
                        jsonrpc_id: None,
                        method: None,
                        path: None,
                        status: Some("accept_failed"),
                        duration_ms: None,
                        payload: None,
                    },
                    &err,
                );
                continue;
            }
        };
        if let Err(err) = handle_http_connection(&mut stream, &state) {
            log_error_with_backtrace(
                &state.logger,
                LogEvent {
                    level: "error",
                    transport: "http",
                    direction: "response",
                    request_id: None,
                    jsonrpc_id: None,
                    method: None,
                    path: None,
                    status: Some("connection_error"),
                    duration_ms: None,
                    payload: None,
                },
                &err,
            );
        }
    }
    Ok(())
}

fn handle_http_connection(stream: &mut TcpStream, state: &AppState) -> AppResult<()> {
    let started = Instant::now();
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut first = String::new();
    if reader.read_line(&mut first)? == 0 {
        return Ok(());
    }
    let parts: Vec<&str> = first.split_whitespace().collect();
    if parts.len() < 2 {
        let err = AppError::InvalidArg("bad request".into());
        log_error_with_backtrace(
            &state.logger,
            LogEvent {
                level: "error",
                transport: "http-admin",
                direction: "response",
                request_id: None,
                jsonrpc_id: None,
                method: None,
                path: None,
                status: Some("bad_request"),
                duration_ms: Some(started.elapsed().as_millis()),
                payload: None,
            },
            &err,
        );
        write_http_json(stream, 400, &json!({"error":"bad request"}))?;
        state.logger.log(LogEvent {
            level: "warn",
            transport: "http-admin",
            direction: "response",
            request_id: None,
            jsonrpc_id: None,
            method: None,
            path: None,
            status: Some("bad_request"),
            duration_ms: Some(started.elapsed().as_millis()),
            payload: Some(&json!({"error":"bad request"})),
        });
        return Ok(());
    }
    let method = parts[0];
    let path = parts[1];
    let transport = if path == "/" || path == "/mcp" {
        "http-mcp"
    } else {
        "http-admin"
    };

    let mut content_length = 0_usize;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let t = line.trim_end();
        if t.is_empty() {
            break;
        }
        if let Some(v) = t
            .to_ascii_lowercase()
            .strip_prefix("content-length:")
            .map(str::to_string)
        {
            content_length = v.trim().parse::<usize>().unwrap_or(0);
        }
    }

    if content_length > MAX_HTTP_BODY_BYTES {
        let err = AppError::InvalidArg(format!(
            "request body exceeds max size: {content_length} > {MAX_HTTP_BODY_BYTES}"
        ));
        log_error_with_backtrace(
            &state.logger,
            LogEvent {
                level: "error",
                transport,
                direction: "response",
                request_id: None,
                jsonrpc_id: None,
                method: Some(method),
                path: Some(path),
                status: Some("bad_request"),
                duration_ms: Some(started.elapsed().as_millis()),
                payload: None,
            },
            &err,
        );
        write_http_json(
            stream,
            400,
            &json!({
                "error": err.to_string()
            }),
        )?;
        return Ok(());
    }

    let mut body = vec![0_u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    let body_text = String::from_utf8(body).unwrap_or_default();
    let body_json = serde_json::from_str::<Value>(&body_text).ok();
    let fallback_payload = if body_text.is_empty() {
        None
    } else {
        Some(json!({"raw": body_text.clone()}))
    };
    let request_payload_ref = body_json.as_ref().or(fallback_payload.as_ref());

    state.logger.log(LogEvent {
        level: "info",
        transport,
        direction: "request",
        request_id: body_json
            .as_ref()
            .and_then(extract_request_id_from_value)
            .as_deref(),
        jsonrpc_id: body_json.as_ref().and_then(|v| v.get("id")),
        method: body_json
            .as_ref()
            .and_then(|v| v.get("method"))
            .and_then(Value::as_str)
            .or(Some(method)),
        path: Some(path),
        status: Some("received"),
        duration_ms: None,
        payload: request_payload_ref,
    });

    match (method, path) {
        ("GET", "/health") => {
            let response = json!({"ok":true});
            write_http_json(stream, 200, &response)?;
            state.logger.log(LogEvent {
                level: "info",
                transport,
                direction: "response",
                request_id: None,
                jsonrpc_id: None,
                method: Some(method),
                path: Some(path),
                status: Some("ok"),
                duration_ms: Some(started.elapsed().as_millis()),
                payload: Some(&response),
            });
            Ok(())
        }
        ("GET", "/mapping") => {
            let response = json!({"mapping": state.default_mapping.get()});
            write_http_json(stream, 200, &response)?;
            state.logger.log(LogEvent {
                level: "info",
                transport,
                direction: "response",
                request_id: None,
                jsonrpc_id: None,
                method: Some(method),
                path: Some(path),
                status: Some("ok"),
                duration_ms: Some(started.elapsed().as_millis()),
                payload: Some(&response),
            });
            Ok(())
        }
        ("PUT", "/mapping") => {
            match parse_mapping_update(&body_text).and_then(|m| {
                state.default_mapping.set(m.clone())?;
                Ok(m)
            }) {
                Ok(map) => {
                    let response = json!({"mapping":map});
                    write_http_json(stream, 200, &response)?;
                    state.logger.log(LogEvent {
                        level: "info",
                        transport,
                        direction: "response",
                        request_id: None,
                        jsonrpc_id: None,
                        method: Some(method),
                        path: Some(path),
                        status: Some("ok"),
                        duration_ms: Some(started.elapsed().as_millis()),
                        payload: Some(&response),
                    });
                    Ok(())
                }
                Err(err) => {
                    log_error_with_backtrace(
                        &state.logger,
                        LogEvent {
                            level: "error",
                            transport,
                            direction: "response",
                            request_id: None,
                            jsonrpc_id: None,
                            method: Some(method),
                            path: Some(path),
                            status: Some("error"),
                            duration_ms: Some(started.elapsed().as_millis()),
                            payload: None,
                        },
                        &err,
                    );
                    let response = json!({"error":err.to_string()});
                    write_http_json(stream, 400, &response)?;
                    state.logger.log(LogEvent {
                        level: "warn",
                        transport,
                        direction: "response",
                        request_id: None,
                        jsonrpc_id: None,
                        method: Some(method),
                        path: Some(path),
                        status: Some("error"),
                        duration_ms: Some(started.elapsed().as_millis()),
                        payload: Some(&response),
                    });
                    Ok(())
                }
            }
        }
        ("POST", "/") | ("POST", "/mcp") => {
            let req = serde_json::from_str::<JsonRpcRequest>(&body_text)
                .map_err(|_| AppError::InvalidArg("invalid JSON-RPC request".into()));

            let req = match req {
                Ok(req) => req,
                Err(err) => {
                    log_error_with_backtrace(
                        &state.logger,
                        LogEvent {
                            level: "error",
                            transport,
                            direction: "response",
                            request_id: None,
                            jsonrpc_id: None,
                            method: Some(method),
                            path: Some(path),
                            status: Some("bad_request"),
                            duration_ms: Some(started.elapsed().as_millis()),
                            payload: None,
                        },
                        &err,
                    );
                    let response = json!({"error": err.to_string()});
                    write_http_json(stream, 400, &response)?;
                    state.logger.log(LogEvent {
                        level: "warn",
                        transport,
                        direction: "response",
                        request_id: None,
                        jsonrpc_id: None,
                        method: Some(method),
                        path: Some(path),
                        status: Some("bad_request"),
                        duration_ms: Some(started.elapsed().as_millis()),
                        payload: Some(&response),
                    });
                    return Ok(());
                }
            };

            let request_id = extract_request_id(&req);
            let request_method = req.method.clone();
            let jsonrpc_id = req.id.clone();

            if jsonrpc_id.is_none() {
                let status = match handle_request(req, state) {
                    Ok(_) => "notification_ok",
                    Err(err) => {
                        log_error_with_backtrace(
                            &state.logger,
                            LogEvent {
                                level: "error",
                                transport,
                                direction: "response",
                                request_id: request_id.as_deref(),
                                jsonrpc_id: None,
                                method: Some(&request_method),
                                path: Some(path),
                                status: Some("notification_error"),
                                duration_ms: Some(started.elapsed().as_millis()),
                                payload: None,
                            },
                            &err,
                        );
                        "notification_error"
                    }
                };
                write_http_no_content(stream, 204)?;
                state.logger.log(LogEvent {
                    level: "info",
                    transport,
                    direction: "response",
                    request_id: request_id.as_deref(),
                    jsonrpc_id: None,
                    method: Some(&request_method),
                    path: Some(path),
                    status: Some(status),
                    duration_ms: Some(started.elapsed().as_millis()),
                    payload: None,
                });
                return Ok(());
            }

            let id = jsonrpc_id.unwrap_or(Value::Null);
            let response = match handle_request(req, state) {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: "2.0",
                    id,
                    result: Some(result),
                    error: None,
                },
                Err(err) => {
                    log_error_with_backtrace(
                        &state.logger,
                        LogEvent {
                            level: "error",
                            transport,
                            direction: "response",
                            request_id: request_id.as_deref(),
                            jsonrpc_id: Some(&id),
                            method: Some(&request_method),
                            path: Some(path),
                            status: Some("error"),
                            duration_ms: Some(started.elapsed().as_millis()),
                            payload: None,
                        },
                        &err,
                    );
                    JsonRpcResponse {
                        jsonrpc: "2.0",
                        id,
                        result: None,
                        error: Some(json!({"code": -32000, "message": err.to_string()})),
                    }
                }
            };

            let response_payload = serde_json::to_value(&response)?;
            write_http_json(stream, 200, &response_payload)?;
            let status = if response.error.is_some() {
                "error"
            } else {
                "ok"
            };
            state.logger.log(LogEvent {
                level: "info",
                transport,
                direction: "response",
                request_id: request_id.as_deref(),
                jsonrpc_id: Some(&response.id),
                method: Some(&request_method),
                path: Some(path),
                status: Some(status),
                duration_ms: Some(started.elapsed().as_millis()),
                payload: Some(&response_payload),
            });
            Ok(())
        }
        _ => {
            let response = json!({"error":"not found"});
            write_http_json(stream, 404, &response)?;
            state.logger.log(LogEvent {
                level: "warn",
                transport,
                direction: "response",
                request_id: None,
                jsonrpc_id: None,
                method: Some(method),
                path: Some(path),
                status: Some("not_found"),
                duration_ms: Some(started.elapsed().as_millis()),
                payload: Some(&response),
            });
            Ok(())
        }
    }
}

fn write_http_json(stream: &mut TcpStream, status: u16, body: &Value) -> AppResult<()> {
    let json = serde_json::to_vec(body)?;
    write_http_bytes(stream, status, "application/json", &json)
}

fn write_http_no_content(stream: &mut TcpStream, status: u16) -> AppResult<()> {
    write_http_bytes(stream, status, "application/json", &[])
}

fn write_http_bytes(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> AppResult<()> {
    let status_text = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    if !body.is_empty() {
        stream.write_all(body)?;
    }
    stream.flush()?;
    Ok(())
}

fn parse_mapping_update(body: &str) -> AppResult<BTreeMap<String, String>> {
    let value: Value = serde_json::from_str(body)?;
    if let Ok(map) = serde_json::from_value::<BTreeMap<String, String>>(value.clone()) {
        return Ok(map);
    }
    if let Some(mapping) = value.get("mapping") {
        return Ok(serde_json::from_value(mapping.clone())?);
    }
    Err(AppError::InvalidArg(
        "expected JSON object or {\"mapping\":{...}}".into(),
    ))
}

fn extract_request_id(req: &JsonRpcRequest) -> Option<String> {
    if req.method == "tools/call"
        && let Some(req_id) = req
            .params
            .get("arguments")
            .and_then(|v| v.get("options"))
            .and_then(|v| v.get("request_id"))
            .and_then(Value::as_str)
    {
        return Some(req_id.to_string());
    }
    req.id.as_ref().map(json_value_to_string)
}

fn extract_request_id_from_value(v: &Value) -> Option<String> {
    if v.get("method").and_then(Value::as_str) == Some("tools/call")
        && let Some(req_id) = v
            .get("params")
            .and_then(|p| p.get("arguments"))
            .and_then(|a| a.get("options"))
            .and_then(|o| o.get("request_id"))
            .and_then(Value::as_str)
    {
        return Some(req_id.to_string());
    }
    v.get("id").map(json_value_to_string)
}

fn json_value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        _ => v.to_string(),
    }
}

fn handle_request(req: JsonRpcRequest, state: &AppState) -> AppResult<Value> {
    match req.method.as_str() {
        "initialize" => {
            let protocol_version = negotiate_protocol_version(&req.params);
            Ok(json!({
                "protocolVersion": protocol_version,
                "serverInfo": {"name": "code-obfuscator-mcp", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {
                    "tools": {"listChanged": false},
                    "resources": {"subscribe": false, "listChanged": false}
                }
            }))
        }
        "resources/list" => Ok(json!({"resources": []})),
        "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
        "tools/list" => Ok(json!({"tools": tools_definitions()})),
        "tools/call" => {
            let params: ToolCallParams = serde_json::from_value(req.params)?;
            call_tool(params, state)
        }
        _ => Ok(json!({})),
    }
}

fn negotiate_protocol_version(params: &Value) -> String {
    if let Some(version) = params.get("protocolVersion").and_then(Value::as_str) {
        return version.to_string();
    }
    if let Some(versions) = params.get("protocolVersions").and_then(Value::as_array)
        && let Some(version) = versions.iter().find_map(Value::as_str)
    {
        return version.to_string();
    }
    "2024-11-05".to_string()
}

fn tools_definitions() -> Vec<Value> {
    vec![
        json!({"name":"ls_tree","description":"List directory tree under root_dir (directories and files) for scoped selection.","inputSchema":{"type":"object","required":["root_dir"],"properties":{"root_dir":{"type":"string"},"max_depth":{"type":"integer","minimum":1},"max_entries":{"type":"integer","minimum":1},"include_hidden":{"type":"boolean"}}}}),
        json!({"name":"ls_files","description":"List files under root_dir as a flat list for quick file targeting.","inputSchema":{"type":"object","required":["root_dir"],"properties":{"root_dir":{"type":"string"},"max_entries":{"type":"integer","minimum":1},"include_hidden":{"type":"boolean"}}}}),
        json!({"name":"pull","description":"Obfuscate selected files from root_dir and return obfuscated payload for workspace materialization.","inputSchema":{"type":"object","required":["root_dir","options"],"properties":{"root_dir":{"type":"string"},"file_paths":{"type":"array","items":{"type":"string"}},"options":{"type":"object","required":["request_id"],"properties":{"request_id":{"type":"string"},"stream":{"type":"boolean"},"enrich_detected_terms":{"type":"boolean"}}}}}}),
        json!({"name":"clone","description":"Obfuscate the full source tree and write the workspace snapshot to workspace_dir for editing.","inputSchema":{"type":"object","required":["root_dir","workspace_dir","options"],"properties":{"root_dir":{"type":"string"},"workspace_dir":{"type":"string"},"options":{"type":"object","required":["request_id"],"properties":{"request_id":{"type":"string"},"stream":{"type":"boolean"},"enrich_detected_terms":{"type":"boolean"}}}}}}),
        json!({"name":"status","description":"Compare current workspace against the stored snapshot for request_id and return added/modified/deleted changes plus context state.","inputSchema":{"type":"object","required":["workspace_dir","options"],"properties":{"workspace_dir":{"type":"string"},"options":{"type":"object","required":["request_id"],"properties":{"request_id":{"type":"string"}}}}}}),
        json!({"name":"push","description":"Apply workspace delta (add/modify/delete) back to source using request_id-bound deobfuscation context.","inputSchema":{"type":"object","required":["workspace_dir","options"],"properties":{"workspace_dir":{"type":"string"},"options":{"type":"object","required":["request_id"],"properties":{"request_id":{"type":"string"},"stream":{"type":"boolean"}}}}}}),
    ]
}

fn call_tool(params: ToolCallParams, state: &AppState) -> AppResult<Value> {
    match params.name.as_str() {
        "ls_tree" => {
            let args: LsTreeArgs = serde_json::from_value(params.arguments)?;
            let result = ls_tree(args)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        "ls_files" => {
            let args: LsFilesArgs = serde_json::from_value(params.arguments)?;
            let result = ls_files(args)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        "pull" => {
            let args: PullArgs = serde_json::from_value(params.arguments)?;
            let result = pull(args, state)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        "clone" => {
            let args: CloneArgs = serde_json::from_value(params.arguments)?;
            let result = clone_workspace(args, state)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        "status" => {
            let args: StatusArgs = serde_json::from_value(params.arguments)?;
            let result = status_workspace(args, state)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        "push" => {
            let args: PushArgs = serde_json::from_value(params.arguments)?;
            let result = push_workspace(args, state)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        _ => Err(AppError::InvalidArg("unknown or disabled tool".into())),
    }
}

fn ls_tree(args: LsTreeArgs) -> AppResult<ProjectTreeResult> {
    let root = resolve_root_dir(&args.root_dir)?;
    let gitignore = RootGitignore::from_root(&root)?;
    let max_depth = args
        .max_depth
        .unwrap_or(DEFAULT_TREE_MAX_DEPTH)
        .min(MAX_TREE_MAX_DEPTH);
    let max_entries = args
        .max_entries
        .unwrap_or(DEFAULT_TREE_MAX_ENTRIES)
        .min(MAX_TREE_MAX_ENTRIES);
    let include_hidden = args.include_hidden.unwrap_or(false);

    let mut entries = Vec::new();
    let mut truncated = false;

    for entry in WalkDir::new(&root)
        .max_depth(max_depth)
        .into_iter()
        .filter_entry(|entry| {
            if entry.path() == root {
                return true;
            }
            let Ok(rel) = entry.path().strip_prefix(&root) else {
                return false;
            };
            !gitignore.is_ignored_rel(rel, entry.file_type().is_dir())
        })
        .flatten()
    {
        let path = entry.path();
        if path == root {
            continue;
        }
        let rel = path
            .strip_prefix(&root)
            .map_err(|_| AppError::InvalidArg("invalid path under root".into()))?;
        if !include_hidden && is_hidden_path(rel) {
            continue;
        }

        let kind = if entry.file_type().is_dir() {
            "dir"
        } else if entry.file_type().is_file() {
            "file"
        } else {
            continue;
        };

        entries.push(TreeEntry {
            path: rel.to_string_lossy().to_string(),
            kind,
        });
        if entries.len() >= max_entries {
            truncated = true;
            break;
        }
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(ProjectTreeResult {
        root_dir: root.to_string_lossy().to_string(),
        entries,
        truncated,
    })
}

fn ls_files(args: LsFilesArgs) -> AppResult<LsFilesResult> {
    let root = resolve_root_dir(&args.root_dir)?;
    let gitignore = RootGitignore::from_root(&root)?;
    let max_entries = args
        .max_entries
        .unwrap_or(DEFAULT_TREE_MAX_ENTRIES)
        .min(MAX_TREE_MAX_ENTRIES);
    let include_hidden = args.include_hidden.unwrap_or(false);

    let mut files = Vec::new();
    let mut truncated = false;

    for entry in WalkDir::new(&root)
        .into_iter()
        .filter_entry(|entry| {
            if entry.path() == root {
                return true;
            }
            let Ok(rel) = entry.path().strip_prefix(&root) else {
                return false;
            };
            !gitignore.is_ignored_rel(rel, entry.file_type().is_dir())
        })
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(&root)
            .map_err(|_| AppError::InvalidArg("invalid path under root".into()))?;
        if !include_hidden && is_hidden_path(rel) {
            continue;
        }
        files.push(rel.to_string_lossy().to_string());
        if files.len() >= max_entries {
            truncated = true;
            break;
        }
    }

    files.sort();
    Ok(LsFilesResult {
        root_dir: root.to_string_lossy().to_string(),
        files,
        truncated,
    })
}

fn pull(args: PullArgs, state: &AppState) -> AppResult<PullResult> {
    let root = resolve_root_dir(&args.root_dir)?;
    let project_files = load_project_files_from_disk(&args.root_dir, &args.file_paths)?;
    let result = compute_pull_payload(
        PullComputationArgs {
            project_files,
            options: args.options,
        },
        state,
    )?;
    state.request_mappings.update_context(
        &result.request_id,
        &root,
        None,
        build_file_hashes(&result.obfuscated_files),
    )?;
    Ok(result)
}

fn clone_workspace(args: CloneArgs, state: &AppState) -> AppResult<CloneResult> {
    let root = resolve_root_dir(&args.root_dir)?;
    let workspace = prepare_workspace_dir(&args.workspace_dir, &root)?;

    let mut pull_result = pull(
        PullArgs {
            root_dir: args.root_dir,
            file_paths: vec![],
            options: args.options,
        },
        state,
    )?;

    record_event(&mut pull_result.events, "cloning");
    write_project_files_to_disk(&workspace, &pull_result.obfuscated_files)?;
    record_event(&mut pull_result.events, "cloned");

    state.request_mappings.update_context(
        &pull_result.request_id,
        &root,
        Some(&workspace),
        build_file_hashes(&pull_result.obfuscated_files),
    )?;

    let cloned_files = pull_result
        .obfuscated_files
        .iter()
        .map(|f| f.path.clone())
        .collect::<Vec<_>>();
    Ok(CloneResult {
        request_id: pull_result.request_id,
        workspace_dir: workspace.to_string_lossy().to_string(),
        cloned_files,
        stats: pull_result.stats,
        events: pull_result.events,
    })
}

fn status_workspace(args: StatusArgs, state: &AppState) -> AppResult<StatusResult> {
    let request_id = require_request_id(&args.options)?;
    let workspace = resolve_root_dir(&args.workspace_dir)?;
    let session = resolve_session_with_workspace(state, &request_id, &workspace)?;
    let root_dir = session
        .root_dir
        .clone()
        .ok_or_else(|| AppError::InvalidArg("request session root_dir is missing".into()))?;
    let current_hashes = read_workspace_hashes(&workspace)?;
    let diff = compute_workspace_diff(&session.baseline_hashes, &current_hashes);
    let clean = diff.added.is_empty() && diff.modified.is_empty() && diff.deleted.is_empty();

    Ok(StatusResult {
        request_id,
        workspace_dir: workspace.to_string_lossy().to_string(),
        clean,
        diff,
        mapping_state: MappingState {
            stored_at_epoch_s: session.stored_at_epoch_s,
            expires_at_epoch_s: session.mapping.expires_at_epoch_s,
            mapping_entries: session.mapping.mapping.forward.len(),
            tracked_files: session.baseline_hashes.len(),
            root_dir,
            workspace_dir: session
                .workspace_dir
                .unwrap_or_else(|| workspace.to_string_lossy().to_string()),
        },
    })
}

fn push_workspace(args: PushArgs, state: &AppState) -> AppResult<PushResult> {
    let request_id = require_request_id(&args.options)?;
    let workspace = resolve_root_dir(&args.workspace_dir)?;
    let session = resolve_session_with_workspace(state, &request_id, &workspace)?;
    let root_dir = session
        .root_dir
        .clone()
        .ok_or_else(|| AppError::InvalidArg("request session root_dir is missing".into()))?;
    let root = resolve_root_dir(&root_dir)?;

    let workspace_files = read_workspace_files(&workspace)?;
    let workspace_map = workspace_files
        .iter()
        .cloned()
        .map(|file| (file.path.clone(), file))
        .collect::<BTreeMap<_, _>>();
    let current_hashes = build_file_hashes(&workspace_files);
    let diff = compute_workspace_diff(&session.baseline_hashes, &current_hashes);

    let mut changed_paths = diff.added.clone();
    changed_paths.extend(diff.modified.clone());
    changed_paths.sort();
    let changed_files = changed_paths
        .iter()
        .filter_map(|path| workspace_map.get(path).cloned())
        .collect::<Vec<_>>();

    validate_files(&changed_files)?;
    fail_fast_on_missing_tokens_for_tracked(&changed_files, &session.mapping)?;
    let restored_files = deobfuscate_files(&changed_files, &session.mapping.mapping.reverse)?;
    ensure_root_is_writable_for_files(&root, &restored_files)?;

    let mut events = vec![];
    record_event(&mut events, "scanning");
    if !restored_files.is_empty() {
        record_event(&mut events, "deobfuscating");
        record_event(&mut events, "applying");
        write_project_files_to_disk(&root, &restored_files)?;
    }
    if !diff.deleted.is_empty() {
        delete_project_files_from_disk(&root, &diff.deleted)?;
    }
    record_event(&mut events, "applied");
    record_event(&mut events, "completed");

    state
        .request_mappings
        .refresh_snapshot(&request_id, current_hashes)?;

    let deleted_files = diff.deleted;
    let deleted_count = deleted_files.len();

    Ok(PushResult {
        request_id,
        applied_files: restored_files.iter().map(|f| f.path.clone()).collect(),
        deleted_files,
        stats: PushStats {
            applied_count: restored_files.len(),
            deleted_count,
            mapping_entries: session.mapping.mapping.reverse.len(),
        },
        events,
    })
}

fn resolve_session_with_workspace(
    state: &AppState,
    request_id: &str,
    workspace: &Path,
) -> AppResult<StoredRequestMapping> {
    let session = state.request_mappings.resolve_session(request_id)?;
    if session.root_dir.is_none() {
        return Err(AppError::InvalidArg(
            "request context is not initialized; run pull or clone first".into(),
        ));
    }
    if let Some(existing_workspace) = &session.workspace_dir {
        let existing = resolve_root_dir(existing_workspace)?;
        if existing != workspace {
            return Err(AppError::InvalidArg(format!(
                "workspace_dir mismatch for request_id: expected {}, got {}",
                existing.display(),
                workspace.display()
            )));
        }
        return Ok(session);
    }
    state
        .request_mappings
        .bind_workspace_dir(request_id, workspace)
}

fn prepare_workspace_dir(workspace_dir: &str, root: &Path) -> AppResult<PathBuf> {
    if workspace_dir.trim().is_empty() {
        return Err(AppError::InvalidArg("workspace_dir cannot be empty".into()));
    }
    let workspace_path = PathBuf::from(workspace_dir);
    if !workspace_path.exists() {
        fs::create_dir_all(&workspace_path)?;
    }
    let workspace = workspace_path.canonicalize().map_err(|_| {
        AppError::InvalidArg(format!("workspace_dir does not exist: {workspace_dir}"))
    })?;
    if !workspace.is_dir() {
        return Err(AppError::InvalidArg(format!(
            "workspace_dir is not a directory: {workspace_dir}"
        )));
    }
    if workspace == root {
        return Err(AppError::InvalidArg(
            "workspace_dir must be different from root_dir".into(),
        ));
    }
    let mut read_dir = fs::read_dir(&workspace)?;
    if read_dir.next().is_some() {
        return Err(AppError::InvalidArg(
            "workspace_dir must be empty for clone".into(),
        ));
    }
    Ok(workspace)
}

fn build_file_hashes(files: &[ProjectFile]) -> BTreeMap<String, u64> {
    files
        .iter()
        .map(|file| (file.path.clone(), hash_text(&file.content)))
        .collect()
}

fn hash_text(input: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    hasher.finish()
}

fn read_workspace_files(workspace: &Path) -> AppResult<Vec<ProjectFile>> {
    let files = fs_ops::read_text_tree(workspace)?;
    Ok(files
        .into_iter()
        .map(|entry| ProjectFile {
            path: entry.rel.to_string_lossy().to_string(),
            content: entry.text,
        })
        .collect())
}

fn read_workspace_hashes(workspace: &Path) -> AppResult<BTreeMap<String, u64>> {
    Ok(build_file_hashes(&read_workspace_files(workspace)?))
}

fn compute_workspace_diff(
    baseline: &BTreeMap<String, u64>,
    current: &BTreeMap<String, u64>,
) -> WorkspaceDiff {
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();

    for (path, hash) in current {
        match baseline.get(path) {
            None => added.push(path.clone()),
            Some(prev_hash) if prev_hash != hash => modified.push(path.clone()),
            _ => {}
        }
    }

    for path in baseline.keys() {
        if !current.contains_key(path) {
            deleted.push(path.clone());
        }
    }

    added.sort();
    modified.sort();
    deleted.sort();
    WorkspaceDiff {
        added,
        modified,
        deleted,
    }
}

fn fail_fast_on_missing_tokens_for_tracked(
    files: &[ProjectFile],
    mapping_payload: &MappingPayload,
) -> AppResult<()> {
    let Some(metadata) = &mapping_payload.metadata else {
        return Ok(());
    };
    for file in files {
        if !metadata
            .original_paths
            .iter()
            .any(|path| path == &file.path)
        {
            continue;
        }
        let expected_tokens = metadata
            .file_tokens
            .get(&file.path)
            .cloned()
            .unwrap_or_default();
        for token in expected_tokens {
            if !file.content.contains(&token) {
                return Err(AppError::InvalidArg(format!(
                    "fail-fast: obfuscated token '{token}' is missing in workspace file '{}'",
                    file.path
                )));
            }
        }
    }
    Ok(())
}

fn deobfuscate_files(
    files: &[ProjectFile],
    reverse_mapping: &BTreeMap<String, String>,
) -> AppResult<Vec<ProjectFile>> {
    let file_entries = to_file_entries(files)?;
    let transformed = obfuscator::transform_files_global(&file_entries, reverse_mapping)?;
    Ok(transformed
        .into_iter()
        .map(|(rel, content)| ProjectFile {
            path: rel.to_string_lossy().to_string(),
            content,
        })
        .collect())
}

fn delete_project_files_from_disk(root: &Path, rel_paths: &[String]) -> AppResult<()> {
    for rel_path in rel_paths {
        validate_rel_path(rel_path)?;
        let target = root.join(rel_path);
        if !target.exists() {
            continue;
        }
        let meta = fs::symlink_metadata(&target)?;
        if meta.file_type().is_symlink() {
            return Err(AppError::InvalidArg(format!(
                "refusing to delete symlink target: {rel_path}"
            )));
        }
        if meta.is_dir() {
            return Err(AppError::InvalidArg(format!(
                "refusing to delete directory path via push: {rel_path}"
            )));
        }
        let canonical = target
            .canonicalize()
            .map_err(|_| AppError::InvalidArg(format!("failed to resolve path: {rel_path}")))?;
        if !canonical.starts_with(root) {
            return Err(AppError::InvalidArg(
                "path traversal outside root_dir is not allowed".into(),
            ));
        }
        fs::remove_file(canonical)?;
    }
    Ok(())
}

fn load_project_files_from_disk(
    root_dir: &str,
    file_paths: &[String],
) -> AppResult<Vec<ProjectFile>> {
    let root = resolve_root_dir(root_dir)?;
    let gitignore = RootGitignore::from_root(&root)?;
    if file_paths.is_empty() {
        let entries = fs_ops::read_text_tree(&root)?;
        return Ok(entries
            .into_iter()
            .map(|entry| ProjectFile {
                path: entry.rel.to_string_lossy().to_string(),
                content: entry.text,
            })
            .collect());
    }

    let mut files = Vec::with_capacity(file_paths.len());
    for rel in file_paths {
        let full = resolve_path_under_root(&root, rel)?;
        if gitignore.is_ignored_abs(&full, false) {
            continue;
        }
        let text = fs::read_to_string(&full)
            .map_err(|_| AppError::InvalidArg(format!("failed to read UTF-8 file: {rel}")))?;
        let rel_path = full
            .strip_prefix(&root)
            .map_err(|_| AppError::InvalidArg("path is outside project root".into()))?;
        files.push(ProjectFile {
            path: rel_path.to_string_lossy().to_string(),
            content: text,
        });
    }
    Ok(files)
}

fn write_project_files_to_disk(root: &Path, files: &[ProjectFile]) -> AppResult<()> {
    for file in files {
        let out = resolve_write_path_under_root(root, &file.path)?;
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
            let canonical_parent = parent.canonicalize().map_err(|_| {
                AppError::InvalidArg(format!("invalid parent path: {}", parent.display()))
            })?;
            if !canonical_parent.starts_with(root) {
                return Err(AppError::InvalidArg("write path escapes root_dir".into()));
            }
        }
        if out.exists() {
            let meta = fs::symlink_metadata(&out)?;
            if meta.file_type().is_symlink() {
                return Err(AppError::InvalidArg(format!(
                    "refusing to write through symlink: {}",
                    file.path
                )));
            }
            if meta.is_dir() {
                return Err(AppError::InvalidArg(format!(
                    "path points to directory, expected file: {}",
                    file.path
                )));
            }
        }
        fs::write(out, &file.content)?;
    }
    Ok(())
}

fn resolve_write_path_under_root(root: &Path, rel_path: &str) -> AppResult<PathBuf> {
    validate_rel_path(rel_path)?;
    let out = root.join(rel_path);
    let parent = out
        .parent()
        .ok_or_else(|| AppError::InvalidArg("file path must include parent directory".into()))?;
    if !parent.exists() {
        fs::create_dir_all(parent)?;
    }
    let canonical_parent = parent
        .canonicalize()
        .map_err(|_| AppError::InvalidArg(format!("failed to resolve parent: {rel_path}")))?;
    if !canonical_parent.starts_with(root) {
        return Err(AppError::InvalidArg(
            "path traversal outside root_dir is not allowed".into(),
        ));
    }
    Ok(out)
}

fn resolve_root_dir(root_dir: &str) -> AppResult<PathBuf> {
    if root_dir.trim().is_empty() {
        return Err(AppError::InvalidArg("root_dir cannot be empty".into()));
    }
    let root = PathBuf::from(root_dir)
        .canonicalize()
        .map_err(|_| AppError::InvalidArg(format!("root_dir does not exist: {root_dir}")))?;
    if !root.is_dir() {
        return Err(AppError::InvalidArg(format!(
            "root_dir is not a directory: {root_dir}"
        )));
    }
    Ok(root)
}

fn resolve_path_under_root(root: &Path, rel_path: &str) -> AppResult<PathBuf> {
    validate_rel_path(rel_path)?;
    let full = root.join(rel_path);
    let canonical = full
        .canonicalize()
        .map_err(|_| AppError::InvalidArg(format!("file does not exist: {rel_path}")))?;
    if !canonical.starts_with(root) {
        return Err(AppError::InvalidArg(
            "path traversal outside root_dir is not allowed".into(),
        ));
    }
    if !canonical.is_file() {
        return Err(AppError::InvalidArg(format!(
            "path is not a file: {rel_path}"
        )));
    }
    Ok(canonical)
}

fn is_hidden_path(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .map(|s| s.starts_with('.'))
            .unwrap_or(false)
    })
}

fn compute_pull_payload(args: PullComputationArgs, state: &AppState) -> AppResult<PullResult> {
    let request_id = require_request_id(&args.options)?;
    validate_files(&args.project_files)?;

    let mut events = vec![];
    record_event(&mut events, "scanning");

    let files = to_file_entries(&args.project_files)?;
    let mut forward_map = state.default_mapping.get();
    if args.options.enrich_detected_terms.unwrap_or(false) {
        let terms = detect_terms(&files)?;
        enrich_with_random(&mut forward_map, &terms, &files, None);
    }

    record_event(&mut events, "obfuscating");
    let transformed = obfuscator::transform_files_global(&files, &forward_map)?;
    let obfuscated_files = transformed
        .into_iter()
        .map(|(rel, content)| ProjectFile {
            path: rel.to_string_lossy().to_string(),
            content,
        })
        .collect::<Vec<_>>();

    record_event(&mut events, "completed");
    if args.options.stream.unwrap_or(false) {
        record_event(&mut events, "streaming_enabled");
    }

    let created_at_epoch_s = now_epoch_s();
    let reverse = invert(&forward_map)?;
    let mut mapping_payload = MappingPayload {
        mapping: MappingFile {
            forward: forward_map.clone(),
            reverse: reverse.clone(),
        },
        created_at_epoch_s,
        expires_at_epoch_s: args.options.security.as_ref().and_then(|s| {
            s.ttl_seconds
                .map(|ttl| created_at_epoch_s.saturating_add(ttl))
        }),
        signature: None,
        encryption: None,
        metadata: Some(build_mapping_metadata(&obfuscated_files, &reverse)),
    };

    if args
        .options
        .security
        .as_ref()
        .and_then(|s| s.sign_mapping)
        .unwrap_or(false)
    {
        let secret = require_env_secret(
            "MAPPING_SECRET_KEY",
            "security.sign_mapping is enabled for options.security.sign_mapping=true",
        )?;
        mapping_payload.signature = Some(sign_payload(&mapping_payload, &secret)?);
    }

    if args
        .options
        .security
        .as_ref()
        .and_then(|s| s.encrypt_mapping)
        .unwrap_or(false)
    {
        let _encrypt_key = require_env_secret(
            "MAPPING_ENCRYPT_KEY",
            "security.encrypt_mapping is enabled for options.security.encrypt_mapping=true",
        )?;
        mapping_payload.encryption = Some("configured".into());
    }

    state
        .request_mappings
        .insert(request_id.clone(), mapping_payload.clone())?;

    Ok(PullResult {
        request_id,
        obfuscated_files,
        stats: Stats {
            file_count: files.len(),
            mapping_entries: forward_map.len(),
        },
        events,
    })
}

fn record_event(events: &mut Vec<StageEvent>, stage: &'static str) {
    events.push(StageEvent {
        stage,
        timestamp_epoch_s: now_epoch_s(),
    });
}

fn build_mapping_metadata(
    obfuscated_files: &[ProjectFile],
    reverse_map: &BTreeMap<String, String>,
) -> MappingMetadata {
    let mut original_paths = Vec::with_capacity(obfuscated_files.len());
    let mut file_tokens = BTreeMap::new();

    for file in obfuscated_files {
        original_paths.push(file.path.clone());
        let tokens = reverse_map
            .keys()
            .filter(|token| file.content.contains(token.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !tokens.is_empty() {
            file_tokens.insert(file.path.clone(), tokens);
        }
    }

    MappingMetadata {
        original_paths,
        file_tokens,
    }
}

fn ensure_root_is_writable_for_files(root: &Path, files: &[ProjectFile]) -> AppResult<()> {
    let mut checked_dirs = BTreeMap::<PathBuf, ()>::new();

    for file in files {
        let out = resolve_write_path_under_root(root, &file.path)?;
        let mut probe_dir = out
            .parent()
            .ok_or_else(|| AppError::InvalidArg("file path must include parent directory".into()))?
            .to_path_buf();
        while !probe_dir.exists() {
            probe_dir = probe_dir
                .parent()
                .ok_or_else(|| {
                    AppError::InvalidArg(format!(
                        "failed to resolve writable parent for {}",
                        file.path
                    ))
                })?
                .to_path_buf();
        }
        if checked_dirs.contains_key(&probe_dir) {
            continue;
        }
        checked_dirs.insert(probe_dir.clone(), ());
        assert_dir_is_writable(root, &probe_dir, &file.path)?;
    }

    Ok(())
}

fn assert_dir_is_writable(root: &Path, dir: &Path, rel_path: &str) -> AppResult<()> {
    let probe_name = format!(".mcp-write-check-{}-{}", std::process::id(), now_epoch_s());
    let probe_path = dir.join(probe_name);
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_path)
    {
        Ok(_) => {
            let _ = fs::remove_file(&probe_path);
            Ok(())
        }
        Err(err) => Err(AppError::InvalidArg(format!(
            "root_dir is not writable for push (target: {rel_path}, root: {}). Mount the project volume as :rw. Underlying error: {err}",
            root.display()
        ))),
    }
}

fn validate_files(files: &[ProjectFile]) -> AppResult<()> {
    if files.len() > MAX_FILES_PER_PROJECT {
        return Err(AppError::InvalidArg(format!(
            "too many files: {} > {}",
            files.len(),
            MAX_FILES_PER_PROJECT
        )));
    }

    for file in files {
        if file.path.trim().is_empty() {
            return Err(AppError::InvalidArg("file path cannot be empty".into()));
        }
        validate_rel_path(&file.path)?;
    }
    Ok(())
}

fn validate_rel_path(path: &str) -> AppResult<()> {
    if path.contains('\0') {
        return Err(AppError::InvalidArg(
            "path contains null bytes and is invalid".into(),
        ));
    }
    let p = Path::new(path);
    if p.is_absolute() {
        return Err(AppError::InvalidArg(
            "absolute paths are not allowed".into(),
        ));
    }
    for component in p.components() {
        if matches!(component, Component::ParentDir | Component::Prefix(_)) {
            return Err(AppError::InvalidArg(
                "parent traversal is not allowed".into(),
            ));
        }
    }
    Ok(())
}

fn to_file_entries(files: &[ProjectFile]) -> AppResult<Vec<FileEntry>> {
    let mut out = Vec::with_capacity(files.len());
    for file in files {
        validate_rel_path(&file.path)?;
        out.push(FileEntry {
            rel: PathBuf::from(&file.path),
            text: file.content.clone(),
        });
    }
    Ok(out)
}

fn validate_mapping_payload(payload: &MappingPayload) -> AppResult<()> {
    if let Some(exp) = payload.expires_at_epoch_s
        && now_epoch_s() > exp
    {
        return Err(AppError::InvalidArg("mapping payload expired".into()));
    }

    if let Some(sig) = &payload.signature {
        let secret = require_env_secret(
            "MAPPING_SECRET_KEY",
            "signed mapping payload validation is enabled",
        )?;
        let expected = sign_payload(payload, &secret)?;
        if &expected != sig {
            return Err(AppError::InvalidArg(
                "mapping payload signature mismatch".into(),
            ));
        }
    }

    Ok(())
}

fn require_request_id(options: &ToolOptions) -> AppResult<String> {
    let request_id = options
        .request_id
        .as_ref()
        .ok_or_else(|| AppError::InvalidArg("options.request_id is required".into()))?;
    if request_id.trim().is_empty() {
        return Err(AppError::InvalidArg(
            "options.request_id cannot be empty".into(),
        ));
    }
    if request_id.len() > MAX_REQUEST_ID_LENGTH {
        return Err(AppError::InvalidArg(format!(
            "options.request_id exceeds max length: {} > {}",
            request_id.len(),
            MAX_REQUEST_ID_LENGTH
        )));
    }
    Ok(request_id.clone())
}

fn sign_payload(payload: &MappingPayload, secret: &str) -> AppResult<String> {
    let data = serde_json::to_string(&payload.mapping)?;
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    secret.hash(&mut hasher);
    payload.created_at_epoch_s.hash(&mut hasher);
    payload.expires_at_epoch_s.hash(&mut hasher);
    serde_json::to_string(&payload.metadata)?.hash(&mut hasher);
    Ok(format!("{:x}", hasher.finish()))
}

fn require_env_secret(key: &str, context: &str) -> AppResult<String> {
    let value = env::var(key)
        .map_err(|_| AppError::InvalidArg(format!("{key} is required when {context}")))?;
    if value.trim().is_empty() {
        return Err(AppError::InvalidArg(format!(
            "{key} cannot be empty when {context}"
        )));
    }
    Ok(value)
}

fn parse_env_bool(key: &str) -> Option<bool> {
    env::var(key).ok().map(|raw| {
        matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn now_epoch_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_message<R: BufRead>(
    reader: &mut R,
    framing: Option<StdioFraming>,
) -> AppResult<Option<(JsonRpcRequest, StdioFraming)>> {
    match framing {
        Some(StdioFraming::ContentLength) => read_message_content_length(reader)
            .map(|opt| opt.map(|req| (req, StdioFraming::ContentLength))),
        Some(StdioFraming::JsonStream) => read_message_json_stream(reader)
            .map(|opt| opt.map(|req| (req, StdioFraming::JsonStream))),
        None => read_message_auto(reader),
    }
}

fn read_message_auto<R: BufRead>(
    reader: &mut R,
) -> AppResult<Option<(JsonRpcRequest, StdioFraming)>> {
    let first = match read_non_whitespace_byte(reader)? {
        Some(b) => b,
        None => return Ok(None),
    };

    if first == b'{' || first == b'[' {
        let req = read_json_request_from_first(reader, first)?;
        return Ok(Some((req, StdioFraming::JsonStream)));
    }

    let mut content_len = parse_content_length_header(&read_line_with_first_byte(reader, first)?)?;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(parsed) = parse_content_length_header(trimmed)? {
            content_len = Some(parsed);
        }
    }

    let len = content_len.ok_or_else(|| AppError::InvalidArg("missing Content-Length".into()))?;
    let mut body = vec![0_u8; len];
    reader.read_exact(&mut body)?;
    let req: JsonRpcRequest = serde_json::from_slice(&body)?;
    Ok(Some((req, StdioFraming::ContentLength)))
}

fn read_message_content_length<R: BufRead>(reader: &mut R) -> AppResult<Option<JsonRpcRequest>> {
    let mut content_len: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(parsed) = parse_content_length_header(trimmed)? {
            content_len = Some(parsed);
        }
    }

    let len = content_len.ok_or_else(|| AppError::InvalidArg("missing Content-Length".into()))?;
    let mut body = vec![0_u8; len];
    reader.read_exact(&mut body)?;
    let req: JsonRpcRequest = serde_json::from_slice(&body)?;
    Ok(Some(req))
}

fn read_message_json_stream<R: BufRead>(reader: &mut R) -> AppResult<Option<JsonRpcRequest>> {
    let first = match read_non_whitespace_byte(reader)? {
        Some(b) => b,
        None => return Ok(None),
    };
    read_json_request_from_first(reader, first).map(Some)
}

fn read_non_whitespace_byte<R: BufRead>(reader: &mut R) -> AppResult<Option<u8>> {
    let mut byte = [0_u8; 1];
    loop {
        let n = reader.read(&mut byte)?;
        if n == 0 {
            return Ok(None);
        }
        if !byte[0].is_ascii_whitespace() {
            return Ok(Some(byte[0]));
        }
    }
}

fn read_line_with_first_byte<R: BufRead>(reader: &mut R, first: u8) -> AppResult<String> {
    let mut line = vec![first];
    let mut byte = [0_u8; 1];
    loop {
        let n = reader.read(&mut byte)?;
        if n == 0 {
            break;
        }
        line.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&line).trim_end().to_string())
}

fn read_json_request_from_first<R: BufRead>(
    reader: &mut R,
    first: u8,
) -> AppResult<JsonRpcRequest> {
    if first != b'{' && first != b'[' {
        return Err(AppError::InvalidArg(
            "unsupported stdio JSON framing".into(),
        ));
    }

    let mut bytes = vec![first];
    let mut depth: i32 = 1;
    let mut in_string = false;
    let mut escaped = false;
    let mut byte = [0_u8; 1];

    while depth > 0 {
        let n = reader.read(&mut byte)?;
        if n == 0 {
            return Err(AppError::InvalidArg(
                "unexpected EOF while reading JSON-RPC message".into(),
            ));
        }
        let b = byte[0];
        bytes.push(b);

        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if b == b'\\' {
                escaped = true;
                continue;
            }
            if b == b'"' {
                in_string = false;
            }
            continue;
        }

        match b {
            b'"' => in_string = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            _ => {}
        }
    }

    let value: Value = serde_json::from_slice(&bytes)?;
    match value {
        Value::Object(_) => Ok(serde_json::from_value(value)?),
        Value::Array(mut items) => {
            if items.is_empty() {
                Err(AppError::InvalidArg(
                    "empty JSON-RPC batch is not supported".into(),
                ))
            } else {
                Ok(serde_json::from_value(items.remove(0))?)
            }
        }
        _ => Err(AppError::InvalidArg("invalid JSON-RPC payload".into())),
    }
}

fn parse_content_length_header(line: &str) -> AppResult<Option<usize>> {
    if let Some((name, value)) = line.split_once(':')
        && name.trim().eq_ignore_ascii_case("content-length")
    {
        let parsed = value
            .trim()
            .parse::<usize>()
            .map_err(|_| AppError::InvalidArg("invalid Content-Length".into()))?;
        return Ok(Some(parsed));
    }
    Ok(None)
}

fn write_message<W: Write>(
    writer: &mut W,
    message: &JsonRpcResponse,
    framing: StdioFraming,
) -> AppResult<()> {
    let body = serde_json::to_vec(message)?;
    match framing {
        StdioFraming::ContentLength => {
            let header = format!("Content-Length: {}\r\n\r\n", body.len());
            writer.write_all(header.as_bytes())?;
            writer.write_all(&body)?;
        }
        StdioFraming::JsonStream => {
            writer.write_all(&body)?;
            writer.write_all(b"\n")?;
        }
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn workspace_diff_detects_added_modified_deleted() {
        let baseline = BTreeMap::from([("a.py".to_string(), 1_u64), ("b.py".to_string(), 2_u64)]);
        let current = BTreeMap::from([("a.py".to_string(), 3_u64), ("c.py".to_string(), 4_u64)]);
        let diff = compute_workspace_diff(&baseline, &current);
        assert_eq!(diff.added, vec!["c.py".to_string()]);
        assert_eq!(diff.modified, vec!["a.py".to_string()]);
        assert_eq!(diff.deleted, vec!["b.py".to_string()]);
    }

    #[test]
    fn prepare_workspace_dir_rejects_root_dir() {
        let dir = tempdir().expect("tmp");
        let root = dir.path().canonicalize().expect("root");
        let err = prepare_workspace_dir(root.to_string_lossy().as_ref(), &root)
            .expect_err("expected validation error");
        assert!(err.to_string().contains("must be different"), "{err}");
    }

    #[test]
    fn fail_fast_tracked_tokens_ignores_added_file_without_metadata_entry() {
        let mapping_payload = MappingPayload {
            mapping: MappingFile {
                forward: BTreeMap::from([("bs".to_string(), "mmm".to_string())]),
                reverse: BTreeMap::from([("mmm".to_string(), "bs".to_string())]),
            },
            created_at_epoch_s: 0,
            expires_at_epoch_s: None,
            signature: None,
            encryption: None,
            metadata: Some(MappingMetadata {
                original_paths: vec!["tracked.py".to_string()],
                file_tokens: BTreeMap::from([("tracked.py".to_string(), vec!["mmm".to_string()])]),
            }),
        };
        let files = vec![ProjectFile {
            path: "new.py".to_string(),
            content: "print('mmm')".to_string(),
        }];
        let res = fail_fast_on_missing_tokens_for_tracked(&files, &mapping_payload);
        assert!(res.is_ok(), "{res:?}");
    }
}
