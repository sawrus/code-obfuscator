# code-obfuscator

MCP server and CLI/TUI utility for safe code obfuscation before LLM usage and reverse application of LLM changes back to your project.

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
flowchart LR
    user["User"] --> ide["Agent IDE<br/>e.g. Codex"]
    ide --> llm["LLM"]
    ide --> mcp["MCP server<br/>code-obfuscator"]
    mcp --> project["Project files on disk"]
    mcp -. "obfuscated_files / llm_output_files" .- llm
```

```mermaid
sequenceDiagram
    actor User
    participant IDE as Agent IDE (Codex)
    participant MCP as MCP server
    participant LLM
    participant Project as Project / root_dir

    User->>IDE: Request a code change
    IDE->>MCP: list_project_tree / obfuscate_project_from_paths(..., options.request_id)
    MCP->>Project: Read selected files
    Project-->>MCP: Source files
    MCP-->>IDE: obfuscated_files
    IDE->>LLM: Send obfuscated_files only
    LLM-->>IDE: Return modified obfuscated_files
    IDE->>MCP: apply_llm_output(root_dir, llm_output_files, options.request_id)
    MCP->>MCP: Deobfuscate LLM output
    MCP->>Project: Apply restored files in root_dir
    MCP-->>IDE: applied_files
    IDE-->>User: Show result
```

## CLI Quick Start

### install

```bash
curl -fsSL https://raw.githubusercontent.com/sawrus/code-obfuscator/main/install | CODE_OBFUSCATOR_INSTALL_REPO=sawrus/code-obfuscator bash
```

Binaries are installed from GitHub Releases: [sawrus/code-obfuscator/releases](https://github.com/sawrus/code-obfuscator/releases).

### execute

```bash
code-obfuscator
```

## MCP Quick Start

### build

```bash
make mcp-docker-build
```

### start

```bash
MCP_HTTP_ADDR=127.0.0.1:18787 \
MCP_DEFAULT_MAPPING_PATH=./mapping.default.json \
./scripts/run-mcp-docker.sh
```

### health check

```bash
curl -i http://127.0.0.1:18787/health
```

## Detailed Documentation

- Full documentation (install lifecycle, CLI/TUI modes, MCP integrations, architecture, troubleshooting): [docs/DETAILS.md](docs/DETAILS.md)
- Security and performance: [docs/SECURITY_AND_PERFORMANCE.md](docs/SECURITY_AND_PERFORMANCE.md)
- Samples: [docs/SAMPLES.md](docs/SAMPLES.md)
- MCP server plan: [docs/MCP_SERVER_PLAN.md](docs/MCP_SERVER_PLAN.md)

## Development

```bash
make build
make test
```
