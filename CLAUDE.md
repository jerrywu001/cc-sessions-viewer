# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this app is

A macOS Tauri 2 desktop app (Vue 3 + Rust) for browsing, viewing, and trashing
local session transcripts from coding agent CLIs — **Claude Code**, **Codex**,
**Grok Build**, **Antigravity CLI (agy)**, and **opencode**. Each CLI stores
JSONL transcripts in its own on-disk layout; this app normalizes them all into
the same project → sessions → chat UI, plus a soft-delete trash that survives
across agents. Grok supports local history and terminal workflows, but not GUI
Chat. The app is read-only against original transcript contents; deletion is a
move into the shared trash, never `rm`.

## Commands

```bash
npm run tauri dev        # full dev (Tauri shell + Vite on :1420)
npm run tauri build      # bundle .app / .dmg into src-tauri/target/release/bundle/
npm run dev              # web-only Vite preview; Tauri invokes will fail
npm run build            # vue-tsc --noEmit + vite build
npm test                 # vitest watch mode
npm run test:run         # vitest single run (CI)
npm run test:coverage    # vitest single run + v8 coverage report
```

There is no linter wired up — `npm run build` (which runs `vue-tsc --noEmit`
first) is the typecheck step.

Unit tests run on **Vitest** (jsdom env) and live under `test/`, mirroring
`src/`. They cover the agent-agnostic logic modules (`format`, `i18n`,
`settings`, `chatToolbar`, `trashToolbar`, `sessionsToolbar`, `export`, `api`,
`fly`, `tooltip`) and the leaf components (`DiffBlock`, `ToolResult`,
`CollapsibleBox`, `Sidebar`, `SidebarTopbar`, `SessionsTopbar`, `TrashTopbar`,
`SettingsModal`). Config is `vitest.config.ts` (separate from
`vite.config.ts`); jsdom polyfills for `matchMedia` / `ResizeObserver` /
`Element.animate` live in `test/setup.ts`. `App.vue`, `views/`, and `modals/`
are stateful shells left to manual/e2e testing and excluded from coverage.
`test/tsconfig.json` is IDE-only — the production build never type-checks
`test/`.

Vite is locked to port `1420` (strictPort) because `tauri.conf.json` hardcodes
that URL. `src-tauri/**` is excluded from Vite's watcher; Rust changes are
picked up by Tauri's own dev loop.

## Architecture

### Two-side split

- **Frontend** (`src/`) is a thin Vue 3 SPA. State lives in `App.vue` refs;
  there is no store. All persistence besides `localStorage` (lang/theme/pin
  prefs) goes through Tauri.
- **Backend** (`src-tauri/src/`) owns *all* filesystem I/O and JSONL parsing.
  Frontend calls it via the `#[tauri::command]` functions in `lib.rs`, wrapped
  by `src/api.ts`. The full handler list lives in `tauri::generate_handler!` at
  the bottom of `lib.rs`; keep it in sync.

The backend is split into:

```
src-tauri/src/
├── lib.rs           // Tauri commands + macOS setup; pure dispatch, no parsing
├── types.rs         // Serializable types shared with the frontend
├── util.rs          // dirs / time / jsonl / text helpers (agent-agnostic)
├── trash.rs         // soft-delete / restore / list / empty (agent-agnostic)
└── agents/
    ├── mod.rs       // `SessionSource` trait + `source(agent)` dispatcher
    ├── claude.rs    // ClaudeSource impl  (~/.claude/projects/<dir>/...)
    ├── codex.rs     // CodexSource impl   (~/.codex/sessions/<YYYY>/...)
    ├── grok.rs      // GrokSource impl    ($GROK_HOME/sessions/<cwd>/<id>/...)
    ├── agy.rs       // Antigravity CLI source
    └── opencode.rs  // opencode source
```

