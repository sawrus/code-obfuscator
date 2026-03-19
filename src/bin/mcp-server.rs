#![allow(dead_code)]

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
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use error::{AppError, AppResult};
use fs_ops::FileEntry;
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
const REQUEST_MAPPING_TTL_SECONDS: u64 = 24 * 60 * 60;
const REQUEST_MAPPING_MAX_ENTRIES: usize = 256;

#[derive(Clone)]
struct AppState {
    default_mapping: MappingStore,
    request_mappings: RequestMappingStore,
    logger: McpLogger,
    allow_direct_deobfuscation: bool,
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
            },
        );
        self.enforce_max_entries(&mut guard);
        Ok(())
    }

    fn resolve(&self, request_id: &str) -> AppResult<MappingPayload> {
        let now = now_epoch_s();
        let mut guard = self
            .inner
            .write()
            .map_err(|_| AppError::InvalidArg("request mapping store lock poisoned".into()))?;
        self.cleanup_locked(&mut guard, now);
        guard
            .get(request_id)
            .cloned()
            .map(|stored| stored.mapping)
            .ok_or_else(|| {
                AppError::InvalidArg(format!("unknown request_id: {request_id}"))
            })
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
struct ObfuscateArgs {
    project_files: Vec<ProjectFile>,
    #[serde(default)]
    options: ToolOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObfuscateFromPathsArgs {
    root_dir: String,
    #[serde(default)]
    file_paths: Vec<String>,
    #[serde(default)]
    options: ToolOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeobfuscateArgs {
    llm_output_files: Vec<ProjectFile>,
    #[serde(default)]
    options: ToolOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeobfuscateFromPathsArgs {
    root_dir: String,
    #[serde(default)]
    file_paths: Vec<String>,
    #[serde(default)]
    options: ToolOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyLlmOutputArgs {
    root_dir: String,
    llm_output_files: Vec<ProjectFile>,
    #[serde(default)]
    options: ToolOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListProjectTreeArgs {
    root_dir: String,
    max_depth: Option<usize>,
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
struct ObfuscateResult {
    request_id: String,
    obfuscated_files: Vec<ProjectFile>,
    stats: Stats,
    events: Vec<StageEvent>,
}

#[derive(Debug, Serialize)]
struct DeobfuscateResult {
    request_id: String,
    restored_files: Vec<ProjectFile>,
    stats: Stats,
    events: Vec<StageEvent>,
}

#[derive(Debug, Serialize)]
struct ApplyLlmOutputResult {
    request_id: String,
    applied_files: Vec<String>,
    stats: Stats,
    events: Vec<StageEvent>,
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

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let logger = McpLogger::from_env()?;
    let mapping_path = env::var("MCP_DEFAULT_MAPPING_PATH").ok().map(PathBuf::from);
    let allow_direct_deobfuscation =
        parse_env_bool("MCP_ALLOW_DIRECT_DEOBFUSCATION").unwrap_or(false);
    let state = AppState {
        default_mapping: MappingStore::new(mapping_path),
        request_mappings: RequestMappingStore::new(
            REQUEST_MAPPING_TTL_SECONDS,
            REQUEST_MAPPING_MAX_ENTRIES,
        ),
        logger,
        allow_direct_deobfuscation,
    };

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

    if let Ok(addr) = env::var("MCP_HTTP_ADDR") {
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
        let state_for_http = state.clone();
        thread::spawn(move || {
            if let Err(err) = run_http_api(&addr, state_for_http) {
                eprintln!("http api error: {err}");
            }
        });
    }

    run_stdio_mcp(state)
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
                Err(_) => "notification_error",
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
            Err(err) => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(json!({"code": -32000, "message": err.to_string()})),
            },
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

    Ok(())
}

fn run_http_api(addr: &str, state: AppState) -> AppResult<()> {
    let listener = TcpListener::bind(addr)?;
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            state.logger.log(LogEvent {
                level: "warn",
                transport: "http",
                direction: "request",
                request_id: None,
                jsonrpc_id: None,
                method: None,
                path: None,
                status: Some("accept_failed"),
                duration_ms: None,
                payload: None,
            });
            continue;
        };
        let _ = handle_http_connection(&mut stream, &state);
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

    let mut body = vec![0_u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    let body_text = String::from_utf8(body).unwrap_or_default();
    let body_json = serde_json::from_str::<Value>(&body_text).ok();
    let transport = if path == "/" || path == "/mcp" {
        "http-mcp"
    } else {
        "http-admin"
    };
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
                    Err(_) => "notification_error",
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
                Err(err) => JsonRpcResponse {
                    jsonrpc: "2.0",
                    id,
                    result: None,
                    error: Some(json!({"code": -32000, "message": err.to_string()})),
                },
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
        "tools/list" => Ok(json!({"tools": tools_definitions(state.allow_direct_deobfuscation)})),
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

fn tools_definitions(allow_direct_deobfuscation: bool) -> Vec<Value> {
    let mut tools = vec![
        json!({"name":"list_project_tree","description":"List directory structure for a project root. Reads only metadata (paths/types), not file contents.","inputSchema":{"type":"object","required":["root_dir"],"properties":{"root_dir":{"type":"string"},"max_depth":{"type":"integer","minimum":1},"max_entries":{"type":"integer","minimum":1},"include_hidden":{"type":"boolean"}}}}),
        json!({"name":"obfuscate_project_from_paths","description":"Read project files from disk inside MCP by root_dir + file_paths and obfuscate them before sending to LLM. Stores the mapping under options.request_id for later deobfuscation.","inputSchema":{"type":"object","required":["root_dir","options"],"properties":{"root_dir":{"type":"string"},"file_paths":{"type":"array","items":{"type":"string"}},"options":{"type":"object","required":["request_id"],"properties":{"request_id":{"type":"string"},"stream":{"type":"boolean"},"enrich_detected_terms":{"type":"boolean"}}}}}}),
        json!({"name":"obfuscate_project","description":"Obfuscate text-only project files before sending to an LLM. Uses server-side default mapping mode and stores the result under options.request_id.","inputSchema":{"type":"object","required":["project_files","options"],"properties":{"project_files":{"type":"array","items":{"type":"object","required":["path","content"],"properties":{"path":{"type":"string"},"content":{"type":"string"}}}},"options":{"type":"object","required":["request_id"],"properties":{"request_id":{"type":"string"},"stream":{"type":"boolean"},"enrich_detected_terms":{"type":"boolean"}}}}}}),
        json!({"name":"apply_llm_output","description":"Accept LLM-produced obfuscated files, resolve the request mapping from options.request_id, deobfuscate inside MCP, and write restored files to root_dir. Supports applying either the full obfuscated file set or only a changed subset. Returns only applied paths and metadata.","inputSchema":{"type":"object","required":["root_dir","llm_output_files","options"],"properties":{"root_dir":{"type":"string"},"llm_output_files":{"type":"array","items":{"type":"object","required":["path","content"],"properties":{"path":{"type":"string"},"content":{"type":"string"}}}},"options":{"type":"object","required":["request_id"],"properties":{"request_id":{"type":"string"},"stream":{"type":"boolean"}}}}}}),
    ];
    if allow_direct_deobfuscation {
        tools.push(json!({"name":"deobfuscate_project_from_paths","description":"Read obfuscated files from disk inside MCP by root_dir + file_paths and deobfuscate them using the stored request mapping.","inputSchema":{"type":"object","required":["root_dir","options"],"properties":{"root_dir":{"type":"string"},"file_paths":{"type":"array","items":{"type":"string"}},"options":{"type":"object","required":["request_id"],"properties":{"request_id":{"type":"string"},"stream":{"type":"boolean"}}}}}}));
        tools.push(json!({"name":"deobfuscate_project","description":"Restore obfuscated files after LLM response using the stored request mapping referenced by options.request_id.","inputSchema":{"type":"object","required":["llm_output_files","options"],"properties":{"llm_output_files":{"type":"array","items":{"type":"object","required":["path","content"],"properties":{"path":{"type":"string"},"content":{"type":"string"}}}},"options":{"type":"object","required":["request_id"],"properties":{"request_id":{"type":"string"},"stream":{"type":"boolean"}}}}}}));
    }
    tools
}

fn call_tool(params: ToolCallParams, state: &AppState) -> AppResult<Value> {
    match params.name.as_str() {
        "list_project_tree" => {
            let args: ListProjectTreeArgs = serde_json::from_value(params.arguments)?;
            let result = list_project_tree(args)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        "obfuscate_project_from_paths" => {
            let args: ObfuscateFromPathsArgs = serde_json::from_value(params.arguments)?;
            let result = obfuscate_project_from_paths(args, state)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        "deobfuscate_project_from_paths" => {
            if !state.allow_direct_deobfuscation {
                return Err(AppError::InvalidArg(
                    "direct deobfuscation tools are disabled; use apply_llm_output".into(),
                ));
            }
            let args: DeobfuscateFromPathsArgs = serde_json::from_value(params.arguments)?;
            let result = deobfuscate_project_from_paths(args, state)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        "obfuscate_project" => {
            let args: ObfuscateArgs = serde_json::from_value(params.arguments)?;
            let result = obfuscate_project(args, state)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        "apply_llm_output" => {
            let args: ApplyLlmOutputArgs = serde_json::from_value(params.arguments)?;
            let result = apply_llm_output(args, state)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        "deobfuscate_project" => {
            if !state.allow_direct_deobfuscation {
                return Err(AppError::InvalidArg(
                    "direct deobfuscation tools are disabled; use apply_llm_output".into(),
                ));
            }
            let args: DeobfuscateArgs = serde_json::from_value(params.arguments)?;
            let result = deobfuscate_project(args, state)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        _ => Err(AppError::InvalidArg("unknown or disabled tool".into())),
    }
}

fn list_project_tree(args: ListProjectTreeArgs) -> AppResult<ProjectTreeResult> {
    let root = resolve_root_dir(&args.root_dir)?;
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

fn obfuscate_project_from_paths(
    args: ObfuscateFromPathsArgs,
    state: &AppState,
) -> AppResult<ObfuscateResult> {
    let project_files = load_project_files_from_disk(&args.root_dir, &args.file_paths)?;
    obfuscate_project(
        ObfuscateArgs {
            project_files,
            options: args.options,
        },
        state,
    )
}

fn deobfuscate_project_from_paths(
    args: DeobfuscateFromPathsArgs,
    state: &AppState,
) -> AppResult<DeobfuscateResult> {
    let llm_output_files = load_project_files_from_disk(&args.root_dir, &args.file_paths)?;
    deobfuscate_project(
        DeobfuscateArgs {
            llm_output_files,
            options: args.options,
        },
        state,
    )
}

fn apply_llm_output(args: ApplyLlmOutputArgs, state: &AppState) -> AppResult<ApplyLlmOutputResult> {
    let request_id = require_request_id(&args.options)?;
    let root = resolve_root_dir(&args.root_dir)?;
    let deobfuscated = deobfuscate_project(
        DeobfuscateArgs {
            llm_output_files: args.llm_output_files,
            options: args.options,
        },
        state,
    )?;

    ensure_root_is_writable_for_files(&root, &deobfuscated.restored_files)?;

    let mut events = deobfuscated.events;
    record_event(&mut events, "applying");
    write_project_files_to_disk(&root, &deobfuscated.restored_files)?;
    record_event(&mut events, "applied");

    let applied_files = deobfuscated
        .restored_files
        .iter()
        .map(|f| f.path.clone())
        .collect();
    Ok(ApplyLlmOutputResult {
        request_id,
        applied_files,
        stats: deobfuscated.stats,
        events,
    })
}

fn load_project_files_from_disk(
    root_dir: &str,
    file_paths: &[String],
) -> AppResult<Vec<ProjectFile>> {
    let root = resolve_root_dir(root_dir)?;
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

fn obfuscate_project(args: ObfuscateArgs, state: &AppState) -> AppResult<ObfuscateResult> {
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
        mapping_payload.signature = Some(sign_payload(&mapping_payload)?);
    }

    if args
        .options
        .security
        .as_ref()
        .and_then(|s| s.encrypt_mapping)
        .unwrap_or(false)
    {
        mapping_payload.encryption = Some("not-configured".into());
    }

    state
        .request_mappings
        .insert(request_id.clone(), mapping_payload.clone())?;

    Ok(ObfuscateResult {
        request_id,
        obfuscated_files,
        stats: Stats {
            file_count: files.len(),
            mapping_entries: forward_map.len(),
        },
        events,
    })
}

fn deobfuscate_project(args: DeobfuscateArgs, state: &AppState) -> AppResult<DeobfuscateResult> {
    let request_id = require_request_id(&args.options)?;
    validate_files(&args.llm_output_files)?;

    let mut events = vec![];
    record_event(&mut events, "scanning");

    let mapping_payload = state.request_mappings.resolve(&request_id)?;
    validate_mapping_payload(&mapping_payload)?;

    let files = to_file_entries(&args.llm_output_files)?;
    fail_fast_on_missing_tokens(&args.llm_output_files, &mapping_payload)?;

    record_event(&mut events, "deobfuscating");
    let transformed = obfuscator::transform_files_global(&files, &mapping_payload.mapping.reverse)?;
    let restored_files = transformed
        .into_iter()
        .map(|(rel, content)| ProjectFile {
            path: rel.to_string_lossy().to_string(),
            content,
        })
        .collect::<Vec<_>>();

    record_event(&mut events, "completed");
    Ok(DeobfuscateResult {
        request_id,
        restored_files,
        stats: Stats {
            file_count: files.len(),
            mapping_entries: mapping_payload.mapping.reverse.len(),
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

fn fail_fast_on_missing_tokens(
    files: &[ProjectFile],
    mapping_payload: &MappingPayload,
) -> AppResult<()> {
    let Some(metadata) = &mapping_payload.metadata else {
        let corpus = files
            .iter()
            .map(|file| file.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for token in mapping_payload.mapping.reverse.keys() {
            if !corpus.contains(token) {
                return Err(AppError::InvalidArg(format!(
                    "fail-fast: obfuscated token '{token}' is missing in LLM output"
                )));
            }
        }
        return Ok(());
    };

    for file in files {
        if !metadata
            .original_paths
            .iter()
            .any(|path| path == &file.path)
        {
            return Err(AppError::InvalidArg(format!(
                "apply_llm_output received unknown file path '{}' ; expected a subset of {:?}",
                file.path, metadata.original_paths
            )));
        }

        let expected_tokens = metadata
            .file_tokens
            .get(&file.path)
            .cloned()
            .unwrap_or_default();
        for token in expected_tokens {
            if !file.content.contains(&token) {
                return Err(AppError::InvalidArg(format!(
                    "fail-fast: obfuscated token '{token}' is missing in LLM output for file '{}'",
                    file.path
                )));
            }
        }
    }
    Ok(())
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
            "root_dir is not writable for apply_llm_output (target: {rel_path}, root: {}). Mount the project volume as :rw. Underlying error: {err}",
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
        let expected = sign_payload(payload)?;
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
    Ok(request_id.clone())
}

fn sign_payload(payload: &MappingPayload) -> AppResult<String> {
    let data = serde_json::to_string(&payload.mapping)?;
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    payload.created_at_epoch_s.hash(&mut hasher);
    payload.expires_at_epoch_s.hash(&mut hasher);
    serde_json::to_string(&payload.metadata)?.hash(&mut hasher);
    Ok(format!("{:x}", hasher.finish()))
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
