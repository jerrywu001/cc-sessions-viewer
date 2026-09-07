<div align="center">

# Sessions Viewer

[![Version](https://img.shields.io/github/v/release/jerrywu001/cc-sessions-viewer?color=blue&label=version)](https://github.com/jerrywu001/cc-sessions-viewer/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/jerrywu001/cc-sessions-viewer/releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![Downloads](https://img.shields.io/github/downloads/jerrywu001/cc-sessions-viewer/total)](https://github.com/jerrywu001/cc-sessions-viewer/releases/latest)
[![Star on GitHub](https://img.shields.io/github/stars/jerrywu001/cc-sessions-viewer?style=flat&logo=github&label=Star%20on%20GitHub)](https://github.com/jerrywu001/cc-sessions-viewer)
[![Vue 3](https://img.shields.io/badge/Vue-3-42b883?logo=vue.js&logoColor=fff)](https://vuejs.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[English](README.md) · [中文](README.zh-CN.md) · **日本語** · [CHANGELOG](CHANGELOG.md)

<p align="center"><strong>Claude Code</strong>、<strong>Codex</strong>、<strong>Grok Build</strong>、<strong>Kimi Code</strong>、<strong>Pi</strong>、<strong>Antigravity CLI</strong>、<strong>opencode</strong> 専用のネイティブデスクトップブラウザ。<br/>7 つの CLI のローカルセッション履歴を一元的に読み取り、検索し、管理します。</p>

</div>

https://github.com/user-attachments/assets/9bcb92a8-e5b8-40e5-b492-af252162309b

---

## 概要

Sessions Viewer は、ローカルのエージェントセッション履歴を検索可能なワークスペースにまとめます。プロジェクトを開いて内容を正確に振り返り、JSONL ファイルを手作業で探すことなく同じ場所から作業を続けられます。

### 読む・探す

- **忠実な再現** — 思考プロセス、ツール呼び出しのペアリング、構造化 Diff、インライン画像を完全に表示。
- **グローバル検索** — プロジェクトを横断して検索し、`⌘⇧F` で該当メッセージへ直行。
- **プロンプトへジャンプ** — すべてのユーザープロンプトを一覧から選び、対象メッセージへスクロールしてハイライト。
- **ビュー履歴** — プロジェクトごとに閲覧・チャットビューの履歴を保存し、検索やお気に入り、一発復帰に対応。

### 作業を続ける

- **アプリ内チャット** — Claude Code と Codex のセッションを内蔵チャットで新規作成・再開。モデル、推論強度（Opus **Ultracode** 対応）、権限モードを切り替え可能。
- **ワンクリック再開** — 埋め込みターミナルまたは **Terminal.app**、**cmux**、**iTerm2**、**Ghostty**、**Warp** でセッションを再開・新規作成。
- **Shell タブ** — エージェントセッションの横で通常のシェルコマンドを実行でき、タブは再起動後も保持。
- **起動引数** — エージェントごとに CLI フラグ（例：`--dangerously-skip-permissions`）を設定し、再開・新規作成時に自動追加。

### プロジェクトを整理する

- **画面分割** — 左右または上下のペインに分割し、タブをペイン間でドラッグ。プロジェクトごとのレイアウトは再起動後も保持。
- **cmux 統合** — 作業ディレクトリでワークスペースを再利用し、実行中のセッションを見つけ、分割方向を自動選択。タブ名にはディレクトリ名を使用。
- **ブックマーク** — よく使うフォルダをサイドバーにピン留めし、エージェントごとに管理。
- **リネームとゴミ箱** — セッション名の変更を CLI に同期し、ソフト削除したセッションは共有ゴミ箱から復元可能。

### 利用状況を把握・共有する

- **統計と料金** — LiteLLM のリアルタイム料金で、プロジェクト・モデル・ツール別にトークン消費とコストを分析。macOS のメニューバーには各エージェントの Today / 7d / 30d 集計を表示。
- **柔軟なエクスポート** — 単一または複数セッションをオフラインで読める Markdown、HTML、可逆 JSON として保存。
- **読み取り専用の安全性** — オリジナルの JSONL は変更・削除しません。

### 対応するセッションソース

Claude Code、Codex、Grok Build、Kimi Code、Pi、Antigravity CLI、opencode に対応しています。Grok Build、Kimi Code、Pi では履歴、ターミナル、エクスポート、分析、再開のワークフローを利用できますが、GUI Chat は意図的に含めていません。

## スクリーンショット

<details>
  <summary>ビジュアルツアーを開く</summary>

<table>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/cover.png" alt="メインビュー — サイドバー、セッション、チャット" />
      <p align="center"><em>メインビュー — サイドバー、セッション一覧、チャット</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/chat.png" alt="忠実な再現 — 思考、ツール呼び出し、構造化 Diff" />
      <p align="center"><em>忠実な再現 — 思考、ツール呼び出し、構造化 Diff</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/split-screen.png" alt="画面分割 — 複数セッションを並べて表示" />
      <p align="center"><em>画面分割 — 複数セッションを並べて表示、タブをペイン間でドラッグ</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/chat-preview.png" alt="アプリ内チャット — Mermaid、表、ファイル メンション、画像添付" />
      <p align="center"><em>アプリ内チャット — Mermaid・表の描画、@ファイル メンション、画像添付</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/session-resume.png" alt="埋め込みターミナルでセッション再開" />
      <p align="center"><em>埋め込みターミナル — ワンクリックで再開・新規作成</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/search.png" alt="グローバル検索オーバーレイ" />
      <p align="center"><em>グローバル検索（⌘⇧F）で目的のメッセージへ直行</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/stats.png" alt="トークン・コスト分析ダッシュボード" />
      <p align="center"><em>プロジェクト · モデル · ツール別のトークン・コスト分析</em></p>
    </td>
    <td width="50%">
      <img src="src/assets/sys-stats.png" alt="メニューバー統計 — 各エージェントのコストとトークン概要" />
      <p align="center"><em>メニューバー統計 — 各エージェントのコストとトークン概要</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/screenshots/model-price.png" alt="モデル料金テーブル" />
      <p align="center"><em>リアルタイムモデル料金表</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/trash.png" alt="共有ゴミ箱と復元" />
      <p align="center"><em>共有ゴミ箱 — ソフト削除とワンクリック復元</em></p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="src/assets/settings.png" alt="設定 — ターミナル選択と起動引数" />
      <p align="center"><em>設定 — ターミナル選択と起動引数</em></p>
    </td>
    <td width="50%">
      <img src="docs/screenshots/export.png" alt="エクスポート HTML のプレビュー" />
      <p align="center"><em>エクスポート HTML — 完全オフライン、ブラウザで開ける</em></p>
    </td>
  </tr>
</table>

</details>

## インストール

[Releases](https://github.com/jerrywu001/cc-sessions-viewer/releases) からプラットフォームに合ったインストーラをダウンロード：

| プラットフォーム | ファイル |
| --- | --- |
| macOS (Apple Silicon + Intel) | `.dmg` |
| Windows x64 | `-setup.exe` / `.msi` |
| Linux x86_64 | `.deb` / `.AppImage` |

macOS 版 `.app` は **ad-hoc 署名済み・未公証** のため、初回起動時に「Apple は…検証できません」というダイアログが出ることがあります。回避方法は 2 つ：

- Finder で `.app` を右クリック → **開く** → ダイアログで再度「開く」を押す（初回のみ）。
- または、ターミナルで隔離属性を外す：
  ```bash
  sudo xattr -dr com.apple.quarantine "/Applications/Sessions Viewer.app"
  ```

Linux 版 `.AppImage` はポータブル形式 —— `chmod +x` で実行可能になります。`.deb` のインストール：
```bash
sudo apt install ./cc-sessions-viewer_<ver>_amd64.deb
```

## 開発

```bash
git clone https://github.com/jerrywu001/cc-sessions-viewer.git
cd cc-sessions-viewer
npm install
npm run tauri dev      # 開発モード
npm run tauri build    # バンドル
```

必要環境：Node 20+、Rust stable。アーキテクチャの詳細は [`CLAUDE.md`](CLAUDE.md) を参照。

## コントリビュート

PR 歓迎。[Conventional Commits](https://www.conventionalcommits.org/)（`feat:` / `fix:` / `docs:` ...）でお願いします。

## Star History

<a href="https://www.star-history.com/?type=date&repos=jerrywu001/cc-sessions-viewer">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jerrywu001/cc-sessions-viewer&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jerrywu001/cc-sessions-viewer&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jerrywu001/cc-sessions-viewer&type=date&legend=top-left" />
 </picture>
</a>

## スポンサー支援
このプロジェクトは限られた個人の時間で維持しています。スポンサー支援は継続的な開発、バグ修正、ドキュメント整備に充てます。

- 🛠️ 継続的な開発とアップデート

- 🐛 迅速なバグ修正と問題解決

- 📚 ドキュメントの改善とサンプルの拡充

カスタム対応や特別な要件は、下記のスポンサー支援窓口からご相談ください。50 米ドルから承りますが、対応可否はその時点の状況によります。

### 支援方法：

- GitHub Sponsors
  
[GitHub Sponsors](https://github.com/sponsors/jerrywu001)（推奨 · 手数料無料）

- Alipay / WeChat
  
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

## ライセンス

[MIT](LICENSE) © jerrywu001 · [@jerrywu185](https://x.com/jerrywu185)
