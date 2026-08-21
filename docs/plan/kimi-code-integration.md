# Kimi Code 接入开发计划

日期：2026-08-21  
状态：待实施

## 目标与范围

在不改变现有 Claude Code、Codex、Grok Build、Antigravity CLI、opencode 行为的前提下，把 **Kimi Code CLI** 接入为一等会话来源。完成后，Kimi 会和已有 agent 一样可在侧栏切换、查看历史、在终端恢复，并进入搜索、统计、回收站、导出、worktree、托盘和桌面宠物等通用链路。

本期包含：

| 能力 | 本期交付方式 |
| --- | --- |
| 会话发现 | 扫描 `$KIMI_CODE_HOME/sessions`，利用 `session_index.jsonl` 加速并以磁盘 `state.json` 为真源兜底。默认根目录为 `~/.kimi-code`。 |
| 消息解析与 Live tail | 解析主 agent 的 `agents/main/wire.jsonl`；会话详情和已有 Markdown/HTML 导出共享该解析结果。 |
| 删除、恢复、永久删除 | 将完整 session 目录作为原子单元；同步维护 Kimi 的 `session_index.jsonl`。 |
| 用量、统计与计费 | 从 `usage.record` 统计主 agent 和子 agent 调用；模型价格严格匹配，不能确定价格时标记未定价。 |
| CLI 环境检查 | 检测 `kimi --version`、安装冲突和 `kimi doctor`；支持官方 macOS/Linux 与 Windows 安装命令。 |
| 内嵌与外部终端 | 恢复使用 `kimi --session <id>`，新会话使用 `kimi`；沿用现有 PTY/外部终端基础设施。 |
| 运行状态 hooks | 向 Kimi 用户级 `config.toml` 以 `[[hooks]]` 形式合并 viewer 管理的状态 hooks。 |
| 搜索、重命名、导出 | 接入通用搜索、应用内 Markdown/HTML 导出和重命名；重命名仅更新 Kimi 支持的 session metadata。 |
| 设置、托盘、worktree、桌面宠物 | 加入 agent 可见性、启动参数、托盘统计、worktree 清理清单和宠物状态信号链路。 |
| 文档与测试 | 更新支持列表、使用说明、风险说明，并补充 Rust/Vitest/人工验收覆盖。 |

明确不在本期：

- **不接入 GUI Chat / Chat composer。** 虽然 Kimi 支持 `-p --output-format stream-json`，本期 `guiChat` 必须保持 `false`，不实现 `agent_chat`、模型菜单、权限弹窗、附件或聊天侧栏协议。
- 不把 Kimi 的 `/fork` 做成 viewer 的会话 fork。现有通用 UI 没有 Kimi 的安全、稳定的非交互 fork 契约。
- 不在应用中暴露 `kimi export` 的原生诊断 ZIP。它默认附带全局诊断日志，可能包含其它会话信息；本期的“导出”是现有应用的 Markdown/HTML 导出。用户仍可在终端自行运行 `kimi export`。
- 不读取、展示、备份或写入 `credentials/`、API key、全局日志、MCP 配置和 Skills。

## 调研结论与实现依据

### 官方 CLI 契约

- 安装：macOS/Linux 为 `curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash`；Windows 为 `irm https://code.kimi.com/kimi-code/install.ps1 | iex`。
- 恢复指定会话为 `kimi --session <sessionId>`，新会话为 `kimi`；`--continue` 仅恢复当前 cwd 最近会话，不能替代指定 ID 恢复。
- 数据根由 `KIMI_CODE_HOME` 控制，未设置时为 `~/.kimi-code`。
- Kimi hooks 位于用户级 `$KIMI_CODE_HOME/config.toml` 的 `[[hooks]]` 数组。每项仅允许 `event`、`matcher`、`command`、`timeout` 四个字段。
- `kimi doctor` 是只读的配置校验入口；`kimi upgrade` 可能交互确认，不能在 viewer 的非交互后台升级流程中直接执行。

参考：

