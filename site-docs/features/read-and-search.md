---
title: Search Claude Code, Codex and opencode History
description: Read supported session records and search titles, IDs or saved user prompts across the selected agent's projects. Understand global search and replay limits.
image: /screenshots/chat.png
---

# Reading and searching sessions

Sessions Viewer presents supported JSONL and opencode SQLite records as a readable conversation. It can show recorded messages, tools, reasoning, diffs and images when the adapter recognizes them. It cannot recreate missing records or reproduce the CLI screen exactly.

![A Claude Code session replayed in Sessions Viewer, with tool calls paired to their results and a rendered diff](/screenshots/chat.png)

## What gets reconstructed

### Tool calls paired with their results

In the file, a call and its result are separate records, sometimes many lines apart, linked by an id. Shown apart they are unreadable. Shown together, you can see what the agent asked for and what came back.

### Thinking blocks, collapsed until you want them

Recorded thinking is often longer than the answer. Supported blocks are folded, and you can expand it per message or for the whole session at once. Consecutive reasoning steps are grouped into one compact row. A global setting and per-session controls can hide reasoning independently of tool calls.

### Diffs rendered as diffs

When a file-edit result carries structured patch data, it becomes a real diff with line numbers and syntax highlighting instead of a block of escaped text. The [Claude Code page](/agents/claude-code#structured-diffs) shows what that data looks like on disk.

### Images back inline

Some agents store pasted screenshots as base64 and others as separate media files. Both render in place, in the message that contained them.

### Mermaid diagrams, tables and math

Anything the agent emitted as Markdown is rendered as Markdown.

## Star sessions for ongoing tasks {#starred-sessions}

Hover over a session in the project list to reveal the star beside its title, then click to mark or unmark it. Keyboard focus within the row also reveals the button. Select **Starred only** in the list toolbar to hide unmarked sessions; select it again to restore the list. This is separate from pinning: stars do not change the normal sort order.

The filter covers the current agent and project, loading the remaining session metadata rather than checking only the first page. Search and sort can still be combined with it; search limits still apply. Star marks are shared across split panes and saved locally by Sessions Viewer, not written into agent transcripts or databases. Stable session IDs retain marks after rename or a storage-path change. Marks do not sync to another installation and are not included in transcript exports. Switching projects resets the filter, not the marks.

## Finding a message {#finding-a-message}

![Global search in Sessions Viewer listing matches from several projects](/screenshots/search.png)

`⌘⇧F` opens global search across the **currently selected agent's projects**, not all seven agents at once. Keyword mode matches session titles and saved **user-message text**; switch to ID mode to find a session ID. Selecting a text match opens the session at the matching user message. Title/ID matches open the session without promising a text-message location.

### Search scope and limits {#search-scope}

Global search does not match assistant answers, thinking, tool arguments/results or project paths. Switch agents to search another source. The normal search excludes archived and trashed sessions; consult the [Codex archive guide](/guide/codex-session-viewer#archived) or [trash view](/features/export-and-trash) instead. The backend returns at most 200 matching sessions, and the modal renders at most 80. It is not an exhaustive full-text export.

The parser only presents supported, recorded fields. For example, opencode tool output over 30 lines is shortened with a remaining-line notice. Keep native files or the database for full archival needs; [parsed-message export](/features/export-and-trash) is not a native backup.

`⌘F` searches inside whatever is currently open, and `⌘G` and `⌘⇧G` step through the matches.

### Jump to a prompt

Long sessions are mostly agent output. The prompt list strips all of it away and shows only what you typed, in order, as a compact list. It is usually the fastest way to find the moment a session went wrong. Pick one and the view scrolls to it.

### Views history

Sessions you have read recently stay in a history list, per project, with title search. Reopening yesterday's session does not mean finding it again. Session previews refresh when their source file changes; Pi previews use the latest user message.

### Large Pi sessions

A Pi transcript opens with its most recent message page first. Scroll upward to load older messages as needed. Search, jump-to-prompt, global search and export still load the full history when they need it.

## Nothing is written back

Reading and searching history do not rewrite the source transcript. Local settings and caches may be written. Rename, trash/restore, continued conversations and editing features have separate write behavior; see [privacy and data handling](/guide/privacy).

Search and opencode display limits reviewed against Sessions Viewer 0.6.0 on 2026-10-06: [search UI](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src/modals/GlobalSearchModal.vue), [command](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src-tauri/src/lib.rs), [search matching](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src-tauri/src/agents/mod.rs) and [opencode adapter](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src-tauri/src/agents/opencode.rs). This is source review, not live CLI acceptance.
