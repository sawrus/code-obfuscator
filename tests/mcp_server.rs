use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::tempdir;

fn send_request(stdin: &mut impl Write, req: &Value) {
    let body = serde_json::to_vec(req).expect("serialize req");
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    stdin.write_all(header.as_bytes()).expect("write header");
    stdin.write_all(&body).expect("write body");
    stdin.flush().expect("flush");
}

fn send_request_json_line(stdin: &mut impl Write, req: &Value) {
    let body = serde_json::to_string(req).expect("serialize req");
    stdin.write_all(body.as_bytes()).expect("write body");
    stdin.write_all(b"\n").expect("write newline");
    stdin.flush().expect("flush");
}

fn send_request_json_stream(stdin: &mut impl Write, req: &Value) {
    let body = serde_json::to_string(req).expect("serialize req");
    stdin.write_all(body.as_bytes()).expect("write body");
    stdin.flush().expect("flush");
}

fn send_request_lowercase_header(stdin: &mut impl Write, req: &Value) {
    let body = serde_json::to_vec(req).expect("serialize req");
    let header = format!("content-length: {}\r\n\r\n", body.len());
    stdin.write_all(header.as_bytes()).expect("write header");
    stdin.write_all(&body).expect("write body");
    stdin.flush().expect("flush");
}

fn read_response_json_line(stdout: &mut impl Read) -> Value {
    let mut line = Vec::new();
    let mut buf = [0_u8; 1];
    loop {
        stdout.read_exact(&mut buf).expect("read byte");
        if buf[0] == b'\n' {
            break;
        }
        line.push(buf[0]);
    }
    serde_json::from_slice(&line).expect("parse response")
}

fn read_response(stdout: &mut impl Read) -> Value {
    let mut header_bytes = Vec::new();
    let mut buf = [0_u8; 1];
    loop {
        stdout.read_exact(&mut buf).expect("read header byte");
        header_bytes.push(buf[0]);
        if header_bytes.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let header = String::from_utf8(header_bytes).expect("header utf8");
    let len = header
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length:"))
        .expect("content length line")
        .trim()
        .parse::<usize>()
        .expect("length parse");

    let mut body = vec![0_u8; len];
    stdout.read_exact(&mut body).expect("read body");
    serde_json::from_slice(&body).expect("parse response")
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind free port");
    listener.local_addr().expect("local addr").port()
}

fn http_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().expect("lock")
}

fn wait_http_ready(addr: &str) {
    for _ in 0..40 {
        if TcpStream::connect(addr).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("http server did not become ready: {addr}");
}

fn http_request(addr: &str, method: &str, path: &str, body: Option<&str>) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect http");
    let body = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(req.as_bytes()).expect("write request");
    let mut resp = String::new();
    stream.read_to_string(&mut resp).expect("read response");
    resp
}

fn http_status_and_json(resp: &str) -> (u16, Value) {
    let mut parts = resp.splitn(2, "\r\n\r\n");
    let header = parts.next().unwrap_or_default();
    let body = parts.next().unwrap_or_default();
    let status = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let body_json = if body.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(body).expect("parse http json body")
    };
    (status, body_json)
}

