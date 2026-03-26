use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
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

fn call_tool(
    stdin: &mut impl Write,
    stdout: &mut impl Read,
    id: i64,
    name: &str,
    arguments: Value,
) -> Value {
    send_request(
        stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":id,
            "method":"tools/call",
            "params":{
                "name":name,
                "arguments":arguments
            }
        }),
    );
    read_response(stdout)
}

fn parse_tool_payload(resp: &Value) -> Value {
    assert!(resp.get("error").is_none(), "{resp}");
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("tool text");
    serde_json::from_str(text).expect("tool payload")
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind free port");
    listener.local_addr().expect("local addr").port()
}

fn http_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().expect("lock")
}

fn wait_http_ready(child: &mut Child, addr: &str) -> bool {
    for _ in 0..40 {
        if TcpStream::connect(addr).is_ok() {
            return true;
        }
        if child.try_wait().expect("poll http child").is_some() {
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
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
        serde_json::from_str(body).expect("parse body json")
    };
    (status, body_json)
}

fn kill_and_wait(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn read_pipe(mut pipe: impl Read) -> String {
    let mut text = String::new();
    pipe.read_to_string(&mut text).expect("read pipe");
    text
}

fn find_tool<'a>(tools: &'a [Value], name: &str) -> &'a Value {
    tools
        .iter()
        .find(|tool| tool["name"] == name)
        .unwrap_or_else(|| panic!("tool {name} not found: {tools:?}"))
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

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":"init-1",
            "method":"initialize",
            "params":{"protocolVersion":"2025-11-25"}
        }),
    );
    let response = read_response(&mut stdout);
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
            "id":1,
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
            "id":2,
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
fn mcp_stdio_logs_to_stderr_without_corrupting_stdout() {
    let dir = tempdir().expect("tmp");
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_DIR", dir.path().join("logs"))
        .env("MCP_LOG_MODE", "default")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":"stdio-log-1",
            "method":"initialize",
            "params":{"protocolVersion":"2025-11-25"}
        }),
    );
    let response = read_response(&mut stdout);
    assert!(response.get("error").is_none(), "{response}");

    drop(stdin);
    let _ = child.wait();

    let stderr_text = read_pipe(stderr);
    assert!(stderr_text.contains("REQUEST"), "{stderr_text}");
    assert!(stderr_text.contains("transport: stdio"), "{stderr_text}");
    assert!(stderr_text.contains("method: initialize"), "{stderr_text}");
}

#[test]
fn mcp_stdio_error_logs_include_backtrace() {
    let dir = tempdir().expect("tmp");
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_DIR", dir.path().join("logs"))
        .env("MCP_LOG_MODE", "default")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":"stdio-log-2",
            "method":"tools/call",
            "params":{"name":"legacy_pull","arguments":{}}
        }),
    );
    let response = read_response(&mut stdout);
    assert!(response["error"].is_object(), "{response}");

    drop(stdin);
    let _ = child.wait();

    let stderr_text = read_pipe(stderr);
    assert!(stderr_text.contains("ERROR"), "{stderr_text}");
    assert!(stderr_text.contains("backtrace:"), "{stderr_text}");
    assert!(
        stderr_text.contains("unknown or disabled tool"),
        "{stderr_text}"
    );
}

