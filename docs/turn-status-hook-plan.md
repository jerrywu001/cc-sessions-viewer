# Codex Tab 运行状态 Hook 优化方案

日期：2026-06-24

## 范围

本方案只处理 Codex 的 tab 运行状态优化。

目标是把 Codex tab 的“运行中 / 完成 / 阻塞 / 失败”状态，从当前的 Codex JSONL 内容推断，改成基于 Codex CLI 官方 hook 生命周期事件。

其他 agent 不纳入本方案。

## 核心思路

Codex CLI 已经提供 hook 能力。`cc-sessions-viewer` 不应该继续监听 Codex session JSONL 再反推“是否运行中”，而应该在软件启动后，自动把 viewer 自己的 Codex hook 配置追加到 Codex 用户级配置里。

改造后的链路：

```text
cc-sessions-viewer 启动
  -> 写入/修复 viewer 自己的 hook 脚本
  -> 检查 ~/.codex/hooks.json
  -> 如果不存在 viewer hook，则追加到对应 Codex hook 事件
  -> Codex CLI 在生命周期节点触发 hook
  -> hook 脚本 append 状态事件到 viewer 的 turn-signals.jsonl
  -> viewer 后端监听 turn-signals.jsonl
  -> 前端更新 Codex tab 标签状态
```

JSONL 仍然用于 Codex 聊天内容读取和 live tail，但不再参与 Codex tab 运行状态判断。

## 安装位置

viewer 自己的 hook 脚本放在 viewer 数据目录：

```text
<data_local_dir>/cc-sessions-viewer/codex-turn-signal-hook.cjs
<data_local_dir>/cc-sessions-viewer/turn-signals.jsonl
```

Codex 配置写到：

```text
~/.codex/hooks.json
```

Codex 也支持 `~/.codex/config.toml` 的 inline hooks，但本方案优先使用 `hooks.json`，原因是：

- JSON 合并逻辑更简单。
- 不需要解析和重写 TOML。
- 不污染项目级 `<repo>/.codex/hooks.json`。
- 用户级配置能覆盖从 viewer 内置终端和外部终端启动的 Codex。

## 自动安装策略

建议 viewer 启动时自动检查和安装 Codex hook，不依赖用户手动点击。

触发时机：

- app 启动时执行一次 `ensure_codex_turn_hook_installed`。
- 设置页保留“修复 Codex hooks”按钮，用于手动修复。
- viewer 升级后，如果 hook 脚本内容变化，覆盖 viewer 数据目录里的脚本，并重新检查 `~/.codex/hooks.json`。

写入规则：

- 只追加 viewer 自己的 hook，不覆盖 Codex 用户已有配置。
- 写入前必须先检查目标事件里是否已经存在 viewer hook。
- 已存在且 command/name/timeout 都符合当前版本时，不写文件。
- 已存在但脚本路径或命令参数过期时，只更新 viewer 自己那一项。
- 不删除用户已有 hook。
- 不重排用户已有 hook。
- 不覆盖整个 `hooks` 对象、事件数组或配置文件。
- `~/.codex/hooks.json` 不存在时创建。
- 文件为空时按空 JSON 对象处理。
- 文件不是合法 JSON 时返回错误，不自动重写。
- hook 脚本执行失败必须 exit 0，不能影响 Codex 正常运行。

## Codex Hook 事件映射

建议安装这些 Codex hook：

| Codex hook | viewer state | 说明 |
| --- | --- | --- |
| `UserPromptSubmit` | `started` | 用户提交 prompt 后触发；prompt 为空时不写信号 |
| `Stop` | `completed` | 本轮 Codex 响应结束 |
| `PermissionRequest` | `blocked` | Codex 即将请求 approval |

不建议映射：

- `PostToolUse`：工具结束不等于本轮结束。
- `PreToolUse`：工具开始不等于本轮开始。
- `SessionStart`：会话启动不等于用户 turn 开始。
- `PreCompact` / `PostCompact`：上下文压缩不是用户 turn 状态。

Codex hook 输入中可使用这些字段：

- `session_id`
- `transcript_path`
- `cwd`
- `hook_event_name`
- `turn_id`

viewer 以 `transcript_path` 作为 tab 状态归因的 `path`。

## 状态信号格式

hook 脚本向 viewer 的信号文件追加 JSONL：