When adding a Tauri command, define it in `lib.rs`, register it in
`tauri::generate_handler!`, then expose it from `api.ts` with the matching
TypeScript types in `src/types.ts`. `serde(rename_all = "camelCase")` is set on
every type in `types.rs` so Rust snake_case fields land in JS as camelCase.

### Session-source abstraction (adding a new agent)

The backend hides each agent's on-disk layout behind a single `SessionSource`
trait defined in `agents/mod.rs`. Currently:

| Agent  | Layout                                                              | Project grouping                |
| ------ | ------------------------------------------------------------------- | ------------------------------- |
| Claude | `~/.claude/projects/<dir>/<sessionId>.jsonl`                        | by project directory            |
| Codex  | `~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl`                | by the `cwd` recorded *inside* each file |
| Grok Build | `$GROK_HOME/sessions/<group>/<session-id>/updates.jsonl` (`$GROK_HOME` defaults to `~/.grok`) | by `summary.json.info.cwd` |
| agy    | `~/.gemini/antigravity-cli/brain/<uuid>/.system_generated/logs/transcript.jsonl` | by `workspace` from `history.jsonl`; fallback: first tool_call path |
| opencode | opencode local session storage | by source metadata |

To add a new agent:

1. Create `src-tauri/src/agents/<name>.rs` with a `<Name>Source` unit struct
   that implements `SessionSource` (every method calls the agent's private
   parsing helpers in the same file).
2. Add `pub mod <name>;` and a match arm in `agents::source()`.
3. Add `"<name>"` to the `Agent` union type in `src/types.ts` — sidebar /
   agent-switcher pick it up automatically.

That's it. The Tauri commands (`list_projects`, `list_sessions`,
`read_session`, `rename_session`, `soft_delete_session`, `resume_session`, …)
all dispatch through `agents::source(&agent)?.<method>()`, so no command
plumbing changes. **Do not** add agent-specific match arms in `lib.rs` or
`trash.rs`; if you can't fit a piece of logic on the trait, the trait shape is
wrong — fix it there.

`list_sessions` is paginated; it sorts by mtime cheaply and only deep-parses
the window slice. `read_session` is the only call that walks the full file.

### Image extraction is per-agent

Image rendering is uniform on the frontend (`Block { kind: "image", imageSrc }`
→ `<img :src="b.imageSrc">`), but each agent encodes images differently:

- Claude: `content[].type == "image"`, `source.{base64|url}`.
- Codex: paired records — `response_item.message` (role=user) carries
  `input_image` blocks with the actual `image_url`, while `event_msg.user_message`
  carries the clean typed text. `agents/codex.rs::read` buffers the images and
  attaches them to the matching `event_msg.user_message` so the user bubble
  ends up as `[image, text]`.

The agent contract is `SessionSource::image_src(block) -> Option<String>`; a
new agent just implements that and uses it inside its own `read_session`.

### Trash is shared across agents

`trash.rs` (one shared module, not per-agent) moves a validated storage unit
into `~/.claude/.session-viewer-trash/` with metadata describing its original
path, agent, project label, deletion timestamp, and storage kind. Most agents
use a JSONL file unit; Grok uses its complete session directory, with
`updates.jsonl` as the display/read entry. The trash dir lives under `~/.claude`
regardless of agent. Restore validates the destination and recreates its parent
before moving the original file or directory back; it must never overwrite an
existing session. The display title remains delegated to
`SessionSource::trash_title`.

### Worktrees

The app-created Git worktrees under `.claude/worktrees/` are shared physical
directories. Claude, Codex, and Grok Build group sessions by `cwd`, so the
worktree UI includes all three and warns with their combined session count
before deletion. Deletion clears only sessions tied to that cwd, then removes
the Git worktree; it does not manage Grok's own worktree registry or delete
`$GROK_HOME/worktrees.db`.

### Diff parsing in tool results

