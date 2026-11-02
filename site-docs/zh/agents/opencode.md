---
title: opencode 的会话记录存在哪
description: 查找 opencode 受支持的 SQLite 历史和 XDG 路径，用 SQL 读记录，或用 Sessions Viewer 搜索提问、导出并在终端恢复会话。
---

# opencode 的会话记录存在哪

此处支持的 opencode 布局把会话保存在 SQLite 库 `~/.local/share/opencode/opencode.db`；配置后改用 `$XDG_DATA_HOME/opencode/opencode.db`：

```
~/.local/share/opencode/opencode.db
```

它遵循 XDG 规范，所以设了 `$XDG_DATA_HOME` 的话以 `$XDG_DATA_HOME/opencode/opencode.db` 为准。macOS 上也一样，尽管 macOS 上大多数应用会用 `~/Library/Application Support`。

在此处支持的布局中，会话是数据库行，不是逐会话 JSONL 文件。应查询 SQLite，不要把递归文本搜索当成对话提取方法。旧的文件存储布局不在此适配器范围内。

## 旧版 SQLite 布局的四张表

```sql
project  -- id (sha1)、worktree（项目目录）、vcs、time_*
session  -- id（"ses_…"）、project_id、parent_id、slug、title、directory、
         -- model (JSON)、tokens_* 五列、cost、time_created、
         -- time_updated、time_archived
message  -- id（"msg_…"）、session_id、time_created、data（JSON 信封）
part     -- id（"prt_…"）、message_id、session_id、time_created、data（JSON 正文）
```

库里还有别的表（`workspace`、`todo`、`permission`、`event`、`credential` 以及迁移记录），但一份对话记录就是 `session` → `message` → `part`。

`message.data` 是 JSON 信封：`role`、`modelID`、`providerID`、`tokens{input,output,reasoning,cache{read,write}}`、`cost`、`time{created,completed}`。`part.data` 是正文，`type` 可能是 `text`、`reasoning`、`tool`、`file`、`step-start`、`step-finish` 等等。

## v2 会话存储布局

适配器按数据库表结构识别布局，不按 CLI 版本字符串判断。新增兼容的 v2 布局使用 `project`、`session_v2`、`session_message`；较新的迁移还会创建 `session_part`。它不是把旧表换个名字：

- `session_message.type` 列保存消息种类，`seq` 决定消息顺序；`data` 不必包含 `role` 或 `id`。
- 用户正文是 `data.text` 和 `data.files`；助手正文是 `data.content`，模型为 `data.model.id`。
- 有 `session_part` 记录时，按 `ordinal` 读取助手正文；没有时仍读内嵌 `content`，避免丢掉迁移前记录。
- 工具调用使用 `name`、`id` 和 `state.content`；图片支持内嵌数据和本库中的 `media:` blob 引用。丢失的图片保留文件条目，不会因此中断整段对话。
- 新旧布局可以共存；相同会话 ID 优先读 v2，旧版独有会话仍保留。自定义 `XDG_DATA_HOME` 规则不变。

查看新消息行可用：

```sql
SELECT id, type, seq, data
FROM session_message
WHERE session_id = 'ses_...'
ORDER BY seq;
```

读取、用户提问搜索、解析后导出及统计均走统一适配。v2 使用事件溯源，直接改投影表可能被后续回放覆盖；**本次不支持在 Sessions Viewer 中重命名、回收或还原 v2 会话**，这些操作会明确报错，应交给 OpenCode 处理。旧版数据库的已有写操作保留。解析后导出不是包含所有事件、设置及后台任务的原生备份。

