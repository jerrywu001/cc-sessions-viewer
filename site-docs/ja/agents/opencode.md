---
title: opencode のセッション履歴の保存場所
description: opencode の対応 SQLite 履歴と XDG パスを確認し、SQL で記録を読みます。Sessions Viewer でプロンプトを検索し、保存・ターミナル再開する方法も説明します。
---

# opencode のセッション履歴の保存場所

ここで対応する opencode の形式は、SQLite データベース `~/.local/share/opencode/opencode.db` にセッションを保存します。設定時は `$XDG_DATA_HOME/opencode/opencode.db` を使います：

```
~/.local/share/opencode/opencode.db
```

XDG Base Directory 仕様に従うため、`$XDG_DATA_HOME` が設定されていれば `$XDG_DATA_HOME/opencode/opencode.db` が優先されます。多くのアプリが `~/Library/Application Support` を使う macOS でも同じです。

この対応形式ではセッションは個別の JSONL ではなくデータベースの行です。再帰的なテキスト検索に頼らず SQLite で照会してください。古いファイル形式はこのアダプターの対象外です。

## 旧 SQLite 形式の 4 テーブル

```sql
project  -- id (sha1)、worktree（プロジェクトディレクトリ）、vcs、time_*
session  -- id（"ses_…"）、project_id、parent_id、slug、title、directory、
         -- model (JSON)、tokens_* 5 列、cost、time_created、
         -- time_updated、time_archived
message  -- id（"msg_…"）、session_id、time_created、data（JSON エンベロープ）
part     -- id（"prt_…"）、message_id、session_id、time_created、data（JSON 本文）
```

データベースには他にもテーブル（`workspace`、`todo`、`permission`、`event`、`credential`、マイグレーション管理）がありますが、1 つの記録は `session` → `message` → `part` です。

`message.data` は JSON エンベロープで、`role`、`modelID`、`providerID`、`tokens{input,output,reasoning,cache{read,write}}`、`cost`、`time{created,completed}` を持ちます。`part.data` が本文で、`type` は `text`、`reasoning`、`tool`、`file`、`step-start`、`step-finish` などです。

## v2 セッション保存形式

CLI のバージョン文字列ではなくテーブル構造で形式を判別します。追加対応する v2 は `project`、`session_v2`、`session_message` を使い、新しい移行では `session_part` も作成します。単なるテーブル名の変更ではありません。

- メッセージの種類は `session_message.type`、順序は `seq` です。`data` に `role` や `id` がない場合もあります。
- ユーザー本文は `data.text` / `data.files`、回答は `data.content`、モデル名は `data.model.id` です。
- 回答に `session_part` があれば `ordinal` 順で読み、なければ内包された `content` を維持します。
- ツールは `name` / `id` / `state.content` を使います。画像はインラインデータと同じ DB 内の `media:` blob 参照に対応し、欠落画像はファイル項目として残します。
- 新旧が共存する場合、同じセッション ID は v2 を優先し、旧形式だけの記録も表示します。`XDG_DATA_HOME` の解決規則は変わりません。

新形式のメッセージ行を確認する SQL：

```sql
SELECT id, type, seq, data
FROM session_message
WHERE session_id = 'ses_...'
ORDER BY seq;
```

閲覧、ユーザープロンプト検索、解析済みエクスポートと統計に対応します。v2 はイベントソーシングを使うため、投影テーブルの直接変更は再生時に上書きされる可能性があります。**今回の更新では v2 のリネーム・ゴミ箱移動・復元には対応せず、明示的にエラーを返します**。これらは OpenCode 側で操作してください。旧形式の既存書き込み機能は維持します。解析済みエクスポートは全イベント・設定・バックグラウンドジョブのネイティブバックアップではありません。

