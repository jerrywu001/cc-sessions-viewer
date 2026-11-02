---
title: 阅读与搜索会话
description: 阅读 Claude Code、Codex 和 opencode 等受支持记录，在当前 agent 的项目间搜索标题、ID 或用户提问，了解全局搜索与回放限制。
image: /screenshots/chat.png
---

# 阅读与搜索会话

Sessions Viewer 把受支持的 JSONL 和 opencode SQLite 记录呈现为可读对话。适配器识别后，可显示已记录的消息、工具、思考、diff 和图片；不能恢复缺失记录，也不是逐像素重放 CLI 屏幕。

![Sessions Viewer 里回放的一个 Claude Code 会话：工具调用和结果配好了对，diff 渲染成了真正的 diff](/screenshots/chat.png)

## 被还原了什么

### 工具调用和它的结果配成对

在文件里，一次调用和它的结果是两条分开的记录，有时隔着很多行，靠一个 id 关联。分开看谁也读不懂。放在一起，才能看出 agent 要了什么、回来的是什么。

### 思考块默认折着，想看再展开

扩展思考经常比答案本身还长。受支持的已记录块会折叠显示，可以单条展开，也可以整个会话一次全展开。连续的推理步骤会合并成一行紧凑内容；全局设置和单个会话都可以隐藏或显示推理，而且不影响工具调用的显示。

### diff 渲染成 diff

文件编辑的结果带结构化 patch 数据时，它会变成带行号和语法高亮的真 diff，而不是一坨转义过的文本。那份数据在磁盘上长什么样，见 [Claude Code 那页](/zh/agents/claude-code#structured-diffs)。

### 图片回到原位

粘贴的截图在有的 agent 里存成 base64，在有的里是独立的媒体文件。两种最后都内联渲染在当时那条消息里。

### Mermaid 图、表格、公式

agent 以 markdown 输出的内容，就按 markdown 渲染。

## 给任务会话加星标 {#starred-sessions}

在项目会话列表中，将鼠标移到某条会话上，标题旁会显示星形按钮，点击即可标星或取消。键盘聚焦该条目时也会显示按钮。点击工具栏的**只看星标**隐藏非星标会话，再点一次恢复。星标与置顶独立，不改变原有排序。

筛选针对当前 agent、当前项目，会补齐剩余会话元数据，不只检查第一页。可以与搜索、排序组合使用，搜索本身的结果上限仍然适用。星标在分屏间同步，由 Sessions Viewer 保存在本机，不改原始会话文件或数据库。稳定会话 ID 可在重命名、存储路径变化后保留星标；星标不会同步到另一台设备，也不随对话导出。切换项目会重置筛选，但不会清除星标。

## 找到某条消息 {#finding-a-message}

![Sessions Viewer 的全局搜索，列出了来自多个项目的命中结果](/screenshots/search.png)

`⌘⇧F` 打开全局搜索，跨**当前选中 agent 的项目**查找，不是同时搜索七家。关键词模式匹配会话标题和已保存的**用户消息正文**；ID 模式查会话 ID。文本命中会打开对应用户消息；标题／ID 命中则打开会话，不保证跳到某条文本消息。

### 搜索范围与限制 {#search-scope}

全局搜索不匹配助手回答、思考、工具参数／结果或项目路径；查另一家记录需要切换 agent。普通搜索不含归档及已回收会话，应查看 [Codex 归档视图](/zh/guide/codex-session-viewer#archived)或[回收站](/zh/features/export-and-trash)。后端最多返回 200 个命中会话，弹窗最多呈现 80 个，不是完整全文导出。

解析器只呈现受支持的已记录字段。例如 opencode 工具输出超过 30 行时会缩短并提示剩余行数。完整归档需保留原始文件或数据库；[解析后消息导出](/zh/features/export-and-trash)不是原生备份。

`⌘F` 在当前打开的内容里搜，`⌘G` 和 `⌘⇧G` 在命中之间来回跳。

### 跳到某次提问

长会话里绝大部分是 agent 的输出。提问列表把这些全剥掉，只按顺序列出你自己敲过的那些话，紧凑的一列。要找「会话是从哪一步开始跑偏的」，这通常是最快的办法。点一条，视图就滚过去。

### 浏览历史

最近读过的会话按项目存成一份历史列表，可按标题搜索。想重新打开昨天那个会话，不用再找一遍。源文件变化后，会话预览会刷新；Pi 的预览则取最新一条用户消息。

### 大型 Pi 会话

打开 Pi 记录时，先读取最近一页消息。向上滚动时再按需加载更早的内容。搜索、跳到提问、全局搜索和导出仍会在需要时读取完整记录。

## 什么都不会写回去

阅读与搜索历史不会重写源记录，但可能写本地设置和缓存。重命名、回收／还原、继续对话及编辑功能有各自的写入行为，见[隐私与数据处理](/zh/guide/privacy)。

搜索与 opencode 展示限制于 2026-10-06 核对 Sessions Viewer 0.6.0：[搜索界面](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src/modals/GlobalSearchModal.vue)、[命令入口](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src-tauri/src/lib.rs)、[匹配实现](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src-tauri/src/agents/mod.rs)与 [opencode 适配器](https://github.com/jerrywu001/cc-sessions-viewer/blob/69e0b4f/src-tauri/src/agents/opencode.rs)。这是源码核对，不是实际 CLI 验收。