fn kill_and_wait(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn read_log(dir: &Path) -> String {
    fs::read_to_string(dir.join("mcp-server.log")).expect("read mcp log")
}

#[test]
fn mcp_roundtrip_obfuscate_then_deobfuscate() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("src")).expect("mkdirs");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"tools/call",
            "params":{
                "name":"obfuscate_project",
                "arguments":{
                    "project_files":[{"path":"src/main.py","content":"def run_order(order_id):\n    return order_id\n"}],
                    "manual_mapping":{"run_order":"x_run"}
                }
            }
        }),
    );
    let obf_response = read_response(&mut stdout);
    assert!(obf_response.get("error").is_none(), "{obf_response}");

    let text = obf_response["result"]["content"][0]["text"]
        .as_str()
        .expect("text payload");
    let payload: Value = serde_json::from_str(text).expect("parse tool result");

    let obf_files = payload["obfuscated_files"].clone();
    let mapping_payload = payload["mapping_payload"].clone();

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params":{
                "name":"apply_llm_output",
                "arguments":{
                    "root_dir": project.to_string_lossy().to_string(),
                    "llm_output_files": obf_files,
                    "mapping_payload": mapping_payload
                }
            }
        }),
    );
    let rev_response = read_response(&mut stdout);
    assert!(rev_response.get("error").is_none(), "{rev_response}");

    let rev_text = rev_response["result"]["content"][0]["text"]
        .as_str()
        .expect("rev text payload");
    let restored: Value = serde_json::from_str(rev_text).expect("restored payload");
    let first_path = restored["applied_files"][0].as_str().expect("applied path");

    assert_eq!(first_path, "src/main.py", "{restored}");
    let applied = fs::read_to_string(project.join("src/main.py")).expect("applied file");
    assert!(applied.contains("run_order"), "{applied}");
    assert!(applied.contains("order_id"), "{applied}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_apply_llm_output_accepts_subset_of_obfuscated_files() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("app")).expect("mkdirs");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{
                "name":"obfuscate_project",
                "arguments":{
                    "project_files":[
                        {"path":"app/query.py","content":"QUERY = \"select * from bs.users\"\n"},
                        {"path":"app/readme.txt","content":"hello world\n"}
                    ],
                    "manual_mapping":{"bs":"mmm"}
                }
            }
        }),
    );
    let obf_response = read_response(&mut stdout);
    assert!(obf_response.get("error").is_none(), "{obf_response}");

    let text = obf_response["result"]["content"][0]["text"]
        .as_str()
        .expect("text payload");
    let payload: Value = serde_json::from_str(text).expect("parse tool result");
    let mapping_payload = payload["mapping_payload"].clone();
    let query_file = payload["obfuscated_files"]
        .as_array()
        .expect("obfuscated files")
        .iter()
        .find(|file| file["path"] == "app/query.py")
        .cloned()
        .expect("query file")
        .as_object()
        .cloned()
        .expect("query file object");

    let mut query_file = Value::Object(query_file);
    query_file["content"] = Value::String(
        query_file["content"]
            .as_str()
            .expect("content")
            .replace("select *", "select id"),
    );

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":4,
            "method":"tools/call",
            "params":{
                "name":"apply_llm_output",
                "arguments":{
                    "root_dir": project.to_string_lossy().to_string(),
                    "llm_output_files": [query_file],
                    "mapping_payload": mapping_payload
                }
            }
        }),
    );
    let apply_response = read_response(&mut stdout);
    assert!(apply_response.get("error").is_none(), "{apply_response}");

    let apply_text = apply_response["result"]["content"][0]["text"]
        .as_str()
        .expect("apply text payload");
    let applied: Value = serde_json::from_str(apply_text).expect("applied payload");
    assert_eq!(
        applied["applied_files"],
        json!(["app/query.py"]),
        "{applied}"
    );

    let query = fs::read_to_string(project.join("app/query.py")).expect("query file");
    assert!(query.contains("select id from bs.users"), "{query}");
    assert!(
        !project.join("app/readme.txt").exists(),
        "readme should not be written on subset apply"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_apply_llm_output_rejects_unknown_subset_paths() {
    let dir = tempdir().expect("tmp");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":5,
            "method":"tools/call",
            "params":{
                "name":"apply_llm_output",
                "arguments":{
                    "root_dir": dir.path().to_string_lossy().to_string(),
                    "llm_output_files":[{"path":"other.py","content":"print('x')"}],
                    "mapping_payload":{
                        "mapping":{
                            "forward":{"bs":"mmm"},
                            "reverse":{"mmm":"bs"}
                        },
                        "created_at_epoch_s": 1,
                        "metadata":{
                            "original_paths":["app/query.py"],
                            "file_tokens":{"app/query.py":["mmm"]}
                        }
                    }
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    let error = response["error"]["message"].as_str().unwrap_or_default();
    assert!(error.contains("unknown file path"), "{response}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_apply_llm_output_fails_when_returned_file_loses_required_token() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("app")).expect("mkdirs");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":6,
            "method":"tools/call",
            "params":{
                "name":"obfuscate_project",
                "arguments":{
                    "project_files":[{"path":"app/query.py","content":"select * from bs.users\n"}],
                    "manual_mapping":{"bs":"mmm"}
                }
            }
        }),
    );
    let obf_response = read_response(&mut stdout);
    assert!(obf_response.get("error").is_none(), "{obf_response}");

    let text = obf_response["result"]["content"][0]["text"]
        .as_str()
        .expect("text payload");
    let payload: Value = serde_json::from_str(text).expect("parse tool result");
    let mapping_payload = payload["mapping_payload"].clone();

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":7,
            "method":"tools/call",
            "params":{
                "name":"apply_llm_output",
                "arguments":{
                    "root_dir": project.to_string_lossy().to_string(),
                    "llm_output_files":[{"path":"app/query.py","content":"select * from users\n"}],
                    "mapping_payload": mapping_payload
                }
            }
        }),
    );
    let response = read_response(&mut stdout);
    let error = response["error"]["message"].as_str().unwrap_or_default();
    assert!(
        error.contains("missing in LLM output for file 'app/query.py'"),
        "{response}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_stdio_handshake_works_with_default_logging_config() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":900,
            "method":"initialize",
            "params":{}
        }),
    );
    let response = read_response(&mut stdout);
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response["result"]["serverInfo"]["name"].as_str(),
        Some("code-obfuscator-mcp")
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_initialize_echoes_client_protocol_version() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request_json_stream(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":"init-1",
            "method":"initialize",
            "params":{"protocolVersion":"2025-11-25"}
        }),
    );
    let response = read_response_json_line(&mut stdout);
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response["result"]["protocolVersion"].as_str(),
        Some("2025-11-25")
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_resources_endpoints_return_empty_lists() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":904,
            "method":"resources/list",
            "params":{}
        }),
    );
    let resources_response = read_response(&mut stdout);
    assert!(
        resources_response.get("error").is_none(),
        "{resources_response}"
    );
    assert!(
        resources_response["result"]["resources"].is_array(),
        "{resources_response}"
    );

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":905,
            "method":"resources/templates/list",
            "params":{}
        }),
    );
    let templates_response = read_response(&mut stdout);
    assert!(
        templates_response.get("error").is_none(),
        "{templates_response}"
    );
    assert!(
        templates_response["result"]["resourceTemplates"].is_array(),
        "{templates_response}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_stdio_accepts_lowercase_content_length_header() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request_lowercase_header(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":901,
            "method":"initialize",
            "params":{}
        }),
    );
    let response = read_response(&mut stdout);
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response["result"]["serverInfo"]["name"].as_str(),
        Some("code-obfuscator-mcp")
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_stdio_accepts_json_line_protocol() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request_json_line(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":902,
            "method":"initialize",
            "params":{}
        }),
    );
    let response = read_response_json_line(&mut stdout);
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response["result"]["serverInfo"]["name"].as_str(),
        Some("code-obfuscator-mcp")
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_stdio_accepts_json_stream_without_newline() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request_json_stream(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":903,
            "method":"initialize",
            "params":{}
        }),
    );
    let response = read_response_json_line(&mut stdout);
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response["result"]["serverInfo"]["name"].as_str(),
        Some("code-obfuscator-mcp")
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_uses_default_mapping_when_manual_not_provided() {
    let dir = tempdir().expect("tmp");
    let mapping_path = dir.path().join("mapping.default.json");
    fs::write(&mapping_path, r#"{"run_order":"x_run"}"#).expect("write mapping");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_DEFAULT_MAPPING_PATH", &mapping_path)
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{
                "name":"obfuscate_project",
                "arguments":{
                    "project_files":[{"path":"src/main.py","content":"def run_order(order_id):\n    return order_id\n"}]
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    assert!(response.get("error").is_none(), "{response}");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text payload");
    let payload: Value = serde_json::from_str(text).expect("payload json");
    let obf_content = payload["obfuscated_files"][0]["content"]
        .as_str()
        .expect("content");
    assert!(obf_content.contains("x_run"), "{payload}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_obfuscate_enrich_detected_terms_opt_in() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":31,
            "method":"tools/call",
            "params":{
                "name":"obfuscate_project",
                "arguments":{
                    "project_files":[{"path":"q.sql","content":"select 1 from users where id = 1"}],
                    "options":{"enrich_detected_terms": true}
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    assert!(response.get("error").is_none(), "{response}");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text payload");
    let payload: Value = serde_json::from_str(text).expect("payload json");
    let forward = payload["mapping_payload"]["mapping"]["forward"]
        .as_object()
        .expect("forward map");
    assert!(!forward.is_empty(), "{payload}");
    assert!(forward.contains_key("users"), "{payload}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_list_project_tree_returns_directory_structure() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("app/sql")).expect("mkdirs");
    fs::write(project.join("app/sql/query.py"), "print('ok')").expect("write file");
    fs::write(project.join("README.md"), "# demo").expect("write file");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":40,
            "method":"tools/call",
            "params":{
                "name":"list_project_tree",
                "arguments":{
                    "root_dir": project.to_string_lossy().to_string(),
                    "max_depth": 5
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    assert!(response.get("error").is_none(), "{response}");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text payload");
    let payload: Value = serde_json::from_str(text).expect("payload json");
    let entries = payload["entries"].as_array().expect("entries");
    let paths = entries
        .iter()
        .filter_map(|e| e["path"].as_str())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"app/sql/query.py"), "{payload}");
    assert!(paths.contains(&"app/sql"), "{payload}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_obfuscate_project_from_paths_reads_files_in_mcp() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("app")).expect("mkdirs");
    fs::write(
        project.join("app/query.py"),
        "QUERY_1 = \"\"\"\nselect 1 from bs.users u where u.id in %(bs_user_ids)s\n\"\"\"\n",
    )
    .expect("write file");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":41,
            "method":"tools/call",
            "params":{
                "name":"obfuscate_project_from_paths",
                "arguments":{
                    "root_dir": project.to_string_lossy().to_string(),
                    "file_paths":["app/query.py"],
                    "manual_mapping":{"bs":"mmm"}
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    assert!(response.get("error").is_none(), "{response}");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text payload");
    let payload: Value = serde_json::from_str(text).expect("payload json");
    let content = payload["obfuscated_files"][0]["content"]
        .as_str()
        .expect("content");
    assert!(content.contains("select 1 from mmm.users"), "{payload}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_deobfuscate_project_from_paths_reads_files_in_mcp() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("app")).expect("mkdirs");
    fs::write(
        project.join("app/query.py"),
        "QUERY_1 = \"\"\"\nselect 1 from mmm.users u where u.id in %(bs_user_ids)s\n\"\"\"\n",
    )
    .expect("write file");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_ALLOW_DIRECT_DEOBFUSCATION", "true")
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":42,
            "method":"tools/call",
            "params":{
                "name":"deobfuscate_project_from_paths",
                "arguments":{
                    "root_dir": project.to_string_lossy().to_string(),
                    "file_paths":["app/query.py"],
                    "mapping_payload":{
                        "mapping":{
                            "forward":{"bs":"mmm"},
                            "reverse":{"mmm":"bs"}
                        },
                        "created_at_epoch_s": 1
                    }
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    assert!(response.get("error").is_none(), "{response}");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text payload");
    let payload: Value = serde_json::from_str(text).expect("payload json");
    let content = payload["restored_files"][0]["content"]
        .as_str()
        .expect("content");
    assert!(content.contains("select 1 from bs.users"), "{payload}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_direct_deobfuscation_disabled_by_default() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":420,
            "method":"tools/list",
            "params":{}
        }),
    );
    let list_response = read_response(&mut stdout);
    assert!(list_response.get("error").is_none(), "{list_response}");
    let tools = list_response["result"]["tools"]
        .as_array()
        .expect("tools array");
    assert!(
        tools.iter().all(|t| t["name"] != "deobfuscate_project"),
        "{list_response}"
    );

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":421,
            "method":"tools/call",
            "params":{
                "name":"deobfuscate_project",
                "arguments":{
                    "llm_output_files":[{"path":"a.py","content":"print('x')"}]
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    let error = response["error"]["message"].as_str().unwrap_or_default();
    assert!(error.contains("disabled"), "{response}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_http_updates_default_mapping() {
    let _guard = http_test_lock();
    let dir = tempdir().expect("tmp");
    let mapping_path = dir.path().join("mapping.default.json");
    fs::write(&mapping_path, "{}").expect("init mapping");
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_DEFAULT_MAPPING_PATH", &mapping_path)
        .env("MCP_HTTP_ADDR", &addr)
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    wait_http_ready(&addr);

    let resp = http_request(
        &addr,
        "PUT",
        "/mapping",
        Some(r#"{"mapping":{"run_order":"x_run"}}"#),
    );
    let (status, _) = http_status_and_json(&resp);
    assert_eq!(status, 200, "{resp}");

    let saved = fs::read_to_string(&mapping_path).expect("saved mapping");
    assert!(saved.contains("run_order"), "{saved}");

    kill_and_wait(&mut child);
}

#[test]
fn mcp_http_jsonrpc_over_root_and_mcp_path() {
    let _guard = http_test_lock();
    let dir = tempdir().expect("tmp");
    let mapping_path = dir.path().join("mapping.default.json");
    fs::write(&mapping_path, r#"{"bs":"mmm"}"#).expect("write mapping");
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_DEFAULT_MAPPING_PATH", &mapping_path)
        .env("MCP_HTTP_ADDR", &addr)
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    wait_http_ready(&addr);

    let init_body = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
    let init_resp = http_request(&addr, "POST", "/", Some(&init_body.to_string()));
    let (init_status, init_json) = http_status_and_json(&init_resp);
    assert_eq!(init_status, 200, "{init_resp}");
    assert_eq!(
        init_json["result"]["serverInfo"]["name"].as_str(),
        Some("code-obfuscator-mcp")
    );

    let tool_body = json!({
        "jsonrpc":"2.0",
        "id":2,
        "method":"tools/call",
        "params":{
            "name":"obfuscate_project",
            "arguments":{
                "project_files":[{"path":"query.py","content":"QUERY_1 = \"\"\"\nselect 1 from bs.users u where u.id in %(bs_user_ids)s\n\"\"\"\n"}]
            }
        }
    });
    let tool_resp = http_request(&addr, "POST", "/mcp", Some(&tool_body.to_string()));
    let (tool_status, tool_json) = http_status_and_json(&tool_resp);
    assert_eq!(tool_status, 200, "{tool_resp}");

    let text = tool_json["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result text");
    let payload: Value = serde_json::from_str(text).expect("tool payload");
    let content = payload["obfuscated_files"][0]["content"]
        .as_str()
        .expect("obfuscated content");
    assert!(content.contains("select 1 from mmm.users"), "{payload}");
    assert!(!content.contains("py_var_"), "{payload}");

    kill_and_wait(&mut child);
}

#[test]
fn mcp_deobfuscate_uses_default_mapping_when_payload_missing() {
    let dir = tempdir().expect("tmp");
    let mapping_path = dir.path().join("mapping.default.json");
    fs::write(&mapping_path, r#"{"run_order":"x_run"}"#).expect("write mapping");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_ALLOW_DIRECT_DEOBFUSCATION", "true")
        .env("MCP_DEFAULT_MAPPING_PATH", &mapping_path)
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":11,
            "method":"tools/call",
            "params":{
                "name":"deobfuscate_project",
                "arguments":{
                    "llm_output_files":[{"path":"a.py","content":"def x_run(order_id):\n    return order_id\n"}]
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    assert!(response.get("error").is_none(), "{response}");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text payload");
    let payload: Value = serde_json::from_str(text).expect("payload json");
    let restored = payload["restored_files"][0]["content"]
        .as_str()
        .expect("restored content");
    assert!(restored.contains("run_order"), "{payload}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_deobfuscate_fails_fast_on_missing_tokens() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_ALLOW_DIRECT_DEOBFUSCATION", "true")
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":10,
            "method":"tools/call",
            "params":{
                "name":"deobfuscate_project",
                "arguments":{
                    "llm_output_files":[{"path":"a.py","content":"print('nothing')"}],
                    "mapping_payload":{
                        "mapping":{
                            "forward":{"run_order":"x_run"},
                            "reverse":{"x_run":"run_order"}
                        },
                        "created_at_epoch_s": 1
                    }
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    let error_text = response["error"]["message"].as_str().unwrap_or_default();
    assert!(error_text.contains("fail-fast"), "{response}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_logs_stdio_requests_and_responses() {
    let dir = tempdir().expect("tmp");
    let log_dir = dir.path().join("logs");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_DIR", &log_dir)
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":101,"method":"tools/list","params":{}}),
    );
    let _ = read_response(&mut stdout);

    drop(stdin);
    let _ = child.wait();

    let log = read_log(&log_dir);
    assert!(log.contains("\"transport\":\"stdio\""), "{log}");
    assert!(log.contains("\"direction\":\"request\""), "{log}");
    assert!(log.contains("\"direction\":\"response\""), "{log}");
}

#[test]
fn mcp_logs_http_requests_and_responses() {
    let _guard = http_test_lock();
    let dir = tempdir().expect("tmp");
    let log_dir = dir.path().join("logs");
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_HTTP_ADDR", &addr)
        .env("MCP_LOG_DIR", &log_dir)
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    wait_http_ready(&addr);

    let health_resp = http_request(&addr, "GET", "/health", None);
    let (health_status, _) = http_status_and_json(&health_resp);
    assert_eq!(health_status, 200, "{health_resp}");

    let init_body = json!({"jsonrpc":"2.0","id":201,"method":"initialize","params":{}});
    let init_resp = http_request(&addr, "POST", "/mcp", Some(&init_body.to_string()));
    let (init_status, _) = http_status_and_json(&init_resp);
    assert_eq!(init_status, 200, "{init_resp}");

    kill_and_wait(&mut child);

    let log = read_log(&log_dir);
    assert!(log.contains("\"transport\":\"http-admin\""), "{log}");
    assert!(log.contains("\"transport\":\"http-mcp\""), "{log}");
    assert!(log.contains("\"method\":\"initialize\""), "{log}");
}

#[test]
fn mcp_log_file_rotation() {
    let dir = tempdir().expect("tmp");
    let log_dir = dir.path().join("logs");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_DIR", &log_dir)
        .env("MCP_LOG_MAX_BYTES", "700")
        .env("MCP_LOG_MAX_FILES", "2")
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    for id in 0..25 {
        send_request(
            &mut stdin,
            &json!({
                "jsonrpc":"2.0",
                "id": id,
                "method":"tools/call",
                "params":{
                    "name":"obfuscate_project",
                    "arguments":{
                        "project_files":[{"path":"q.sql","content": format!("select 1 from bs.users where name = '{}'", "x".repeat(80))}],
                        "manual_mapping":{"bs":"mmm"}
                    }
                }
            }),
        );
        let _ = read_response(&mut stdout);
    }

    drop(stdin);
    let _ = child.wait();

    assert!(
        log_dir.join("mcp-server.log.1").exists(),
        "rotation file missing in {}",
        log_dir.display()
    );
}
