<div align="center">

# Sessions Viewer

[![Version](https://img.shields.io/github/v/release/jerrywu001/cc-sessions-viewer?color=blue&label=version)](https://github.com/jerrywu001/cc-sessions-viewer/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/jerrywu001/cc-sessions-viewer/releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![Downloads](https://img.shields.io/github/downloads/jerrywu001/cc-sessions-viewer/total)](https://github.com/jerrywu001/cc-sessions-viewer/releases/latest)
[![Vue 3](https://img.shields.io/badge/Vue-3-42b883?logo=vue.js&logoColor=fff)](https://vuejs.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**English** · [中文](README.zh-CN.md) · [日本語](README.ja.md) · [CHANGELOG](CHANGELOG.md)

<p align="center">A native desktop browser for <strong>Claude Code</strong>, <strong>Codex</strong>, <strong>Grok Build</strong>, <strong>Kimi Code</strong>, <strong>Antigravity CLI</strong>, and <strong>opencode</strong>.<br/>Read, search, and manage local session transcripts from all six in one place.</p>

</div>

https://github.com/user-attachments/assets/9bcb92a8-e5b8-40e5-b492-af252162309b

---

## Key Features

- **Faithful replay** — thinking chains, tool-call pairings, structured diffs, and inline screenshots
- **Global search** — cross-project instant search (⌘⇧F) jumps to the exact message
- **In-app chat** — start or resume Claude Code or Codex sessions in a built-in chat with live model, reasoning-effort (incl. Opus **Ultracode**), and permission-mode pickers — no terminal required
- **Grok Build history + TUI** — browse, search, export, analyze, rename, trash/restore, and resume local Grok sessions; Grok GUI Chat is intentionally not included
- **Kimi Code history + TUI** — browse, search, export, analyze main and subagent usage, rename, trash/restore, and resume local Kimi sessions; Kimi GUI Chat is intentionally not included
- **One-click resume** — resume or start a session in an embedded terminal or external app — supports **Terminal.app**, **cmux**, **iTerm2**, **Ghostty**, and **Warp**
- **Shell terminal tabs** — open pure shell tabs alongside agent sessions for running arbitrary commands in the project directory; tabs persist across restarts
- **Split panes** — split any project into side-by-side or stacked panes, each with its own tab strip; drag tabs to reorder within a pane or move them between panes, with keyboard shortcuts for every action (see Settings → Shortcuts). Every project remembers its own layout across restarts
- **cmux deep integration** — auto-reuses existing workspace by cwd, locates running sessions with blue flash, smart split direction, and directory-named tabs
- **Launch arguments** — per-agent CLI flags (e.g. `--dangerously-skip-permissions`) appended on resume / new session
- **Jump to prompt** — locate button lists all user prompts; click to scroll and flash the target message
- **Views history** — per-project, searchable history of every view you've opened, with favorites; jump back to any past read or chat view in one click
- **Deep stats** — aggregate token spend and cost with live model pricing from LiteLLM; slice by project, model, or tool
- **Menu bar stats** — macOS tray icon shows at-a-glance Today / 7d / 30d cost and tokens per agent
- **Live model pricing** — browseable pricing table including Claude, Codex, and Grok / xAI, auto-updated from upstream
- **Flexible export** — single session or batches to offline-readable Markdown, HTML, or lossless JSON
- **Bookmarks** — pin any folder to the sidebar for quick access, per agent
- **Rename & delete** — session renames sync back to the CLI; soft-delete moves to shared trash with restore support
- **Read-only safety** — original JSONL is never touched, never `rm`

## Screenshots

<table>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/cover.png" alt="Main view — sidebar, sessions, and chat" />
      <p align="center"><em>Main view — sidebar, sessions, chat</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/chat.png" alt="Faithful replay — thinking, tool calls, structured diffs" />
      <p align="center"><em>Faithful replay — thinking, tool calls, structured diffs</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/split-screen.png" alt="Split panes — multiple sessions side by side" />
      <p align="center"><em>Split panes — multiple sessions side by side, drag tabs between panes</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/chat-preview.png" alt="In-app chat — Mermaid, tables, file mentions and image attachments" />
      <p align="center"><em>In-app chat — Mermaid & tables, @-mention files, attach images</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/session-resume.png" alt="Embedded terminal resume" />
      <p align="center"><em>Embedded terminal — one-click resume or new session</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/search.png" alt="Global search overlay" />
      <p align="center"><em>Global search (⌘⇧F) jumps to the message</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/stats.png" alt="Token & cost analytics" />
      <p align="center"><em>Token & cost analytics by project, model, tool</em></p>
    </td>
    <td width="50%">
      <img src="src/assets/sys-stats.png" alt="Menu bar stats — per-agent cost and token overview" />
      <p align="center"><em>Menu bar stats — per-agent cost & token overview</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/model-price.png" alt="Live model pricing table" />
      <p align="center"><em>Live model pricing</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/trash.png" alt="Shared trash with restore" />
      <p align="center"><em>Shared trash — soft-delete with one-click restore</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="src/assets/settings.png" alt="Settings — terminal picker and launch arguments" />
      <p align="center"><em>Settings — terminal picker & launch arguments</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/export.png" alt="Exported HTML preview" />
      <p align="center"><em>Exported HTML — fully offline, opens in any browser</em></p>
    </td>
  </tr>
</table>

## Install

Grab the latest installer from [Releases](https://github.com/jerrywu001/cc-sessions-viewer/releases):

| Platform | File |
| --- | --- |
| macOS (Apple Silicon + Intel) | `.dmg` |
| Windows x64 | `-setup.exe` / `.msi` |
| Linux x86_64 | `.deb` / `.AppImage` |

On macOS the `.app` is **ad-hoc signed but not notarized**, so first launch may show *"Apple cannot verify…"*. Two ways past it:

- Right-click the app in Finder → **Open** → confirm in the dialog (one-time).
- Or strip the quarantine attribute in Terminal:
  ```bash
  sudo xattr -dr com.apple.quarantine "/Applications/Sessions Viewer.app"
  ```

On Linux the `.AppImage` is portable — `chmod +x` and run. The `.deb` installs with:
```bash
sudo apt install ./cc-sessions-viewer_<ver>_amd64.deb
```

## Kimi Code

Kimi Code sessions are discovered from `$KIMI_CODE_HOME` (default: `~/.kimi-code`). The viewer reads each session's `state.json` and `agents/main/wire.jsonl`, and includes `agents/*/wire.jsonl` when calculating usage. Resume opens `kimi --session <id>`; new sessions run `kimi` in the selected project directory.

Kimi sessions are directory-backed. Rename updates Kimi session metadata; trash, restore, and permanent deletion operate on the complete session directory and keep `session_index.jsonl` in sync. Kimi worktree sessions are grouped by their real `cwd`.

Settings can install five user-level Kimi status hooks in `$KIMI_CODE_HOME/config.toml`. Only hooks managed by Sessions Viewer are changed, and invalid or incompatible TOML is left untouched. Kimi GUI Chat is not supported in this release.

For privacy, the viewer does not read `credentials/`, global logs, MCP configuration, or skills. Its Markdown/HTML exports use the viewed session only. `kimi export` is not invoked by the app; its diagnostic ZIP can include global logs, so inspect it before sharing.

## Development

```bash
git clone https://github.com/jerrywu001/cc-sessions-viewer.git
cd cc-sessions-viewer
npm install
npm run tauri dev      # dev mode
npm run tauri build    # bundle
```

Prereqs: Node 20+, Rust stable. See [`CLAUDE.md`](CLAUDE.md) for architecture notes.

## Contributing

PRs welcome. Please use [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, ...).

## Star History

<a href="https://www.star-history.com/?type=date&repos=jerrywu001/cc-sessions-viewer">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jerrywu001/cc-sessions-viewer&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jerrywu001/cc-sessions-viewer&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jerrywu001/cc-sessions-viewer&type=date&legend=top-left" />
 </picture>
</a>

## Sponsorship Support
Maintaining an open-source project requires significant time and resources. Your sponsorship will directly support:

- 🛠️ Continuous development and updates

- 🐛 Swift bug fixes and issue resolution

- 📚 Documentation improvements and expanded examples

### Ways to contribute:

- GitHub Sponsors
  
[GitHub Sponsors](https://github.com/sponsors/jerrywu001) (Recommended · Zero fees)

- Alipay/Wechat
  
<table style="display: flex; width: 500px;">
  <tr>
    <td style="margin-right: 16px;">
      <img style="width: 150px;" src="https://www.js-bridge.com/alipay.jpg" />
    </td>
    <td style="margin-right: 16px;">
      <img style="width: 150px;" src="https://www.js-bridge.com/wechat.jpg" />
    </td>
  </tr>
</table>

## License

[MIT](LICENSE) © jerrywu001 · [@jerrywu185](https://x.com/jerrywu185)

> Friend link: [linux.do](https://linux.do/)
