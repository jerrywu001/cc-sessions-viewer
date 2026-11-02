---
title: Agent Compatibility and Transcript Samples
description: Check supported session formats, chat and resume capabilities, evidence levels, and downloadable synthetic JSONL/SQLite examples with tested output.
---

# Which coding-agent formats are supported?

Sessions Viewer supports the seven layouts below. **Adapter review and synthetic command tests are not CLI end-to-end certification.** Most rows describe the 0.6.0 implementation reviewed on 2026-10-05; the OpenCode row also includes the v2 storage update linked below. This does not promise compatibility with every release.

## Capability and evidence matrix {#support-matrix}

All seven have history browsing/search, export and terminal resume. Only Claude Code and Codex have in-app chat. Resume requires the corresponding installed/configured CLI; syntax is documented in the [command comparison](/agents/#resume-commands).

| Agent | Format covered | In-app chat | Validation evidence | CLI release tested end-to-end |
| --- | --- | --- | --- | --- |
| [Claude Code](/agents/claude-code) | JSONL `message.content` strings/blocks | Yes | Synthetic extraction + [Rust parser projection](#offline-parser-tests) | Not validated in this docs pass |
| [Codex](/agents/codex) | `session_meta`, `event_msg`, `response_item` | Yes | Synthetic `cwd`/text + [Rust parser projection](#offline-parser-tests) | Not validated in this docs pass |
| [Grok Build](/agents/grok-build) | ACP updates in `updates.jsonl` | No | Synthetic object/array chunks | Not validated in this docs pass |
| [Kimi Code](/agents/kimi-code) | Main wire primary events; adapter has legacy fallback | No | Synthetic primary events only | Not validated in this docs pass |
| [Pi](/agents/pi) | Session header and message/tree entries | No | Synthetic text from all stored branches | Not validated in this docs pass |
| [Antigravity CLI](/agents/antigravity-cli) | Step-based `transcript*.jsonl` | No | Synthetic multiline XML extraction | Not validated in this docs pass |
| [opencode](/agents/opencode) | Legacy SQLite and v2 `session_v2` / `session_message` / optional `session_part` | No | Synthetic read-only SQL + Rust parser regression | Not validated in this docs pass |

OS installers are provided for macOS, Windows and Linux; the shell examples use macOS/Linux syntax. This is not a per-OS GUI certification. Antigravity's supported format has no usage fields. Older layouts, custom roots, images, tools, branches and provider behavior need the relevant reference page and [troubleshooting](/guide/troubleshooting).

## Download safe, reproducible examples {#synthetic-samples}

These are intentionally minimal **synthetic extraction fixtures**, not real conversations or full CLI-ready sessions. Optional image/tool fields are incomplete; do not import them as real history or resume them. Paths and prompts are invented. The JSONL/SQL files and <a href="/examples/expected.json" download>expected output manifest</a> are the same inputs checked by `npm run docs:test`.

Download a sample, then replace the filename in its agent reference's text-extraction command. Expected lines are listed in order:

| Agent | Download | Expected text |
| --- | --- | --- |
| Claude Code | <a href="/examples/claude-code.jsonl" download>claude-code.jsonl</a> | `Question`, `Answer`, `Legacy string` |
| Codex | <a href="/examples/codex.jsonl" download>codex.jsonl</a> | `Question`, `Answer`; metadata `cwd` is `/synthetic/project` |
| Grok Build | <a href="/examples/grok-build.jsonl" download>grok-build.jsonl</a> | `Question`, `Answer` |
| Kimi Code | <a href="/examples/kimi-code.jsonl" download>kimi-code.jsonl</a> | `Question`, `Answer`, `Legacy string` |
| Pi | <a href="/examples/pi.jsonl" download>pi.jsonl</a> | `Question`, `Answer`, `Other branch` (not active-branch-only) |
| Antigravity CLI | <a href="/examples/antigravity-cli.jsonl" download>antigravity-cli.jsonl</a> | `Question`, `on two lines` (one multiline prompt) |
| opencode (legacy) | <a href="/examples/opencode.sql" download>opencode.sql</a> | `user` and `Question` from the text query |
| opencode (v2) | <a href="/examples/opencode-v2.sql" download>opencode-v2.sql</a> | Raw rows: `user`, then `assistant`; the parser displays `Question`, then split-part `Answer` |

For example, after downloading `claude-code.jsonl`:

```bash
jq -r 'select(.type=="user" or .type=="assistant")
  | .message.content
  | if type=="string" then . else .[]? | select(.type=="text") | .text end' \
  claude-code.jsonl
```

```text
Question
Answer
Legacy string
```

The opencode sample is SQL that creates a reduced schema. Load it into a **new temporary database**, never your actual opencode database:

```bash
temp_dir="$(mktemp -d)"
sqlite3 "$temp_dir/synthetic.db" < opencode.sql
sqlite3 "file:$temp_dir/synthetic.db?mode=ro" \
  "SELECT json_extract(data, '$.text') FROM part WHERE json_extract(data, '$.type') = 'text';"
```

Expected output is `Question`. The regression suite also verifies that parent-only session listing excludes the sub-agent, cost aggregation includes it, and read queries leave the synthetic database unchanged. It does not validate provider billing or a real opencode migration.

The v2 sample uses an upstream session-table subset, not a complete native database. Load it only into a new temporary database. The [v2 reference](/agents/opencode) documents the query and the read-only limitation for rename, trash and restore.

## What do the offline parser tests prove? {#offline-parser-tests}

Keep three verification scopes separate:

1. **`jq` / SQL extraction** — `npm run docs:test` runs the documented commands on the small samples above. It also checks content/SEO regressions and screenshot integrity. Command output is not the application's parser output.
2. **Actual Rust file parsers** — on 2026-10-06, two isolated tests passed on macOS for the 0.6.0 Claude Code and Codex parsers and their message post-processing. They compare roles, text, Claude thinking, tool names/arguments/IDs/results and message order with a public expected projection. They also check that parsing leaves the temporary input unchanged. Codex receives an empty title index instead of loading the user's real one. No home discovery, CLI, tool execution or provider request runs.
3. **Real CLI / GUI end-to-end acceptance** — still separate and not performed here. Image behavior, browsing/search/export/resume, installed CLI releases and provider behavior are not certified by those parser fixtures. No verified CLI version has been added to the matrix.

| Offline parser fixture | Expected messages |
| --- | --- |
| <a href="/examples/claude-code-adapter.jsonl" download>claude-code-adapter.jsonl</a> | User question → assistant thinking/text/tool call → user tool result → legacy user string |
| <a href="/examples/codex-adapter.jsonl" download>codex-adapter.jsonl</a> | User question → assistant tool call → user tool result → assistant answer; duplicate response text and unrelated metadata are ignored |

Download <a href="/examples/adapter-expected.json" download>adapter-expected.json</a> for exact checked fields. These richer synthetic fixtures are still not complete importable or resumable CLI sessions. Tool arguments, including `echo synthetic`, are data, not executed commands.

From the repository root, with Rust/Tauri build dependencies already cached:

```bash
npm run docs:test
npm run docs:test:adapters
```

An additional isolated OpenCode v2 parser test reads the public SQL fixture above. It checks nullable titles, Windows-style stored paths, message order, split parts, recorded cost and tokens; it does not run OpenCode or a GUI.

The adapter command uses `cargo test --manifest-path src-tauri/Cargo.toml --lib --locked --offline docs_public_fixture`. It runs these three tests and fails rather than fetching uncached crates. The existing Rust CI job includes them; the Node documentation job does not require Rust. This is reproducible local evidence, not proof of production deployment or indexing.

## How to report a compatibility change {#report-change}

Include app version, CLI version, OS, record layout, custom-root configuration and the failing operation. Share a minimal synthetic/redacted shape, not credentials or a full private database. A CLI version is added to a verified-runtime matrix only after that version and operation actually run successfully.

Evidence: [adapter source revision 22fefc6](https://github.com/jerrywu001/cc-sessions-viewer/tree/22fefc6/src-tauri/src/agents). See [privacy](/guide/privacy), [installation](/guide/install) and [project maintenance](/guide/about).
