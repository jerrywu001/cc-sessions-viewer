<div align="center">

# Sessions Viewer

[![Version](https://img.shields.io/github/v/release/jerrywu001/cc-sessions-viewer?color=blue&label=version)](https://github.com/jerrywu001/cc-sessions-viewer/releases)
[![Star on GitHub](https://img.shields.io/github/stars/jerrywu001/cc-sessions-viewer?style=flat&logo=github&label=Star%20on%20GitHub)](https://github.com/jerrywu001/cc-sessions-viewer)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/jerrywu001/cc-sessions-viewer/releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![Downloads](https://img.shields.io/github/downloads/jerrywu001/cc-sessions-viewer/total)](https://github.com/jerrywu001/cc-sessions-viewer/releases/latest)
[![Vue 3](https://img.shields.io/badge/Vue-3-42b883?logo=vue.js&logoColor=fff)](https://vuejs.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**English** · [中文](README.zh-CN.md) · [日本語](README.ja.md) · [CHANGELOG](CHANGELOG.md)

<p align="center">A native desktop browser for <strong>Claude Code</strong>, <strong>Codex</strong>, <strong>Grok Build</strong>, <strong>Kimi Code</strong>, <strong>Pi</strong>, <strong>Antigravity CLI</strong>, and <strong>opencode</strong>.<br/>Read, search, and manage local session transcripts from all seven in one place.</p>

</div>

https://github.com/user-attachments/assets/9bcb92a8-e5b8-40e5-b492-af252162309b

---

## What it does

Sessions Viewer turns local agent transcripts into a searchable workspace. Open a project, inspect exactly what happened, then continue the work from the same place without manually hunting through JSONL files.

### Read and find context

- **Faithful replay** — preserve thinking chains, tool-call pairings, structured diffs, and inline screenshots.
- **Global search** — search across projects and jump to the exact matching message with `⌘⇧F`.
- **Jump to prompt** — scan every user prompt in a compact list, then scroll and flash the selected message.
- **Views history** — revisit recent read and chat views, with per-project search and favorites.

### Continue the work

- **Built-in chat** — start or resume Claude Code and Codex sessions with model, reasoning-effort (including Opus **Ultracode**), and permission-mode controls.
- **One-click resume** — open a session in an embedded terminal or in **Terminal.app**, **cmux**, **iTerm2**, **Ghostty**, or **Warp**.
- **Shell tabs** — run regular shell commands beside agent sessions; tabs persist across restarts.
- **Launch arguments** — configure per-agent CLI flags such as `--dangerously-skip-permissions` for new and resumed sessions.

### Keep projects organized

- **Split panes** — arrange side-by-side or stacked panes, drag tabs between panes, and keep each project's layout across restarts.
- **cmux integration** — reuse workspaces by working directory, find running sessions, choose smart split directions, and name tabs after directories.
- **Bookmarks** — pin frequently used folders to the sidebar for quick access.
- **Rename and trash** — sync session renames back to the CLI and soft-delete sessions with restore support.

### Understand usage and share results

- **Stats and pricing** — inspect token spend and cost by project, model, or tool using live LiteLLM pricing; macOS menu bar stats show Today / 7d / 30d totals per agent.
- **Flexible export** — export one session or a batch as offline-readable Markdown, HTML, or lossless JSON.
- **Read-only safety** — source JSONL files are never modified or removed.

### Supported session sources

Claude Code, Codex, Grok Build, Kimi Code, Pi, Antigravity CLI, and opencode. Grok Build, Kimi Code, and Pi provide history, terminal, export, analysis, and resume workflows; their GUI chat is intentionally not included.

## Screenshots

<details>
  <summary>Open the visual tour</summary>

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

</details>

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
This project is maintained in my spare time. Sponsorship helps cover ongoing development, bug fixes, and documentation.

- 🛠️ Continuous development and updates

- 🐛 Swift bug fixes and issue resolution

- 📚 Documentation improvements and expanded examples

For custom work or other special requests, contact me through one of the sponsorship options below. Requests start at US$50 and depend on current availability.

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
