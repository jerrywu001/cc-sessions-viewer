<div align="center">

# Sessions Viewer

[![Version](https://img.shields.io/github/v/release/jerrywu001/cc-sessions-viewer?color=blue&label=version)](https://github.com/jerrywu001/cc-sessions-viewer/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/jerrywu001/cc-sessions-viewer/releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![Downloads](https://img.shields.io/github/downloads/jerrywu001/cc-sessions-viewer/total)](https://github.com/jerrywu001/cc-sessions-viewer/releases/latest)
[![Star on GitHub](https://img.shields.io/github/stars/jerrywu001/cc-sessions-viewer?style=flat&logo=github&label=Star%20on%20GitHub)](https://github.com/jerrywu001/cc-sessions-viewer)
[![Vue 3](https://img.shields.io/badge/Vue-3-42b883?logo=vue.js&logoColor=fff)](https://vuejs.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[English](README.md) · **中文** · [日本語](README.ja.md) · [CHANGELOG](CHANGELOG.md)

<p align="center">一个专为 <strong>Claude Code</strong>、<strong>Codex</strong>、<strong>Grok Build</strong>、<strong>Kimi Code</strong>、<strong>Pi</strong>、<strong>Antigravity CLI</strong> 和 <strong>opencode</strong> 打造的原生桌面浏览器。<br/>在一处读取、搜索并管理七个 CLI 的本地会话记录。</p>

</div>

https://github.com/user-attachments/assets/9bcb92a8-e5b8-40e5-b492-af252162309b

---

## 项目定位

Sessions Viewer 将本地 agent 会话记录整理成可搜索的工作区。打开项目，准确回看发生了什么，再从同一位置继续工作，无需手动翻找 JSONL 文件。

### 阅读与定位

- **忠实还原** — 完整呈现思考链路、工具调用配对、结构化 Diff 与内嵌截图。
- **全局搜索** — 跨项目搜索并直达具体消息，快捷键为 `⌘⇧F`。
- **定位提问** — 在紧凑列表中浏览所有用户提问，点击后滚动到目标消息并闪烁高亮。
- **视图历史** — 按项目保存可搜索的阅读和聊天视图，支持收藏和一键返回。

### 继续工作

- **应用内对话** — 在内置聊天里新开或续聊 Claude Code、Codex 会话，实时切换模型、推理强度（含 Opus **Ultracode**）与权限模式。
- **一键恢复** — 在窗口内嵌终端或 **Terminal.app**、**cmux**、**iTerm2**、**Ghostty**、**Warp** 中恢复或新建会话。
- **Shell 标签** — 在 agent 会话旁运行普通 shell 命令，标签状态跨重启保留。
- **启动参数** — 为每个 agent 配置 CLI 参数（如 `--dangerously-skip-permissions`），新建或恢复时自动追加。

### 管理项目

- **分屏** — 左右并排或上下堆叠多个分屏，拖动标签在分屏间移动；每个项目的布局跨重启保留。
- **cmux 集成** — 按工作目录复用 workspace、定位运行中的会话、智能选择拆分方向，并按目录名命名标签。
- **书签** — 将常用文件夹固定到侧栏，按 agent 独立管理。
- **重命名与回收站** — 会话重命名同步回 CLI，软删除后可从共享回收站恢复。

### 统计与导出

- **统计与实时价格** — 基于 LiteLLM 实时价目按项目、模型或工具分析 Token 与成本；macOS 菜单栏显示各 agent 的 Today / 7d / 30d 汇总。
- **灵活导出** — 单会话或批量导出为离线可读的 Markdown、HTML 或无损 JSON。
- **只读安全** — 原始 JSONL 始终只读，不会被修改或删除。

### 支持的会话来源

Claude Code、Codex、Grok Build、Kimi Code、Pi、Antigravity CLI 和 opencode。Grok Build、Kimi Code 与 Pi 支持历史记录、终端、导出、统计和续跑流程；它们的 GUI Chat 暂不包含在内。

## 截图

<details>
  <summary>展开查看产品截图</summary>

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

</details>

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
这个项目依靠业余时间维护。赞助会用于日常开发、问题修复和文档维护。

- 🛠️ 持续开发与更新

- 🐛 快速修复 Bug、解决问题

- 📚 完善文档、补充更多示例

如需定制功能或其他特殊支持，请通过下方赞助渠道联系我，50 美元起；是否承接以当前时间安排为准。

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
