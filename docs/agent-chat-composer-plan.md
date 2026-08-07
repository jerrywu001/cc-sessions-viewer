# 方案：参考 Codex IDE 插件 / Codex app 的 Codex 对话能力

## 1. 目标

当前项目已经能查看 Codex 历史会话，并能通过内嵌终端运行 Codex CLI。但用户体验仍然偏“终端套壳”：想继续对话时需要切到 TUI。

目标是只针对 **Codex** 增加类 VS Code Codex 插件 / Codex app 的对话式体验：

- 在 Codex 会话详情页底部提供 composer。
- 用户直接输入下一轮任务，不需要打开内嵌终端。
- 运行过程用结构化事件驱动 UI，而不是解析终端 ANSI 输出。
- 展示 thread/turn 状态、流式回复、工具/命令/文件变更、审批请求。
- 完成后与当前 JSONL 历史视图和会话列表保持同步。

本方案不覆盖 Claude Code 和 Gemini。

## 2. 参考对象

### Codex IDE extension

参考点：

- 聊天线程式交互。
- composer 下方有模型、reasoning effort、approval mode。
- 支持把文件、选中内容、当前上下文加入 thread。
- 支持 slash commands，例如 `/status`、`/review`。
- 与 Codex CLI 共享本地配置。

### Codex app

参考点：

- 一个窗口管理多个 project/thread。
- 每个 thread 有自己的运行状态和 scoped terminal。
- 审批、sandbox、Git diff、MCP、web search 等都围绕 thread 展开。
- 用户能在 thread 运行期间观察进度、审批动作、继续追问。

### Codex app-server

官方文档说明 `codex app-server` 是 Codex 用来驱动 rich client 的接口，例如 VS Code extension。它提供：

- authentication
- conversation history
- approvals
- streamed agent events
- thread / turn / item 生命周期

因此，本项目的 Codex 对话能力应优先基于 `codex app-server`，而不是隐藏 PTY。

本机 `codex-cli 0.142.0` 已验证支持 `codex app-server`，并能生成 app-server TypeScript schema。当前 schema 中的核心方法是 `thread/start`、`thread/resume`、`turn/start`、`turn/interrupt`；消息增量通过 `item/agentMessage/delta` 通知推送；审批类交互通过 `ServerRequest` 发送给客户端，客户端需要用同一个 JSON-RPC `id` 返回决策。

## 3. 非目标

第一版不做：

- 不接 OpenAI API，不新增 API key 管理。
- 不支持 Claude Code / Gemini。
- 不把终端 ANSI 输出反解析成消息。
- 不完整复刻 Codex app 的 worktree、cloud、browser preview、Git commit/push/PR。
- 不实现完整 VS Code 插件的 IDE selection sync；先用本项目已有会话和项目上下文。

## 4. 现有可复用能力

- `src/views/ChatView.vue` 已经按统一 `Msg[]` 渲染 Codex JSONL 历史，支持工具调用、工具结果、搜索、定位、折叠、Live 追加。
- `src/App.vue` 的 `openChat()` 已经能 `readSession()` 并调用 `watchSession()`。
- `src-tauri/src/agents/codex.rs` 已经能扫描 Codex 会话、解析 JSONL、读取标题、重命名、统计、识别 turn 状态。
- `src-tauri/src/watch.rs` 已经能监听 JSONL 文件变化并推送 `session:append`。
- `src/terminals.ts`、`src-tauri/src/pty.rs` 已经提供内嵌 TUI，可作为 app-server 不可用时的 fallback 或高级终端入口。

这些能力保留，但新的 Codex composer 主路径不依赖 PTY。

## 5. 总体架构

新增 `CodexAppServerRuntime`：

```text
ChatComposer
  -> api.codexTurnStart(...)
  -> Rust codex_app_server.rs
  -> codex app-server stdio JSON-RPC
  -> streamed notifications
  -> conversation://* Tauri events
  -> ChatView 增量渲染
```

核心原则：

- **Codex 主路径：app-server**。
- **历史对齐：JSONL read/watch**。
- **兜底入口：现有内嵌 TUI**。

## 6. 后端设计