依据为 [OpenCode 固定提交的表定义](https://github.com/anomalyco/opencode/blob/dbcd5ce9485f3338375d1fe9d92864c91e263030/packages/core/src/session/sql.ts)及[消息定义](https://github.com/anomalyco/opencode/blob/dbcd5ce9485f3338375d1fe9d92864c91e263030/packages/schema/src/session-message.ts)，并用合成数据库回归。未使用 #96 报告者的实际数据库验证，也不代表 Windows 真机认证。

## 子 agent 会话

`parent_id` 非空的会话是别的会话派生出来的子 agent 运行。应用在普通历史列表中隐藏它们，但统计包括它们已记录的工作。子会话可能包含用量与成本；本地模型、服务商计价或缺失字段都可能产生零记录成本，不能据此断定它是单独计费的 API 调用。

## 用只读方式打开

检查时 opencode 的 TUI 可能正在写入。以只读连接避免意外修改；繁忙或变化中的数据库仍可能需要等待活动结束后重试。下方命令也遵循配置的 XDG 数据根目录：

```bash
sqlite3 "file:${XDG_DATA_HOME:-$HOME/.local/share}/opencode/opencode.db?mode=ro" ".tables"
```

## 用 SQL 读旧版 opencode 记录

按时间倒序列出会话，带项目和花费：

```sql
SELECT s.id, p.worktree, s.title, s.cost,
       datetime(s.time_created/1000, 'unixepoch') AS created
FROM session s
JOIN project p ON p.id = s.project_id
WHERE s.parent_id IS NULL
ORDER BY s.time_created DESC
LIMIT 20;
```

按顺序打印一个会话的文本：

```sql
SELECT json_extract(m.data, '$.role') AS role,
       json_extract(pt.data, '$.text') AS text
FROM message m
JOIN part pt ON pt.message_id = m.id
WHERE m.session_id = 'ses_...'
  AND json_extract(pt.data, '$.type') = 'text'
ORDER BY m.time_created, pt.time_created;
```

按项目汇总会话已记录成本：

```sql
SELECT p.worktree, ROUND(SUM(s.cost), 2) AS usd, COUNT(*) AS sessions
FROM session s JOIN project p ON p.id = s.project_id
GROUP BY p.worktree ORDER BY usd DESC;
```

最后这条有意包含子 agent 行，汇总的是已保存的 `session.cost`，不是服务商发票。桌面统计则读取 assistant 消息成本；缺失或不一致记录可能导致两种汇总不同。

## 为什么成本必须从库里取 {#cost-from-db}

opencode 可使用不同服务商或本地模型，仅按模型名查价目表未必符合实际配置。应用读取 assistant 消息中的 `modelID`、`tokens` 和 `cost`，不按应用价目表重新计价。缺失的数值成本目前按零处理，只表示未测量，不代表确认免费。记录成本不是独立核验的发票，财务决策应对照服务商实际用量和账单。见[用量与成本边界](/zh/features/stats#cost-vs-bill)。

## 怎么恢复这个会话

在项目目录运行 `opencode --session SESSION_ID`，需要已安装 opencode。应用提供[终端恢复](/zh/features/resume)，不提供 opencode 内置对话。

## 查看与搜索 opencode 历史 {#view-and-search}

[安装 Sessions Viewer](/zh/guide/install)，选择 opencode，再选项目阅读受支持的 SQLite 会话。用 `⌘⇧F`（macOS）或 `Ctrl+Shift+F`（Windows/Linux）跨 opencode 项目查标题和用户提问，也可切换为 ID 模式。不搜索助手回答或工具输出，详见[搜索范围](/zh/features/read-and-search#search-scope)。

检查隐私后可导出 Markdown、HTML 或解析后消息 JSON；完整归档应保留原数据库。[导出限制](/zh/features/export-and-trash)包括外链或无法读取的图片。继续工作使用已安装的 opencode CLI 在终端恢复，不是 opencode 应用内对话。Sessions Viewer 是独立开源项目，不是 opencode 官方产品。恢复或分享前请查看[隐私说明](/zh/guide/privacy)。

## 依据与限制

路径、记录成本与操作说明于 2026-10-06 核对 [0.6.0 opencode 适配器](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src-tauri/src/agents/opencode.rs)和[搜索实现](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src-tauri/src/agents/mod.rs)。这是源码与合成测试范围，不是 CLI 实际运行认证。上游：[opencode 文档](https://opencode.ai/docs/)。

<!--@include: ../../.vitepress/snippets/reference-zh.md-->

本页 `session` / `message` / `part` SQL 示例仅针对旧版 SQLite 布局；v2 的查询见上节。旧文件存储布局不在范围内。旧版的显式重命名、回收、还原会写数据库，v2 的这些写操作暂不支持。见[找不到会话的排障指南](/zh/guide/troubleshooting)。

## 或者用应用打开

[Sessions Viewer](/zh/guide/) 以只读方式查这个库，列表里藏掉子 agent 会话但统计里照算，把 opencode 的对话和那些 JSONL 的 agent 并排显示在同一个界面里。它同时还读 [Claude Code](/zh/agents/claude-code)、[Codex](/zh/agents/codex)、[Grok Build](/zh/agents/grok-build)、[Kimi Code](/zh/agents/kimi-code)、[Pi](/zh/agents/pi) 和 [Antigravity CLI](/zh/agents/antigravity-cli)。