#[test]
fn mcp_tools_list_exposes_git_style_tools_only() {
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
            "method":"tools/list",
            "params":{}
        }),
    );

    let response = read_response(&mut stdout);
    assert!(response.get("error").is_none(), "{response}");

    let tools = response["result"]["tools"].as_array().expect("tools array");
    let names = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();

    assert!(names.contains(&"ls_tree"), "{response}");
    assert!(names.contains(&"ls_files"), "{response}");
    assert!(names.contains(&"pull"), "{response}");
    assert!(names.contains(&"clone"), "{response}");
    assert!(names.contains(&"status"), "{response}");
    assert!(names.contains(&"push"), "{response}");

    assert!(!names.contains(&"list_project_tree"), "{response}");
    assert!(!names.contains(&"obfuscate_project"), "{response}");
    assert!(
        !names.contains(&"obfuscate_project_from_paths"),
        "{response}"
    );
    assert!(!names.contains(&"apply_llm_output"), "{response}");
    assert!(!names.contains(&"deobfuscate_project"), "{response}");

    let pull = find_tool(tools, "pull");
    assert!(
        pull["inputSchema"]["properties"]["options"]["required"]
            .as_array()
            .expect("options required")
            .iter()
            .any(|value| value.as_str() == Some("request_id")),
        "{pull}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_ls_tree_and_ls_files_respect_hidden_and_truncation() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("app")).expect("mkdirs");
    fs::write(project.join("app/a.py"), "print('a')\n").expect("write");
    fs::write(project.join("app/b.py"), "print('b')\n").expect("write");
    fs::write(project.join(".secret.py"), "print('secret')\n").expect("write");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    let ls_tree_small = call_tool(
        &mut stdin,
        &mut stdout,
        4,
        "ls_tree",
        json!({
            "root_dir": project.to_string_lossy().to_string(),
            "max_entries": 1
        }),
    );
    let tree_payload_small = parse_tool_payload(&ls_tree_small);
    assert_eq!(tree_payload_small["truncated"].as_bool(), Some(true));
    let entries_small = tree_payload_small["entries"].as_array().expect("entries");
    assert_eq!(entries_small.len(), 1, "{tree_payload_small}");

    let ls_files_default = call_tool(
        &mut stdin,
        &mut stdout,
        5,
        "ls_files",
        json!({
            "root_dir": project.to_string_lossy().to_string(),
            "max_entries": 1
        }),
    );
    let files_payload_default = parse_tool_payload(&ls_files_default);
    assert_eq!(files_payload_default["truncated"].as_bool(), Some(true));
    let files_default = files_payload_default["files"].as_array().expect("files");
    assert_eq!(files_default.len(), 1, "{files_payload_default}");
    assert!(
        files_default.iter().all(|item| {
            !item
                .as_str()
                .unwrap_or_default()
                .split('/')
                .any(|part| part.starts_with('.'))
        }),
        "{files_payload_default}"
    );

    let ls_files_hidden = call_tool(
        &mut stdin,
        &mut stdout,
        6,
        "ls_files",
        json!({
            "root_dir": project.to_string_lossy().to_string(),
            "include_hidden": true,
            "max_entries": 20
        }),
    );
    let files_payload_hidden = parse_tool_payload(&ls_files_hidden);
    let files_hidden = files_payload_hidden["files"].as_array().expect("files");
    assert!(
        files_hidden
            .iter()
            .any(|item| item.as_str() == Some(".secret.py")),
        "{files_payload_hidden}"
    );
    assert_eq!(files_payload_hidden["truncated"].as_bool(), Some(false));

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_legacy_tool_names_are_rejected() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    let response = call_tool(
        &mut stdin,
        &mut stdout,
        10,
        "obfuscate_project",
        json!({
            "project_files":[{"path":"a.py","content":"print('x')"}],
            "options":{"request_id":"legacy-1"}
        }),
    );
    let message = response["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("unknown or disabled tool"), "{response}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_pull_obfuscates_root_dir_subset() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("app")).expect("mkdirs");
    fs::write(project.join("app/query.py"), "select * from bs.users\n").expect("write");
    fs::write(project.join("app/ignore.txt"), "hello\n").expect("write");

    let mapping_path = dir.path().join("mapping.default.json");
    fs::write(&mapping_path, r#"{"bs":"mmm"}"#).expect("write mapping");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_DEFAULT_MAPPING_PATH", &mapping_path)
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    let response = call_tool(
        &mut stdin,
        &mut stdout,
        11,
        "pull",
        json!({
            "root_dir": project.to_string_lossy().to_string(),
            "file_paths": ["app/query.py"],
            "options": {"request_id":"pull-1"}
        }),
    );

    let payload = parse_tool_payload(&response);
    let files = payload["obfuscated_files"].as_array().expect("files");
    assert_eq!(files.len(), 1, "{payload}");
    assert_eq!(files[0]["path"].as_str(), Some("app/query.py"));
    assert!(
        files[0]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("mmm.users"),
        "{payload}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_gitignore_filters_ls_files_and_pull_clone() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    let workspace = dir.path().join("workspace");
    fs::create_dir_all(project.join("app")).expect("mkdirs");
    fs::create_dir_all(project.join("ignored_dir")).expect("mkdirs");
    fs::write(project.join(".gitignore"), "ignored_dir/\n*.secret\n").expect("write");
    fs::write(project.join("app/query.py"), "select * from bs.users\n").expect("write");
    fs::write(
        project.join("ignored_dir/drop.py"),
        "select * from bs.audit\n",
    )
    .expect("write");
    fs::write(project.join("app/token.secret"), "xxx\n").expect("write");

    let mapping_path = dir.path().join("mapping.default.json");
    fs::write(&mapping_path, r#"{"bs":"mmm"}"#).expect("write mapping");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_DEFAULT_MAPPING_PATH", &mapping_path)
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    let files_response = call_tool(
        &mut stdin,
        &mut stdout,
        30,
        "ls_files",
        json!({
            "root_dir": project.to_string_lossy().to_string(),
            "max_entries": 20
        }),
    );
    let files_payload = parse_tool_payload(&files_response);
    let files = files_payload["files"].as_array().expect("files");
    assert!(
        files.iter().any(|f| f.as_str() == Some("app/query.py")),
        "{files_payload}"
    );
    assert!(
        files
            .iter()
            .all(|f| f.as_str() != Some("ignored_dir/drop.py")
                && f.as_str() != Some("app/token.secret")),
        "{files_payload}"
    );

    let pull_response = call_tool(
        &mut stdin,
        &mut stdout,
        31,
        "pull",
        json!({
            "root_dir": project.to_string_lossy().to_string(),
            "options": {"request_id":"gitignore-pull-1"}
        }),
    );
    let pull_payload = parse_tool_payload(&pull_response);
    let pull_files = pull_payload["obfuscated_files"]
        .as_array()
        .expect("pull files");
    assert!(
        pull_files
            .iter()
            .any(|f| f["path"].as_str() == Some("app/query.py")),
        "{pull_payload}"
    );
    assert!(
        pull_files
            .iter()
            .all(|f| f["path"].as_str() != Some("ignored_dir/drop.py")
                && f["path"].as_str() != Some("app/token.secret")),
        "{pull_payload}"
    );

    let clone_response = call_tool(
        &mut stdin,
        &mut stdout,
        32,
        "clone",
        json!({
            "root_dir": project.to_string_lossy().to_string(),
            "workspace_dir": workspace.to_string_lossy().to_string(),
            "options": {"request_id":"gitignore-clone-1"}
        }),
    );
    let clone_payload = parse_tool_payload(&clone_response);
    let cloned_files = clone_payload["cloned_files"].as_array().expect("cloned");
    assert!(
        cloned_files.iter().all(|f| {
            f.as_str() != Some("ignored_dir/drop.py") && f.as_str() != Some("app/token.secret")
        }),
        "{clone_payload}"
    );
    assert!(!workspace.join("ignored_dir/drop.py").exists());
    assert!(!workspace.join("app/token.secret").exists());

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_pull_skips_explicit_file_path_ignored_by_gitignore() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("app")).expect("mkdirs");
    fs::write(project.join(".gitignore"), "*.secret\n").expect("write");
    fs::write(project.join("app/token.secret"), "value\n").expect("write");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    let response = call_tool(
        &mut stdin,
        &mut stdout,
        33,
        "pull",
        json!({
            "root_dir": project.to_string_lossy().to_string(),
            "file_paths": ["app/token.secret"],
            "options": {"request_id":"gitignore-explicit-1"}
        }),
    );
    let payload = parse_tool_payload(&response);
    let files = payload["obfuscated_files"]
        .as_array()
        .expect("obfuscated_files");
    assert!(files.is_empty(), "{payload}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_clone_writes_full_obfuscated_tree_to_workspace() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    let workspace = dir.path().join("workspace");
    fs::create_dir_all(project.join("app")).expect("mkdirs");
    fs::write(project.join("app/query.py"), "select * from bs.users\n").expect("write");

    let mapping_path = dir.path().join("mapping.default.json");
    fs::write(&mapping_path, r#"{"bs":"mmm"}"#).expect("write mapping");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_DEFAULT_MAPPING_PATH", &mapping_path)
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    let response = call_tool(
        &mut stdin,
        &mut stdout,
        12,
        "clone",
        json!({
            "root_dir": project.to_string_lossy().to_string(),
            "workspace_dir": workspace.to_string_lossy().to_string(),
            "options": {"request_id":"clone-1"}
        }),
    );

    let payload = parse_tool_payload(&response);
    let cloned = payload["cloned_files"].as_array().expect("cloned files");
    assert!(
        cloned.iter().any(|f| f.as_str() == Some("app/query.py")),
        "{payload}"
    );

    let workspace_file =
        fs::read_to_string(workspace.join("app/query.py")).expect("workspace file");
    assert!(workspace_file.contains("mmm.users"), "{workspace_file}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_status_reports_added_modified_deleted() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    let workspace = dir.path().join("workspace");
    fs::create_dir_all(project.join("app")).expect("mkdirs");
    fs::write(project.join("app/query.py"), "select * from bs.users\n").expect("write");
    fs::write(project.join("app/remove.py"), "print('remove')\n").expect("write");

    let mapping_path = dir.path().join("mapping.default.json");
    fs::write(&mapping_path, r#"{"bs":"mmm"}"#).expect("write mapping");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_DEFAULT_MAPPING_PATH", &mapping_path)
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    let clone_resp = call_tool(
        &mut stdin,
        &mut stdout,
        13,
        "clone",
        json!({
            "root_dir": project.to_string_lossy().to_string(),
            "workspace_dir": workspace.to_string_lossy().to_string(),
            "options": {"request_id":"status-1"}
        }),
    );
    assert!(clone_resp.get("error").is_none(), "{clone_resp}");

    let query_path = workspace.join("app/query.py");
    let query_content = fs::read_to_string(&query_path).expect("query");
    fs::write(&query_path, query_content.replace("select *", "select id")).expect("rewrite query");
    fs::remove_file(workspace.join("app/remove.py")).expect("remove tracked file");
    fs::write(workspace.join("app/new.py"), "print('mmm record')\n").expect("add file");

    let status_resp = call_tool(
        &mut stdin,
        &mut stdout,
        14,
        "status",
        json!({
            "workspace_dir": workspace.to_string_lossy().to_string(),
            "options": {"request_id":"status-1"}
        }),
    );
    let payload = parse_tool_payload(&status_resp);
    assert_eq!(payload["clean"].as_bool(), Some(false), "{payload}");

    let modified = payload["diff"]["modified"].as_array().expect("modified");
    let added = payload["diff"]["added"].as_array().expect("added");
    let deleted = payload["diff"]["deleted"].as_array().expect("deleted");

    assert!(
        modified.iter().any(|v| v.as_str() == Some("app/query.py")),
        "{payload}"
    );
    assert!(
        added.iter().any(|v| v.as_str() == Some("app/new.py")),
        "{payload}"
    );
    assert!(
        deleted.iter().any(|v| v.as_str() == Some("app/remove.py")),
        "{payload}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_push_applies_add_modify_delete() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    let workspace = dir.path().join("workspace");
    fs::create_dir_all(project.join("app")).expect("mkdirs");
    fs::write(project.join("app/query.py"), "select * from bs.users\n").expect("write");
    fs::write(project.join("app/remove.py"), "print('remove')\n").expect("write");

    let mapping_path = dir.path().join("mapping.default.json");
    fs::write(&mapping_path, r#"{"bs":"mmm"}"#).expect("write mapping");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_DEFAULT_MAPPING_PATH", &mapping_path)
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    let clone_resp = call_tool(
        &mut stdin,
        &mut stdout,
        15,
        "clone",
        json!({
            "root_dir": project.to_string_lossy().to_string(),
            "workspace_dir": workspace.to_string_lossy().to_string(),
            "options": {"request_id":"push-1"}
        }),
    );
    assert!(clone_resp.get("error").is_none(), "{clone_resp}");

    let query_path = workspace.join("app/query.py");
    let query_content = fs::read_to_string(&query_path).expect("query");
    fs::write(&query_path, query_content.replace("select *", "select id")).expect("rewrite query");
    fs::remove_file(workspace.join("app/remove.py")).expect("remove tracked file");
    fs::write(workspace.join("app/new.py"), "print('mmm created')\n").expect("add file");

    let push_resp = call_tool(
        &mut stdin,
        &mut stdout,
        16,
        "push",
        json!({
            "workspace_dir": workspace.to_string_lossy().to_string(),
            "options": {"request_id":"push-1"}
        }),
    );

    let payload = parse_tool_payload(&push_resp);
    let applied = payload["applied_files"].as_array().expect("applied files");
    let deleted = payload["deleted_files"].as_array().expect("deleted files");

    assert!(
        applied.iter().any(|v| v.as_str() == Some("app/query.py")),
        "{payload}"
    );
    assert!(
        applied.iter().any(|v| v.as_str() == Some("app/new.py")),
        "{payload}"
    );
    assert!(
        deleted.iter().any(|v| v.as_str() == Some("app/remove.py")),
        "{payload}"
    );

    let query_source = fs::read_to_string(project.join("app/query.py")).expect("source query");
    assert!(
        query_source.contains("select id from bs.users"),
        "{query_source}"
    );

    let new_source = fs::read_to_string(project.join("app/new.py")).expect("new source");
    assert!(new_source.contains("bs created"), "{new_source}");

    assert!(
        !project.join("app/remove.py").exists(),
        "removed file must not exist after push"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_http_jsonrpc_supports_new_pull_tool() {
    let _guard = http_test_lock();
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("app")).expect("mkdirs");
    fs::write(project.join("app/query.py"), "select * from bs.users\n").expect("write");

    let mapping_path = dir.path().join("mapping.default.json");
    fs::write(&mapping_path, r#"{"bs":"mmm"}"#).expect("write mapping");
    let (addr, mut child) = (0..8)
        .find_map(|_| {
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
            if wait_http_ready(&mut child, &addr) {
                Some((addr, child))
            } else {
                kill_and_wait(&mut child);
                None
            }
        })
        .expect("start http server on a free port");

    let body = json!({
        "jsonrpc":"2.0",
        "id":1,
        "method":"tools/call",
        "params":{
            "name":"pull",
            "arguments":{
                "root_dir": project.to_string_lossy().to_string(),
                "file_paths":["app/query.py"],
                "options":{"request_id":"http-pull-1"}
            }
        }
    });

    let resp = http_request(&addr, "POST", "/mcp", Some(&body.to_string()));
    let (status, json_body) = http_status_and_json(&resp);
    assert_eq!(status, 200, "{resp}");

    let text = json_body["result"]["content"][0]["text"]
        .as_str()
        .expect("tool text");
    let payload: Value = serde_json::from_str(text).expect("payload");
    let content = payload["obfuscated_files"][0]["content"]
        .as_str()
        .expect("content");
    assert!(content.contains("mmm.users"), "{payload}");

    kill_and_wait(&mut child);
}

#[test]
fn mcp_http_only_mode_disables_stdio_lifecycle_and_serves_requests() {
    let _guard = http_test_lock();
    let (addr, mut child) = (0..8)
        .find_map(|_| {
            let port = free_port();
            let addr = format!("127.0.0.1:{port}");
            let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
                .env("MCP_HTTP_ADDR", &addr)
                .env("MCP_DISABLE_STDIO", "true")
                .env("MCP_LOG_MODE", "system")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn server");
            if wait_http_ready(&mut child, &addr) {
                Some((addr, child))
            } else {
                kill_and_wait(&mut child);
                None
            }
        })
        .expect("start http-only server on a free port");

    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let body = json!({
        "jsonrpc":"2.0",
        "id":"http-only-init",
        "method":"initialize",
        "params":{"protocolVersion":"2025-11-25"}
    });

    let resp = http_request(&addr, "POST", "/mcp", Some(&body.to_string()));
    let (status, json_body) = http_status_and_json(&resp);
    assert_eq!(status, 200, "{resp}");
    assert_eq!(
        json_body["result"]["protocolVersion"].as_str(),
        Some("2025-11-25")
    );

    kill_and_wait(&mut child);

    let stdout_text = read_pipe(stdout);
    let stderr_text = read_pipe(stderr);
    assert!(stdout_text.contains("transport: http"), "{stdout_text}");
    assert!(!stdout_text.contains("transport: stdio"), "{stdout_text}");
    assert!(!stderr_text.contains("transport: stdio"), "{stderr_text}");
}