- [Kimi Code 安装页](https://www.kimi.com/code/en)
- [Getting started](https://moonshotai.github.io/kimi-code/en/guides/getting-started)
- [Sessions and context](https://moonshotai.github.io/kimi-code/en/guides/sessions.html)
- [Data locations](https://moonshotai.github.io/kimi-code/en/configuration/data-locations.html)
- [Hooks](https://moonshotai.github.io/kimi-code/en/customization/hooks.html)
- [kimi command](https://moonshotai.github.io/kimi-code/en/reference/kimi-command.html)

### 本机样本（Kimi 0.38.0）确认的格式

两个基础会话与官方文档一致，根目录结构为：

```text
$KIMI_CODE_HOME/
├── session_index.jsonl
└── sessions/
    └── wd_<slug>_<sha256-prefix>/
        └── session_<uuid>/
            ├── state.json
            ├── agents/main/wire.jsonl
            └── agents/<subagent-id>/wire.jsonl
```

- `state.json` 含 `id`、`cwd`、`createdAt`、`updatedAt`、`title`、`isCustomTitle`、`archived`、`lastPrompt` 和 agent 清单。
- `session_index.jsonl` 每行提供 `sessionId`、`sessionDir`、`workDir`，但磁盘目录和 `state.json` 才是发现与恢复的真源，索引可能残留或落后。
- 主 `wire.jsonl` 的重要事件包括 `turn.prompt`、`context.append_loop_event`（`content.part`、`tool.call`、`tool.result`、`step.end`）、`llm.request`、`usage.record`、`turn.ended`。
- 实际样本的 `AskUserQuestion` 是 `context.append_loop_event` 下的 `tool.call`，`event.name='AskUserQuestion'`。调用稳定使用 `turnId`、`step`、`stepUuid`、`uuid`、`toolCallId` 和 `args.questions[]`；对应 `tool.result` 仅以 `toolCallId`/`parentUuid` 关联，未重复 turn/step。样本有两次前台单题、每题三个选项，结果均为 `event.result.output` 中的 JSON `{ "answers": { "<question text>": "<selected label>" } }`，且 result 的 `parentUuid` 等于 call 的 `uuid`。
- Kimi 内置契约允许每次 1–4 题、每题 2–4 选项、`multi_select`，并自动提供 Other。前台取消的成功结果为 `{ "answers": {}, "note": "User dismissed…" }`；不支持时是 error text。`background=true` 则先返回 task 状态文本，真正答案异步进入 background task，不能把这个即时回执误当作已选答案。
- `usage.record` 记录每次模型调用的 `inputOther`、`output`、`inputCacheRead`、`inputCacheCreation`。`step.end.usage` 在样本中可能为空，且与 `usage.record` 重复，故不能两者相加。
- 同一 session 可含子 agent；主对话只应读取 `agents/main/wire.jsonl`，但统计必须包含每个 `agents/*/wire.jsonl` 的真实模型调用。
- 另一条含子 agent 的真实样本确认：`usage.record` 是**顶层** wire event，不在 `context.append_loop_event` 内；两个 wire 合计 27 条 `llm.request`、26 条 `usage.record`，且两者都没有 `turnId`、`step` 或 `stepUuid`。同一 agent 的同一个 step 可并发写入多个 `tool.call`，随后分别写入 `tool.result`；结果必须由 `toolCallId` 关联。实际 `tool.result` 的值在 `event.result` object 内，`output` 与 `isError` 都在这个 object 中。样本工具输出虽引用图片本地路径，但 wire 没有二进制图片 content；viewer 不得依路径读取或嵌入本地文件。

## 目标架构

```text
KIMI_CODE_HOME
  └─ sessions/<workDirKey>/<sessionId>/              ← 一个可删除/恢复的完整目录
       ├─ state.json                                  ← 项目、标题、时间、状态
       └─ agents/{main, subagent}/wire.jsonl          ← 事件、消息、工具、usage
             │
             ├─ KimiSource (agents/kimi.rs)
             │    ├─ 会话列表 / 详情 / 搜索 / rename / terminal command
             │    ├─ 主 wire 解析为 Msg[]
             │    └─ 全 agents wire 解析为 Turn[]
             │
             ├─ 既有 SessionSource 通用层
             │    ├─ 搜索、watch、会话用量、流式统计
             │    └─ 回收站目录原子移动
             │
             └─ UI
                  ├─ history / terminal / export / settings
                  └─ tray / worktree / desktop pet
```

路径和安全边界：只接受位于已解析 `$KIMI_CODE_HOME/sessions` 下、形如 `<workDirKey>/<sessionId>/agents/main/wire.jsonl` 的真实文件。发现、删除、恢复、hard delete 都拒绝符号链接、`..`、根目录外路径和不完整 session 目录，且永不触碰 Kimi 的 credentials、全局 logs、plugins 或其它非会话数据；只有 hooks 阶段会按最小化规则更新 `config.toml` 中 viewer 自己的项。

## 分阶段实施计划

### 1. 建立 Kimi 会话源与发现契约

涉及：`src-tauri/src/agents/kimi.rs`（新增）、`src-tauri/src/agents/mod.rs`、`src-tauri/src/lib.rs` 的仅必要注册点。

1. 新增 `KimiSource`，在 agent registry 中声明模块并将 `"kimi"` 路由到该 source；不在 `lib.rs`、搜索、回收站或终端驱动中增加 `agent == "kimi"` 分支。
2. 实现 `kimi_home()`：优先读取非空 `KIMI_CODE_HOME`，否则使用 `dirs::home_dir()/".kimi-code"`。所有会话、index、hook 配置路径都从同一函数取得。
3. 读取 `session_index.jsonl` 作为候选目录索引，同时遍历 `sessions/*/session_*` 补足缺失条目；去重键为 canonical session root。索引指向不存在、越界或不完整目录时静默忽略，不让一个坏 index 行阻断其它会话。
4. 从 `state.json` 获取 `cwd`、标题、创建/更新时间、归档状态与 session ID。标题优先 `title`，为空时使用清理后的 `lastPrompt`，再回退 ID；本期不新增 Kimi archive 筛选器，`archived` session 也保留在历史发现结果中，避免静默隐藏用户数据；项目按 `cwd` 聚合，`workDirKey` 作为稳定 `ProjectInfo.dirName`。
5. 生成 `SessionMeta` 时：
   - `path` 固定为主 transcript `agents/main/wire.jsonl`；
   - `id` 优先 state ID，回退目录名；
   - `size` 为不跟随 symlink 的整个 session 目录大小；
   - `messageCount` 为主 wire 中有效 `turn.prompt` 数；
   - `created`/`modified` 使用 state 时间并以主 wire/state mtime 兜底。
6. 实现 `validate_session_path`、`session_storage_unit`、`validate_restore_target` 和 `source_mtime`。storage unit 的 root 是 session 目录，entry 相对路径为 `agents/main/wire.jsonl`，并以所有相关 wire/state 的最大 mtime 作为缓存失效锚点。
7. 设置 `watch_target` 为主 wire；新增 append 或 rewrite 后由既有 watcher 触发整段安全重读。主 wire 不存在时静默退回一次性读取，不监听子 agent wire。
8. 对会话详情读取建立一致 snapshot：先读取 `state.json` 和 main wire 的完整 bytes，再复核 file identity、size、mtime；任一文件在读取期间变化则丢弃该次组合结果并短暂退避重试。watcher 对连续 append 做 debounce；半写入尾行只显示等待下一次刷新，不得把新 state、旧 wire 或不完整工具对分别提交给详情、搜索、导出和统计。全 agents usage 统计以同一轮 snapshot 的各 wire 为单位，子 agent 的后续刷新不得污染已提交 main transcript。

验收：给出不存在根目录、空 index、失效 index、两项目、多 session、定制 `KIMI_CODE_HOME`、目录中含 symlink 等 fixture，项目和分页列表均稳定且不越界。

### 2. 消息、工具、搜索和会话辅助信息解析

涉及：`src-tauri/src/agents/kimi.rs`、现有 `SessionSource` 通用搜索/usage 缓存，无前端协议扩展。

1. 写一个容错 JSONL 事件迭代器：坏行、半行、未知 `type` 必须跳过；解析不能因一次未完成写入而使整个会话详情失败。`mcp.tools_discovered`、`llm.tools_snapshot`、`permission.set_mode` 等配置/发现事件不是历史工具调用，默认只作受限诊断 metadata，不能伪造消息、触发工具行为或将完整 schema 写入日志。
2. `read_session` 只读取 `agents/main/wire.jsonl`，按事件时间/文件顺序重建 `Msg[]`：
   - `turn.prompt.input[]` 的文本合并成 `role: user`；
   - `content.part.part.text` 追加到对应 assistant 回复；`part.think` 映射为 `thinking` block；
   - `tool.call` 映射为带稳定 `toolCallId` 和 JSON 参数的 `tool_use` block；
   - `tool.result` 映射为相同 ID 的 `tool_result`；按实际 schema 从 `event.result` object 提取受限长度的 `output`、`note` 和 `isError`，并对 future scalar/object result 安全降级；
   - `toolCallId` 是唯一的 call/result 关联键。一个 turn/step 可有多个并发 tool.call，result 可晚到或与其他事件交错；缺失、重复或不匹配 ID 的 result 保留为未关联诊断结果，绝不按相邻位置或“最近工具”猜配；
   - 用 wire physical ordinal 维持原始事件顺序，用 turn/step/uuid 只做归属与去重，不能假设 user/assistant/工具交替。
3. `context.append_message` 仅作为早期格式或缺少 `turn.prompt`/loop event 时的 fallback，避免把同一上下文记录和事件流重复渲染。
4. 对缺失、取消和未结束 turn：保留已落盘 user prompt、已收到的思考/文本/工具、finish/error 状态和已有 usage；不要伪造“完成”消息，也不能仅因错误文本存在就把整轮或所有子 agent 伪装为 provider failure。模型名仅在有可靠调用关联时显示。
5. 实现 `last_prompt`（优先 state 的 `lastPrompt`，再从主 wire 反查）和 `context_usage`（最后一条规范 `usage.record`），供现有列表副标题和上下文徽标复用。
6. 实现 `contains_text`/用户文本提取，搜索范围为标题、ID、cwd 和真实用户 `turn.prompt`。不搜索工具输出、思考、API key、诊断 metadata、图片路径或任何外部文件内容；继续复用通用取消令牌和命中文本定位。
7. 不根据工具 arguments/output 内的本地路径读取、嵌入或预览文件。当前 Kimi 0.38 样本没有 image content schema；未来若出现二进制或 data URL block，先以独立 fixture 定义 MIME、解码、尺寸、内存和导出上限，未识别时只显示 placeholder。
8. 确认现有 `messagesToMarkdown`/`messagesToHtml` 直接消费同一份解析 snapshot 的通用 `Msg`；补齐 Kimi agent 头像/标签后，导出无需 Kimi 专属格式分支。导出须保留 partial/cancelled/failed/未关联工具结果的可见状态，不能静默丢失。

验收：样本中用户消息、assistant text、think、Bash/Read/Grep/Agent/MCP 工具及工具结果顺序正确；同 step 并发 call 与乱序 result 均按 `toolCallId` 正确关联；连续 tail 时不重复；搜索只命中标题/ID/cwd/用户 prompt，且不会因工具路径读取本地文件；导出的 Markdown/HTML 与详情使用同一 snapshot。

### 2.1 AskUserQuestion 历史卡片（Kimi 特化输入，复用通用只读 UI）

目标是让 Kimi 会话详情以与 Claude 历史相同的只读结构化选择卡展示提问、选项和已选答案；这**不是** GUI Chat，也不允许 viewer 对运行中的 Kimi 回写答案。

1. 在主 wire 解析器中识别 `context.append_loop_event.event.type='tool.call' && event.name='AskUserQuestion'`。创建 assistant `tool_use` block，`toolName='AskUserQuestion'`、`toolId=event.toolCallId`；把已验证的 `args` 序列化为 tool input。一般工具可保留原 JSON；该工具必须规范化为共享卡片格式：`questions[]`、`question`、`header`、`options[{label,description}]`、`multiSelect`。兼容 Kimi 的 `multi_select`，不可因 snake_case 而把多选误渲染成单选。
2. 对应的 `tool.result` 通过 `toolCallId` 关联，即使 result 事件缺 turn/step/timestamp 也不可按“最近一条工具”猜配。`parentUuid == tool.call.uuid` 仅作为样本一致性诊断，主键始终是 `toolCallId`。result 生成同 id 的 `tool_result` block，保留原始 `output` 字符串和 `isError`；这样现有 `resultByToolId` 能跨消息回填到提问卡，独立的协议 result 行会自动隐藏，不会产生一个伪装为用户消息的 JSON 气泡。
3. 扩展共享 `parseQuestionAnswers`：先在受限大小内严格解析 Kimi `{answers: Record<string,string>, note?: string}`，只接受 plain object 和 string value；成功时返回 answers，随后再 fallback 到 Claude 的历史 `"问题"="答案"` 格式。Kimi 空 `answers` + dismissed/cancelled note 应显示“已取消”，error result 显示失败，未知 JSON/普通文本只显示已完成或普通工具错误，绝不能猜测推荐项或把 task id 当选择。多选结果按 Kimi 契约是逗号分隔 label：先做完整 label 精确匹配；只有所有候选 label 均不含逗号且拆分后的每项唯一匹配时才勾选多项。存在歧义时保留原 answer 文本、不勾选任何候选，不能错标答案。
4. 强化共享 `parseQuestionRequest`/normalizer：兼容 `multiSelect` 与 `multi_select`；限制 1–4 题、每题 2–4 项、非空且去重的 question/option label、字符串长度和 JSON 输入大小。历史损坏、重复 question 或未知结构不交给卡片，以普通可展开 tool call 降级，避免同一个答案 key 错标到多道题。
5. `background=true` 的 card 显示“后台提问 / 等待答案”，即时 `task_id` 回执不进入 `historyAnswers`。本期不承诺从 background task、后续自动通知或 `TaskOutput` 逆向拼接最终答案；只有同一 `toolCallId` 的规范 answers JSON 才显示为已回答。这个限制和 fixture 要写入文档，避免虚假的历史选择。
6. `ChatQuestionPrompt` 的 agent 标题改用通用 `agentLabel(agent)`，不能沿用当前“Codex 否则 Claude”的二元判断；Kimi 历史卡必须显示 Kimi。现有 `ChatView` 已将 `AskUserQuestion` 设为始终显示并隐藏其独立 result，接入时验证这一通用逻辑对 Kimi 同样成立，而不是增加 `agent === 'kimi'` 特判。
7. 会话详情、HTML 导出使用同一卡片数据；Markdown 以问题、选项、已选值/取消状态的文本形式导出。全局 session 搜索仍只索引真实用户 `turn.prompt`，不把结构化题目、选项或 Other 输入加入索引；会话内的 tools 搜索可按既有 `tools-other` scope 命中卡片。

验收：用脱敏 synthetic fixture 覆盖单选、多选、Other、多个问题、已回答、dismissed、error、background-pending、result 早/晚到、缺失 result、错误 `toolCallId`、重复/超限 input 和 JSON/Claude legacy answer 双格式；全部仅为只读回放，工具显示开关关闭时该卡仍可见，Kimi 标题正确且没有裸 JSON tool result。

### 3. 重命名与目录型回收站一致性

涉及：`src-tauri/src/agents/mod.rs`、`src-tauri/src/agents/kimi.rs`、`src-tauri/src/trash.rs`、必要的 `src-tauri/src/lib.rs` 调用点。

1. 先用一个隔离的临时 Kimi 会话执行原生 `/title`，记录 CLI 实际写入的 `state.json` 标题字段组合；以该事实定义 rename，而不是猜测 `titleKind` 的枚举值。
2. `rename_session` 保留 `state.json` 的未知字段，仅原子更新官方确认的 title/custom-title 字段；沿用 `validate_rename_name`，写入失败不能留下截断 JSON。
3. 扩展 `SessionSource` 的回收站扩展契约，使 directory source 能在移动前提供额外 metadata，并在软删、恢复、hard delete 后完成自有索引维护。该扩展应保持现有文件型 agent 无感。
4. 软删 Kimi session 时：
   - 先把完整 session 目录移动到现有 viewer trash；
   - sidecar metadata 保存原 root、entry、storage kind 和相匹配的 `session_index.jsonl` 行；
   - 成功移动后原子重写 index，移除对应 session ID 的行；
   - index 更新失败时报告明确错误并尽力回滚到可恢复状态，绝不悄悄制造“CLI 看得到但 viewer 看不到”的半完成状态。
5. 恢复时先严格验证目标仍在 Kimi sessions 根、目录名/state ID/主 wire 均匹配，目标目录不存在才移动；随后按 sidecar 的原始 index 行恢复且按 session ID 去重。冲突或损坏时保留 trash 原件并报错。
6. 永久删除同时删除完整 directory unit 和相应 index 行；仅在确实为空时清理父 `workDirKey` 目录，绝不删除 `$KIMI_CODE_HOME/sessions` 本身。
7. 回收站列表、标题提取、目录大小和恢复后的搜索/统计缓存失效均走 `KimiSource`，不为 Kimi 复制一套 UI。
8. Kimi 没有跨进程 session lock。已知内嵌 terminal 仍在运行时禁用 rename/delete/restore；外部 CLI 只能 best-effort：操作前后 re-stat/reparse state、main/subagent wires 与 index，检测到变动即取消或报告冲突，绝不对增长中的 wire 做整体重写。rename 仅原子替换已验证 revision 的 state；目录移动与 index 更新失败时保留可恢复原件，不能让 state/index/wire 分裂。

验收：软删后 `kimi --session <id>` 与 viewer 均不再发现；恢复后两者都能恢复；session 的 logs、plans、tasks 和子 agent wire 随目录完整往返；冲突、坏 metadata、越界路径、symlink、index 写失败均不损坏原数据。

### 4. 用量、统计和价格策略

涉及：`src-tauri/src/agents/kimi.rs`、`src-tauri/src/stats/pricing.rs`（仅必要的严格匹配辅助）、既有 stream/aggregate/tray。

1. 将每个**顶层** `usage.record` 视为一次权威模型调用；`step.end.usage` 只用于完成信息或 `usage.record` 缺失时的受控 fallback，绝不与前者相加。不得从 tool result、相邻 assistant 或空 `step.end` 推算 usage；中止/失败调用只要已有 usage 也必须入账。
2. 本机 0.38 样本的 `usage.record` 与 `llm.request` 均没有 `turnId`/`step`/`stepUuid`，且 request 数可多于 usage。因此以 `usage.record` 的 `agentId + model + physical ordinal/time` 为调用身份；仅当每 agent 的未匹配 request 队列能唯一、顺序一致地匹配时，才附加 `llm.request` 的 provider、调用时间和工具 metadata。任何不唯一、缺失或多出的 request 均不配对、不补算；保留 usage 的原始 model/token，并标记 provider/价格未知而非猜测。
3. 将 Kimi usage 映射到通用 `UsageSummary`：
   - `inputOther → inputTokens`
   - `output → outputTokens`
   - `inputCacheRead → cacheReadInputTokens`
   - `inputCacheCreation → cacheCreationInputTokens`
   - 其它字段为 0，并以统一 `finalize()` 算 total。
4. `read_turns` 和 `usage_summary` 扫描 session 中每个 `agents/*/wire.jsonl`，将子 agent 的调用计入父 session 总量、项目、Top Sessions、托盘和时间轴；会话详情始终只显示 main。
5. 解析 `tool.call` 填充通用 by-tool、Bash 首命令、MCP server 和 activity 分类所需字段。子 agent 没有用户 prompt 时仍记录 API 调用和工具，但不伪造用户文本。
6. 价格采用**严格、安全、可解释**的策略：
   - 使用 wire 中原始 provider/model，只有明确的官方/目录精确匹配才调用 `cost_usd_strict`；
   - 禁止落入现有通用 `cost_usd` 的 Claude 平均价 fallback；
   - Kimi 可配置 OpenAI、Anthropic 或任意自定义 endpoint，不能从字符串猜测真实账单；未匹配价格时 cost 为 0 并增加 `unpricedCallCount`；
   - OAuth/订阅与第三方代理均将 USD 视为模型目录估算而非账单；若将来要显示“估算”标识，应使用通用的 provider-neutral 字段，而不是复用 Grok 专属文案。
7. 为模型别名（例如 wire 中的 `provider/model`）添加严格归一化测试；仅在官方文档和价格目录都能证明等价时添加别名，不能因为名称相似而跨 provider 归并。

验收：一个 `usage.record` 恰好是一条 call；带 cache 的 session 总量正确；`step.end` 重复/空 usage 不会双算；主/子 agent 合计正确；未知或自定义 provider 不出现伪造 Claude 成本；范围筛选和 tray 三个时间窗口与 Stats 页面一致。

### 5. 共享 UI、设置、搜索、导出与 worktree

涉及：`src/types.ts`、`src/agentMeta.ts`、`src/settings.ts`、`src/App.vue`、`src/components/icons.ts`、`src/locales/{en,zh,zh-TW,ja}.ts`、相关 Vitest。

1. 将 `kimi` 加入 `Agent` 联合类型、`ALL_AGENTS`、icons、avatar、四种语言的 agent/统计 scope 文案和所有 exhaustive map。
2. 增加 `AGENT_META.kimi`：`history/terminal/worktree/hooks/stats/pricing = true`，`guiChat = false`。所有入口继续通过 capability gate 判断，不添加散落的 Kimi 名称白名单。
3. 持久化升级兼容：
   - `enabledAgents:v1` 缺少 Kimi 时按新 agent rollout 规则启用；新安装的默认可见列表也包含 Kimi；
   - `launchArgs:v1` 增加 `kimi` 字段，读取旧对象时补空字符串且不丢弃其它 agent 参数；
   - 保持项目排序、最近记录、view tabs、导出历史按现有 `Agent` 键自动隔离。
4. 通用搜索、重命名、批量删除、回收站、会话级/全局统计和 Markdown/HTML 导出应在 source/type 接入后自然生效；Kimi 的 `AskUserQuestion` 复用既有 `ChatQuestionPrompt` 只读卡与 result-by-tool-id 合并机制，同时修复共享 parser 的 Kimi JSON answers、snake_case 多选和 agent 名称泛化。新增针对 Kimi 的端到端调用测试，避免因类型断言遗漏。
5. 将 `kimi` 加入 `WORKTREE_AGENTS`。Kimi 的 `state.cwd` 必须使用真实 worktree 路径，使 worktree 创建后可被侧栏发现，移除 worktree 时会统计并 hard-delete 该 cwd 下的 Kimi session 目录及 index 行。
6. 更新 worktree 数量提示和注释为“能力驱动/包含 Kimi”，并验证 Kimi 没有会话时仍使用既有 worktree 占位项目逻辑。

验收：升级前 localStorage 不报错且 Kimi 可见、启动参数重启后保留；Kimi 不出现 New Chat/quick-open Chat 入口；导出、搜索、重命名、回收站和删除 worktree 均正确指向 Kimi 数据。

### 6. CLI 环境检查、外部终端和内嵌终端

涉及：`src-tauri/src/cli_env.rs`、`src-tauri/src/agents/kimi.rs`、`src/components/CliEnvironmentCheck.vue`、`src/components/icons.ts`、i18n、终端测试。

1. 在 `CLI_SPECS` 新增 `kimi`：二进制名 `kimi`，官方 Unix/Windows 安装命令和官方文档链接；安装仍由现有显式用户点击动作触发。
2. 版本检查使用登录 shell 的 `kimi --version`，诊断沿用 `which -a`/`where.exe` 的多安装冲突检测。识别官方单二进制安装路径为 `system`，npm 安装仍按既有 shim/`node_modules` 规则识别。
3. 对已安装 Kimi 追加只读 `kimi doctor` 健康检查；UI 只显示“配置可用/失败”和经过截断、去敏的错误摘要，不回传 config 内容、token、环境变量或日志。
4. 不调用交互式 `kimi upgrade` 作为后台升级。CLI 卡片保留已安装/版本/健康状态；官方 `tui.toml` 默认自动更新由 Kimi 自己管理，用户需要时在外部终端运行 `kimi upgrade`。
5. `resume_command` 返回 `kimi --session <id>`，`new_session_command` 返回 `kimi`；二者按现有 `AgentCommand` 规则接收用户持久化的 extra args。命令参数必须以结构化 arguments 传递，不能拼接未转义 session ID。
6. 使用现有 login + interactive shell PTY 和外部终端启动器，因此 nvm、npm 和官方 installer 写入的 PATH 都能生效；不把 `~/.kimi-code/bin/kimi` 硬编码为唯一可执行文件。

验收：本机官方 installer 路径、npm 路径、PATH 缺失、多个 `kimi`、损坏 config 和用户 `KIMI_CODE_HOME` 都得到明确结果；会话可从内嵌/外部终端恢复，新 session 在项目 cwd 下创建。

### 7. 运行状态 hooks 与桌面宠物

涉及：`src-tauri/src/turn.rs`、`src-tauri/src/turn_signal_hook.cjs`、`src-tauri/src/agents/kimi.rs`、`src/api.ts`、`src/turnHookStatus.ts`、Settings hooks UI、desktop pet 测试。

1. 为 Kimi 定义最小状态映射：

| Kimi hook | viewer state | 理由 |
| --- | --- | --- |
| `TurnStarted` | `started` | 一轮真实开始。 |
| `Stop` | `completed` | 正常准备结束 turn。 |
| `StopFailure` | `failed` | 本轮因错误结束。 |
| `PermissionRequest` | `blocked` | 正在等待用户批准。 |
| `Interrupt` | `completed` | 清除 running；当前统一状态模型没有 cancelled，且不是失败。 |

2. 在 `$KIMI_CODE_HOME/config.toml` 中只合并 viewer 自己的 `[[hooks]]`：根据命令是否引用 viewer hook script 判断归属，先移除旧版 viewer 项再追加新版项；保留顺序和所有用户 hook。遇到 TOML 无法解析或顶层类型不兼容时停止并报错，绝不覆盖文件。
3. `TurnHookInstallResult`、`TurnHookStatus`、设置页 hook inventory 和 i18n 均新增 Kimi，整体“已安装”状态需要包含已启用的 Kimi hooks；页面展示真实 Kimi config 路径。
4. Kimi hook stdin 有 `session_id`、`cwd`，但没有 `transcript_path`。扩展 relay script 接受 `kimi` 并写入 session ID/cwd；在 Rust 侧新增 `find_main_wire_path(session_id, cwd)`，从 Kimi 会话源解析主 wire。目录刚创建而尚未落盘时使用有限重试，解析失败则不写假路径。
5. hook 脚本继续 fail-open：任何 JSON、I/O 或路径解析异常均 exit 0，不能改变 Kimi 的 hook stdout/阻断语义；每个 hook timeout 设为 5 秒。
6. 既有 `emit_turn_signal`、desktop task snapshot 和 `resolve_desktop_pet_session` 通过 source registry 找到 Kimi path/title。桌面宠物不需要 Kimi 专属动画；收到 started/blocked/completed/failed 后沿用现有优先级、通知和点击打开会话流程。

验收：首次安装只添加五项 Kimi hooks，重复安装不重复；已有用户 hooks 不变；坏 TOML 不覆写；模拟 session ID 信号能进入正确 Kimi tab/宠物状态；完成、失败、权限、打断均能清除或转换运行态。

### 8. 托盘、文档和回归验证

涉及：`src-tauri/src/stats/tray.rs`、`README.md`、`README.zh-CN.md`、必要的日文 README/产品文档、`docs/plan/kimi-code-integration.md`。

1. 将 `kimi` 纳入 `TRAY_AGENT_NAMES`、可见性同步和 agent summary。只有用户在 Settings 中启用 Kimi 时才显示；没有近 30 天活动仍显示零值，和现有可计费 agent 一致。
2. 更新支持 agent 的 README 段落：安装、会话数据根、恢复、KIMI_CODE_HOME、主/子 agent 统计、目录型删除恢复、hooks、worktree 与本期 GUI Chat 不支持。
3. 明确安全/隐私文案：应用内导出不含 Kimi 全局日志；`kimi export` 的 ZIP 默认可能含全局日志，分享前应检查；viewer 不读取 credentials。
4. 保留本文件作为设计与验收记录；实现完成后补充实际版本兼容范围、已验证的原生 `/title` state 写法和已知 Kimi 格式变更。

## 测试计划

### Rust 单元与集成测试

- 使用**合成、无真实 prompt/凭证**的 fixture，覆盖 Kimi root、index、state、main/subagent wire。
- 发现：空/缺失 root、失效 index、磁盘兜底、cwd 项目聚合、分页、归档字段、KIMI_CODE_HOME、symlink 与路径逃逸拒绝。
- 解析：text/think/tool call/tool result、同 step 并发 tool.call、早/晚到 result、缺失/重复/失配 `toolCallId`、`event.result` object 的成功/错误/超长 output、坏 JSONL、半行、取消 turn、`context.append_message` fallback、last prompt、watch target；`mcp.tools_discovered`/tools snapshot 不伪造历史调用；工具路径不触发本地文件读取；`AskUserQuestion` 的 stable `toolCallId` 关联、Kimi JSON answers/dismissed/error/background 回执、snake_case multi-select、失配/重复/超限题目降级。
- 一致性：state/main/subagent wire 在读取期间 append 或 rewrite、半写入尾行、watch debounce、详情/搜索/导出/统计同 snapshot，以及只重刷子 agent usage 不改变已提交 main transcript。
- 统计：顶层 `usage.record` 字段映射、usage/step.end 去重、`llm.request` 多于 usage、缺失 turn/step 时的保守 provider 关联、cache、子 agent 合计、时间范围、Bash/MCP、strict pricing 缺失标记。
- 回收站：目录原子移动、index 移除/恢复/去重、hard delete、目标冲突、坏 metadata、失败回滚、父目录清理，以及外部 Kimi 写入期间的 revision 冲突不修改原会话。
- hooks：TOML 新建、合并、重装去重、保留用户项、旧 viewer command 更新、坏 TOML 不覆写、Kimi session ID 到 wire 路径解析、状态映射。
- CLI：version 提取、官方/native 与 npm 多安装诊断、doctor 结果脱敏、结构化 resume/new command。
- tray：Kimi visibility、空活动保留、token/cost 累计及三时间窗口。

### Vitest

- `Agent`/`AGENT_META`/icons 的 exhaustive map，断言 Kimi `guiChat === false` 且其它目标 capability 为 true。
- `settings.ts` 的旧 `enabledAgents:v1`、旧 `launchArgs:v1` 迁移和 Kimi 持久化。
- Settings CLI 卡片标签、文档链接、doctor 成功/失败展示，以及无 GUI Chat 的 quick-open 守卫。
- Stats scope、tray enabled payload、worktree agent 列表、导出标题/agent label、桌面宠物 task 解析。
- `chatQuestion.ts` 同时解析 Claude legacy 回显与 Kimi JSON answers；`ChatQuestionPrompt` 用 Kimi 名称、正确显示单/多选/Other/取消/失败/等待且不会将 background `task_id` 误标为回答；AskUserQuestion 在隐藏普通工具时依然显示、裸 result 不重复显示。

### 手工验收

1. 用本机两个基础会话确认项目、标题、消息、工具、搜索、统计和导出；其中含 `AskUserQuestion` 的会话必须显示 Kimi 的只读选择卡、三个选项及已选状态，而不是原始 JSON 或 Claude 标签。使用含子 agent 的会话确认并发同 step 工具、失败 tool result、主/子 agent 账目合并，以及 request 数与 usage 数不一致时不产生重复调用或虚构 provider。
2. 在隔离项目执行 `/title`，用 CLI `/sessions` 和 viewer 双向确认 rename；软删、恢复、永久删除后分别用 `kimi --session <id>` 验证。
3. 在主仓库创建 worktree，Kimi 在 worktree 中新开会话，确认列表归属和删除 worktree 时的 session 清理提示/结果。
4. 分别从内嵌 terminal 和外部 terminal 恢复会话；验证 CLI 健康检查、PATH 缺失提示和 KIMI_CODE_HOME 自定义根。
5. 安装 hooks 后，从普通 terminal 发 prompt、触发权限、完成、失败和中断；确认 tab、托盘、桌面宠物和点击回到会话的状态一致。
6. 在 Kimi 连续调用工具时反复刷新详情、搜索、导出和统计；验证它们只在完整 snapshot 后一起更新，半写入尾行不产生重复 tool result/usage。随后在外部 Kimi 保持运行时尝试重命名、删除和恢复，确认 Viewer 阻止或报告 revision 冲突，未重写或拆分 session 数据。

### 质量门槛

- `cargo fmt --check`
- `cargo test`（含新增 Kimi fixture）
- `npm run test:run`
- `npm run build`
- macOS 实机 smoke test；如发布支持 Windows，再补 PowerShell 路径、官方 installer 和外部终端 smoke test。

## 实施顺序与风险控制

推荐顺序：先完成 1–4 的 Rust source/数据安全/统计测试，再做 5–6 的 UI 与终端，随后接入 7 的 hooks/宠物，最后完成 8 的托盘、文档和人工回归。不要先暴露 UI 开关再补数据源，避免用户看到不可恢复或错误计费的会话。

主要风险及约束：

- **Kimi 内部格式会随 CLI 版本变化。** 解析器必须事件白名单、unknown 事件忽略、合成 fixture 锁定已支持版本，并在 README 标明验证版本（当前样本为 0.38.0）。
- **重命名 metadata 语义需由 CLI 实验确认。** 在确认 `/title` 的字段组合前，不实现写入逻辑；这是 rename phase 的前置 gate。
- **价格不能等同账单。** Kimi 能使用订阅、OAuth 和任意 provider；严格未定价优先于错误金额。
- **原生导出和 wire 可能含敏感内容。** 不自动调用 `kimi export`，测试 fixture 不复制真实 prompt、文件路径或 credentials。
- **hooks 是观测而非安全机制。** 只能 fail-open、最小事件集、只管理自身 TOML 项，不能改变 Kimi 的 tool approval 行为。
