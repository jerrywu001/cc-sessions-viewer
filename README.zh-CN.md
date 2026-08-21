<div align="center">

# Sessions Viewer

[![Version](https://img.shields.io/github/v/release/jerrywu001/cc-sessions-viewer?color=blue&label=version)](https://github.com/jerrywu001/cc-sessions-viewer/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/jerrywu001/cc-sessions-viewer/releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![Downloads](https://img.shields.io/github/downloads/jerrywu001/cc-sessions-viewer/total)](https://github.com/jerrywu001/cc-sessions-viewer/releases/latest)
[![Vue 3](https://img.shields.io/badge/Vue-3-42b883?logo=vue.js&logoColor=fff)](https://vuejs.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[English](README.md) · **中文** · [日本語](README.ja.md) · [CHANGELOG](CHANGELOG.md)

<p align="center">一个专为 <strong>Claude Code</strong>、<strong>Codex</strong>、<strong>Grok Build</strong>、<strong>Kimi Code</strong>、<strong>Antigravity CLI</strong> 和 <strong>opencode</strong> 打造的原生桌面浏览器。<br/>在一处读取、搜索并管理六个 CLI 的本地会话记录。</p>

</div>

https://github.com/user-attachments/assets/9bcb92a8-e5b8-40e5-b492-af252162309b

---

## 核心特性

- **忠实还原** — 完整呈现思考链路、工具调用配对、结构化 Diff 与内嵌截图
- **全局搜索** — 跨项目秒搜（⌘⇧F）直达具体消息
- **应用内对话** — 在内置聊天里新开或续聊 Claude Code、Codex 会话，实时切换模型、推理强度（含 Opus **Ultracode**）与权限模式，无需打开终端
- **Grok Build 历史与 TUI** — 浏览、搜索、导出、统计、重命名、回收站恢复和续跑本地 Grok 会话；明确不提供 Grok GUI Chat
- **Kimi Code 历史与 TUI** — 浏览、搜索、导出、统计主/子 agent 用量、重命名、回收站恢复和续跑本地 Kimi 会话；明确不提供 Kimi GUI Chat
- **一键恢复** — 在窗口内嵌终端或外部终端中直接恢复/新建会话——支持 **Terminal.app**、**cmux**、**iTerm2**、**Ghostty** 和 **Warp**
- **Shell 终端标签** — 在 agent 会话旁开启纯 shell 标签页，直接在项目目录执行任意命令；标签状态跨重启保留
- **分屏** — 把任意项目拆成左右并排或上下堆叠的多个分屏，每个分屏有独立的标签栏；标签可在分屏内重新排序，也可拖到其他分屏，每个操作都有快捷键（见 设置 → 快捷键）。每个项目的分屏布局跨重启保留
- **cmux 深度集成** — 按 cwd 自动复用已有 workspace，定位正在运行的会话并蓝色闪烁提示，智能选择拆分方向，新标签页自动以目录名命名
- **启动参数** — 为每个 agent 单独配置 CLI 参数（如 `--dangerously-skip-permissions`），恢复/新建会话时自动追加
- **定位提问** — 聊天标题栏的定位按钮列出所有用户提问，点击即滚动到目标消息并闪烁高亮
- **视图历史** — 每个项目独立、可搜索的「打开过的视图」历史，支持收藏；一键回到任意历史的只读或对话视图
- **深度统计** — 基于 LiteLLM 实时价目聚合 Token 消耗与成本，按项目/模型/工具多维分析
- **菜单栏统计** — macOS 托盘图标一览各 agent 的 Today / 7d / 30d 花费与 Token 量
- **实时模型价格** — 可浏览的 Claude、Codex 与 Grok / xAI 价格表，数据源自动更新
- **灵活导出** — 单会话或批量导出为离线可读的 Markdown / HTML / 无损 JSON
- **书签** — 将任意文件夹固定到侧栏快速访问，按 agent 独立管理
- **重命名与删除** — 会话重命名同步回 CLI，软删除移入共享回收站并支持还原
- **只读安全** — 原始 JSONL 全程只读，绝不物理抹除

## 截图

<table>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/cover.png" alt="主视图 — 侧栏、会话与聊天" />
      <p align="center"><em>主视图 — 侧栏、会话列表与聊天</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/chat.png" alt="忠实还原 — 思考、工具调用、结构化 Diff" />
      <p align="center"><em>忠实还原 — 思考、工具调用、结构化 Diff</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/split-screen.png" alt="分屏 — 多个会话并排显示" />
      <p align="center"><em>分屏 — 多个会话并排显示，标签可在分屏间拖动</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/chat-preview.png" alt="应用内聊天 — Mermaid、表格、@ 提及文件与图片附件" />
      <p align="center"><em>应用内聊天 — Mermaid 与表格渲染、@ 提及文件、粘贴图片</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/session-resume.png" alt="内嵌终端恢复会话" />
      <p align="center"><em>内嵌终端 — 一键恢复或新建会话</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/search.png" alt="全局搜索浮层" />
      <p align="center"><em>全局搜索（⌘⇧F）直达目标消息</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/stats.png" alt="Token 与成本统计面板" />
      <p align="center"><em>按项目 · 模型 · 工具维度分析 Token 与成本</em></p>
    </td>
    <td width="50%">
      <img src="src/assets/sys-stats.png" alt="菜单栏统计 — 各 agent 花费与 Token 概览" />
      <p align="center"><em>菜单栏统计 — 各 agent 花费与 Token 概览</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/model-price.png" alt="模型价格面板" />
      <p align="center"><em>实时模型价格面板</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/trash.png" alt="共享回收站与恢复" />
      <p align="center"><em>共享回收站 — 软删除，一键恢复</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="src/assets/settings.png" alt="设置 — 终端选择与启动参数" />
      <p align="center"><em>设置 — 终端选择与启动参数</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/export.png" alt="浏览器中预览导出的 HTML" />
      <p align="center"><em>导出 HTML — 完全离线，浏览器直接打开</em></p>
    </td>
  </tr>
</table>

## 安装

到 [Releases](https://github.com/jerrywu001/cc-sessions-viewer/releases) 下载对应平台的安装包：

| 平台 | 文件 |
| --- | --- |
| macOS (Apple Silicon + Intel) | `.dmg` |
| Windows x64 | `-setup.exe` / `.msi` |
| Linux x86_64 | `.deb` / `.AppImage` |

macOS 上 `.app` 是 **ad-hoc 签名、未公证**，首次打开可能弹出「Apple 无法验证…」。两种绕过方式：

- Finder 里右键应用 → **打开** → 弹窗里再确认（一次即可）。
- 或在终端清掉隔离属性：
  ```bash
  sudo xattr -dr com.apple.quarantine "/Applications/Sessions Viewer.app"
  ```

Linux 上 `.AppImage` 是便携格式 —— `chmod +x` 后直接运行。`.deb` 安装：
```bash
sudo apt install ./cc-sessions-viewer_<ver>_amd64.deb
```

## Kimi Code

Kimi Code 会话从 `$KIMI_CODE_HOME` 发现，默认目录为 `~/.kimi-code`。应用读取每个会话的 `state.json` 与 `agents/main/wire.jsonl`，统计时会纳入 `agents/*/wire.jsonl` 的子 agent 用量。恢复会话执行 `kimi --session <id>`；新会话会在当前项目目录执行 `kimi`。

Kimi 会话以目录为存储单元。重命名更新 Kimi 会话 metadata；删除、恢复和永久删除都以完整会话目录为单位，并同步 `session_index.jsonl`。worktree 会话按真实 `cwd` 归类。

设置页可在 `$KIMI_CODE_HOME/config.toml` 安装五个用户级 Kimi 状态 hooks。应用只修改自己管理的 hooks；TOML 无法解析或顶层类型不兼容时不会写入。本版本不支持 Kimi GUI Chat。

隐私方面，应用不会读取 `credentials/`、全局日志、MCP 配置或 Skills。Markdown/HTML 导出只使用当前查看的会话；应用不会调用 `kimi export`，其诊断 ZIP 可能包含全局日志，分享前请自行检查。

## 开发

```bash
git clone https://github.com/jerrywu001/cc-sessions-viewer.git
cd cc-sessions-viewer
npm install
npm run tauri dev      # 开发模式
npm run tauri build    # 打包
```

依赖：Node 20+、Rust stable。架构详见 [`CLAUDE.md`](CLAUDE.md)。

## 贡献

欢迎 PR。请使用 [Conventional Commits](https://www.conventionalcommits.org/)（`feat:` / `fix:` / `docs:` ...）。

## Star History

<a href="https://www.star-history.com/?type=date&repos=jerrywu001/cc-sessions-viewer">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jerrywu001/cc-sessions-viewer&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jerrywu001/cc-sessions-viewer&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jerrywu001/cc-sessions-viewer&type=date&legend=top-left" />
 </picture>
</a>

## 赞助支持
维护一个开源项目需要投入大量时间与精力。你的赞助将直接用于：

- 🛠️ 持续开发与更新

- 🐛 快速修复 Bug、解决问题

- 📚 完善文档、补充更多示例

### 赞助方式：

- GitHub Sponsors
  
[GitHub Sponsors](https://github.com/sponsors/jerrywu001)（推荐 · 零手续费）

- 支付宝 / 微信
  
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
