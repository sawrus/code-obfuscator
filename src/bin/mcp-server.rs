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
use std::fs;
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

const MAX_FILES_PER_PROJECT: usize = 1_000_000;

#[derive(Clone)]
struct AppState {
    default_mapping: MappingStore,
    logger: McpLogger,
}

#[derive(Clone)]
struct MappingStore {
    inner: Arc<RwLock<BTreeMap<String, String>>>,
    path: Option<PathBuf>,
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
    JsonLine,
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
struct ProjectFile {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ObfuscateArgs {
    project_files: Vec<ProjectFile>,
    #[serde(default)]
    manual_mapping: BTreeMap<String, String>,
    #[serde(default)]
    options: ToolOptions,
}

#[derive(Debug, Deserialize)]
struct DeobfuscateArgs {
    llm_output_files: Vec<ProjectFile>,
    mapping_payload: Option<MappingPayload>,
    #[serde(default)]
    options: ToolOptions,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct ToolOptions {
    request_id: Option<String>,
    stream: Option<bool>,
    enrich_detected_terms: Option<bool>,
    security: Option<SecurityOptions>,
}

#[derive(Debug, Clone, Deserialize)]
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
}

#[derive(Debug, Serialize)]
struct ObfuscateResult {
    request_id: String,
    obfuscated_files: Vec<ProjectFile>,
    mapping_payload: MappingPayload,
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
struct Stats {
    file_count: usize,
    mapping_entries: usize,
}

#[derive(Debug, Serialize)]
struct StageEvent {
    stage: &'static str,
    timestamp_epoch_s: u64,
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
    let state = AppState {
        default_mapping: MappingStore::new(mapping_path),
        logger,
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
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {"name": "code-obfuscator-mcp", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"tools": {"listChanged": false}}
        })),
        "tools/list" => Ok(json!({"tools": tools_definitions()})),
        "tools/call" => {
            let params: ToolCallParams = serde_json::from_value(req.params)?;
            call_tool(params, state)
        }
        _ => Ok(json!({})),
    }
}

fn tools_definitions() -> Vec<Value> {
    vec![
        json!({"name":"obfuscate_project","description":"Obfuscate text-only project files before sending to an LLM. Uses global mapping mode (no --deep).","inputSchema":{"type":"object","required":["project_files"],"properties":{"project_files":{"type":"array","items":{"type":"object","required":["path","content"],"properties":{"path":{"type":"string"},"content":{"type":"string"}}}},"manual_mapping":{"type":"object","additionalProperties":{"type":"string"}},"options":{"type":"object","properties":{"request_id":{"type":"string"},"stream":{"type":"boolean"},"enrich_detected_terms":{"type":"boolean"}}}}}}),
        json!({"name":"deobfuscate_project","description":"Restore obfuscated files after LLM response. Uses provided mapping_payload or falls back to server default mapping.","inputSchema":{"type":"object","required":["llm_output_files"],"properties":{"llm_output_files":{"type":"array","items":{"type":"object","required":["path","content"],"properties":{"path":{"type":"string"},"content":{"type":"string"}}}},"mapping_payload":{"type":"object"},"options":{"type":"object","properties":{"request_id":{"type":"string"},"stream":{"type":"boolean"}}}}}}),
    ]
}

fn call_tool(params: ToolCallParams, state: &AppState) -> AppResult<Value> {
    match params.name.as_str() {
        "obfuscate_project" => {
            let args: ObfuscateArgs = serde_json::from_value(params.arguments)?;
            let result = obfuscate_project(args, state)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        "deobfuscate_project" => {
            let args: DeobfuscateArgs = serde_json::from_value(params.arguments)?;
            let result = deobfuscate_project(args, state)?;
            Ok(json!({"content":[{"type":"text","text":serde_json::to_string(&result)?}]}))
        }
        _ => Err(AppError::InvalidArg("unknown tool".into())),
    }
}

fn obfuscate_project(args: ObfuscateArgs, state: &AppState) -> AppResult<ObfuscateResult> {
    validate_files(&args.project_files)?;
    let request_id = args
        .options
        .request_id
        .unwrap_or_else(|| format!("req-{}", now_epoch_s()));

    let mut events = vec![];
    record_event(&mut events, "scanning");

    let files = to_file_entries(&args.project_files)?;
    let mut forward_map = if args.manual_mapping.is_empty() {
        state.default_mapping.get()
    } else {
        args.manual_mapping
    };
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
            reverse,
        },
        created_at_epoch_s,
        expires_at_epoch_s: args.options.security.as_ref().and_then(|s| {
            s.ttl_seconds
                .map(|ttl| created_at_epoch_s.saturating_add(ttl))
        }),
        signature: None,
        encryption: None,
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

    Ok(ObfuscateResult {
        request_id,
        obfuscated_files,
        mapping_payload,
        stats: Stats {
            file_count: files.len(),
            mapping_entries: forward_map.len(),
        },
        events,
    })
}

