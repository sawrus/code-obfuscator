# code-obfuscator

MCP server and CLI/TUI utility for safe code obfuscation before LLM usage and reverse application of LLM changes back to
your project.

## Architecture Diagrams

### code obfuscator case

```mermaid
sequenceDiagram
    actor User
    participant CodeObfuscator as code obfuscator
    participant AgentIDE as Agent IDE (Codex)
    participant LLM
    User->>CodeObfuscator: Prepare full project in obfuscated form
    CodeObfuscator->>CodeObfuscator: Obfuscate selected project files
    CodeObfuscator-->>AgentIDE: Return obfuscated project context
    AgentIDE->>LLM: Send obfuscated context for implementation
```

### MCP Case

```mermaid
sequenceDiagram
    actor User
    participant IDE as Agent IDE (Codex)
    participant MCP as MCP server
    participant LLM
    participant Project as Project / root_dir

    User->>IDE: Request a code change
    IDE->>MCP: ls_tree / pull(..., options.request_id) or clone(..., options.request_id)
    MCP->>Project: Read source files
    Project-->>MCP: Source files
    MCP-->>IDE: obfuscated_files
    IDE->>LLM: Send obfuscated_files only
    LLM-->>IDE: Return modified obfuscated_files
    IDE->>MCP: status(workspace_dir, options.request_id) / push(workspace_dir, options.request_id)
    MCP->>MCP: Deobfuscate and apply workspace delta
    MCP->>Project: Apply add/modify/delete in source root
    MCP-->>IDE: applied_files / deleted_files
    IDE-->>User: Show result
```

## CLI Quick Start

### install

```bash
curl -fsSL https://raw.githubusercontent.com/sawrus/code-obfuscator/main/install | CODE_OBFUSCATOR_INSTALL_REPO=sawrus/code-obfuscator bash
```

Binaries are installed from GitHub
Releases: [sawrus/code-obfuscator/releases](https://github.com/sawrus/code-obfuscator/releases).

### execute

```bash
code-obfuscator
```

## MCP Quick Start

### build

```bash
make mcp-docker-build
```

### codex (stdio)

Register MCP server in Codex:

```bash
codex mcp remove code_obfuscator >/dev/null 2>&1 || true
PROJECTS_HOST_DIR="${MCP_PROJECTS_HOST_DIR:-$HOME/projects}"
CONTAINER_NAME="${CONTAINER_NAME:-code-obfuscator-mcp}"
codex mcp add code_obfuscator -- \
  docker run --rm -i --name "$CONTAINER_NAME" \
  -e MCP_DEFAULT_MAPPING_PATH=/data/mapping.default.json \
  -e MCP_LOG_STDOUT=false \
  -v "$HOME/mcp/code-obfuscator/mapping.default.json:/data/mapping.default.json:ro" \
  -v "$PROJECTS_HOST_DIR:/workspace/projects:rw" \
  code-obfuscator-mcp:local
```

This command only saves the MCP configuration. It does not start the container immediately.
Codex launches the stdio server on first MCP use.

### codex (http)

1. Start the HTTP MCP server:

```bash
MCP_HTTP_ADDR=127.0.0.1:18787 \
MCP_PROJECTS_HOST_DIR=$HOME/projects \
MCP_DEFAULT_MAPPING_PATH=$HOME/mcp/code-obfuscator/mapping.default.json \
./scripts/run-mcp-docker.sh
```

2. Register the HTTP endpoint in Codex:

```bash
codex mcp remove code_obfuscator >/dev/null 2>&1 || true
codex mcp add code_obfuscator --url http://127.0.0.1:18787/mcp
```

For `http`, Codex connects to the already running endpoint. Unlike `stdio`, Codex does not start the server for you.

`MCP_PROJECTS_HOST_DIR` maps to `-v "<ABS_PATH>:/workspace/projects:rw"` inside Docker.

### health check

```bash
curl -i http://127.0.0.1:18787/health
```

## Detailed Documentation

- Full documentation (install lifecycle, CLI/TUI modes, MCP integrations, architecture,
  troubleshooting): [docs/DETAILS.md](docs/DETAILS.md)
- Security and performance: [docs/SECURITY_AND_PERFORMANCE.md](docs/SECURITY_AND_PERFORMANCE.md)
- Samples: [docs/SAMPLES.md](docs/SAMPLES.md)
- MCP server plan: [docs/MCP_TOOLS.md](docs/MCP_TOOLS.md)

## Development

```bash
make build
make test
```