根拠は固定コミットの[テーブル定義](https://github.com/anomalyco/opencode/blob/dbcd5ce9485f3338375d1fe9d92864c91e263030/packages/core/src/session/sql.ts)と[メッセージ定義](https://github.com/anomalyco/opencode/blob/dbcd5ce9485f3338375d1fe9d92864c91e263030/packages/schema/src/session-message.ts)、合成データベースの回帰テストです。#96 報告者の DB や Windows 実機では検証していません。

## サブエージェントのセッション

`parent_id` が非 NULL のセッションは、別のセッションから派生したサブエージェントの実行です。通常の履歴一覧では非表示ですが、統計には記録済みの作業を含めます。使用量とコストを持つ場合がある一方、ローカルモデル、料金設定、欠落フィールドで記録コストがゼロになることもあります。別途課金された API 呼び出しの証拠ではありません。

## 読み取り専用で開く

確認中に opencode の TUI が書き込んでいる可能性があります。意図しない変更を避けるため読み取り専用で開きます。使用中のデータベースでは、処理が落ち着いてから再試行が必要な場合もあります。次のコマンドは設定済みの XDG 保存先にも従います：

```bash
sqlite3 "file:${XDG_DATA_HOME:-$HOME/.local/share}/opencode/opencode.db?mode=ro" ".tables"
```

## 旧形式の opencode 記録を SQL で読む

プロジェクトとコスト付きで、新しい順にセッションを一覧：

```sql
SELECT s.id, p.worktree, s.title, s.cost,
       datetime(s.time_created/1000, 'unixepoch') AS created
FROM session s
JOIN project p ON p.id = s.project_id
WHERE s.parent_id IS NULL
ORDER BY s.time_created DESC
LIMIT 20;
```

1 つのセッションのテキストを順番に出力：

```sql
SELECT json_extract(m.data, '$.role') AS role,
       json_extract(pt.data, '$.text') AS text
FROM message m
JOIN part pt ON pt.message_id = m.id
WHERE m.session_id = 'ses_...'
  AND json_extract(pt.data, '$.type') = 'text'
ORDER BY m.time_created, pt.time_created;
```

プロジェクト別の記録済みセッションコスト：

```sql
SELECT p.worktree, ROUND(SUM(s.cost), 2) AS usd, COUNT(*) AS sessions
FROM session s JOIN project p ON p.id = s.project_id
GROUP BY p.worktree ORDER BY usd DESC;
```

最後のクエリはサブエージェント行を含めます。保存された `session.cost` の合計であり、プロバイダーの請求書ではありません。デスクトップ統計は assistant メッセージのコストを読み、欠落・不整合があると両者の合計は一致しない場合があります。

## コストをデータベースから取る理由 {#cost-from-db}

opencode は各種プロバイダーやローカルモデルを使えます。モデル名だけの価格表が実際の設定と合うとは限りません。ビューアは assistant メッセージの `modelID`、`tokens`、`cost` を読み、アプリの価格表で再計算しません。数値コストがない場合は現在ゼロとして扱いますが、未測定であって無料の確認ではありません。記録コストは独立検証済みの請求書ではなく、財務判断ではプロバイダーの実際のレポートと照合してください。[使用量とコストの制限](/ja/features/stats#cost-vs-bill)を参照できます。

## セッションを再開するには

opencode をインストールし、プロジェクトのディレクトリで `opencode --session SESSION_ID` を実行します。アプリは[ターミナルでの再開](/ja/features/resume)に対応し、opencode の内蔵チャットには対応していません。

## opencode 履歴の閲覧と検索 {#view-and-search}

[Sessions Viewer をインストール](/ja/guide/install)し、opencode とプロジェクトを選ぶと対応 SQLite セッションを読めます。`⌘⇧F`（macOS）または `Ctrl+Shift+F`（Windows/Linux）で opencode のプロジェクトを横断し、タイトルとユーザープロンプトを検索できます。ID モードも利用できます。回答とツール出力は対象外です。[検索範囲](/ja/features/read-and-search#search-scope)を確認してください。

機密内容を確認して Markdown、HTML、解析済みメッセージ JSON を保存できます。完全な保存には元データベースを残します。[保存の制限](/ja/features/export-and-trash)には外部画像や読めない画像も含まれます。続行はインストール済み opencode CLI のターミナル再開であり、opencode のアプリ内チャットではありません。Sessions Viewer は独立したオープンソースプロジェクトで、opencode の公式製品ではありません。再開・共有前に[プライバシー](/ja/guide/privacy)を確認してください。

## 根拠と制限

パス・記録コスト・操作説明は 2026-10-06 に [0.6.0 opencode アダプター](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src-tauri/src/agents/opencode.rs)と[検索処理](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src-tauri/src/agents/mod.rs)で確認しました。ソースと合成テストの範囲で、実際の CLI 動作認証ではありません。上流：[opencode ドキュメント](https://opencode.ai/docs/)。

<!--@include: ../../.vitepress/snippets/reference-ja.md-->

`session` / `message` / `part` の SQL 例は旧 SQLite 形式向けです。新形式は v2 節を参照してください。旧ファイル形式は対象外です。旧 SQLite 形式の明示的なリネーム・ゴミ箱移動・復元は DB に書き込みますが、v2 ではこれらをブロックします。[確認手順](/ja/guide/troubleshooting)を参照してください。

## アプリで開く

[Sessions Viewer](/ja/guide/) はこのデータベースを読み取り専用で参照し、一覧ではサブエージェントのセッションを隠しつつ統計には算入し、opencode の会話を JSONL ベースのエージェントと同じ画面に並べて表示します。[Claude Code](/ja/agents/claude-code)、[Codex](/ja/agents/codex)、[Grok Build](/ja/agents/grok-build)、[Kimi Code](/ja/agents/kimi-code)、[Pi](/ja/agents/pi)、[Antigravity CLI](/ja/agents/antigravity-cli) にも対応しています。