```json
{
  "schemaVersion": 1,
  "agent": "codex",
  "path": "/abs/path/to/rollout.jsonl",
  "state": "started",
  "sessionId": "optional-session-id",
  "turnId": "optional-turn-id",
  "cwd": "/abs/project",
  "hookEventName": "UserPromptSubmit",
  "source": "hook",
  "ts": 1782300000000
}
```

必填字段：

- `agent`: 固定为 `codex`
- `path`: Codex transcript 绝对路径；拿不到时不写信号
- `state`: `started | completed | blocked | failed`

可选字段：

- `sessionId`: 来自 `session_id`
- `turnId`: 来自 `turn_id`
- `cwd`: 来自 hook input
- `hookEventName`: 来自 `hook_event_name`
- `ts`: hook 脚本写入时间

前端状态映射：

| hook state | tab turnState | 含义 |
| --- | --- | --- |
| `started` | `working` | Codex 开始处理本轮 |
| `completed` | active tab 为 `idle`，非 active tab 为 `review` | 本轮正常结束 |
| `blocked` | `blocked` | 等待 approval 或用户介入 |
| `failed` | `error` | 本轮失败 |

## Hook 脚本行为

命令形态：

```text
node "<data_dir>/cc-sessions-viewer/codex-turn-signal-hook.cjs" started "<data_dir>/cc-sessions-viewer/turn-signals.jsonl"
```

脚本逻辑：

1. 从 stdin 读取 Codex hook input JSON。
2. 读取 `transcript_path`。
3. 如果没有 `transcript_path`，直接 exit 0。
4. `started` 状态需要检查 prompt 是否非空；空 prompt 不写信号。
5. 组装统一状态信号。
6. append 一行 JSON 到 `turn-signals.jsonl`。
7. 捕获所有异常，日志写 stderr，最终 exit 0。

脚本不做的事：

- 不解析 Codex session JSONL。
- 不调用 Tauri command。
- 不修改 Codex hook stdout 语义。
- 不阻塞、拒绝或修改用户 prompt。

## hooks.json 合并规则

推荐 viewer hook item 带稳定标识：

```json
{
  "type": "command",
  "name": "cc-sessions-viewer-turn-status",
  "command": "node \".../codex-turn-signal-hook.cjs\" started \".../turn-signals.jsonl\"",
  "timeout": 5
}
```

合并规则：

1. 读取 `~/.codex/hooks.json`。
2. 如果不存在，创建空对象。
3. 如果为空文件，按 `{}` 处理。
4. 如果 JSON 无法解析，停止安装并返回错误。
5. 对 `UserPromptSubmit`、`Stop`、`PermissionRequest` 分别检查。
6. 如果目标事件不存在，创建该事件数组。
7. 如果目标事件中已存在 `name = cc-sessions-viewer-turn-status` 的 hook：
   - command/timeout 正确：不修改。
   - command/timeout 过期：只替换这一项。
8. 如果目标事件中不存在 viewer hook，追加新 hook。
9. 用户已有 hook 保持原样。

不能做：

- 不能覆盖整个 `hooks` 对象。
- 不能覆盖整个事件数组。
- 不能删除用户 hook。
- 不能重排用户 hook。
- 不能写项目级 `.codex/hooks.json`。

## Codex Trust 提示

Codex 的非托管 command hook 可能需要用户 review/trust。

viewer 可以自动追加配置，但如果 Codex 没有触发状态信号，需要在 UI 上提示：

```text
Codex hook 已写入 ~/.codex/hooks.json。
如果状态没有更新，请在 Codex CLI 中运行 /hooks 并信任 cc-sessions-viewer-turn-status。
```

这属于安装后的可见诊断，不应该阻塞 viewer 启动。

## 后端改造

当前后端已有 `turn::start_signal_watcher`，可以继续复用。

需要改造：

1. 新增安装入口
   - Tauri command：`ensure_codex_turn_hook_installed()`
   - app 启动时调用一次。
   - 设置页“修复 Codex hooks”按钮也调用它。

2. 写 hook 脚本
   - 新增或复用 `write_hook_script()`。
   - 输出到 `<data_local_dir>/cc-sessions-viewer/codex-turn-signal-hook.cjs`。

3. 合并 Codex hooks
   - 新增 `install_codex_turn_hook()`。
   - 只处理 `~/.codex/hooks.json`。
   - 按“存在则不写、过期只更新 viewer 自己项、不存在才追加”规则实现。