新增 `src-tauri/src/codex_app_server.rs`。

职责：

- 启动并持有 `codex app-server` 子进程。
- 使用 stdio transport，不开放 WebSocket 端口。
- 完成 `initialize` / `initialized`。
- 管理 request id、pending responses、notification 分发。
- 提供 Tauri command：
  - `codex_app_server_status`
  - `codex_thread_start`
  - `codex_thread_resume`
  - `codex_turn_start`
  - `codex_turn_interrupt`
  - `codex_server_request_respond`
  - `codex_app_server_stop`
- 把 app-server notification 转成前端事件：
  - `conversation://thread`
  - `conversation://turn-state`
  - `conversation://item-started`
  - `conversation://item-delta`
  - `conversation://item-completed`
  - `conversation://server-request`
  - `conversation://error`

失败处理：

- 找不到 `codex` 或不支持 `app-server`：前端显示“结构化对话不可用”，提供“用内嵌终端继续”按钮。
- app-server 进程退出：当前 thread 标为 error，允许重启。
- 协议解析失败：记录错误并停止当前 runtime，避免 UI 保持假运行状态。

## 7. 前端设计

新增：

- `src/components/ChatComposer.vue`
- `src/codexAppServerRuntime.ts`
- `src/conversationEvents.ts`

修改：

- `src/views/ChatView.vue`
- `src/App.vue`
- `src/api.ts`
- `src/i18n.ts`
- `src/style.css`

### ChatComposer

功能：

- 多行输入。
- Enter 发送，Shift+Enter 换行。
- 发送、停止按钮。
- model switcher。
- reasoning effort switcher。
- approval mode / sandbox 状态展示。
- context chips：
  - 当前 project
  - 当前 session/thread
  - 选中的历史消息
  - 后续可扩展文件引用
- slash command 入口：
  - MVP：`/status`
  - 后续：`/review`、`/new`

### ChatView

当前 ChatView 是历史渲染器。改造后变成 Codex thread view：

- 上半部分继续渲染已有 JSONL 消息。
- app-server turn 运行时，将流式 item 临时追加到 UI。
- turn completed 后刷新 `readSession()`，用 JSONL 结果替换临时流式状态。
- app-server `ServerRequest` 以 `ApprovalPromptBar` 或对应交互条展示。
- app-server 不可用时，显示 fallback 操作，不影响原历史查看能力。

## 8. 数据流

### 打开已有 Codex 会话

```text
openChat(session)
  -> readSession(codex, session.path)
  -> watchSession(codex, session.path)
  -> codexThreadResume(session.id or app-server thread id)
  -> ChatView 显示历史 + composer
```

需要注意：当前项目的 `SessionMeta.id` 来自 Codex JSONL/session 文件，是否能直接映射到 app-server `threadId` 需要实测。若不能直接 resume：

- MVP 可先用 `thread/start` 创建新的 app-server thread，并把当前历史摘要/引用作为 context。
- 或在后端增加 Codex thread id 映射探测。

### 发送一轮 turn

```text
ChatComposer.send(input)
  -> codex_turn_start(threadId, input, model, effort, approvalMode, cwd, contextItems)
  -> app-server emits item/delta/approval/turn notifications
  -> runtime 更新状态
  -> ChatView 增量渲染
  -> turn completed
  -> refresh readSession/listSessions
```

### 审批

```text
app-server ServerRequest
  -> conversation://server-request
  -> ApprovalPromptBar / request-specific prompt
  -> 用户 approve once / approve session / deny
  -> codex_server_request_respond(requestId, decision)
  -> turn 继续或失败
```

MVP 先处理这些 request：

- `item/commandExecution/requestApproval`
- `item/fileChange/requestApproval`
- `item/permissions/requestApproval`

其它 request 先显示原始摘要并提供“打开内嵌终端/取消”的降级路径。

### 停止

```text
用户点停止
  -> codex_turn_interrupt(threadId, turnId)
  -> app-server emits turn completed/interrupted
  -> runtime state = idle/error/review
```

## 9. 状态模型

前端统一使用：