fn deobfuscate_project(args: DeobfuscateArgs, state: &AppState) -> AppResult<DeobfuscateResult> {
    validate_files(&args.llm_output_files)?;
    let request_id = args
        .options
        .request_id
        .unwrap_or_else(|| format!("req-{}", now_epoch_s()));

    let mut events = vec![];
    record_event(&mut events, "scanning");

    let mapping_payload = resolve_deobfuscation_mapping(args.mapping_payload, state)?;
    validate_mapping_payload(&mapping_payload)?;

    let files = to_file_entries(&args.llm_output_files)?;
    fail_fast_on_missing_tokens(&files, &mapping_payload.mapping.reverse)?;

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

fn resolve_deobfuscation_mapping(
    maybe_payload: Option<MappingPayload>,
    state: &AppState,
) -> AppResult<MappingPayload> {
    if let Some(payload) = maybe_payload {
        return Ok(payload);
    }

    let forward = state.default_mapping.get();
    let reverse = invert(&forward)?;
    Ok(MappingPayload {
        mapping: MappingFile { forward, reverse },
        created_at_epoch_s: now_epoch_s(),
        expires_at_epoch_s: None,
        signature: None,
        encryption: None,
    })
}

fn record_event(events: &mut Vec<StageEvent>, stage: &'static str) {
    events.push(StageEvent {
        stage,
        timestamp_epoch_s: now_epoch_s(),
    });
}

fn fail_fast_on_missing_tokens(
    files: &[FileEntry],
    reverse_map: &BTreeMap<String, String>,
) -> AppResult<()> {
    let corpus = files
        .iter()
        .map(|f| f.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for token in reverse_map.keys() {
        if !corpus.contains(token) {
            return Err(AppError::InvalidArg(format!(
                "fail-fast: obfuscated token '{token}' is missing in LLM output"
            )));
        }
    }
    Ok(())
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

fn sign_payload(payload: &MappingPayload) -> AppResult<String> {
    let data = serde_json::to_string(&payload.mapping)?;
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    payload.created_at_epoch_s.hash(&mut hasher);
    payload.expires_at_epoch_s.hash(&mut hasher);
    Ok(format!("{:x}", hasher.finish()))
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
        Some(StdioFraming::JsonLine) => {
            read_message_json_line(reader).map(|opt| opt.map(|req| (req, StdioFraming::JsonLine)))
        }
        None => read_message_auto(reader),
    }
}

fn read_message_auto<R: BufRead>(
    reader: &mut R,
) -> AppResult<Option<(JsonRpcRequest, StdioFraming)>> {
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with('{') {
            let req: JsonRpcRequest = serde_json::from_str(trimmed)?;
            return Ok(Some((req, StdioFraming::JsonLine)));
        }

        let mut content_len = parse_content_length_header(trimmed)?;
        loop {
            let mut next_line = String::new();
            let n = reader.read_line(&mut next_line)?;
            if n == 0 {
                return Ok(None);
            }
            let trimmed_next = next_line.trim_end();
            if trimmed_next.is_empty() {
                break;
            }
            if let Some(parsed) = parse_content_length_header(trimmed_next)? {
                content_len = Some(parsed);
            }
        }

        let len =
            content_len.ok_or_else(|| AppError::InvalidArg("missing Content-Length".into()))?;
        let mut body = vec![0_u8; len];
        reader.read_exact(&mut body)?;
        let req: JsonRpcRequest = serde_json::from_slice(&body)?;
        return Ok(Some((req, StdioFraming::ContentLength)));
    }
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

fn read_message_json_line<R: BufRead>(reader: &mut R) -> AppResult<Option<JsonRpcRequest>> {
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: JsonRpcRequest = serde_json::from_str(trimmed)?;
        return Ok(Some(req));
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
        StdioFraming::JsonLine => {
            writer.write_all(&body)?;
            writer.write_all(b"\n")?;
        }
    }
    writer.flush()?;
    Ok(())
}