4. 扩展 signal payload
   - `TerminalTurnPayload` 增加可选字段：`schema_version`、`session_id`、`turn_id`、`cwd`、`hook_event_name`、`source`、`ts`。
   - 继续兼容旧 `{ agent, path, state }`。
   - `process_signal_file` 继续容忍坏行和半行。

5. 停用 Codex JSONL 状态推断
   - Codex tab 不再调用 `watch_session_turn`。
   - Codex 状态只能来自 hook 信号、PTY 进程状态和本地输入临时状态。

## 前端改造

需要改造：

- `src/api.ts`
  - 增加 `ensureCodexTurnHookInstalled()`。
  - Codex tab 不再调用 `watchSessionTurn` / `unwatchSessionTurn`。

- `src/terminals.ts`
  - Codex 分支删除默认 `ensureSessionTurnWatch(tab, true)`。
  - Codex hook 事件进入 `markTabTurnStarted/Completed/Blocked/Failed` 时，source 写 `hook`。
  - Codex tab 的 `turnWatchPath` 不再需要。

- `src/tabStatus.ts`
  - 保留 `pty-input` 作为用户敲回车后的即时反馈。
  - hook 信号优先级高于 `pty-input`。
  - `clearLocalWorkingTurn` 不能清掉 hook 来源的 `working`。

- `src/components/SettingsModal.vue`
  - 增加“修复 Codex hooks”按钮或状态提示。
  - 展示 `~/.codex/hooks.json` 写入结果。
  - 增加 Codex `/hooks` trust 提示。

## 必须移除的 Codex 旧逻辑

最终实现不能只新增 Codex hook。必须停止 Codex JSONL 状态判断链路，否则误触发仍会存在。

需要移除或停止使用：

- Codex tab 创建后默认调用 `ensureSessionTurnWatch(tab, true)` 的路径。
- Codex tab 关闭时对 `api.unwatchSessionTurn(...)` 的依赖。
- `src-tauri/src/agents/codex.rs::classify_turn_state`，如果它只服务于 tab 状态推断。
- `src-tauri/src/turn.rs` 中 `infer_codex_turn_state` 对 Codex JSONL 的状态推断。

如果 `watch_session_turn` 仍被其他范围使用，本方案不要求一次性删除整个 command；但 Codex 不能再走这条链路。

Codex 验收时，全仓应确认：

- Codex tab 不再调用 `watch_session_turn`。
- 修改 Codex JSONL 只能刷新聊天内容，不能改变 Codex tab 的 `turnState`。
- Codex tab 的运行态只接受 Codex hook 信号、PTY 进程状态和本地输入临时状态。

## 验收标准

功能验收：

- viewer 启动后自动检查 `~/.codex/hooks.json`。
- 不存在 viewer hook 时，追加 `UserPromptSubmit`、`Stop`、`PermissionRequest` 三个 hook。
- 已存在相同 viewer hook 时，不重复写入。
- 已存在旧 viewer hook 时，只更新 viewer 自己那一项。
- 用户已有 Codex hooks 不被删除、不被重排。
- Codex 提交 prompt 后，tab 进入 `working`。
- Codex 正常完成后，tab 进入 `review/idle`。
- Codex 请求 approval 时，tab 进入 `blocked`。
- 修改 Codex JSONL 不会触发 tab 进入 `working`。

代码验收：

- Codex hook 事件 source 为 `hook`。
- Codex JSONL classifier 不再驱动 tab 状态。
- `~/.codex/hooks.json` 合并逻辑有去重测试。
- 非法 JSON 不被覆盖。

测试建议：

- Rust 单测：
  - `hooks.json` 不存在时创建。
  - 空文件按 `{}` 处理。
  - 非法 JSON 返回错误且不覆盖。
  - 已有用户 hook 时追加 viewer hook。
  - 已有相同 viewer hook 时不写。
  - 已有旧 viewer hook 时只替换 viewer hook。
  - signal file 半行不消费。

- 前端单测：
  - Codex `pty-input -> hook completed`。
  - Codex `hook started -> clearLocalWorkingTurn` 不误清。
  - Codex 新建 session 的 `sessionPath` 迟到时，pending hook signal 能绑定到 tab。

## 风险

- Codex 可能要求用户 trust 非托管 command hook。viewer 自动写配置不一定等于 Codex 立即执行。
- 外部终端中用户手动启动 Codex，只有在用户级 hook 配置生效时才能追踪。
- hook 写信号是 best-effort，不能影响 Codex 本身运行。

## 参考资料

- Codex hooks: https://developers.openai.com/codex/hooks
