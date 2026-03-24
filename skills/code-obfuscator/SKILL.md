---
name: code-obfuscator
type: skill
description: MCP git-style obfuscation workflow (`ls_tree`, `ls_files`, `pull`, `clone`, `status`, `push`) with cross-IDE execution patterns for Codex, Claude, and Gemini.
inputs:
  - task_goal
  - source_root
  - workspace_dir
  - request_id
outputs:
  - changed_files
  - status_summary
  - push_result
allowed-tools: Read, Write, Edit, Bash, Grep, Glob
---

# Code Obfuscator MCP Skill

## Purpose

Use this skill when an agent must safely edit code through obfuscated context and apply results back using MCP git-style semantics.

This skill is designed for:
1. Codex Agent IDE / codex CLI
2. Claude with MCP connector
3. Gemini with MCP-compatible connector/bridge

## Canonical Tool Surface

Only use these MCP tools:
1. `ls_tree`
2. `ls_files`
3. `pull`
4. `clone`
5. `status`
6. `push`

Never use legacy names:
1. `list_project_tree`
2. `obfuscate_project`
3. `obfuscate_project_from_paths`
4. `apply_llm_output`
5. `deobfuscate_project`
6. `deobfuscate_project_from_paths`

## Core Invariants

1. Reuse one `options.request_id` for the full lifecycle.
2. Keep MCP path arguments stable across `clone -> status -> push`.
3. Edit only obfuscated workspace files.
4. Always run `status` before `push` and after `push`.
5. Expect `clean=true` after successful `push`.

## Standard Execution Flow

### Flow A: Full workspace (recommended)

1. Discover files:
```text
tools/list
ls_tree(root_dir)
ls_files(root_dir)
```
2. Create obfuscated workspace snapshot:
```text
clone(root_dir, workspace_dir, options.request_id)
```
3. Edit obfuscated files in workspace.
4. Validate pending delta:
```text
status(workspace_dir, options.request_id)
```
5. Apply round-trip:
```text
push(workspace_dir, options.request_id)
```
6. Verify clean state:
```text
status(workspace_dir, options.request_id)
```

### Flow B: Partial context (targeted)

1. `pull(root_dir, file_paths, options.request_id)` for scoped obfuscation.
2. If edits must be applied to source tree, switch to `clone` flow for workspace materialization and `push`.

## Path Model (Important)

Many IDE sandboxes cannot write MCP runtime paths (for example `/workspace/...`) directly from local shell tools.

Use dual-path model when needed:
1. MCP calls: runtime path (example `/workspace/projects/test-obf`).
2. Shell edits: host mirror path mounted to that runtime directory.

If sandbox blocks writes, do not skip `push`. Re-map workspace path to writable host mirror and retry full flow.

## Safety Guardrails

1. Reject or stop on any tool failure.
2. Do not bypass `status` checks.
3. Do not write outside project root.
4. Do not follow symlinks for writes/deletes.
5. Do not alter mapping payload manually in client prompts.
6. Keep operation request-scoped by `request_id`.

## Prompt Contract For Agents

When instructing another agent, require:
1. strict tool order: `ls_tree -> clone -> status -> push`
2. one fixed `request_id`
3. explicit `root_dir` and `workspace_dir`
4. explicit requirement for `clean=true` after push
5. stop-on-error with exact error text

## Cross-IDE Playbooks

See detailed recipes in:
1. `references/tool-cheatsheet.md`
2. `references/ide-playbooks.md`
3. `references/server-config-examples.md`
4. `references/prompt-templates.md`

Agent-specific policy profiles:
1. `agents/codex.yaml`
2. `agents/claude.yaml`
3. `agents/gemini.yaml`

## Quick Acceptance Checklist

Before finishing task:
1. `tools/list` exposes only git-style tools.
2. Edited files were obfuscated workspace files, not source originals.
3. `status` before push shows expected diff.
4. `push` returns applied/deleted lists.
5. Final `status` is clean.
6. Final response includes outcome summary with changed paths.

## Troubleshooting Entry Points

1. `unknown request_id` -> wrong `request_id` or expired session.
2. `root_dir does not exist` -> wrong runtime path/mount.
3. `refusing to write through symlink` -> source path guardrail triggered.
4. push writes denied -> source volume mounted read-only.
5. no diff before push -> workspace file edits were not applied to bound workspace.