```ts
type CodexConversationState =
  | 'idle'
  | 'starting'
  | 'working'
  | 'blocked'
  | 'review'
  | 'error'
```

状态来源：

- `turn/start` 成功：`working`
- app-server ServerRequest：`blocked`
- app-server item delta：保持 `working`
- app-server turn completed：`review`
- 用户查看或刷新完成：`idle`
- app-server error / exit：`error`

## 10. 分阶段实现

### Phase 1：Codex app-server 连通

任务：

- 新增 Rust `codex_app_server.rs`。
- 启动 `codex app-server` stdio。
- 实现 initialize / initialized。
- 实现最小 JSON-RPC request/response/notification 循环。
- 提供 `codex_app_server_status`。

验证：

- 能检测当前机器是否支持 `codex app-server`。
- 不支持时给出明确错误。
- app-server 进程退出能被 UI 感知。

### Phase 2：已有会话页 composer

任务：

- 新增 `ChatComposer.vue`。
- 在 Codex `ChatView` 底部显示 composer。
- 实现 `codex_turn_start`。
- 监听 agent message delta 并临时渲染。
- turn completed 后刷新 `readSession()`。

验证：

- 打开 Codex 会话后可发送新 prompt。
- 回复能流式显示。
- 完成后历史视图与 JSONL 对齐。
- 切换 session 不串消息。

### Phase 3：审批与停止

任务：

- 处理 app-server `ServerRequest`。
- 增加 `ApprovalPromptBar`。
- 实现 approve/deny。
- 实现 `codex_turn_interrupt`。

验证：

- 需要审批时 composer 进入 blocked。
- 用户同意后 turn 继续。
- 用户拒绝后 turn 结束或进入 error/review。
- 停止按钮能中断正在运行的 turn。

### Phase 4：Codex 插件式体验补齐

任务：

- model switcher。
- reasoning effort switcher。
- approval mode 展示和切换。
- context chips：选中消息加入上下文。
- `/status` slash command。
- fallback 到内嵌 TUI。

验证：

- model/effort 传到 app-server turn 参数。
- 选中历史消息能作为 context 发送。
- `/status` 能展示 thread id、运行状态、上下文信息。
- app-server 不可用时可一键打开现有内嵌 TUI。

## 11. 风险与处理

| 风险 | 影响 | 处理 |
|---|---|---|
| 本机 Codex 版本不支持 app-server | composer 不可用 | 检测版本/能力，fallback 到内嵌 TUI |
| app-server 协议变化 | 事件解析失败 | 后端集中封装协议，失败时明确报错 |
| JSONL session id 与 app-server thread id 不一致 | 无法直接 resume 历史 thread | MVP 先创建新 thread 并带入历史上下文；后续补映射 |
| ServerRequest 类型不全 | blocked 状态不准 | 先处理命令/文件/权限三类 request，未知 request 展示原始摘要 |
| 流式临时消息与 JSONL 刷新重复 | UI 重复消息 | turn completed 后以 `readSession()` 结果为准替换临时消息 |
| 长期 app-server 子进程泄漏 | 后台资源占用 | App unmount/window close 时 stop；session 切换时清理 runtime |

## 12. 与旧方案的区别

`docs/issue-1-in-app-llm-chat-plan.md` 是“选中历史上下文，调用外部 LLM API 问答”的方案。

本文方案是“把本项目做成 Codex rich client”，通过 `codex app-server` 驱动本机 Codex agent，目标是接近 VS Code Codex 插件 / Codex app，而不是外部 API 聊天助手。

## 13. 推荐结论

只做 Codex 时，推荐路线非常明确：

1. **主路径接 `codex app-server`**。
2. **UI 参考 Codex IDE extension / Codex app 的 composer + thread 状态 + approval**。
3. **现有 JSONL 解析继续用于历史展示和 turn 完成后的对齐**。
4. **现有内嵌 TUI 作为 fallback，不作为主实现**。

最小可用标准：

- Codex 会话页底部出现 composer。
- 发送 prompt 后 app-server 开启一轮 turn。
- 回复流式显示。
- turn 完成后刷新为正式 JSONL 历史。
- app-server 不可用时能清楚提示并回退到内嵌终端。
