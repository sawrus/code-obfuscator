codex mcp remove code_obfuscator
codex mcp add code_obfuscator -- \
  docker run --rm -i \
  -e MCP_DEFAULT_MAPPING_PATH=/data/mapping.default.json \
  -e MCP_LOG_STDOUT=false \
  -v $HOME/mcp/code-obfuscator/mapping.default.json:/data/mapping.default.json:ro \
  -v $HOME/projects:/workspace/projects:ro \
  code-obfuscator-mcp:local
