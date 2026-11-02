---
title: "opencode Session History: SQLite Paths and Viewer"
description: Find opencode's supported SQLite history, check XDG paths, read sessions with SQL and use Sessions Viewer to search prompts, export and resume.
---

# Where opencode stores session history

The opencode layout supported here stores sessions in a SQLite database at `~/.local/share/opencode/opencode.db`, or `$XDG_DATA_HOME/opencode/opencode.db` when configured:

```
~/.local/share/opencode/opencode.db
```

It follows the XDG base directory spec, so `$XDG_DATA_HOME/opencode/opencode.db` wins if that variable is set. That applies on macOS too, where most apps would use `~/Library/Application Support`.

In this supported layout, sessions are database rows rather than individual JSONL files. Query SQLite to inspect them instead of assuming a recursive text search can extract a conversation. Older file-based layouts are outside this adapter's scope.

## The four tables in the legacy SQLite layout

```sql
project  -- id (sha1), worktree (the project directory), vcs, time_*
session  -- id ("ses_…"), project_id, parent_id, slug, title, directory,
         -- model (JSON), tokens_* (5 columns), cost, time_created,
         -- time_updated, time_archived
message  -- id ("msg_…"), session_id, time_created, data (JSON envelope)
part     -- id ("prt_…"), message_id, session_id, time_created, data (JSON body)
```

The database has more tables (`workspace`, `todo`, `permission`, `event`, `credential` and migration bookkeeping), but a transcript is `session` → `message` → `part`.

`message.data` is a JSON envelope: `role`, `modelID`, `providerID`, `tokens{input,output,reasoning,cache{read,write}}`, `cost`, `time{created,completed}`. `part.data` is the body, with a `type` of `text`, `reasoning`, `tool`, `file`, `step-start`, `step-finish` and others.

## The v2 session storage layout

The adapter detects database tables rather than relying on a CLI version string. The added v2 compatibility reads `project`, `session_v2` and `session_message`; newer migrations also create `session_part`. This is not just a table rename:

- The `session_message.type` column identifies message kinds and `seq` orders them; `data` need not contain `role` or `id`.
- User messages store `data.text` and `data.files`; assistant messages store `data.content` and `data.model.id`.
- Assistant `session_part` rows override inline content in `ordinal` order. Messages without parts retain inline content, including records written before that migration.
- Tools use `name`, `id` and `state.content`. Images can use inline bytes or `media:` references to blobs in the same database. Missing images remain file entries instead of aborting the transcript.
- Both layouts can coexist: v2 wins for duplicate session IDs while legacy-only sessions remain visible. Custom `XDG_DATA_HOME` resolution is unchanged.

Inspect new message rows with:

```sql
SELECT id, type, seq, data
FROM session_message
WHERE session_id = 'ses_...'
ORDER BY seq;
```

Reading, saved-user-prompt search, parsed-message export and statistics share the normalized adapter. V2 is event-sourced: editing projection tables directly can be undone by replay. **V2 rename, trash and restore are not supported in Sessions Viewer in this update** and return an explicit error; manage these actions in OpenCode. Existing legacy write operations remain available. Parsed exports are not native backups of every event, setting or background job.

This compatibility is based on OpenCode's pinned [table definitions](https://github.com/anomalyco/opencode/blob/dbcd5ce9485f3338375d1fe9d92864c91e263030/packages/core/src/session/sql.ts) and [message schema](https://github.com/anomalyco/opencode/blob/dbcd5ce9485f3338375d1fe9d92864c91e263030/packages/schema/src/session-message.ts), with synthetic database regression tests. It has not been verified against the database from issue #96 or certified on a Windows machine.

## Sub-agent sessions

A session with a non-null `parent_id` is a sub-agent run spawned by another session. The viewer hides these from the ordinary history list but includes their recorded work in statistics. They can contain usage and costs; a local model, provider pricing or missing fields can yield zero recorded cost. A sub-agent run is not proof of a separately billed API call.

## Open it read-only

opencode's TUI may be writing while you inspect the database. Use a read-only connection to avoid accidental writes; busy or changing databases can still require retrying after activity settles. The command also respects a configured XDG data root:

