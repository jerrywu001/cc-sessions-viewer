# Changelog

All notable changes to this project are documented here. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); semver via [release-please](https://github.com/googleapis/release-please) from [Conventional Commits](https://www.conventionalcommits.org/).

> ⚙️ Maintained by [release-please](https://github.com/googleapis/release-please): conventional-commit subjects on `main` are collected into the open "Release PR"; merging that PR writes the new section here and tags `vX.Y.Z` on the same commit. Hand-edits to released sections will be preserved across future runs.

---

## [v0.3.21]

### Features

- **Configurable HTML export timestamps** — HTML exports include per-message timestamps by default; Settings can now disable them for future exports without changing timestamps shown in the conversation view.
- **Markdown media rendering** — standard `![alt](https://...)` images, `![alt](https://.../video.mp4)` videos, and safe raw `<video><source></video>` blocks now render inline in conversation details and HTML exports, with image lightbox viewing, native video controls, and an extracted first-frame poster when the video host permits it.

### Bug Fixes

- **Grok Build math rendering** — namespaced the application's modal-overlay styles so they no longer collide with KaTeX's internal `.overlay` elements, preventing vector notation from covering the conversation.
- **URL prompts preserved** — session-detail parsing no longer mistakes URLs ending in file extensions, including media links, for local file attachments.

## [v0.3.20]

### Features

- **Grok Build local-session support** — the viewer now discovers Grok Build sessions from `~/.grok/sessions`, reads their `updates.jsonl` transcript, groups them by working directory, and supports session search, export, rename, trash/restore, reveal, and resume in the embedded terminal. Grok Build remains intentionally read-only in the GUI chat composer.
- **Grok Build integration across the app** — Grok Build is available in agent settings, CLI environment checks, hooks, worktree handling, session lifecycle indicators, export history, global search, statistics, tray statistics, and the live pricing table.
- **Grok Build hooks and launch defaults** — Settings can inspect, refresh, and enable Grok Build lifecycle hooks, while terminal launches use Grok's supported `--yolo` behavior.
- **Background media batch export** — Saved background images and MP4 videos can now be exported together into a new folder inside a selected location, using their original display filenames instead of internal UUID/hash cache names.

### Improvements

- **Pricing source resilience** — live model pricing now requests `js-bridge.com/api/models` first and automatically falls back to `models.dev/api.json` when the preferred source has a network, HTTP, response, or parsing failure. Existing cached prices remain available during a failed refresh.
- **Fresh pricing source migration** — pricing cache storage moves to a new version so upgrades refresh through the preferred source immediately; restoring defaults also clears prior pricing caches before refetching.
- **Agent-aware search guidance** — global and project search prompts now name the currently selected agent, making it clear that searches are scoped to that agent's sessions.

### Bug Fixes

- **Grok Build session opening** — opening a session now preserves the agent, project, and split-pane context captured at click time, preventing asynchronous refreshes or navigation changes from reading the wrong source or leaving a newly opened tab out of view.
- **Export-history navigation** — opening a historical export now switches to the saved agent and project context before restoring the original transcript.

### Tests

- Added Grok Build parser, storage-boundary, stats, hooks, settings, pricing, export, search, and session-list coverage, including the preferred pricing-source order and models.dev fallback contract.

## [v0.3.19]

### Features

- **Global search scope selector** — the global search overlay gains a Session ID / Keyword scope dropdown. Keyword mode ranks matches by message text and excludes raw session IDs; ID mode searches session IDs directly and skips the message body, so pinning down a specific transcript by its ID is precise instead of noisy.
- **Prompt navigation rail** — the conversation view now shows a Codex-style turn rail down the left edge, with one marker per user prompt. Hovering or tabbing to a marker expands a floating preview of that prompt (its text plus the assistant's opening reply), and clicking it jumps straight to the message. The rail keeps the current prompt centered in a configurable marker window so long sessions stay navigable.

### Improvements

- **Restore default settings** — the "Clear cache" action is replaced by "Restore default settings", which now resets every application preference (chat spacing, prompt rail, background, launch args, enabled agents, desktop pet, sidebar width, window close prompt, and more) in one step instead of only clearing pin / sink prefs.
- **Configurable navigation markers** — Settings → Chat gains a "Navigation markers" slider (21–71, default 41) controlling how many prompt markers the rail keeps around the current message.

### Bug Fixes

- **Session lifecycle UI tidy** — removed the duplicated retry label from the Chat composer status row, and the embedded session tab now closes cleanly on a graceful exit (e.g. double Ctrl+C) instead of lingering after the process has ended.

### Tests

- Added coverage for the search scope selector, the prompt navigation rail (marker rendering, active highlight, and jump behavior), the configurable marker count, and the restore-defaults path.

## [v0.3.18]

### Bug Fixes

- **Session-lock recovery** — GUI Chat now explains which Chat, terminal, or external Codex process is holding a session, and offers an in-place Retry button after the other owner is closed.
- **Embedded terminal recovery guidance** — terminal resume failures now use terminal-specific instructions, including closing the current embedded terminal before reopening the session.
- **Chat-to-Read cleanup** — switching a live Chat to Read mode now stops and releases its background Chat process, preventing subsequent embedded-terminal resumes from being rejected by a stale Chat writer lock.
- **Inline pasted-image labels** — Codex and ClaudeCode chat now identify pasted images from the real message attachment order, including legacy mixed file/image messages, while leaving ordinary image attachments unlabeled.

### Tests

- Added coverage for session-lock retry behavior, terminal-specific recovery messages, releasing Chat ownership when switching to Read mode, and mixed-attachment pasted-image binding.

## [v0.3.17]

### Bug Fixes

- **Reliable chat-runtime cleanup** — runtime child processes and Codex output writers are now owned and released exactly once, preventing orphaned processes, duplicate output handling, and stale active-turn state.
- **Idle chat recovery** — when a GUI chat process exits unexpectedly, its session lease is released before a delayed retry. The composer stays usable and queued messages are retained until recovery succeeds.
- **Codex Chat startup on macOS** — Codex app-server no longer starts in an incompatible isolated process group, preventing initialization timeouts that incorrectly showed `Session ended`.
- **Drafts survive chat remounts** — switching views or reopening a GUI chat no longer discards unsent text, images, or file attachments.
- **Stable live-status timer** — long tool summaries no longer squeeze elapsed time onto multiple lines; the timer stays readable while the tool label wraps.
- **Wallpaper-aware rich content** — file diffs, Markdown tables, Mermaid diagrams, and block math now use readable translucent surfaces over a custom background.

### Maintenance

- **Rust 1.97 CI compatibility** — grouped PTY launch options into a typed configuration object so strict Clippy checks pass on the current stable toolchain.

## [v0.3.16]

### Bug Fixes

- **Background media clarity** — background-video entries are marked with an MP4 tag, and light-theme terminal text selections remain readable when a custom background image is active.
- **Codex app-server startup timing** — GUI chat waits for the app-server to be ready before sending a turn, avoiding early-send failures.
- **Codex turn cleanup and chat contrast** — completed Codex writers are released promptly, and user message bubbles use a softer contrast treatment.

### Tests

- Added coverage for MP4 background-media labeling and app-server readiness before a chat turn begins.

## [v0.3.15]

### Features

- **Edit and fork chat prompts** — cancelled or historical user prompts can be restored to the composer and continued in a new branch without carrying the discarded prompt or partial answer forward.
- **Clearer chat navigation** — GUI chat tabs surface running, completed, blocked, and failed states consistently; project drag ordering includes a floating preview and clearer insertion feedback.

### Improvements

- **More resilient composer interactions** — improved draft switching, IME handling, and `@` file mentions, including relative workspace paths and an explicit empty-result state.

### Tests

- Added coverage for editable prompt history, fork boundaries, composer mentions and drafts, project reordering, and GUI-chat tab status transitions.

## [v0.3.14]

### Features

- **Custom appearance backgrounds** — Settings now has an Appearance section for importing PNG, JPEG, WebP, GIF, AVIF, or MP4 backgrounds. Imported media is cached in an in-app wallpaper library, so saved images and videos can be previewed, switched, opened in Finder, or removed without choosing the source file again.
- **Background-aware interface controls** — the appearance settings include independent sliders for background opacity and border/divider opacity. Chat, terminal, Git Diff, settings, list hover states, code blocks, and statistics surfaces adapt automatically when a wallpaper is enabled while preserving the original opaque appearance when none is configured.
- **Persistent project ordering** — projects retain the default most-recently-updated order on first load, and can then be manually reordered from the sidebar. The saved order is per agent, survives refreshes and live project updates, and keeps pinned/sunk groups and worktree families intact.

### Improvements

- **Project-list drag interaction** — sidebar project ordering now uses a dedicated drag handle, a pointer-driven reorder interaction, and an in-list insertion indicator. The handle appears only on hover so normal project rows do not reserve unused leading space.
- **Wallpaper contrast consistency** — shared light/dark border and divider tokens are applied across the interface, keeping surface separation readable over custom backgrounds.

### Tests

- Added focused coverage for background-media preferences and library operations, background opacity settings, persisted project order, hover-only drag handles, insertion targets, and reorder events.

## [v0.3.13]

### Features

- **Codex GUI guided follow-ups** — queued Codex messages can now be guided into the active turn without interrupting it. Guided messages are delivered in order after the current response completes, while the internal control payload remains hidden from both live and restored chat history.
- **Local Git branch controls** — Git repositories now expose the current branch in GUI chat, the composer, and the session-list header. The compact menu can refresh repository state, switch clean working trees between local branches, create a local branch without switching to it, and safely delete merged, non-current local branches.
- **Working-tree change indicator** — repositories with local modifications show an uncommitted-file count. Selecting it opens the project's Git Diff tab on the working tree; refreshing branch state also refreshes any open working-tree diff views.
- **Configurable GUI chat spacing** — Settings now includes a persisted spacing slider for message, thinking, and tool blocks, with the existing layout retained as the default and an adjustable range from 30% to 150% in 2% increments.

### Bug Fixes

- **Codex app-server startup** — Codex GUI now starts the app-server with its default stdio transport, restoring reliable startup behavior.
- **Running tool-label overflow** — long live tool labels, including shell commands, now wrap and clamp within the chat layout instead of overflowing the conversation pane.

### Tests

- Added focused coverage for guided-message ordering and transcript filtering, the chat-spacing setting, local branch operations, uncommitted-change refresh notifications, and session-list branch controls.

## [v0.3.12]

### Features

- **Semantic live tool activity** — GUI chats now summarize running tools by intent, including file reads and edits, searches, Git commands, builds, and tests. Parallel activity is tracked independently, and recently completed work remains visible briefly for continuity.

### Bug Fixes

- **Visible shell commands during execution** — live Bash activity now shows the bounded command text alongside its semantic summary, making the command currently being run directly inspectable.
- **GUI Escape interruption and focus** — Escape handling now routes to the active GUI-chat interruption flow, while sending a message preserves composer focus for continued input.
- **Reconnect and service-unavailable retries** — retry-state detection now recognizes reconnecting progress and service-unavailable responses, including attempt counters reported by Codex.

### Tests

- Added coverage for semantic tool summaries, parallel tool activity, shell command display, GUI Escape/focus handling, and reconnect retry-state parsing.

## [v0.3.11]

### Features

- **Inline file reference preservation** — `@file` references embedded within prompt text (e.g., "analyze @scripts/release.sh execution") now preserve the full prompt context instead of being stripped out. File attachment tags are still generated, but the original reference stays readable in the message body. Only leading and trailing file references are removed from the displayed text.
- **Tool call visibility control** — added a "Show tool calls" toggle in Settings → Chat that hides process tools like Bash, Read, and Grep by default. File changes and interactive prompts (permissions, questions) remain visible regardless of the setting. Each read-only session can override the global setting with a toolbar toggle.
- **Live tool activity tracking** — the GUI now tracks the current tool being executed and displays real-time status (calling/completed/failed) in the running state line, even when tool cards are hidden.
- **Turn outcome feedback** — completed, cancelled, and failed turn states are now tracked and briefly displayed in the status line after each turn finishes.
- **Text-based diff rendering** — tool results containing plain-text `git diff` output are now automatically detected and rendered with syntax highlighting, diff statistics, and the same visual treatment as structured file changes.
- **Historical AskUserQuestion display** — read-only transcripts now render past `AskUserQuestion` prompts with their original questions and user answers, including localized status labels for answered/skipped states.

### Bug Fixes

- **Xcode build log path protection** — paths in Xcode build error logs (result bundles, Pods.xcodeproj references) are no longer extracted as file attachments, preventing diagnostic text from being fragmented.
- **File reference deduplication** — when agent-specific parsers (Claude, Codex) have already created file tags for inline references, the generic post-processor now skips duplicate attachment extraction.
- **Relative path resolution** — file references like `@package.json` and `@scripts/release.sh` are now resolved relative to the session's `cwd` when available, improving attachment accuracy for repository-relative paths.

### Tests

- Added 11 new tests covering inline reference preservation, edge-only stripping, text diff detection, Xcode log protection, file reference deduplication, turn outcome tracking, and tool activity state transitions.

## [v0.3.10]

### Features

- **Codex GUI plugin mentions** — added the `@...` plugin picker for Codex GUI chat with colored plugin rows, sticky section headers, mention deletion behavior in the composer, and structured Codex app-server text elements so built-in plugins render as icon + plugin name in GUI, read mode, and HTML exports.
- **GUI plan mode parity** — Codex GUI plan mode now uses app-server collaboration mode, read-only sandboxing, streamed plan rendering, and a local "Implement this plan?" confirmation when the app-server emits a plan without a separate prompt. Claude GUI also treats `/plan` as a client action instead of sending the literal command to Claude Code.
- **Remembered GUI chat settings** — Codex and Claude GUI chats now remember the last selected permission mode, model, and effort for new chats and resume flows. Default GUI permissions are set to the highest available mode for each agent.
- **Running-turn interrupt** — pressing Escape in the GUI composer can stop the most recent running turn, matching the existing stop action more closely.
- **Claude Opus 5 model option** — added `claude-opus-5` as "Opus 5" in the Claude subscription model menu, moved Opus 4.8 into More models, and made Opus 5 the new standard-context auto-pick.

### Bug Fixes

- **GUI file-change rendering while running** — Codex GUI now keeps file create/update/delete results visible while a turn is still executing instead of folding them away until the chat is reopened.
- **Plugin mention transcript/export rendering** — fixed plugin mentions being displayed as raw `plugin://...` links in GUI/read views and HTML export output.
- **Light-mode HTML export styling** — improved exported HTML styling for light mode so plugin/tool content matches the in-app rendering.
- **Git diff refresh** — refreshing a Git diff tab now reloads the currently selected file after file content changes, including edits that remove lines.
- **Collapsible output overflow** — replaced show more/show less expansion with bounded vertical scrolling for long rendered blocks, including shared rendering used by all agents and HTML exports.
- **Question prompt agent label** — GUI question prompts now label the requesting agent correctly, so Codex questions no longer say "Claude is asking".
- **Claude plan approval flow** — allowing Claude's `ExitPlanMode` request now switches the GUI back to the normal Claude permission mode before continuing the implementation turn.

### Tests

- Added and updated focused coverage for GUI plan mode, plugin mentions, question prompt labels, chat setting persistence, Git diff refresh behavior, collapsible overflow rendering, HTML export output, and Claude/Codex model menu behavior.

## [v0.3.9]

### Bug Fixes

- **Desktop pet activity clipping** — adjusted the floating pet activity layer so task status content is no longer clipped by the transparent pet window.
- **Desktop pet activity display** — simplified the activity UI to reduce overlay complexity while keeping the active task state visible and covered by component tests.

### Performance

- **Bundled desktop pet spritesheets** — recompressed the nine bundled Codex pet WebP atlases with conservative alpha settings, reducing the spritesheet payload from about 11.28 MiB to about 2.09 MiB while preserving the 1536 × 2288 atlas dimensions.

## [v0.3.8]

### Bug Fixes

- **Bundled Codex pets** — included the full Codex desktop pet sprite catalog so users without a detectable Codex/ChatGPT desktop install still have all official pets available, and stabilized the hover animation behavior.
- **Windows event loop crashes** — patched the vendored `tao` event loop integration to prevent reentrant event loop crashes on Windows and added panic logging support around that path.
- **Vendored `tao` warnings** — silenced warnings from the vendored windowing dependency so strict Rust warning checks stay clean.

## [v0.3.7]

### Features

- **Terminal selection deletion** — terminal input now tracks cursor/selection state closely enough to delete selected in-terminal text with Backspace, with focused coverage for selection and tab-status behavior.

### Bug Fixes

- **Codex terminal scrollback** — preserved Codex terminal scrollback across terminal lifecycle updates so existing output is not dropped unexpectedly.
- **Desktop pet window bounds** — tightened the desktop pet transparent window bounds so the pet and status content use less unnecessary screen area.
- **Terminal Backspace selection handling** — refined Backspace behavior around terminal selections to avoid leaving stale selected text behind.

### Maintenance

- **Local worktrees** — ignored local worktree directories to keep repository status cleaner during development.

## [v0.3.6]

### Features

- **Desktop pet task activity** — the desktop pet now reflects active session work: running, ready, blocked, and failed tasks drive the pet animation and show a compact status pill. Hovering the pill opens a task activity panel with the current agent, session title, and state, and clicking a task brings the related session back into focus. Task updates flow from the existing turn-state hooks and are acknowledged when the session is opened.
- **Codex Desktop pet sprites** — the desktop pet catalog now imports official Codex/ChatGPT pet spritesheets from the installed desktop app instead of using the old bundled SVG mascots. Settings → Desktop Pet lists Codex pets and custom pets, supports refresh/open-directory actions, and previews the selected spritesheet with the same atlas player used by the floating pet.
- **Custom desktop pets** — users can add their own pets under `~/.codex/pets/<pet>/` with `pet.json` and a PNG/WebP spritesheet. Custom pets use the same atlas validation and selection flow as the imported Codex pets.

### Bug Fixes

- **Opencode session deletion** — deleting opencode sessions now handles `opencode://...` virtual paths correctly. The app exports the SQLite session rows to a trash dump, deletes the session/message/part rows from the opencode database, and can restore the dump back into the database from the shared trash UI.
- **Codex pet discovery on current installs** — official pet import no longer depends on the older `webview/files/assets/files` asar layout. The importer recursively finds `*-spritesheet-*.webp` assets, scans ChatGPT.app/Codex.app locations on macOS, and also checks the `codex-plusplus` backup location used by some installs.
- **Desktop pet fallback** — when no official or custom spritesheet is available, the floating pet and Settings preview show a lightweight Codex fallback avatar instead of an empty transparent window.
- **Desktop pet activity hover** — the activity panel stays open while moving between the trigger and the tray, so the task list no longer disappears before it can be clicked.
- **Desktop pet transparent window sizing** — the floating pet window now resizes with the visible pet/status content, reducing the invisible WebView area that can intercept clicks around the pet.

### Tests

- Added coverage for opencode trash dump/delete/restore, nested Codex spritesheet discovery, atlas playback, empty-catalog fallback rendering, desktop pet preference sync, task activity ordering, hover behavior, and window-size updates.

## [v0.3.4]

### Features

- **Session creation-time sorting** — session lists can now be sorted by creation time in either direction, alongside the existing updated-time / size / message-count sort modes. The option is available from the sessions topbar sort menu, and the list toolbar has a dedicated creation-sort shortcut that toggles newest-created ↔ oldest-created. When a creation timestamp is missing, sorting falls back to the session modified time so older transcripts remain sortable.
- **Codex custom-provider defaults for new GUI chats** — fresh Codex GUI chats now read the top-level `model` and `model_reasoning_effort` from `~/.codex/config.toml` when Codex is configured with a custom model provider. The GUI initializes its model/effort controls from those values and sends the same values to the Codex app-server, so local reverse-proxy setups such as CC Switch no longer start with the app's built-in Codex defaults. Existing chat resume/fork flows keep their previous behavior and are not overwritten by the global config.

### Bug Fixes

- **Codex apply_patch diffs render again** — newer Codex CLI versions can wrap `apply_patch` calls inside the JavaScript `exec` tool, which made applied patches appear as generic command output instead of structured diffs. The transcript parser now recovers the underlying patch, reconnects wrapped patch result events to the original tool call, and marks failed patch applications as errors.
- **Creation sort direction is clearer** — the creation-time sort shortcut now uses a two-arrow icon that highlights the active direction directly, avoiding ambiguous localized labels and making ascending vs descending state visible at a glance.
- **Strict Clippy cleanup** — fixed Codex parser issues that failed the stricter Rust lint pass, keeping the Tauri test/build pipeline clean.

## [v0.3.3]

### Features

- **Codex `/side` side chat** — Codex GUI chat gains a floating side panel (mirroring Claude's `/btw`), accessible via `Ctrl+J`, the `/side` slash command, or the side-panel button. Forks the current Codex thread via the app-server so the side chat inherits context without polluting the main conversation. Session files are purged on close to keep the session list clean.
- **Slash command one-press delete** — pressing Backspace when the cursor is within a recognized slash command token (the blue-highlighted `/git-push`, `/side`, etc.) deletes the entire token in one keystroke instead of character-by-character.
- **Slash command hover tooltip** — hovering over a recognized slash command in the composer now shows a tooltip with the command name and description, using the app's custom tooltip system rather than native browser tooltips.
- **Tab strip auto-scroll** — opening a new session, chat, or TUI tab now automatically scrolls the tab strip to reveal the newly activated tab, so it's always visible even when many tabs are open.

### Bug Fixes

- **Topbar search bar not restoring after tab switch** — switching from a TUI or session tab to a chat GUI or git diff tab and then back left the global topbar empty (no "Search terminal" / "Search sessions" bar). The topbar condition now checks whether a TUI tab is active before checking for chat/git view tabs, so the correct topbar always renders when switching back.
- **Git diff tab showing "Search sessions"** — the git diff view tab fell through to the SessionsTopbar condition, showing an irrelevant search bar. Git tabs now render an empty topbar since GitChangesView has its own built-in toolbar.
- **Washed-out Codex TUI colors on Windows** — Codex neither queries the terminal background (no `OSC 11`) nor reads `COLORFGBG` on Windows, so it always paints with its dark-theme palette and every foreground comes out pale under the light theme. Rather than discarding those colors (which flattened hints, paths, and syntax highlighting to black), the SGR normalizer now converts Codex's dark palette into its light twin: it mirrors each color's lightness in HSL and keeps hue and saturation. The full brightness ladder survives the flip — body text becomes the darkest, faint separators become the lightest — and accents keep their color instead of turning to mud. The foreground mirror is Windows-only, since that's where Codex is confirmed to mis-detect the background; other platforms and the dark theme are untouched. Also fixes extended-color parameters (`38;2;40;…`) being mis-read as standalone SGR codes and silently corrupting the color.

---

## [v0.3.2]

### Bug Fixes

- **Windows embedded terminal couldn't find nvm-installed CLIs on MSI installs** — with an MSI (WiX) install the in-app terminals reported `npm` / `node` / `codex` as "not a command", while EXE/NSIS installs worked and native (non-nvm) Node was fine everywhere. An MSI advertised-shortcut launches the app in a context that cannot traverse directory symlinks during command lookup, so nvm-for-windows' node directory (`NVM_SYMLINK`, a symlink to the versioned install) sat on `PATH` yet nothing inside it resolved. The PowerShell PATH refresh now expands `%VAR%` registry entries and, for every `PATH` directory that is a reparse point, appends its resolved real target — command lookup then hits the non-symlink path. Native (real-directory) installs are untouched. Applies to the embedded terminal, session resume, and CLI version detection.
- **Codex TUI Shift+Enter didn't insert a newline on Windows** — Codex reads keys via the Windows console API and, there, only accepts Alt+Enter (`ESC`+`CR`) as a newline; a raw `\n` / Ctrl+J or a kitty `[13;2u` sequence is ignored (see codex#4401). Shift+Enter is now intercepted directly in the terminal key handler (the `onData` + `shiftHeld` path is unreliable under WebView2) and mapped to the Alt+Enter byte sequence on Windows, while macOS / Linux keep sending `\n`.
- **Codex composer background broke across blank lines** — a blank line inside a multi-line Codex prompt split the grey user-message background, leaving a white gap on the blank line and everything after it. The background repair now spans interior blank lines (continuing while more composer content follows) and stops at the footer (`model · path`), keeping the background continuous without bleeding onto the footer.

---

## [v0.3.1]

### Features

- **Ctrl+Del delete line in chat composer** — pressing Ctrl+Del in the GUI chat input deletes the entire line at the cursor position, matching common editor behavior.
- **Shift+Enter newline in TUI terminal** — Shift+Enter now inserts a newline (`\n`) instead of submitting (`\r`) in both TUI and shell terminal tabs, matching native terminal behavior.
- **Keyboard shortcuts reorganized** — Settings → Shortcuts now splits chat-specific shortcuts (Ctrl+S stash, Ctrl+Del delete line, Shift+Enter newline, ⌘U attach files, ⌘J side chat) into a dedicated "Chat (GUI)" group, separate from session-level shortcuts.

### Bug Fixes

- **Text selection floating at small font sizes** — font sizes below 14px caused selected text to visually "float" away from the cursor due to CSS `body.style.zoom` coordinate mismatch in WKWebView. Replaced with Tauri's native `webview.setZoom()` which keeps selection, right-click menus, and mouse events aligned at all zoom levels.
- **Right-click context menu position offset** — native Tauri menu popups in the terminal strip appeared offset from the click position when font size was not 14px. `LogicalPosition` coordinates are now multiplied by the zoom factor to compensate.
- **Split/close pane key repeat blocked** — holding down ⌘D / ⌘⇧D / ⌘⇧W no longer rapid-fires split or close pane actions; only the initial keypress is honored.
- **Windows CLI environment check found zero installs** — the diagnosis used a bare `where`, which in PowerShell is an alias for `Where-Object`, so `where claude` matched nothing and every CLI showed as "not installed". Now uses `where.exe`; the `--version` probe invokes quoted paths via the `&` call operator (a bare `'…' --version` is a PowerShell parse error); the several launcher shims one npm install drops (`codex`, `codex.cmd`, `codex.ps1`) collapse to a single install; and package-manager detection reads the shim for a `node_modules` reference instead of guessing from the install directory (the old hard-coded `\nvm\` match only worked on one machine's layout).
- **Duplicate version line in CLI environment card** — the per-install version is now shown only when multiple conflicting installs exist; with a single install it no longer duplicates the version already in the card header.
- **Codex resume — friendly hint on stale model provider** — resuming an older Codex session whose recorded `model_provider` is no longer defined in `~/.codex/config.toml` (e.g. renamed) failed inside the embedded TUI with a raw `Model provider \`…\` not found` dump. The terminal now appends a localized, actionable hint (naming the missing provider and noting the transcript stays fully viewable) whenever a Codex resume exits non-zero on a config/provider load failure.

---

## [v0.3.0]

### Features

- **Codex GPT-5.6 models** — add GPT-5.6-Sol, GPT-5.6-Terra, and GPT-5.6-Luna to the Codex model menu. Default remains GPT-5.5; new models appear at the top of the list.
- **Model-specific effort levels** — Luna adds a `max` effort tier; Terra and Sol add `max` + `ultra`, matching the Codex CLI's per-model reasoning levels.
- **Codex system commands** — add `/goal`, `/plan`, `/compact`, `/review`, and `/archive` to the Codex chat slash menu. `/goal`, `/plan`, `/compact` are also available for Claude.
- **`/archive` client action** — typing `/archive` in Codex chat calls `codex archive <id>` to archive the session, closes the chat tab, and refreshes the session list.
- **Archived session guard** — clicking chat/resume on an archived Codex session shows a confirmation dialog with the `codex unarchive <id>` command, copies it to the clipboard, and opens a terminal tab for pasting.
- **Font size slider** — replace the 3-option segmented control (Small / Normal / Large) with a continuous 12–18 px slider and a live preview line. Zoom is deferred to settings modal close to prevent UI jitter during drag.
- **Font family setting** — new text input in Settings → General for customizing the app's CSS `font-family`. Includes a live preview line (same pattern as font size). Applied on settings modal close; persisted to localStorage. Empty value uses the default system font stack.
- **Lazy terminal tab restore** — terminal tabs are now lazily restored on startup, avoiding blocking the main thread when many saved tabs exist.

### Bug Fixes

- **Terminal theme switching** — switching themes while a Codex embedded terminal was open left hardcoded inline `background-color` from the old theme on xterm DOM spans. Now the `xterm-scrollable-element` container background is updated manually, and a DOM walk strips inappropriate background/foreground colors based on luminance. New output from a light-started CLI in dark mode is normalized via `normalizeDarkSgr` (inverse of the existing light-mode ANSI normalizer).
- **Tab restore race condition** — the `currentLayout` deep watcher could fire `persistViewTabs` with an empty tab list before `loadSavedViewTabs` ran, overwriting saved tabs on reload. A `_viewTabsRestoreComplete` guard now blocks premature writes.
- **Codex subtitle "---"** — `last_user_text` was picking up `<skill>` / `<context>` / `<environment_context>` injection messages instead of the real user prompt, producing a `---` subtitle from the YAML frontmatter. These injections are now skipped.
- **More Models submenu clipping** — the submenu could extend below the viewport when the trigger button was near the bottom of the screen. Added vertical flip detection (`bottom: 0` when space is insufficient).
- **Path detection false positives** — `parse_line_as_path` treated any `/`-prefixed token (e.g. `/fork`, `/model`) as an absolute file path. Now requires a directory separator, file extension, or disk existence before promoting to a file block.
- **Slash command prefix** — Codex system commands (`/model`, `/export`, `/rename`, `/clear`) now always insert with `/` prefix; only skills use the `$` prefix. Previously all items used `$` for Codex.
- **Slash command trailing text** — `/model`, `/export`, `/rename` now intercept even when followed by trailing text, preventing accidental message sends.

---

## [v0.2.10]

### Features

- **Dracula terminal theme** — embedded xterm terminal now uses the standard Dracula color palette when the Dracula theme is active, matching the app's `--bg` and accent colors.
- **Git diff tree collapse/expand** — directory nodes in the git diff file tree now toggle correctly on click (fixed reactivity issue where expand state was lost on each computed re-evaluation).
- **Git diff empty state** — when there are no working changes, the git diff panel shows a centered message instead of an empty split layout, with a hint to browse commit history.
- **Live session watcher reconnect** — switching away from a session tab (to git/list/etc.) and back now correctly restarts the file watcher, restoring live-tailing updates from external CLI sessions.

### Bug Fixes

- **Homebrew upgrade formula** — opencode upgrade via Homebrew now uses the correct formula name (`opencode`) instead of the npm package name (`opencode-ai`), fixing "No available formula" errors.
- **User bubble text selection** — user message bubbles now explicitly allow text selection in WKWebView, fixing inability to drag-select text in user messages.
- **Windows console flash suppression** — background child processes (git, open-url, editor, resume) use `CREATE_NO_WINDOW` to prevent console window flashes on Windows.
- **Tab state persistence on exit** — intercept `ExitRequested` to save tab state and flush WebView localStorage before exit, fixing lost or partial tab restore after quit.

---

## [v0.2.9]

### Features

- **Git diff viewer** — new diff tab type with a file tree sidebar, commit history dropdown, and syntax-highlighted diff output. Shows working-tree changes (git diff) by default; selecting a commit from the dropdown shows the diff against that commit. Empty state message when there are no working changes. Per-tab isolation via `:key`, per-project diff numbering (Diff 1/2/3), and per-pane active-tab restore across restarts. Tab context-menu labels adapt for git tabs ("Close other Diffs" / "Close project Diffs").
- **NewMenu shared component** — extracted the `+` dropdown and right-click context menus (used across session list, detail view, and git tabs) into a reusable `NewMenu.vue` component.
- **Per-session subtitle** — session cards now show a last-prompt subtitle line below the title for all four agents (Claude, Codex, agy, opencode), giving a quick glimpse of what each session is about without opening it.
- **Project path correction** — when a session JSONL records a `cwd` that is a subdirectory of the actual project root, `best_project_root` now walks up to find the nearest VCS root, so sessions show under the correct project instead of fragmenting into phantom subdirectory projects.

### Bug Fixes

- **Terminal selection copy on Windows** — preserved terminal text selection when copying with Ctrl+C on Windows, preventing the selection from being cleared before the copy operation completes.

---

## [v0.2.8]

### Features

- **Opencode agent support** — new first-class agent for [opencode](https://opencode.ai) sessions via `~/.local/share/opencode/opencode.db` (SQLite). Sessions are discovered from the database grouped by project (worktree), paginated, searchable, readable, renameable, resumable (`opencode --session <id>`), forkable, and soft-deletable through the shared trash. Sub-agent sessions (parent_id non-null) are excluded from the main session list but are counted in usage stats. Live tail watches the database's WAL file for real-time message streaming. Full support across all app surfaces: sidebar agent switcher, Settings visibility toggle / launch args (`--auto`), CLI environment check with upgrade/upgrade-all integration, tray quick stats, and token/cost statistics.
- **Opencode pricing data** — pricing table includes opencode-specific models (DeepSeek V4, Qwen3.7, GLM-5, Minimax M2/M3, Kimi K2, Grok Build, etc.) sourced from models.dev, with direct-provider pricing preferred over opencode gateway pricing. Free-tier model pricing resolved by stripping the `-free` suffix. Non-chat models (text-embedding, tts, whisper, dall-e, video, imagine) are now filtered from the pricing table.
- **Markdown rendering enhancements** — inline math (`$...$`) and display math (`$$...$$`) now render via KaTeX (lazy-loaded, new `mathRender.ts` module). Added support for opencode's `~~~inline code~~~` syntax, strikethrough (`~~text~~`), highlight (`==text==`), superscript (`^text^`), subscript (`~text~`), footnote references (`[^n]`), and italic emphasis (`*text*`).
- **Mermaid diagram export as PNG** — each mermaid diagram now has a download button; clicking it opens a save dialog and exports the SVG at 2x scale as a PNG via Tauri's native save dialog and `writeBinaryFile`.
- **Split pane buttons in terminal strip** — horizontal and vertical split buttons added to the terminal tab strip for quick access without needing the right-click menu.
- **Agent switcher collapses to icons-only** — when 3 or more agents are enabled, the sidebar agent switcher collapses to icon-only buttons with tooltips for each agent name, keeping the topbar compact.
- **Bun package manager detection** — CLI environment check now detects and labels installations managed by Bun, with `bun add -g` as the upgrade strategy.
- **Pricing table noise filtering** — expanded model name filtering to exclude video, imagine, text-embedding, tts, whisper, and dall-e entries. Added o3/o4 model family classification.

### Bug Fixes

- **Mermaid gantt chart width** — gantt and other wide diagrams now use the SVG viewBox intrinsic width as min-width with horizontal scroll instead of being squeezed to the container width.
- **Session ID validation** — allowed underscores in session IDs (e.g. opencode's `ses_UUID_UUID` format) across resume, PTY spawn, GUI chat start, and fork operations.
- **Windows updates** — switched to NSIS installer for Windows updates.

### Internal

- **SessionSource trait extensions** — added `source_mtime`, `contains_text`, `watch_target`, and `validate_session_path` methods to the trait, allowing non-file-based agents (like opencode's SQLite backend) to provide their own implementations instead of relying on filesystem assumptions.
- **Binary file write** — new `writeBinaryFile` Tauri command and frontend API for base64-encoded binary writes, used by mermaid PNG export.

---

## [v0.2.6]

### Features

- **Antigravity CLI support** — replaces the Gemini agent with Antigravity CLI (`agy`) across the app. Sessions are discovered from `~/.gemini/antigravity-cli/brain/` and Antigravity Chat's IDE store, grouped by workspace, searchable, readable, renameable, and resumable through `agy --conversation <id>` when the session came from the CLI. IDE-created Antigravity Chat sessions are shown read-only with an explicit "open in Antigravity Chat" path instead of trying to resume them in a terminal.
- **Antigravity transcript parser** — reads `transcript_full.jsonl` when available, falls back to `transcript.jsonl`, strips `<USER_REQUEST>` wrappers, renders model thinking, assistant text, tool calls, tool results, web/question events, and `CODE_ACTION` unified diffs. Antigravity's checkpointed transcripts are handled as rewriteable files rather than append-only logs.
- **Antigravity app integration** — added Antigravity branding, labels, localized strings, README copy, Settings visibility / launch-args support, CLI environment detection, and update checks via Antigravity's platform manifest plus the built-in `agy update` command.
- **File and image extraction from messages** — text blocks now lift inline `@[path]`, `@path`, quoted paths, absolute paths, and pasted local image paths into dedicated file/image blocks so shared files and screenshots render like first-class attachments.
- **Codex transcript polish** — Codex assistant messages now carry the current model hint, function-call outputs can render image blocks, and post-processing lifts file/image references from Codex sessions too.
- **Custom stats date range** — the token/cost stats view gains a calendar range picker plus new presets (this month, last 3 months, last 6 months) alongside today / 7 days / 30 days. Custom spans flow through to the backend scanner as `custom:YYYY-MM-DD:YYYY-MM-DD`, and the header shows the resolved date range for whichever selection is active.
- **Paste images into the embedded terminal** — on macOS, Cmd+V of an image inside a TUI or shell tab now saves it to a temp file and pastes the file path into the terminal, so `claude` / `codex` pick it up as an attachment. Backed by a new `save_clipboard_image` command.
- **Open project folder from the context menu** — the project right-click menu gains an "Open folder" action that reveals the project directory in Finder.

### Bug Fixes

- **Local image rendering** — local image blocks now go through Tauri's asset protocol (`convertFileSrc`) so screenshots and file attachments render correctly in packaged builds.
- **Live session freshness** — when the app regains focus it re-reads the active session, restarts the watcher, checks terminal turn state, and briefly shows the live-tail indicator if new messages arrived while the window was inactive.
- **Antigravity watcher and turn tracking** — live-tail and turn-state polling use the preferred Antigravity transcript file and expose explicit check commands so rewritten transcripts and focus changes do not leave stale chat or tab status.
- **Antigravity stats behavior** — Antigravity is hidden from token/cost stats scopes because its transcripts do not expose usage data yet, while pricing keeps Gemini-family rows under an Antigravity/Gemini label for model reference.
- **Chat layout details** — assistant bubbles no longer stretch full-width, long tool chips truncate cleanly, and model badges strip trailing parenthesized provider/details noise.
- **Terminal cursor jitter during full-screen redraws** — the PTY reader now coalesces the many small chunks a TUI redraw (e.g. Claude Code's ink UI) gets split into within a short quiet window (2 ms debounce, 16 ms / 128 KB cap), so a whole redraw lands in a single frontend event and paints in one frame. This replaces the old "hide the cursor while output is busy" workaround — the cursor no longer visibly flickers to the status bar and back.
- **Agent switch no longer drops you to the welcome page** — switching agents in the sidebar now remembers each agent's last-visited project and its active tab and optimistically restores it (the project list loads in parallel and only falls back to welcome if that project is truly gone), instead of forcing you to re-pick a project every time.

## [v0.2.5]

### Features

- **Pin / sink individual sessions** — each session card in the list gains "Pin to top" (▲) and "Send to bottom" (▼) actions, mirroring the project-level pin/sink. Pinned sessions float to the top with a brand accent stripe; sunk sessions dim and fall to the bottom. Ordering is a stable sort layered over the existing mtime / search-relevance order (pinned → normal → sunk), the state is per-agent (keyed by `agent::path`) and persisted to `localStorage`, and toggling the active state clears it. Because the list is paginated by mtime, pinning a session only reorders the sessions already loaded — scroll far enough to load an old session and it floats up as expected. The per-card refresh button was removed to make room; refresh still lives in the list header.

### Performance

- **Virtual-scrolled chat transcript** — long sessions (thousands of messages) used to mount every `.msg-row` at once (tens of thousands of DOM nodes) and run all the Markdown / Shiki / Mermaid work synchronously on first paint, so opening and scrolling a big session stalled. The chat view now renders only the visible window plus an overscan buffer via `@tanstack/vue-virtual`, with per-row heights measured dynamically (`offsetHeight`, not `getBoundingClientRect` — the app's `body { zoom: 0.9 }` makes the latter under-measure by ~10% and visibly overlap rows) so async-expanding code blocks, images, and diagrams re-measure and self-correct without leaving gaps. Jump-to-message, flash-to-message, and scroll-to-bottom were re-implemented on top of the virtualizer's `scrollToIndex`.
- **Cached Markdown rendering** — `renderText` is now wrapped in a bounded (3000-entry) LRU cache keyed by raw text, so a message re-mounting as it scrolls in and out of the virtual window costs zero re-parsing instead of re-running the whole Markdown/table pipeline each time.
- **Faster session-list scan for large Claude projects** — switching into a project with many multi-tens-of-MB transcripts took ~10–16s. Three fixes: (1) the page window is scanned in parallel across cores with `rayon` (order preserved); (2) per-line scanning reads raw bytes with a reused buffer and pulls `type` / `cwd` from a cheap 4KB **prefix scan** (`json_str_field_prefix`) instead of `serde`-parsing every line — message bodies can be hundreds of MB, and full-parsing them was the root of the stall; only the few lines that actually need the title / custom-title / attachment get a full parse; (3) `scan()` results are cached by `(mtime, size)`, so re-entering an unchanged project returns instantly.
- **Watcher no longer re-reads on unrelated writes** — because the live watcher subscribes to the session file's *parent directory*, any sibling session being appended fired an event that triggered a full `read_session` of the (possibly tens-of-MB) target file, pinning CPU. `process_change` now compares a cheap `(mtime, size)` fingerprint first and short-circuits when the target file is unchanged; real appends change the fingerprint and are read as before.

## [v0.2.4]

### Features

- **Per-pane side chat** — the "btw" side chat is now scoped to each split pane rather than a single global session, so every pane holds its own independent side chat.

### Bug Fixes

- **Windows release build** — prefer PowerShell 7 and bypass `ExecutionPolicy` for the Windows CLI launch; exclude the dev-only `dev-mcp` MCP Bridge WebSocket plugin from release builds; and lower the release profile to `opt-level=2` with LTO disabled to get the Windows build linking reliably.
- **Blank sidebar on a fresh Claude install** — `list_projects` returned an error when `~/.claude/projects/` didn't exist yet (fresh CLI install); it now returns an empty list so bookmarks are still processed and the sidebar isn't blank.
- **Silence Windows dead-code warning** — macOS-only helpers (`wants_editor`, …) are now gated behind `#[cfg]` so they don't warn on the Windows build.

## [v0.2.3]

### Features

- **Split panes** — split any project view into multiple side-by-side / stacked panes, each with its own tab strip. Panes form a recursive horizontal/vertical tree, and every project remembers its own layout and per-pane tabs across relaunches. Drag tabs to reorder within a pane or move them between panes, with a live drop indicator and a drag preview that follows the cursor; saved-session pills interleave into the strip on the shared timeline. Hold `Cmd/Ctrl` to reveal project shortcuts (`Cmd/Ctrl+1–9`) and `Cmd/Ctrl+Shift` for tab shortcuts (`Cmd/Ctrl+Shift+1–9`); the focused pane is highlighted while the others dim
- **Code-block language labels** — each code block now shows an always-on language label to the left of its copy button, hidden for unknown / unsupported languages
- **Refreshed subscription model menu** — Fable 5 leads the list and Sonnet 5 replaces Sonnet 4.6 (moved to More, older Opus kept in More); Fable 5 also joins the Ultracode effort tier alongside Opus 4.7 / 4.8

### Bug Fixes

- **Highlight fenced blocks tagged with aliases** — fences labeled `js`, `ts`, `py`, `sh`, `yml`, … are normalized to Shiki's canonical language name so they highlight correctly instead of rendering plain
- **Honor ReClaude routing on Windows** — the ReClaude toggle is now respected by the Windows GUI-chat and embedded-terminal command builders (previously ignored); external-terminal resume intentionally keeps ReClaude off
- **New chat no longer bills credits by default** — auto-pick now skips the credit-gated Fable 5, so a fresh chat defaults to Opus 4.8 instead of consuming usage credits
- **Homebrew CLI upgrades** — upgrade commands are prefixed with `HOMEBREW_NO_INSTALL_FROM_API=1` to avoid API-based install failures

## [v0.2.1]

### Features

- **CLI Environment Check** — new Settings tab that detects locally installed Claude Code, Codex, and Gemini CLI versions, compares against npm latest, and offers one-click upgrade. Skeleton loading animation during initial scan; spinning refresh icon; per-CLI upgrade spinner. Upgrade results distinguish success from "version unchanged" with actionable error messages. Supports multi-node-manager environments (nvm / hermes / volta / fnm) by resolving the sibling `npm` binary and forcing `NPM_CONFIG_PREFIX` to the correct node root so upgrades write to the right global tree.
- **Diagnose install conflicts** — a single "Diagnose conflicts" button in the CLI Environment header runs `which -a` across all installed CLIs in parallel, deduplicates by canonical resolved path (filtering temp/shim paths), and reports each installation with its path, version, package manager source (Homebrew Cask / nvm / Volta / fnm / npm / system), and a "Default" badge. Warns when multiple installations are detected.
- **reclaude process wrapper** — optional "Use reclaude" toggle in Settings; when enabled, both embedded terminal (PTY) and GUI chat sessions prefix the agent command with `reclaude`, routing through the reclaude daemon's auth/proxy chain (same mechanism as IDE "Claude Process Wrapper"). Backend detects reclaude install status and daemon port from `~/.reclaude/state.json`.
- **TUI tab memory per project** — switching between projects now remembers and restores the active terminal tab for each project, including shell tabs. Persisted to localStorage alongside the existing View tab memory.

### Bug Fixes

- **Settings modal no longer closes on backdrop click** — only the close button (×) dismisses the modal, preventing accidental closure during CLI upgrades or diagnosis
- **Prevent double-spawn on rapid clicks** — added a `_spawnLock` guard to new-session, new-shell, and new-GUI-chat entry points so fast double-clicks don't open duplicate tabs
- **Windows console window suppressed** — `CREATE_NO_WINDOW` flag added to `curl` (usage API) and PowerShell (CLI env) subprocess spawns on Windows, preventing flashing console windows
- **Side chat closed on project/agent switch** — switching projects or agents now properly terminates the side chat subprocess and closes the floating panel

## [v0.2.0]

### Features

- **In-app agent chat (GUI)** — start or resume Claude / Codex / Gemini sessions in a built-in chat, no terminal required. Live pickers for model, reasoning-effort, and permission-mode; an **Auto mode** for hands-off runs; and per-session token / rate-limit badges (5h & weekly usage) in the composer
- **Reasoning-effort slider with Ultracode** — Faster↔Smarter effort slider aligned with the Claude client; Opus 4.7 / 4.8 expose an extra **Ultracode** notch (= `xhigh` + workflows) with an animated fill, and labels are unified (`xhigh` → "Xhigh")
- **Views history** — a per-project, searchable dropdown between the **List** and **View** tabs listing every view you've opened. Favorites (★) pin to the top and stay visually distinct; read vs chat entries are marked with their own icon; pick any entry to render it back into the View tab. New GUI chats join the list automatically and auto-title from their first message, matching the session list
- **Persistent View tab** — the View tab now stays put when you click **List** or open a terminal (it's a background tab, closed only via its own ×), and is restored on relaunch even if you quit while on a terminal tab

### Bug Fixes

- **Rename works in chat mode** — the rename (pencil) action in a live chat now opens the dialog; the new title syncs to the header, the session list, and the matching Views history entry
- **Views dropdown navigation** — switching between chat and read views from the dropdown no longer drops you back to the session list; renaming a session no longer closes the live chat / loses the View tab; right-click inside the dropdown is suppressed
- **Restore no longer loses the View tab** — quitting with a terminal/session tab in front used to discard the open View on relaunch; the View tab is now restored regardless of which tab was focused at exit

## [v0.1.15]

### Features

- **Per-agent visibility toggles** — added Claude / Codex / Gemini on-off switches in Settings → General; disabling an agent hides it from the sidebar and home-screen switcher so users who only use a subset (e.g. CC + Codex, no Gemini) get a cleaner UI. At least one agent must stay enabled, the preference persists across launches, and the app auto-switches away if you disable the agent you're currently viewing (fix #32)
- **Redesigned Settings modal** — reworked to a left icon-nav + right scrollable content layout (matching the Claude desktop client): wider fixed-height window, the nav lists General / Advanced / Shortcuts with icons and active highlighting, and each setting now shows its description inline beneath the title instead of behind an info-tooltip

## [v0.1.13]

### Features

- **Tab keyboard shortcuts** — added `Cmd/Ctrl+T` (new tab), `Cmd/Ctrl+W` (close tab), `Cmd/Ctrl+R` (rename tab) shortcuts with matching File menu entries and Settings page display
- **Fallback pricing for unknown models** — third-party models without an entry in the models.dev price table now use the average of Claude Sonnet 4.6 / Opus 4.7 / Opus 4.8 prices instead of showing $0.00

### Bug Fixes

- **Fix zero token stats for third-party model sessions** — sessions using non-Anthropic models (e.g. mimo-v2.5-pro) showed `0 in | 0 out | 0 cached | 0 written` because streaming intermediate JSONL entries (with 0 usage) shared the same `message.id` as the final entry; the aggregator's cross-file dedup kept the first (empty) record and skipped the real one. Now coalesces duplicate `message.id` entries within a turn, keeping the one with actual usage data.
- **Infer cache creation for third-party models** — models that report `cache_read_input_tokens` but always return `cache_creation_input_tokens: 0` now get cache creation inferred from the growth of `cache_read` between consecutive API calls, splitting each call's reported `cache_read` into actual read (previously cached) + inferred creation (delta), with per-call totals preserved
- **Fix terminal Cmd/Ctrl+W/T/R swallowed by xterm** — terminal key handler now lets `Cmd+W`, `Cmd+T`, `Cmd+R` (macOS) / `Ctrl+W`, `Ctrl+T`, `Ctrl+R` (Linux/Windows) bubble up to the app-level handler instead of being consumed by the terminal
- **Stop rendering system-injected records as "Me"** — compaction summaries, skill injections, task notifications, `<local-command-stdout>` / command output, system prompts, and cross-session "teammate" messages are `type:"user"` JSONL records but were never typed by the human, yet showed up as user ("Me") bubbles. They now render as labeled, collapsible cards with the agent prefix — notification / teammate payloads as clean key/value rows, command output as monospace, compaction & meta as markdown — and are excluded from the user-prompt count and the session-title fallback
- **Render `[Request interrupted by user]` as a system event** — the interrupt marker (and its "for tool use" variant) now renders as a centered system-event line, the same as the rename event, instead of a "Me" bubble
- **HTML / Markdown export parity** — exports now reflect the above: system-injected records export as labeled collapsible cards rather than "Me" bubbles, and interrupt / rename markers export as centered system lines
- **Fix wrapped buttons in the delete confirmation dialog** — "Delete permanently" / "Move to Trash" labels no longer break onto two lines; buttons keep their natural width and the dialog is slightly wider to fit all three on one row

## [v0.1.12]

### Features

- **Shell terminal tabs** — new "New terminal" option opens a pure interactive shell (no agent CLI) in the project directory, useful for running arbitrary commands alongside agent sessions
- **New session dropdown menu** — the "+" button in both the session list and terminal strip now shows a dropdown with "New agent session" and "New terminal" options
- **Shell tab persistence** — shell tabs are saved on exit and restored on next launch, same as agent tabs
- **Tab rename for shell & unmatched tabs** — shell tabs and newly created tabs that haven't matched a session yet can now be renamed directly via the tab context menu

### Bug Fixes

- **Fix Chinese IME input in global search** — remove `:value` binding that caused Vue re-renders to clobber the IME composition buffer, losing characters mid-input
- **Fix search blocking the main thread** — `search_sessions` now runs on `spawn_blocking` so the Tauri async runtime stays responsive during heavy searches
- **Fix new tab binding to wrong session** — snapshot known session paths at tab creation time so `reconcileNewTabs` only matches genuinely new sessions, not old ones with updated mtime

### Performance

- **Global search debounce reduced** — lowered from 900ms to 350ms for snappier results while still protecting against excessive searches
- **Removed ~280 lines of dead global search CSS** — styles are now scoped inside `GlobalSearchModal.vue`

## [v0.1.11]

### Features

- **In-app auto update** — integrated Tauri updater and process plugins; check for updates, download, install, and relaunch directly from the Settings page
- **Session replay fix** — prevent terminal from getting stuck in "running" state during session replay
- **Slash command fix** — avoid slash commands falsely triggering terminal running state

## [v0.1.10]

### Features

- **Worktree grouping in sidebar** — Claude Code worktree sessions are now nested under their parent project with an indented layout, git-branch icon, and collapsible toggle (fix #20)
- **Global search optimization** — dedicated 4-thread rayon pool for cross-project parallel scanning, cache-aware hot path to skip disk I/O, and CJK query optimization to avoid unnecessary `to_lowercase` allocation

### UI

- **Search UX** — increased debounce from 450ms to 900ms, show loading state immediately, clear previous results on new input, and improved IME composition handling

## [v0.1.9]

### Features

- **Terminal tab state persistence** — tabs are saved on exit and restored on next launch with lazy hydration (rendered as dashed pills, hydrated on click), avoiding N×xterm+PTY startup cost (fix #18)

### Performance

- **Replace `regex` with `regex-lite`** — binary size reduced by ~856KB; `regex-lite` drops Unicode tables and DFA engine, sufficient for the simple keyword patterns used in activity classification
- **Remove `staticlib` crate-type** — eliminates redundant static library output from release builds
- **Make `mcp-bridge` optional** — moved to an opt-in feature gate (`dev-mcp`), no longer linked in release builds
- **Compress cmux icons** — resized from 843×844 to 128×128 (179K+100K → 17K+13K)

### Bug Fixes

- Fix topbar border 1px misalignment with sidebar
- Replace all hardcoded Chinese error strings in Rust backend with English

## [v0.1.8]

### Features

- **Font size setting** — Settings → General gains a "Font Size" segment control (Small / Normal / Large) with visual "A" icon previews. Uses CSS `zoom` to scale the entire UI proportionally (0.9× / 1.0× / 1.1×). Persisted to localStorage; defaults to Normal.
- **Switch pricing data source to models.dev** — Replace LiteLLM upstream with [models.dev](https://models.dev) for model pricing. models.dev is significantly more responsive to new model launches (e.g. Claude Fable 5 was available on launch day while LiteLLM lagged by days). Covers all three CLI agents (Claude / Codex / Gemini) with cache pricing and context window data included.
- **Pricing page "open source" button** — Add an external-link icon next to the pricing page title that opens models.dev in the system browser.

### Bug Fixes

- Fix packaged `.app` unable to launch cmux — resolve cmux binary path via user login shell (`$SHELL -l -c "which cmux"`) so it works even when the bundled app has a minimal system PATH
- Fix flaky pricing unit test caused by concurrent `with_remote` / `seed_test_prices` sharing the same global key — tests now use exclusive keys and restore-on-cleanup semantics

## [v0.1.7]

### Bug Fixes

- Fix cross-agent tab fallback: closing a tab now only falls back to same-agent tabs, fix #14
- Fix PTY environment: remove `npm_config_prefix` to prevent nvm conflict for Codex/Gemini sessions
- Fix tab status border: only active tabs show colored state borders
- Fix view navigation on tab close: correctly return to session list or chat view
- Fix embedded Codex terminal light theme input box black background (PR #17 by @KodeChicken)
- Fix terminal tab "unviewed" status dot not clearing after click (PR #17 by @KodeChicken)
- Fix update checker version source
- Fix Clippy warnings and Rust compilation warnings

### Features

- Add terminal spawning overlay with loading animation
- Add batch project delete from sidebar
- Add settings tooltip icons replacing verbose descriptions
- Add Claude transcript JSONL-based tab status inference — terminal tabs now track idle/working/blocked/review for Claude sessions, not just Codex and Gemini (PR #17 by @KodeChicken)
- Add multi-agent session state tracking for terminal tabs (PR #13 by @KodeChicken)
- Add new session creation from terminal strip empty area (PR #13 by @KodeChicken)
- **Drop macOS accessibility permission requirement** — external terminal resume (iTerm2 / Warp / Terminal.app) now uses `open -a` with a temp `.command` script instead of osascript, eliminating the macOS accessibility permission prompt (fix #16)

### Performance

- Optimize shiki highlighting with explicit language imports and JS regex engine
- Add Rust release profile optimizations (strip, LTO, codegen-units, opt-level)
- Use `shallowRef` for large reactive arrays (chatMsgs, sessions, trash)

### Refactor

- Extract turn state classification into per-agent shared functions (codex.rs, gemini.rs)
- Clean up TerminalStrip context menu: remove unused browser actions
- Replace `.at(-1)` with `.slice(-1)[0]` in tests to fix IDE TS diagnostics
- Unify agent CLI launch command rendering (PR #13 by @KodeChicken)
- Extract independent `tabStatus` module from TerminalStrip, unify `statusKind` consumption (PR #17 by @KodeChicken)
- Optimize terminal tab status styling (PR #13 by @KodeChicken)

## [v0.1.6]

### Added

- **Shiki syntax highlighting** — fenced code blocks in chat messages now use [Shiki](https://shiki.style/) for accurate, language-aware syntax highlighting. Supports 30+ languages (JS/TS/Python/Rust/Go/Java/C/C++/HTML/CSS/Vue/Svelte/SQL/Dart/Swift/Kotlin/Zig/etc.) with three themes (github-light, github-dark, dracula) that auto-switch with the app theme. Line numbers are rendered via CSS counters. Falls back to plain `<pre>` for unrecognized languages.
- **Markdown link rendering** — `[label](url)` links in chat text now render as clickable `<a>` tags. External URLs open in a new tab; local file paths render with a `data-local-file-link` attribute for future file-open integration.
- **Markdown bullet list rendering** — consecutive `- item` / `* item` lines in chat text now render as proper `<ul>` lists with styled markers, instead of raw text with dashes.
- **Codex apply_patch visualization** — Codex's `apply_patch` tool calls now render as a structured diff view with per-file sections, operation badges (Update / Add / Delete / Move), and syntax-highlighted `+` / `-` / context lines with add/delete counts — instead of raw patch text.
- **Codex live session watcher** — opening a Codex session now tails the JSONL file in real-time (debounced filesystem events + polling fallback), streaming new messages into the chat view as the agent works. Same mechanism already existed for Claude; now covers Codex too.
- **Terminal tab "New session" button** — a `+` button pinned to the right end of the terminal strip. Always visible regardless of how many tabs are open (tabs scroll independently). Triggers the same new-session flow as `Cmd+N`.
- **Terminal tab right-click context menu** — right-click a terminal tab to rename, close, close others, or close all project terminals. Uses native Tauri Menu on macOS/Windows with an HTML fallback. (PR #12 by @KodeChicken)
- **Add Folder shortcut (`Cmd+O`)** — quickly add a bookmark folder from the keyboard.

### Changed

- **Chat layout responsive width** — `.chat-inner` max-width changed from fixed `860px` to `min(86%, 1800px)`, scaling better on wide screens.
- **Sidebar drag-resize improvements** — smoother resize handle UX. (PR #10 by @KodeChicken)
- **Sidebar project path display** — cleaner truncation of long paths. (PR #9 by @KodeChicken)

### Fixed

- **AppImage crash on Intel Arc + Wayland** — stripped bundled `libwayland-client.so` from the AppImage, which conflicted with the system library on Intel Arc GPUs causing a segfault at launch. The AppImage now uses the host's Wayland library. (fix #11)
- **Embedded terminal cursor flicker** — disabled cursor blink, added quiet-cursor mode during busy output, and deduplicated resize events to reduce visual noise in the embedded xterm.js terminal.
- **Terminal copy/paste shortcut conflict** — `Ctrl+C` / `Ctrl+V` in the embedded terminal no longer conflicts with the app's keyboard shortcuts. (PR #9 by @KodeChicken)
- **Sidebar not refreshing after adding bookmark** — `addBookmarkByPath` now calls `loadProjects()` before the duplicate check, so the sidebar updates immediately.

## [v0.1.5]

### Added

- **Menu bar stats (macOS)** — tray icon opens a native NSMenu with per-agent cost and token cards. Each card shows Today / 7 Days / 30 Days spend and token count, styled with brand-colored accent bars (Claude orange, Codex teal, Gemini purple). Auto-refreshes on every menu open with an animated spinner on the Refresh button. Background refresh every 5 minutes.
- **Hide messages via right-click** — right-click any message in the chat view to toggle its visibility. Hidden messages collapse into a subtle placeholder; a "Show all" action in the chat toolbar restores them. Per-session, persisted to localStorage.
- **Settings & menu bar stats screenshots** — added to all three READMEs (EN / 中文 / 日本語).

## [v0.1.4]

### Added

- **Jump to prompt** — chat header gains a locate button (crosshair icon) that opens a dropdown listing all user prompts in the session. Includes a search box for quick filtering. Clicking an entry scrolls to and flashes the target message.
- **External terminal app picker** — Settings → Advanced gains a "Terminal app" dropdown (only visible when "Use external terminal" is enabled) letting users choose which terminal to open for Resume / New Session. Supports **Terminal.app** (default), **cmux**, **iTerm2**, **Ghostty**, and **Warp**. Only installed terminals are shown; the app auto-detects availability on macOS at startup.
- **cmux deep integration** — smart workspace reuse: queries `cmux workspace list --json` to find an existing workspace with the same `cwd`; if a matching session is already running (checked via `surface.list` RPC), focuses the exact surface + triggers a blue flash indicator instead of opening a duplicate. New splits auto-pick direction (right vs down) by comparing the focused pane's width/height ratio. New workspaces are named after the project directory.
- **iTerm2 support** — uses AppleScript `write text` to open a new tab (or reuse the launch window when iTerm2 was just started).
- **Ghostty support** — launches via `open -a Ghostty.app` with `--working-directory` and `-e` flags.
- **Warp support** — activates Warp, opens a new tab, escapes from Agent mode, pastes the command via clipboard (⌘V) to avoid keystroke garbling, and sets the tab title via OSC 0 escape sequence.
- **Terminal app brand icons** — each option in the dropdown shows its brand icon: Terminal.app (terminal prompt), cmux (official PNG, theme-adaptive), iTerm2 (green `$_` on dark bg), Ghostty (official ghost mark from ghostty.org), Warp (cyan-purple gradient).
- **Launch arguments per agent** — Settings → Advanced lets users configure extra CLI flags (e.g. `--dangerously-skip-permissions`, `--yolo`) per agent, appended when resuming or starting sessions. One-click fill button for common defaults.
- **Smart default terminal** — if cmux is installed, it's auto-selected as the default external terminal on first launch. Persists once the user manually picks a different option.
- **Bookmarks** — add arbitrary folders to the sidebar per agent, stored in a backend file.

### Changed

- **Clear cache now resets all settings** — the "Clear cache" button in Settings → General now also resets external terminal toggle, terminal app choice, and launch arguments. Button is no longer disabled when pin/sink prefs are empty.
- **README refresh** — updated screenshots, added session resume and session GIF demos.

### Fixed

- **Terminal.app / iTerm2 double-window on cold start** — when the terminal app process was not running, `activate` + `do script` / `create window` would open two windows (one blank default + one with the command). Now detects whether the process is already running via System Events and reuses the launch window when cold-starting.
- **Spawn debounce** — added a 2-second per-cwd debounce to `spawn_terminal` to prevent accidental double-opens from rapid clicks. Applies to all terminal types.
- **Ghostty false detection** — `which ghostty` matched cmux's bundled Ghostty binary; now only checks `/Applications/Ghostty.app`.

## [v0.1.3]

### Added

- **Embedded terminal — "Open in window"** — resume or start a session inside the app window via an `xterm.js` terminal backed by `portable-pty`, instead of shelling out to Terminal.app. Chat header gains List / View / per-session terminal tabs.
- **macOS menu-bar tray + close-to-tray** — closing the window hides to a tray icon (Show / Statistics / Settings / Quit) instead of quitting; ⌘Q still exits.
- **Lossless JSON export + Export history view** — export a session to a re-importable JSON envelope; a sidebar view lists past exports (capped at 50, dedup by original path) that reopen the original transcript.
- **"This month" stats range** — separate from "Last 30 days"; matches calendar-month accounting tools and stays honest in the first week of the month.
- **Auto display-name for new models** — `claude-…` / `gpt-…` / `gemini-…` IDs render as "Opus 4.9" / "GPT-5.6 Codex" / "Gemini 3 Pro" by pattern, so brand-new versions display correctly with no table edit.
- **Live model pricing view + sidebar "More" menu** — kebab dropdown next to the trash icon holds Export history and a new **Live model pricing** page. Lists Claude / Codex / Gemini rates from LiteLLM upstream (Context / Input / Output / Cache read / Cache write columns), sticky search box (Enter to filter) + brand-icon anchor chips for jumping between families, Refresh button with a Stats-style full-area loader. Sorted newest version first (version-tuple based, tier names ignored); filters noise (`@default` aliases, `gpt-oss-*`, image / audio / realtime / transcribe / search-preview variants, `gpt-35-*` Azure dupes).
- **Markdown rendering for chat text — GFM tables + Mermaid diagrams** — `renderText` now parses GFM tables (header + separator + body, with `:--` / `--:` / `:--:` alignment, inline formatting in cells, horizontal scroll wrapper for wide tables) and ```mermaid``` fenced blocks (lazy-loaded `mermaid.js`, light/dark theme-reactive, error fallback shows source + error). Applies to all three agents through the single `ChatView` text path; HTML export bakes the SVG in at export time so the offline file renders without a runtime mermaid dep.
- **JSON syntax highlighting in tool calls and JSON tool results** — tool_use args (always JSON) are pretty-printed and colorized (key / string / number / bool / null tokens). Tool result text that looks like JSON (strips cat -n line numbers first to detect) gets the same treatment in place — original layout preserved, just tokens colored. Color tokens go light/dark via separate hues. Applies to both the chat view (via `ChatView.vue` + `ToolResult.vue`) and the HTML export. Detection is relaxed enough to still color the truncated JSON that `Read` with `limit:N` returns (the file is cut mid-object, `JSON.parse` can't succeed, but the `"key":` pattern survives).
- **Unified-diff syntax highlighting in tool results** — text-form `git diff` / patch output in `tool_result` (e.g. Bash running `git diff`) now renders with row-level coloring: file headers, `index` / `--- a/x` / `+++ b/x` metadata, `@@ -m,n +p,q @@` hunk headers, and `+` / `-` / context rows each get their own class. Existing structured-patch results (Claude `structuredPatch`) keep going through the interactive `DiffBlock` component — this only affects the text path that used to render as a plain `<pre>`. Detection order is diff-before-JSON so a patch on a JSON file colors as diff, not as malformed JSON. Mirrored to the HTML export.
- **Codex session filtering — internal & archived** — reads Codex's SQLite `threads` table to tag internal / archived sessions, queries the Codex app-server via JSON-RPC for ranking metadata, and scans `~/.codex/archived_sessions/` when archived visibility is enabled. Session list shows rank, "review session", and "archived" pill badges. Two new toggles in settings: "Show internal sessions" and "Show archived sessions" (archived defaults to on).
- **Codex (blue-toned light) & Dracula (classic dark) themes** — two new theme presets alongside the existing light / dark / system. CSS variables now cover font sizes and diff colors for full theme adaptability. Codex / Dracula items also appear in the native macOS menu bar.
- **"Use external terminal" setting** — toggle in Settings → Advanced to resume / start sessions in the system terminal (macOS Terminal.app, Windows PowerShell/cmd, Linux gnome-terminal/konsole/xterm) instead of the built-in xterm.js. Off by default (embedded terminal is the default).
- **Settings redesign — General / Advanced tabs** — settings split into two tabs: General (language, theme, data, about) and Advanced (terminal, Codex). Language and theme pickers replaced with custom Geist-style dropdown menus; Codex / terminal toggles use compact iOS-style switches instead of oversized card buttons.

### Changed

- **"All time" stats range → "Last 6 months"** — unbounded scans were slow and rarely useful; bounded to 6 calendar months back. Stale `'all'` in localStorage silently migrates to `'months6'`.
- **Model pricing now live from LiteLLM** — replaced the hand-curated table with a runtime fetch from LiteLLM (24h disk cache, retry on failure). New models price automatically on next launch.
- **Sidebar refresh button moved next to `{agent} · N projects`** — was on the topbar far from the agent switcher; now sits on the agent/count row so "refresh this agent" reads more naturally. Only the active agent reloads; other agents stay untouched.
- **Batch select / delete / export controls moved into list body** — previously lived in the topbar, causing visual overlap with other icon rows. Now inline in the session-list and trash-list headers, closer to the content they act on.
- **Single-session stats now fold Claude sub-agent JSONLs into the parent** — opening "Session stats" from a parent now feeds `<parent>/subagents/*.jsonl` into the same `Aggregator`, so cost / calls match the global by-session leaderboard row. Codex / Gemini unchanged (no sub-agent concept).
- **HTML export now renders GFM tables, inline markdown, and Mermaid diagrams** — previously was `escapeHtml + <br>` with no markdown. Now runs through `renderText`; mermaid SVGs are baked in at export time (one-shot, current-theme color), so the exported `.html` stays self-contained and offline-readable.
- **Release notes built from conventional commits** — `changelogithub` groups feat/fix/perf commits and adds a contributors footer.
- **Conventional commit types lowercased** — `Feat` → `feat`, etc., so `changelogithub` groups correctly.
- **Dev-only MCP Bridge in debug builds** — lets an AI assistant screenshot, snapshot the DOM, run JS, and watch IPC against the running app.

### Fixed

Stats accuracy — all numbers now reconcile with codeburn across every range:
- **Gemini cost ~2× too high and calls ~70% too high** — `tokens.input` is the total prompt size including cached tokens; we billed it at input rate **and** billed `cached` at cache-read rate, double-charging the cached portion. Fix: `input_tokens = totalInput − cached`. Also skip streaming sub-events (no `tokens` / no `model` / all-zero) that aren't independently billed.
- **Claude session count inflated by ~50%** — subagent JSONLs (`<project>/<parent>/subagents/agent-*.jsonl`) counted as standalone sessions in Stats but not in the sidebar. Fix: fold them onto the parent session id. Cost / tokens unchanged.
- **Anthropic 1h cache_creation underbilled ~6–8%** — 1h cache writes cost 1.6× the 5-minute rate, but we read only the lump-sum field and billed everything at 5min. Fix: read the `ephemeral_1h_input_tokens` split, add the 0.6× premium.
- **OpenAI reasoning tokens billed at $0** — `reasoning_output_tokens` (the hidden chain-of-thought) was dropped from `cost_usd`. Fix: bill at output rate. Anthropic unaffected (already folded into `output_tokens`).
- **Codex calls ~3× inflated** — every `function_call` / `agent_message` became a separate call. Fix: emit one `CallRecord` per `event_msg.token_count` event (one real API call), folding tool metadata into the next call.
- **"Today" / "7 days" / "30 days" rolled by 24h, not calendar days** — KPIs inflated 5–10× when a session crossed local midnight. Fix: switch to local calendar-day boundaries.
- **Range filter only at session level** — entire sessions counted toward "Today" if the file's mtime fell in the window, regardless of when individual turns happened. Fix: apply the window per-turn.
- **Cost / token formatting** — `$38.55` no longer rounds to `$39`, `240.5K` no longer rounds to `241K`. Always 2 decimals for USD, always 1 decimal for tokens.
- **Opus 4.8 mislabeled "Opus 4" and 3× overpriced** — missing pricing-table entry fell through to the base Opus 4 tier.
- **OpenAI / Gemini rate cards re-verified** — fixed several wrong or placeholder rows (`gpt-5.1-codex-mini`, the `gpt-5.2` / `5.4` / `5.5` family, `gemini-3.1-pro-preview`, `gemini-3-flash-preview`, `gemini-2.5-flash-lite` cache read).
- **`refresh_pricing` Tauri command was a sync blocking call** — `ureq::get(...).call()` ran on the main thread with a 20s timeout, freezing the webview (CSS animations, mouse cursor, everything). Now `async fn` + `spawn_blocking` — measured 0 stalls > 100ms over a 6-second refresh.
- **GPT 5.x not sorted above 4.x in pricing view** — `gpt-oss-120b` parsed "120" as a version, sorting it above `gpt-5` (version [5]). Now filters non-chat variants (`gpt-oss-*`, image / audio / realtime / transcribe / search-preview, `gpt-35-*` Azure dupes) before sort.
- **Horizontal scrollbar in light-mode markdown tables looked nearly black** — global scrollbar styles didn't reach `.md-table-wrap`. Now scoped scrollbar styling (7px thin, transparent track, `color-mix(--text, 22%, transparent)` thumb), Firefox via `scrollbar-color`.

## [v0.1.2] — 2026-05-25

### Added

- **Linux build target** — release pipeline now also runs on `ubuntu-22.04` and uploads `*.deb` (Debian/Ubuntu) and `*.AppImage` (portable) alongside the existing macOS `.dmg` / `.app.tar.gz` and Windows `.msi` / `*-setup.exe`. The runner installs `libwebkit2gtk-4.1-dev` + the standard Tauri 2 toolchain. Release notes body and asset-glob updated accordingly. Pinned to `ubuntu-22.04` (not `ubuntu-latest`) so binaries link against an older glibc and run on a wider range of distros. `.rpm` skipped on purpose — `rpmbuild` isn't preinstalled on the runner and AppImage covers RPM-based distros.
- **Animated "scanning" placeholder on the Stats page** — replaced the static bar-chart icon with a four-bar SVG that pulses on staggered delays, plus a trailing dots animation (`.` → `..` → `...`) on the "Discovering sessions" label. Honors `prefers-reduced-motion`.
- **Single-day fallback for the Daily activity chart** — sessions that only span one day used to render as a lonely dot in a vast empty plot. Now they fall through to a centered summary card (date · cost · calls) inside the same block; multi-day data still renders the dual-axis line+bar chart.
- **"Clear" button for the Welcome screen's Recent projects** — small muted action in the section header, removes the current agent's entire recents list (other agents untouched). i18n: English / 简体中文 / 繁體中文 / 日本語.
- **Stats overview dashboard** (`/stats`) — full-app Token usage & cost analytics page reachable from the sidebar topbar and per-session from the ChatTopbar's "Stats" button. Scope (All agents / Claude / Codex / Gemini) and Range (Today / 7d / 30d / All time) pill filters. Streaming partial snapshots: as the Rust worker chews through JSONLs it emits incremental aggregates so the UI fills in card-by-card instead of waiting for the whole scan.
- **Hero KPI cards** — Cost / Calls / Sessions / Cache hit rate as 4 standalone cards with icons (`Wallet` / `Activity` / `MessageCircle` / `Zap`), `font-variant-numeric: tabular-nums`, light-mode elevation + dark-mode borders, hover lift micro-interaction. Tokens-in / out / cached / written rendered below with hairline dividers.
- **Daily activity chart** — dual-axis: soft-grey columns for calls (right axis), brand smooth line + gradient area fill + emphasized points for cost (left axis). Renders via AntV G2 with theme-reactive colors.
- **By Model / By Activity** — horizontal bar charts with a curated 8-color categorical palette (`blue → violet → emerald → amber → pink → teal → indigo → orange`), light/dark variants. Tooltips show `$X.XX (Y.Y%)`.
- **By Project / Top Sessions / By Tool / By Shell / By MCP** — bar-list rows with rank, name, gradient progress bar, value, and meta count. Click a project or session row to jump straight into it.
- **Per-session stats** — entering Stats from the chat topbar locks scope to `session:<agent>:<path>`; daily, top-sessions, by-project panels are hidden in this mode (no meaning for a single file). "Back" button on the stats topbar returns to the original chat.
- **Codex cost & model breakdown** — recognizes the model from `turn_context.payload.model` (the JSONL location updated by recent Codex versions); pricing table covers `gpt-5` / `gpt-5.1` / `gpt-5.3-codex` / `gpt-5.5` / `o3` / `o4-mini` / `gpt-4o` / `gpt-4.1` families.
- **AntV G2 v5** replaces `chart.js` + `vue-chartjs` for all charts; smaller surface, theme-reactive, no canvas re-bind on data changes.
- **Shared `chartPalette.ts`** — single source of truth for chart brand / text-mute / grid / soft-bar / stroke colors and the categorical palette; used by every G2 chart so theme switches re-render all charts consistently.
- **Dashboard-style section cards** — white-on-tray layout (`stats-body` uses `--surface-2`, `stats-block` uses `--surface` with soft shadow in light mode, border-only in dark), card titles get a 3×14 blue→indigo accent stripe and a hairline divider, padding bumped 14→18/20 px for breathing room.
- **Live tail for in-progress sessions** — opening a session now starts a backend `notify` watcher (`watch_session` / `unwatch_session`) on its JSONL. New lines written by the CLI emit `session:append` events; the frontend appends them to the open chat and either auto-scrolls (if you're within 100 px of the bottom) or surfaces a `N new ↓` pill so you can jump down on demand. File truncation / replacement emits `session:reset` (full re-read) and deletion emits `session:gone` (closes the view). Single-subscription model + 200 ms debounce keeps overhead trivial. Read-only sessions in the Trash do not start a watcher. A pulsing `● Live` indicator next to the session ID confirms the watcher is active.

### Changed

- **"Check for updates" wired up to GitHub Releases** — previously a stub that always said "up to date". `api.checkUpdate()` now `fetch`es `/repos/jerrywu001/cc-sessions-viewer/releases/latest`, strips the leading `v` from `tag_name`, and compares against `app_version` with a small `compareVer` helper. 404 (no releases yet) is treated as up-to-date silently; other HTTP errors throw so the Settings modal surfaces "Update check failed". `UpdateInfo` gains an optional `htmlUrl` for a future "View release" link. The Rust `check_update` stub and unused `UpdateInfo` struct were removed.
- **Sidebar project toggle is now context-aware** — re-clicking the active project while a chat is open closes the chat and returns to the session list (instead of collapsing the project to the welcome screen). A second click — now on the list view — collapses as before. Two-step toggle matches user mental model: "back, then close".
- **`lib::agents` / `lib::stats` are now `pub`** so the `examples/test_dedup.rs` verification binary (which links against the lib crate externally) can drive the dedup pipeline directly. CI's `clippy --all-targets -- -D warnings` exercises this on every PR.
- **Daily activity bucketing fixed** — was bucketing all of a session's cost / calls / tokens into the day of `last_modified` (file mtime), so a Mon→Fri session dumped 5 days of cost on Friday. Now bucketed per-turn by `turn.timestamp_ms`, matching codeburn exactly (verified within 1% on real data).
- **Claude message-id dedup across files** — Claude JSONL records every assistant message across multiple lines (one per content block: thinking / text / tool_use), and resumed / forked / sub-agent sessions re-copy the same `message.id`. Aggregator now keeps a `seen_message_ids: HashSet<String>` and skips repeats; a session whose every call is a duplicate is dropped entirely (mirrors codeburn's `if (session.apiCalls > 0)`). Result: input tokens / cost roughly halved for users with heavy fork / sub-agent usage.
- **Claude sub-agent JSONLs counted in stats** — new `SessionSource::discover_stats_sessions` trait method enumerates `<projects>/<dir>/<sessionId>/subagents/*.jsonl` for Claude (Codex / Gemini keep the default impl). Chat session list is unchanged so sub-agents don't clutter the UI.
- **Codex `cached_input_tokens` semantics** — Codex's `total_token_usage.input_tokens` already includes cached tokens (unlike Claude where `input_tokens` is the new portion only). Aggregating naively double-counted cache reads, inflating `input` by ~8× for cache-heavy usage. Reader now subtracts `cached_input_tokens` so `in` / `cached` columns are disjoint and totals match codeburn.
- **`bar-fill` color** — switched from solid brand (orange-red) to a `blue → indigo` linear gradient (matching the chart palette's primary colors) so the activity / project / top-session / tool / shell / MCP bars stop looking like one giant red wall.

### Fixed

- **Single-session stats stuck on return** — `watch(props.session?.path)` was gated on `if (isSession.value)`, so when leaving session mode the gate flipped to `false` before the callback ran and the backend stream stayed on `session:<…>` scope, leaving the Stats page showing a single session's data even after "Back". Watcher now always calls `refresh()` and picks the global scope when `session` clears.
- **"By model" donut invisible** — legend at `position: 'right'` inside a narrow column starved the donut of width and truncated labels to `GP…`. Replaced with the categorical horizontal-bar chart.

## [v0.1.1] — 2026-05-23

### Changed

- **Release pipeline split into `build` + `publish`** — `tauri-action` no longer creates GitHub releases; a separate `softprops/action-gh-release` job downloads artifacts from the build matrix and publishes one release with `generate_release_notes: true` (auto-fills "What's Changed" + "New Contributors" from PRs / commits since the previous tag). Bundles upload unconditionally with `if-no-files-found: error` so missing artifacts fail fast. Added a `concurrency` group keyed by ref to prevent double tag-push fights.

### Added

- **Three-agent session support** — browse **Claude Code** (`~/.claude/projects/`), **Codex** (`~/.codex/sessions/`), and **Gemini CLI** (`~/.gemini/tmp/`) sessions in one app, normalized into a shared project → sessions → chat view. Claude and Codex group by project directory; Gemini groups by the `slug` directory, with `cwd` read from each slug's sibling `.project_root` file. Agent switch in the sidebar / welcome screen surfaces all three; Trash mixes them with color-coded badges.
- **Empty-state welcome screen** — with no project selected, the main area lists recently opened projects (per agent) for one-click jump-back, an agent switch, and a link to the project repository. Each recent entry can be removed individually via a hover-revealed ×.
- **Project sidebar** with pin / sink / rename and an agent switch (Claude 🟠 / Codex 🟢 / Gemini 🔵) at the top.
- **Chat replay** — text, thinking blocks, tool calls, structured diffs (Claude `structuredPatch`), inline images, sidechain badge. Tool results of non-file-mutating tools (read / search / shell etc.) embed inside the parent tool call's collapsible body; only Write / Edit / MultiEdit / NotebookEdit / `apply_patch` results stay as standalone diff rows so file mutations remain visually distinct.
- **In-session search with scope filter** — search across the whole conversation or scope to user messages, agent replies (incl. file-mutating edits), or tool noise; previous / next jump with a live match counter.
- **Collapse / expand all tool calls** in one click to hide tool clutter and focus on the conversation.
- **Image lightbox** for screenshots embedded in transcripts.
- **Session list keyword search** (Rust-side) — typing in the list toolbar hits a backend `search_sessions` over the current project, matching session titles **and your own message text** (the local array only carries metadata). Cancellable mid-typing in the React-Fiber style: every new keystroke aborts the in-flight scan and only fires a fresh one once input settles.
- **Session list toolbar** — sort by recency / size / message count, filter to sessions that have an ID, and multi-select for batch ops.
- **Global search** (⌘⇧F / Ctrl+Shift+F) — an Algolia-style overlay over the current agent, scoped to **session titles and your own messages** (assistant text, thinking blocks, and tool calls are intentionally excluded — that's where the noise lives). Click a hit to jump straight to the exact matching message with a flash animation. Keyboard-driven (↑↓ to navigate, ↵ to open, Esc to dismiss); recent queries are kept with per-entry removal.
  - **Performance** — rayon-parallel project scan + ASCII fast-path byte filter as a pre-screen + per-file `(path, mtime)` cache of extracted user-text; results capped at 200 server-side / 80 rendered with a "+N more" hint.
  - **Cancellability** — cooperative bail via an `AtomicU64` generation counter on the Rust side; any new request (or an explicit `cancel_search`) makes the running scan stop on the next loop check.
- **Resume or start fresh** — open Terminal in a project to resume an existing session (`claude --resume <id>` / `codex resume <id>` / `gemini …`) or start a brand-new one. Session-id is validated by a strict allowlist before shelling out.
- **New session in terminal** — start a fresh `claude` / `codex` / `gemini` session in a project's directory straight from the session-list header; the header also gains refresh and delete-project actions.
- **Export single session** to Markdown or HTML via native Save-As dialog; HTML inlines avatar SVGs and the full stylesheet so the file renders offline.
- **Batch export / delete** in the session list — toggle multi-select from the list toolbar to move many sessions to Trash in one go, or export them all into a chosen folder as Markdown / HTML (`export-YYYYMMDD-HHMMSS-{md,html}/`).
- **Soft-delete trash** shared across all three agents under `~/.claude/.session-viewer-trash/`; restore puts the JSONL back to its original parent dir; in-chat system-event row surfaces session renames.
- **Trash list improvements** — keyword-highlighted search, click a trashed entry to preview its full transcript, and a hover spotlight matching the session list.
- **Fly animations** — single-session restore arcs back to its project in the sidebar, and deleting a whole project arcs to Trash, mirroring the existing delete-to-trash animation.
- **Native application menu** — full **File / Edit / View / Find / Window / Help** menu on macOS with accelerators (⌘N new session, ⌘B toggle sidebar, ⌘E export, ⌘, settings, ⌘⌃T trash, ⌘F in-session search, ⌘G / ⌘⇧G prev/next match, ⌘⇧F global search). Theme and Language submenus use `CheckMenuItem` and stay in sync with the in-app prefs via a `menu:sync` event bridge.
- **macOS native chrome** — unified topbar (`NSToolbar` `unifiedCompact`), hidden title, drag region.
- **Light / dark / system theme**; reactive i18n in **English / 简体中文 / 繁體中文 / 日本語**, with first-launch auto-detection from the OS language (falls back to English when no locale matches).
- **Custom singleton `v-tooltip` directive** — replaces the native `title=` attribute everywhere; fades in / out with a 250 ms hover delay and flips above when there is no room below.
- **Agent brand icons** next to "Claude" / "Codex" / "Gemini" labels in the chat role tag, dispatched via a global `agentIcons` dictionary (`material-icon-theme:claude`, `arcticons:openai-chatgpt`, `material-icon-theme:gemini-ai`).
- **Vitest test suite** (309 unit tests across logic modules + leaf components, jsdom env) and a GitHub Actions CI workflow (typecheck, unit tests, `cargo clippy` / `cargo test`).

### Changed

- Toast notifications now appear top-center instead of bottom.
- Projects whose working directory no longer exists show a **"Directory missing"** tag; actions that depend on that directory (resume, new session, refresh) are hidden for them — in both the session list and the sidebar context menu.
- Clicking the already-selected project deselects it (toggle), returning to the welcome screen.
- The Trash toolbar hides its sort / multi-select controls when there is one item or none.
- Debounce intervals tuned per surface — 450 ms for the heavy global-search backend call, 280 ms for the session-list backend search and in-chat search, 220 ms for purely client-side filtering; all surfaces are IME-composition-safe.

### Fixed

- Queued user messages — text typed while the agent is still working — were dropped from the Claude transcript; they now render correctly, including messages that contain images.
- **Search-jump scroll** in long sessions — clicking a global-search hit could land at the wrong scroll position because images, code highlighting, and structured-diff blocks kept pushing the target row down after the initial scroll. `ChatView.flashMessage` now self-stabilizes via a rAF loop that re-reads `offsetTop` each frame for ~1.6 s and yields immediately on any user wheel / pointerdown / keydown.
