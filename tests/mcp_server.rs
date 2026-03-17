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

fn send_request_lowercase_header(stdin: &mut impl Write, req: &Value) {
    let body = serde_json::to_vec(req).expect("serialize req");
    let header = format!("content-length: {}\r\n\r\n", body.len());
    stdin.write_all(header.as_bytes()).expect("write header");
    stdin.write_all(&body).expect("write body");
    stdin.flush().expect("flush");
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
                "name":"deobfuscate_project",
                "arguments":{
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
    let first_content = restored["restored_files"][0]["content"]
        .as_str()
        .expect("content");

    assert!(first_content.contains("run_order"));
    assert!(first_content.contains("order_id"));

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
    fs::write(&mapping_path, r#"{"mostbet":"mmm"}"#).expect("write mapping");
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
                "project_files":[{"path":"query.py","content":"QUERY_1 = \"\"\"\nselect 1 from mostbet.users u where u.id in %(mostbet_user_ids)s\n\"\"\"\n"}]
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
                        "project_files":[{"path":"q.sql","content": format!("select 1 from mostbet.users where name = '{}'", "x".repeat(80))}],
                        "manual_mapping":{"mostbet":"mmm"}
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