```bash
sqlite3 "file:${XDG_DATA_HOME:-$HOME/.local/share}/opencode/opencode.db?mode=ro" ".tables"
```

## Reading a legacy opencode transcript with SQL

List sessions with their project and cost, newest first:

```sql
SELECT s.id, p.worktree, s.title, s.cost,
       datetime(s.time_created/1000, 'unixepoch') AS created
FROM session s
JOIN project p ON p.id = s.project_id
WHERE s.parent_id IS NULL
ORDER BY s.time_created DESC
LIMIT 20;
```

Print one session's text, in order:

```sql
SELECT json_extract(m.data, '$.role') AS role,
       json_extract(pt.data, '$.text') AS text
FROM message m
JOIN part pt ON pt.message_id = m.id
WHERE m.session_id = 'ses_...'
  AND json_extract(pt.data, '$.type') = 'text'
ORDER BY m.time_created, pt.time_created;
```

Sum session-recorded costs per project:

```sql
SELECT p.worktree, ROUND(SUM(s.cost), 2) AS usd, COUNT(*) AS sessions
FROM session s JOIN project p ON p.id = s.project_id
GROUP BY p.worktree ORDER BY usd DESC;
```

The last query includes sub-agent rows on purpose. It sums `session.cost`, which is a stored value, not a provider invoice. Desktop statistics instead read assistant-message costs; the two aggregations need not agree if records are missing or inconsistent.

## Why cost has to come from the database {#cost-from-db}

opencode can use different providers or local models, so a price table keyed only by model name may not reflect that setup. The viewer reads `modelID`, `tokens` and `cost` from assistant-message records rather than repricing them with its catalog. Missing numeric message costs currently default to zero; this means unmeasured cost, not verified free work. Recorded cost is not an independently verified invoice. Compare financial decisions with the provider's actual usage report; see [usage and cost boundaries](/features/stats#cost-vs-bill).

## How do I resume this session?

Run `opencode --session SESSION_ID` from the project's directory with opencode installed. The app offers [terminal resume](/features/resume), not in-app opencode chat.

## View and search opencode history {#view-and-search}

[Install Sessions Viewer](/guide/install), select opencode, then choose a project to read its supported SQLite sessions. Use `⌘⇧F` (macOS) or `Ctrl+Shift+F` (Windows/Linux) for session titles and saved user prompts across opencode projects, or switch to ID mode. This does not search assistant answers or tool output; see [search scope](/features/read-and-search#search-scope).

Export Markdown, HTML or parsed-message JSON after reviewing private content; keep the native database for archival needs. [Export limits](/features/export-and-trash) include external/unreadable images. Continuing uses the installed opencode CLI in a terminal, not an opencode in-app chat. Sessions Viewer is an independent open-source project, not an official opencode product. Check [privacy](/guide/privacy) before resuming or sharing.

## Source and limitations

Path, recorded-cost and viewer-operation wording reviewed on 2026-10-06 against the [0.6.0 opencode adapter](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src-tauri/src/agents/opencode.rs) and [search implementation](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src-tauri/src/agents/mod.rs). This is source review and synthetic-test scope, not CLI runtime certification. Upstream: [opencode documentation](https://opencode.ai/docs/).

<!--@include: ../.vitepress/snippets/reference-en.md-->

The `session` / `message` / `part` SQL examples target the legacy SQLite layout; see the v2 section for its query. Older file-based layouts remain out of scope. Explicit legacy rename/trash/restore actions write to the database; their v2 equivalents are currently blocked. See [missing-session troubleshooting](/guide/troubleshooting).

## Or open it in an app

[Sessions Viewer](/guide/) queries this database read-only, hides sub-agent sessions from the list while still counting them in the stats, and shows opencode conversations next to your JSONL-based agents in the same view. It also reads [Claude Code](/agents/claude-code), [Codex](/agents/codex), [Grok Build](/agents/grok-build), [Kimi Code](/agents/kimi-code), [Pi](/agents/pi) and [Antigravity CLI](/agents/antigravity-cli).
