#![allow(dead_code)]

#[path = "../error.rs"]
mod error;
#[path = "../fs_ops.rs"]
mod fs_ops;
#[path = "../language.rs"]
mod language;
#[path = "../mapping.rs"]
mod mapping;
#[path = "../obfuscator.rs"]
mod obfuscator;

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use error::{AppError, AppResult};
use fs_ops::FileEntry;
use mapping::{MappingFile, detect_terms, enrich_with_random, invert};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_FILES_PER_PROJECT: usize = 1_000_000;

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
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
    mapping_payload: MappingPayload,
    #[serde(default)]
    options: ToolOptions,
}

#[derive(Debug, Default, Deserialize)]
struct ToolOptions {
    request_id: Option<String>,
    stream: Option<bool>,
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

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut out = stdout.lock();

    while let Some(req) = read_message(&mut reader)? {
        let id = req.id.clone().unwrap_or(Value::Null);
        if req.id.is_none() {
            continue;
        }

        let response = match handle_request(req) {
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

        write_message(&mut out, &response)?;
    }

    Ok(())
}

fn handle_request(req: JsonRpcRequest) -> AppResult<Value> {
    match req.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {"name": "code-obfuscator-mcp", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {
                "tools": {"listChanged": false}
            }
        })),
        "tools/list" => Ok(json!({"tools": tools_definitions()})),
        "tools/call" => {
            let params: ToolCallParams = serde_json::from_value(req.params)?;
            call_tool(params)
        }
        _ => Ok(json!({})),
    }
}

fn tools_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "obfuscate_project",
            "description": "Obfuscate text-only project files before sending to an LLM. Uses global mapping mode (no --deep).",
            "inputSchema": {
                "type": "object",
                "required": ["project_files"],
                "properties": {
                    "project_files": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["path", "content"],
                            "properties": {
                                "path": {"type": "string"},
                                "content": {"type": "string"}
                            }
                        }
                    },
                    "manual_mapping": {"type": "object", "additionalProperties": {"type": "string"}},
                    "options": {"type": "object"}
                }
            }
        }),
        json!({
            "name": "deobfuscate_project",
            "description": "Restore obfuscated files after LLM response using the mapping payload from obfuscate_project.",
            "inputSchema": {
                "type": "object",
                "required": ["llm_output_files", "mapping_payload"],
                "properties": {
                    "llm_output_files": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["path", "content"],
                            "properties": {
                                "path": {"type": "string"},
                                "content": {"type": "string"}
                            }
                        }
                    },
                    "mapping_payload": {"type": "object"},
                    "options": {"type": "object"}
                }
            }
        }),
    ]
}

fn call_tool(params: ToolCallParams) -> AppResult<Value> {
    match params.name.as_str() {
        "obfuscate_project" => {
            let args: ObfuscateArgs = serde_json::from_value(params.arguments)?;
            let result = obfuscate_project(args)?;
            Ok(json!({"content": [{"type": "text", "text": serde_json::to_string(&result)?}]}))
        }
        "deobfuscate_project" => {
            let args: DeobfuscateArgs = serde_json::from_value(params.arguments)?;
            let result = deobfuscate_project(args)?;
            Ok(json!({"content": [{"type": "text", "text": serde_json::to_string(&result)?}]}))
        }
        _ => Err(AppError::InvalidArg("unknown tool".into())),
    }
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

fn obfuscate_project(args: ObfuscateArgs) -> AppResult<ObfuscateResult> {
    validate_files(&args.project_files)?;
    let request_id = args
        .options
        .request_id
        .unwrap_or_else(|| format!("req-{}", now_epoch_s()));

    let mut events = Vec::new();
    record_event(&mut events, "scanning");

    let files = to_file_entries(&args.project_files)?;
    let mut forward_map = args.manual_mapping;
    let terms = detect_terms(&files)?;
    enrich_with_random(&mut forward_map, &terms, &files, None);

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

fn deobfuscate_project(args: DeobfuscateArgs) -> AppResult<DeobfuscateResult> {
    validate_files(&args.llm_output_files)?;
    let request_id = args
        .options
        .request_id
        .unwrap_or_else(|| format!("req-{}", now_epoch_s()));

    let mut events = Vec::new();
    record_event(&mut events, "scanning");

    validate_mapping_payload(&args.mapping_payload)?;

    let files = to_file_entries(&args.llm_output_files)?;
    fail_fast_on_missing_tokens(&files, &args.mapping_payload.mapping.reverse)?;

    record_event(&mut events, "deobfuscating");
    let transformed =
        obfuscator::transform_files_global(&files, &args.mapping_payload.mapping.reverse)?;
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
            mapping_entries: args.mapping_payload.mapping.reverse.len(),
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
    if let Some(exp) = payload.expires_at_epoch_s {
        if now_epoch_s() > exp {
            return Err(AppError::InvalidArg("mapping payload expired".into()));
        }
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

fn read_message<R: BufRead>(reader: &mut R) -> AppResult<Option<JsonRpcRequest>> {
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
        if let Some(v) = trimmed.strip_prefix("Content-Length:") {
            let parsed = v
                .trim()
                .parse::<usize>()
                .map_err(|_| AppError::InvalidArg("invalid Content-Length".into()))?;
            content_len = Some(parsed);
        }
    }

    let len = content_len.ok_or_else(|| AppError::InvalidArg("missing Content-Length".into()))?;
    let mut body = vec![0_u8; len];
    reader.read_exact(&mut body)?;
    let req: JsonRpcRequest = serde_json::from_slice(&body)?;
    Ok(Some(req))
}

fn write_message<W: Write>(writer: &mut W, message: &JsonRpcResponse) -> AppResult<()> {
    let body = serde_json::to_vec(message)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}