When a Claude `tool_result` carries a `structuredPatch`, `parse_structured_patch`
in `agents/claude.rs` converts it into the `DiffHunk[]` shape consumed by
`components/DiffBlock.vue`. Anything not in that shape just shows as text in
`<pre>`. The frontend does not parse diffs itself. If a future agent also
emits structured diffs, give it its own parser in its agent module rather than
hoisting `parse_structured_patch` into `util.rs`.

### Resume = AppleScript → Terminal

`resume_session` shells out to `osascript` to open Terminal.app, `cd` into the
project dir, and run a per-agent CLI (`claude --resume <id>` /
`codex resume <id>` / …). The CLI string comes from
`SessionSource::resume_cli`, and `lib.rs` validates the session id with a
strict allowlist (`[A-Za-z0-9-]+`) because the id is interpolated into a shell
command.

### Windows PowerShell command execution — INVARIANTS

**PowerShell is NOT a POSIX shell.** Treating it like `sh` has silently broken
"node CLIs not recognized on Windows" 3+ times (empty results / false "not
installed" / `?` versions, no crash). When building a command string for
`powershell.exe`/`pwsh.exe` (`cli_env.rs`, `agent_command.rs`, `pty.rs`):

1. **`where.exe`, never bare `where`** — `where` is an alias for `Where-Object`.
2. **Quoted path → call operator: `& 'C:\x.exe' --version`** — a bare `'…' --version`
   is a parse error. (A bare command name on PATH doesn't need `&`.)
3. **Escape `'` by doubling (`''`), not POSIX `'\''`** — see `powershell_quote`.
4. **Prefix every CLI command with `powershell_refresh_path()`** — GUI-inherited PATH
   can miss nvm/npm dirs; it rebuilds `$env:Path` from the registry (`$processPath`
   first — don't reorder).
5. **`-ExecutionPolicy Bypass`** — npm/nvm shims are `.ps1`, blocked by default.

Keep `#[cfg(windows)]` shell strings separate from their `#[cfg(unix)]` twins (see
`get_version_at_path`); never share one string across both.

### macOS titlebar / traffic lights

The CSS topbar is 40px and shares background with the sidebar; the unified
look depends on AppKit growing the native titlebar to match. `pin_traffic_lights`
in `lib.rs` attaches an empty `NSToolbar` with `unifiedCompact` style — the
*supported* AppKit way to extend the titlebar. The setup hook re-pins on
`WindowEvent::Resized` (and intentionally *not* on Focused/ThemeChanged, which
breaks click→drag tracking). Don't try to manually `setFrameOrigin` the
window buttons; it visually works but corrupts drag-region tracking.

### Reactive i18n + theme

- `src/settings.ts` holds `lang` and `theme` as `ref`s persisted to
  `localStorage`. `applyTheme()` is wrapped in `watchEffect`, so toggling
  theme/lang re-renders Vue templates that read those refs automatically.
- `t(key, vars)` in `src/i18n.ts` reads `lang.value` — that read is what makes
  every template using `t()` reactive. Don't cache `t()` results outside of a
  computed/template.

### Design system

`src/style.css` defines a Codex-inspired neutral token set
(`--surface`, `--surface-hover`, `--border`, `--text`, `--accent`, ...) with a
`:root` (light) and `:root.theme-dark` override block. Brand color
(`--brand` = Claude orange or Codex green) is only used for tiny accents like
the active-project count badge and the agent badge in the trash list — primary
buttons and active surfaces use neutral foreground inversion (Codex style).

Icons are inline SVG components in `src/components/icons.ts`. Do not introduce
emoji icons in chrome — they were intentionally removed for a cleaner look.
Tailwind v4 is installed but most styling uses the CSS-variable tokens above;
new components should follow the existing class-name convention rather than
inlining utility classes.

Tooltips use the custom `v-tooltip` directive (registered in `src/main.ts`,
implemented in `src/tooltip.ts`) rather than the native `title=` attribute —
native tooltips render in a system font and look out of place in this UI.
When adding a new button or icon, write `v-tooltip="t('...')"`, not `:title`.
