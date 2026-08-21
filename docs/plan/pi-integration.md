# Pi Agent 接入开发计划

## 目标、结论与边界

将 Pi (`pi`) 接入 Sessions Viewer，作为新的本地 CLI agent，支持 Pi 会话的发现、阅读、树分支浏览、搜索、重命名、回收站恢复、统计计费、内嵌/外部终端、CLI 检查、运行状态、工作树、托盘、桌面宠物和文档。

沿用本轮既定范围：**不接入 GUI Chat**。Pi 虽提供 JSON/RPC mode，但本次不启动 `agent_chat`、不实现 Composer、模型/思考等级控件、图片附件、权限对话或 RPC 交互。新建和恢复均在原生 Pi TUI 中完成。

Pi 不应被当作单一模型供应商。它是可接入多 provider 的 harness：本机样本实际混有 `deepseek` 与 `github-copilot` provider。因此它有统计和已记录成本能力，但不新增一个伪造的“Pi 模型价格表 family”。

## 已完成的真实数据与文档调研

### 本机安装与数据

| 项目 | 实测结果 |
| --- | --- |
| 二进制 | `/Users/wuchao/.nvm/versions/node/v22.21.1/bin/pi`（npm 全局包的 `dist/cli.js` 链接） |
| 版本 | `0.84.2` |
| 默认 agent 根 | `~/.pi/agent`；本机未设置 `PI_CODING_AGENT_DIR` / `PI_CODING_AGENT_SESSION_DIR`，全局 settings 也未设置 `sessionDir`。 |
| 本机会话 | 3 条：2 条位于 `~/.pi/agent/sessions/--Users-wuchao-apps-claude-session-viewer--/`，另有 1 条位于 `~/.pi/agent/sessions/--Users-wuchao-develop-flutter-sales-app--/`；每条都是一个 `.jsonl` 文件。 |
| 实例版本 | 三条 header 均为 session format v3，均无 `parentSession`，均为线性树（无一父多子节点）。 |
| 较长样本 | 62 行：11 user、28 assistant、19 toolResult，调用了 `bash` / `edit` / `read` / `write`。 |
| 新增工具密集样本 | 这是一个分析期间仍在追加的活动会话：首次观察为 214 行，本次最后一致快照为 333 行（5 user、162 assistant、162 toolResult、1 `bashExecution`，以及各 1 条 `model_change` / `thinking_level_change`）；之后 3 秒未再变化。树仍线性且物理相邻 `parentId` 连续，但它证明一轮中可以连续出现大量 `assistant(toolUse) → toolResult`，完全不满足“user 与 assistant 交替”的假设。最终快照有 162 个 toolCall 与 162 个均带 `toolCallId` 的 toolResult；关联必须按 ID，不能按相邻顺序猜测。实际调用过 `bash`、Dart 和 `chrome-devtools_*` 等外部/MCP 工具。 |
| 内容与用量 | assistant 记录含 text、thinking、toolCall；`usage` 有 input/output/cacheRead/cacheWrite/totalTokens，部分含 reasoning；每次 assistant 都保存成本分项与 `cost.total`。 |
| 中止与工具结果 | 新样本有 1 条 `stopReason='aborted'` 的 assistant：只有 thinking、仍有 persisted usage，并有非空 `errorMessage`，但没有 `rawStopReason`。另有 `toolResult` 按顺序混合多个 text 与 image block（截图结果），其 `details` 是不固定的 object；实际见过 `server/tool`、`error/server`、diff/patch 以及空 object。toolResult 均未带 usage。 |
| 样本成本 | 两条 session 的 persisted assistant `cost.total` 汇总分别为 `$0.00322335` 与 `$0.0081914952`。 |
| 命名/分支/压缩 | 三条样本均尚无 `session_info`、label、compaction、branch_summary、custom 条目，不能据此假设生产数据没有它们。 |

调研仅读取了路径、结构、JSON key、类型、模型/provider、token/cost 数值和树完整性；没有读取或记录 `auth.json`、会话正文、API key 或信任配置内容。

### 官方契约（以 Pi 0.84 文档和本机安装包为准）

- 默认存储在 `~/.pi/agent/sessions/--<cwd-with-slashes-replaced>--/<timestamp>_<uuid>.jsonl`；session root 优先级为 `--session-dir`、`PI_CODING_AGENT_SESSION_DIR`、settings 的 `sessionDir`、默认目录。应用只能发现前 3 项中持久可知的配置，不可能自动发现其他进程临时传入的 `--session-dir`。
- 每行是 JSON object。header 是 `{ type: 'session', version, id, timestamp, cwd }`；其他 entries 使用 `id` / `parentId` 构成树。
- 本机 Pi 的 `SessionManager._buildIndex()` 在读取时将每个非 header entry 依序赋给 `leafId`，即**文件最后一个非 header entry 是 Pi 重开后默认 active leaf**。默认 transcript 必须从这个 leaf 沿 parentId 回溯到根，再按时间正序显示。
- `message` entry 的 role 可为 user、assistant、toolResult、bashExecution、custom、branchSummary、compactionSummary 等；树本身还可含 model_change、thinking_level_change、compaction、branch_summary、custom、custom_message、label、session_info。
- Pi 用最新的 `session_info`（全文件倒序；空 name 表示显式清名）作为显示名称。`/name` 的原生语义是追加 session_info entry，而不是修改 header 或文件名。
- CLI 新建为 `pi`，恢复特定会话可用 `pi --session <path|id>`；产品恢复必须使用绝对 JSONL 路径，避免 partial id 发生跨项目歧义。
- Pi 的 `/tree` 在同一文件内切换分支，`/fork`/`/clone` 才建立新文件；`/export` 可生成 HTML，`/share` 会上传 private GitHub gist。
- Pi 没有固定 hooks 配置文件，但全球/项目 extensions 可订阅 `before_agent_start`、`agent_start`、`agent_end`、`agent_settled`、`turn_start`、`turn_end`、`session_start`、`session_shutdown` 等事件。`agent_settled` 是自动重试、自动 compact 和 follow-up 都结束后的真正 idle 信号。
- Pi 没有内建 sub-agent、plan mode 或 permission popup。扩展可以实现这些功能，但它们没有稳定的核心 session schema，不能在本次承诺自动解析为子 agent、blocked 状态或 GUI Chat。

### `/tree` 精确语义与 Viewer 安全护栏（以本机 Pi 0.84.2 为准）

`/tree` 不是持久化的“当前分支”开关，而是 Pi TUI 对同一 JSONL 树的一次导航操作。接入必须同时区分 **历史树**、**Viewer 的只读查看位置** 和 **正在运行 Pi 的内存 leaf**；三者不能混为一个 `activeBranch` 字段。

| 方面 | Pi 0.84.2 的实际行为 | Sessions Viewer 约束 |
| --- | --- | --- |
| 默认恢复位置 | `SessionManager._buildIndex()` 按 JSONL **物理行顺序**遍历，每个非 header entry 都覆盖一次 `leafId`；因此重开时是“最后一条完整有效非 header entry”，不是树上按拓扑计算的 terminal leaf，也不是最新 timestamp。 | `leafId` 缺省时严格复现此规则；保留物理行号。不得以 timestamp、最后 assistant、或“无 children 的最新节点”替代。显式传入不存在/损坏的 entry id 应报错，不可悄悄回退到另一分支。 |
| `/tree` 的可选项 | 原生树包含所有 entry 类型；默认 UI 只是隐藏 label/custom/model/thinking/session_info 等 bookkeeping，用户仍可切换到 all filter 并选中它们。子节点仅用于显示时按 timestamp 排序。 | 后端返回完整 `PiTreeNode`；前端可以默认折叠 metadata，却不能在模型层丢弃它。无效 timestamp 的同级排序只作为展示，绝不能影响 lineage、默认 leaf 或搜索定位。 |
| 选择 user/custom message | `navigateTree()` 会把运行中 leaf 设为该 entry 的 `parentId`（根则为 `null`），并把原 content 放回编辑器；用户下一次提交才从 parent 创建新的 child。`custom_message` 也遵循此规则，即使 `display=false`。 | Viewer 打开搜索命中或树节点时只显示“通向该节点的历史路径”，不模拟 native 的编辑器回填/leaf 后退，更不写入文件。原生编辑分支是 TUI 的功能，不是本期 GUI Chat 能力。 |
| 选择其他 entry | assistant、toolResult、bashExecution、compaction、branch_summary、custom、label、model/thinking change、session_info 等，native leaf 就是所选 entry；下一次 append 才从其创建 child。branch tip 因而不保证是 assistant。 | branch/list/render/export 的 leaf 类型必须为任意合法 non-header entry；禁止 `lastAssistant`、`user/assistant` 交替或“工具总跟在 assistant 后”之类假设。 |
| 导航是否写盘 | 无摘要时仅调用 `branch()`/`resetLeaf()`，只改内存；退出后丢失。若用户选择摘要，Pi 在新的运行 leaf 追加 `branch_summary`（可再追加 label）；树内编辑 label 也会追加 `label`。下一次消息才使普通分支可见。 | branch selector、搜索跳转和导出必须只传 `read_session(path, leafId)` 的查看参数，绝不调用、仿造或持久化 `branch()`。所有有写副作用的按钮（重命名、删除、恢复）与 branch viewer 分离。 |
| 重新打开已选分支 | `pi --session <file>` 不接受 tree entry id；重载后仍回到物理最后 entry。`/fork`/`/clone` 则会新建 JSONL，其中只有选定路径（label 会被重建），并通过 header `parentSession` 指向旧文件。 | 在某条非默认 branch 上点“在终端恢复”前必须提示：将打开 Pi 原生默认 leaf，不能恢复 Viewer 的临时选择。不得把 Viewer leaf 存进 terminal persistence 后宣称 Pi 会恢复它。 |
| label 与标题 | `labelsById` 和 session name 都从**全文件**顺序解析最新状态：label entry 可挂在任意 branch，却作用于任意 target；最新 `session_info` 也可以不在当前路径。它们不是 branch-local 状态。 | 标签显示为“会话共享标注”，标题为“会话级标题”；不得把它们当 selected branch 的历史事实或据此推断 branch 名称。Viewer 本期不提供 Pi label 编辑。 |

历史路径与 Pi 实际喂给模型的 context 也必须分开：`getBranch()` 是 leaf 到 root 的完整 entry lineage；`buildContextEntries()` 会以路径上最后一个 compaction 为边界，省略已摘要的旧 entries。包内 `docs/session-format.md` 已描述新格式可带 `retainedTail`，而本机 0.84.2 runtime 的实现仍只使用 `firstKeptEntryId`；两种格式不能互相假定。主会话视图、搜索和历史导出渲染完整 lineage；若以后显示“native effective context”，必须按 Pi 版本分别 materialize，并标注为诊断视图。`retainedTail` 是已有 message 的物化 checkpoint，不是新请求、不可进入主 transcript/search、也不可重复计费。

v1 还没有 `id`/`parentId`：Pi 首次打开会随机生成 8-hex id、把线性关系写成 v2，再把 `hookMessage` 改为 v3 的 `custom`，并**重写原文件**。Viewer 对未迁移 v1 只能用 `v1:<physical-ordinal>` 作为本次响应内的临时查看 id，绝不可把它存成可恢复 terminal/搜索定位、也不可据它 append rename；文件 revision 改变后必须失效重读。v2 的 `hookMessage` 按 legacy custom 降级，不能误渲染为 user/assistant。这样既能只读历史，也不会与 Pi 的随机迁移 id 竞争。

## 最终功能范围

| 能力 | 设计结论 |
| --- | --- |
| 会话发现 | 扫描已解析的 Pi session roots 下的 JSONL，按 header.cwd 聚合项目；不依赖编码目录名作为 cwd 真值。 |
| 树状历史 | 默认渲染 Pi 本次重开时的默认 lineage；提供 session 内 terminal-branch picker 与 tree 查看位置，绝不把平行分支串成一条对话或伪造已切换的 native leaf。 |
| 消息与工具 | 把所选 lineage 的 user/assistant/toolResult/built-in tool 归一为 `Msg[]`；保留 Pi entry id 与查看位置用于切换和定位。 |
| 删除、恢复、重命名 | 单文件回收站；重命名通过追加原生兼容的 `session_info` entry。 |
| 搜索与导出 | 全树用户文本搜索并定位对应 lineage；Markdown/HTML 导出当前查看位置，JSON 导出保留 Pi v3 tree 原始 entries。 |
| 统计计费 | 全树唯一 entries 的 persisted usage/cost，含 compaction/branch summary 的单独 usage；不把放弃分支的实际花费丢弃。 |
| 终端与 CLI | Pi 检查、诊断、内嵌/外部 terminal 新建及按绝对路径恢复。 |
| 状态与宠物 | 由 app 托管的 Pi global extension 写运行状态 relay；托盘和桌面宠物复用通用状态/会话解析。 |
| worktree | 以 header.cwd 验证工作树归属；确认后加到前后端 worktree 删除/分组列表。 |
| 显式排除 | GUI Chat/RPC、Pi account quota、自动升级、Pi `/share` 上传、任意第三方 extension 的自定义协议/伪 sub-agent。 |

## 能力声明与跨 agent 契约

前端新增 `Agent = 'pi'`，在 `AGENT_META` 声明：

```text
history=true, terminal=true, guiChat=false, worktree=pending-verification,
hooks=true, stats=true, pricing=true
```

`pricing=true` 仅表示 Pi persisted session usage 中可得到成本，**不表示** Pi 有自己的上游模型 catalog。PricingView 继续按实际 provider/model family（Claude/OpenAI/Grok/Gemini 等）组织参考价目；Pi session cost 首先来自其持久化 `usage.cost.total`，来源在 UI 中标记为“Pi recorded cost”。

将 Pi 排入统一的 agent 顺序（建议 Claude → Codex → Kimi → Pi → Grok → agy → opencode），并为新增用户选择明确默认可见策略。所有入口读取 capability，避免用新的 `agent === 'pi'` allowlist 取代通用逻辑。

后端新增 `src-tauri/src/agents/pi.rs` 实现 `SessionSource`，并只在 `agents/mod.rs::source()` 加 `"pi"` 分派。解析、路径、树和 Pi 特化的计费逻辑都应留在该模块，不能扩散到 `lib.rs`、`trash.rs` 或前端。

## 分阶段开发计划

### 阶段 0：fixture、版本兼容和 agent 影响面基线

1. 从本机三条 session 生成**脱敏** fixtures：v3 linear、assistant thinking/toolCall、toolResult、model/thinking changes 和 persisted usage/cost。新增一条高密度 tool-loop fixture（少量 user、连续大量 `assistant(toolUse) → toolResult`、顶层 `message.role='bashExecution'`、Dart/MCP 工具名、toolCall/toolResult 通过 `toolCallId` 配对而非位置配对、toolResult 的多个有序 text/image block、不同形状的 `details`、isError），以及一条含 `aborted + errorMessage + thinking + usage` 的 assistant。另手造 Pi 官方 schema 覆盖的 branched tree、session_info（含清空名）、跨 branch label、compaction（0.84 的 `firstKeptEntryId` 与文档的 `retainedTail`）、branch_summary、custom/custom_message、bashExecution、v1 无 id 的线性记录、v2 `hookMessage`、坏 JSON、dangling parent、循环/self-parent、重复 id、乱序 timestamp 与“物理最后 entry 非 assistant”的场景。
2. 固定支持 session versions 1–3：v1 linear 使用仅本次读取有效的 physical-ordinal synthetic id，并禁用 rename/持久 terminal leaf；v2 `hookMessage` 降级为 custom；v3 走原始 tree parser。Pi 原生迁移 v1 会随机生成 id 并重写文件，所以 viewer revision 变化必须清除旧 location/cache。超出已知版本以安全降级读取 header/可识别 messages，并在开发诊断中可见。
3. 建立新 agent 穷举清单：`src/types.ts`、`agentMeta.ts`、icons、settings、export history、StatsView；Rust `agents/mod.rs`、stats stream、tray、worktree、turn hook；CLI type unions/UI；四语 locale 和三种 README。
4. 记录 data-root precedence 与解析结果的只读诊断。不得读 `auth.json`，不得把 `models.json`、`models-store.json`、`trust.json` 的敏感字段复制到应用日志或 UI。

### 阶段 1：会话根、发现、项目聚合和元数据

1. 实现 `pi_agent_dir()`：非空 `PI_CODING_AGENT_DIR` 优先，否则 `~/.pi/agent`。实现 `pi_session_root()`：非空 `PI_CODING_AGENT_SESSION_DIR` 优先，否则读取 agent 根的 `settings.json.sessionDir`（按 Pi 文档解析 `~`、绝对/相对路径），最终回退 `agent_dir/sessions`。
2. 只递归扫描解析后的 root 内常规 `.jsonl` 文件；拒绝 symlink、path traversal、root 外 canonical path 和超出资源上限的大文件。`--session-dir` 的一次性外部覆盖因为没有持久索引，文档中明确为“不可自动发现”。
3. 对每个文件只先读取 header：要求 `type='session'`、合法 id/timestamp、非空 cwd；以 header.cwd 正规化后作为 `ProjectInfo.dir_name` 和展示路径。编码目录名仅作候选/一致性诊断，不能替代 cwd。
4. `SessionMeta.id` 使用 header id，`path` 使用 JSONL 绝对路径，`fileName` 为文件名，created 使用 header timestamp，modified 使用文件 mtime。size 是单文件大小。
5. 标题优先最新 `session_info.name`（最新空名称意味着清除）→ Pi 默认恢复 lineage 的第一条用户可显示文本 → 文件名/日期。不得把 branch_summary/custom 内容误当标题。
6. `messageCount` 默认计 Pi 默认恢复 lineage 的可视 user/assistant message 数，同时暴露 `piBranchCount` 和 `piEntryCount`，使列表不会把“整棵树花费”误称为“当前对话条数”。
7. 新会话直到首个 assistant entry 才可能被 Pi 持久落盘；项目刷新沿用现有退避刷新机制，不能假定按下 `pi` 后立即有可扫描文件。
8. 对正在写入的会话，单次读取必须基于一次完整 bytes snapshot 建树；读取前后 file identity/size/mtime 改变时丢弃本轮结果并短暂退避重试。文件 watcher 对连续 append 做 debounce，半写入尾行只报“等待下一次刷新”，不得将不完整 `assistant/toolResult` 对展示、搜索或统计分别提交，从而制造相互矛盾的 session snapshot。

### 阶段 2：树解析、会话阅读、分支选择与搜索

1. 读完整 JSONL 后建立保留物理 ordinal 的 `id → entry` map，并先验证每个 parentId、循环、重复 id、无 header、错误时间和损坏行。v1 在 map 前生成仅本次读取有效的 synthetic ordinal id，且不暴露为可持久化 leaf；v2 `hookMessage` 作 legacy custom。半写入的尾行和无法解析/缺必要 id 的 entry 仅作为诊断跳过；dangling parent 的完整 entry 保留为带诊断的孤立 root（与 Pi `getTree()` 一致），其 lineage 在该点停止。发现环时中止回溯，绝不无限循环。重复 id/self-parent/环和孤立 root 均进入只读降级：不提供 entry-targeted action 或 append rename，避免与 Pi 的“Map 后写覆盖前写”实现产生不可预测的目标。
2. `read_session(path, leafId?)` 的缺省 leaf 为文件最后一个完整有效 non-header entry，匹配本机 Pi 0.84.2 的 `_buildIndex()`；从该 leaf 按 `parentId` 回溯至 root 后 reverse，即 Pi 重开后的 active lineage。`leafId` 只能是合法、已验证的非 header entry；显式非法 id 返回可诊断错误而非 fallback。每次读取、搜索和导出都传同一个 leaf id，保证 UI 不会在不同 API 间悄悄换 branch。
3. 新增两层 Pi tree model/API：`PiTreeNode` 保留全树和 children，`PiViewLocation { entryId, lineage }` 是纯只读的查看位置。branch picker 默认列出无 children 的 terminal nodes，也可在树浏览器中“查看至此节点”；两者均允许 metadata 成为 node/tip。会话视图显示“Pi 默认恢复位置”或“仅查看至此节点”，而非笼统的 `active branch`；选择不写入 Pi、不改变任何终端中的 in-memory leaf，也不承诺外部 Pi 重开会记住选择。
4. `message` 映射：
   - user 的 string 或 text/image contents → user Msg；图像只保留显示安全的本地 data/placeholder 策略，尺寸超限时不内联；
   - assistant text → text、thinking → thinking、toolCall → tool_use，保留 toolCall id/name/arguments，以及 `stopReason`、可选 `rawStopReason` 和 `errorMessage`。`aborted` assistant 即使没有最终 text，也须显示为已中止的历史 turn；不能因它有 `errorMessage` 或没有 `rawStopReason` 而丢弃 thinking/usage，亦不能把它伪装为 provider `error`；
   - toolResult 按 toolCallId 生成 tool_result，保留 isError 和**原始顺序**的多个 image/text block。缺失、重复或不匹配的 toolCallId 保留为未关联的诊断结果，绝不按相邻位置猜配；image 应走与用户图片相同的 MIME/解码/尺寸上限和 placeholder 策略。`details` 视为不透明、可选、形状不稳定的诊断 metadata（例如 MCP `server/tool`、error、diff/patch），仅做大小受限的安全键值展示或折叠；原始工具文字、截图数据和 details 不进入全文搜索、计费或调试日志，绝不因 tool name、details 或 arguments 执行、信任或走 `chrome-devtools_*` / `dart_*` 等特殊分支；
   - `bashExecution` 是独立的 `message.role`，映射为工具执行（command、output、exitCode/cancelled/truncated），避免丢掉 Pi 的 `!`/bash 记录；它既不应冒充 assistant text，也不应被要求拥有 `toolCallId` 或相邻 assistant；
   - custom_message 仅在 `display=true` 时渲染为明确的 extension/system message；`display=false` 只作为不可见 context metadata 计数，绝不将其正文放入普通 transcript、全文搜索或 Markdown/HTML；custom、label、model_change、thinking_level_change、session_info 作为 metadata，不冒充用户或 assistant；
   - compaction 与 branch_summary 显示低调摘要卡，不能把摘要再次算为用户消息。
5. 上述 parser 还需处理 extension 自定义 content、未知 role、非数组 content、半写入 JSONL 尾行和 future type；所有未知数据均安全忽略或降级为无执行能力的文本。
6. 搜索必须索引**全树**用户文本和 title/ID/path。命中附带 Pi `entryId`、物理 ordinal 与可到达的 terminal leaf ids，不能使用现有只携带 session path 的 `SearchHit`。打开时优先选当前 Pi 默认 leaf（若该 path 包含命中）；否则选择其后代中物理 ordinal 最大的 terminal leaf，或让用户明确选 branch。不得由 timestamp/文本相似度猜路径；命中 user message 时展示包含该 message 的 lineage，不能模拟 native `/tree` 的“leaf 后退并回填编辑器”。

### 阶段 3：重命名、单文件回收站、恢复与目录安全

1. `SessionStorageKind::File`：Pi 删除和恢复移动单一 JSONL，不存在 Kimi/Codex 那种全局 index、SQLite 或目录旁车需要同步。通用回收站已支持 file storage，但 Pi source 必须给出正确 `SessionStorageUnit` 和 `trash_title`。
2. `validate_restore_target` 仅允许恢复到当前解析的 Pi session root 之下的普通 `.jsonl`，对绝对/相对 metadata、软链、重复目的地、非 Pi header 和 custom root 做防御校验。
3. Pi 自带 picker 在有 `trash` CLI 时也会使用系统回收站；本应用移动到自己的 session-viewer trash 与 Pi 原生行为不同，但都是可恢复路径。UI/文档要说明两种回收站互不互相列出，绝不永久删除原生系统回收站项目。
4. 重命名不改文件名、header、用户正文或历史 `session_info`。追加一条 Pi v3 兼容 session_info：collision-checked 唯一 8-hex entry id、parentId=写前复读到的最后完整有效 non-header entry、ISO timestamp、去除 CR/LF 的 name；通过 append-only 写入，追加前后校验文件变化/冲突。空名称应追加显式清名而非拒绝。标题是全文件状态，故该 entry 即使最终落在非 Viewer 所选 branch，Pi 重载仍会采用它。
5. 活跃 Pi 进程也可能持有与磁盘默认 leaf 不同的**内存** leaf；Pi 没有跨进程 session lock。本应用已知的内嵌 Pi/tab 活跃时禁止 rename/delete/restore。对外部 Pi 仅能以 mtime 稳定窗口、写前 re-stat/reparse、单行完整 append 和失败重试做 best-effort，不能承诺消除 race：Pi 随后可能从旧内存 leaf append 成 sibling，或继续写入已被移走的 inode。发生风险时不修改内存 title，并明确提示用户先退出外部 Pi；严禁 temp rewrite 覆盖增长中的 JSONL。
6. 对 soft delete、restore、permanent delete、批量删除、已打开 tab/view/export-history 的清理以及外部 Pi 同时占用文件的恢复策略建回归测试。被判定树结构不安全（重复 id/环/无有效 header）的文件仍可只读导出/诊断，但不允许重命名或按 entry 定位的写操作。

### 阶段 4：全树统计、成本来源与托盘

1. `read_turns` 的统计路径扫描全树全部唯一 persisted entry id，而会话 UI 只读 selected lineage。这保证 `/tree` 放弃的分支、`/fork` 前的实际调用和 compact/branch summary 的真实消耗不会从账目消失；同一 entry 因属于多个 lineage 也只能计一次。`retainedTail` 只是 compaction 内的 materialized context 副本，永不作为另一个 usage/call 计入。
2. 每个 assistant message 的 `usage.input`、`output`、`cacheRead`、`cacheWrite`、可选 `reasoning`、`totalTokens` 都按 entry id 仅记一次；这包括只有 thinking 的 `aborted` assistant。`totalTokens` 不与组成字段相加；它仅作展示/一致性校验。无 usage 的 assistant 仍可计消息/工具，不计 token/cost；toolResult（包括有 image/details 的结果）没有独立 usage 时也不得从相邻 assistant 复制或推算用量。
3. compaction、branch_summary 自己可携带生成摘要的 usage，作为独立 LLM call 统计；toolResult 的可选 nested usage 只有在与父 assistant usage 不同且有稳定 toolCallId/entry id 时才单独入账，防止重复计算。未知/未来 compaction 的 `retainedTail` 内 assistant usage 一律不扫描；它可能与 tree 中 persisted assistant 相同，也可能只是 checkpoint payload，缺少可安全去重的独立调用身份。
4. `usage.cost.total` 为有限且非负数时，优先作为 `cost_usd`，并在内部 `cost_source='pi_recorded'` 标记。样本已经证明该字段存在且非零。成本分项可用于校验，但不能与 total 叠加。
5. `usage.cost.total` 缺失、无效或 provider 明确为订阅/自定义且 Pi 记录为不可用时：仍统计 token/call，尝试现有严格 provider/model 定价；只有严格命中才估算美元，否则增加 `unpricedCallCount`。绝不使用 Claude/Codex/Grok 平均价或把 Pi 本身当 provider。
6. 扩展 `CallRecord`/aggregator/UI 文案以区分 `pi_recorded`、strict catalog、official estimate、unpriced 四类成本来源。避免现有“官方价格估算”提示把 Pi 记录成本误描述为外部账单。
7. `stats/stream.rs` 增加 `pi` 的 all / single / `all:<visible agents>` scope；`settings.readStatsScope()`、StatsView 的 `asAgent`、scope label 与 tests 同步更新。
8. 将 Pi 加入 `stats/tray.rs::TRAY_AGENT_NAMES`、enabled 过滤、名字、品牌色和测试，保证 Pi 被启用时在 Today/7d/30d 显示，即使没有本月调用。
9. 不为 `PricingView` 新增 `pi` family；如模型本身严格命中已存在 provider family，可按其 provider/model 归类。Pi 只是运行该模型的 agent，独立 family 会掩盖多 provider 真相。

### 阶段 5：前端入口、持久化、导出与故意排除

1. 更新 Agent 联合、`AGENT_META`、agent icon/label、`ALL_AGENTS`、enabled-agent defaults/migration/reset、`LaunchArgs` 的读写/重置、export history 的 `VALID_AGENTS` 和 Stats scope 的 localStorage 校验。旧 localStorage 必须自动补 `{ pi: ... }`，不会在升级后使 settings 页面崩溃。
2. 侧栏、Welcome、NewMenu、全局搜索、回收站、Top Sessions、通知标题、view history、terminal persistence 走现有通用 Agent 路径；为 Pi 加品牌图标并使 HTML export assistant avatar 不是 Claude 冒充。
3. NewMenu 仅显示“新建会话（TUI）”和 terminal；quick open 设置为 chat 时在 Pi 上明确提示不支持或降级为 TUI。不得让 `chatSessions.ts` / `agent_chat.rs` / Composer / PermissionPrompt / model picker 尝试创建 Pi GUI session。
4. Markdown/HTML 导出显示当前 `PiViewLocation`、leaf/entry id 和“历史 lineage（非 native effective context）”说明；不应无提示地把别的分支拼接进去，也不导出 `display=false` custom message 正文。JSON 导出新增 Pi v3 envelope，保留 header、all entries、selected leaf、schema version 与 renderer version，做到树结构可重放；通用 `Msg[]` JSON 不能被宣传为 Pi session 的无损备份。
5. 继续使用应用自身导出，不调用 Pi `/export`；更不调用 `/share`（会上传 private GitHub gist，属于额外外部写入）。
6. Pi 没有单一 OAuth account quota。`usage.ts` / `usage_api.rs` 固定为 Anthropic API，Pi 会话上不显示 Claude 订阅额度，也不读取 Pi 的 `auth.json`。

### 阶段 6：CLI 检查、内嵌终端与外部终端

1. 在 `cli_env.rs` 增加 `CliSpec { name: 'pi', binary: 'pi', npm_package: '@earendil-works/pi-coding-agent' }`。版本检测以 `pi --version`，latest 可走 npm registry，但要区分版本信息和可自动升级授权。
2. CLI 环境诊断显示登录 shell 实际解析的 pi 路径、重复安装、版本、`PI_CODING_AGENT_DIR` / session root 的**路径级**健康检查以及 `pi --help` 支持关键参数；不打印 auth、provider key 或 settings 内可能敏感的 proxy/credential 内容。
3. 使用 `pi update`/`pi update pi` 可能更新资源或产生网络/交互副作用；本次 CLI UI 不自动运行它，`upgrade_all_clis` 也跳过 Pi。提供官方安装/手动升级文档链接；安装动作必须沿用官方 `npm install -g --ignore-scripts @earendil-works/pi-coding-agent` 的安全前提或仅提供手动入口。
4. 更新 `src/types.ts` 的 CLI 联合类型、`CliEnvironmentCheck.vue` 的 label、icon、官方 URL 与所有 diagnosis/install/upgrade store key。Pi 卡片支持刷新与诊断；不显示一键升级。
5. `resume_command` 为 `AgentCommand::new('pi').arg('--session').arg(absolute_jsonl_path)`；新建为 `AgentCommand::new('pi')`。两者均允许当前每-agent extra args，但须按现有 POSIX/PowerShell quote 规则处理。恢复命令不传 Viewer leaf，因为 Pi CLI 没有相应参数：若用户当前在非默认 `PiViewLocation`，launch UI 先显示“Pi 将按 JSONL 最后 entry 恢复”的确认/提示。
6. 内嵌 PTY 走通用流程，不复制 Codex 专属 Windows 粘贴/编辑器颜色/retry 逻辑；增加 macOS/Linux/Windows 的 new/resume、包含空格路径、额外参数和未安装 CLI 测试。
7. 外部 terminal 同走 `new_session` / `resume_session`。验证 cwd 设为 Pi header.cwd、绝对 session path 恢复正确、`PI_CODING_AGENT_DIR` 环境继承一致，且 UI 不把 `--continue` 错当指定 session 恢复。

### 阶段 7：Pi extension 状态 relay、桌面宠物与 hooks 设置

1. Pi 没有 Kimi/Claude 风格静态 hooks。改为生成 app 托管的 global extension，例如 `~/.pi/agent/extensions/cc-sessions-viewer-turn-status.ts`，并原子合并 `~/.pi/agent/settings.json.extensions`，仅添加/更新本应用的绝对 extension path，保留未知 JSON 字段、packages、skills、prompts、themes 和用户 entries。
2. 扩展直接写应用既有 turn-signal JSONL，不经 stdin hook script。每条 payload 至少带 `{ agent:'pi', path: sessionFile, sessionId, cwd, state, source:'hook' }`；无 persistent session file（`--no-session`）时不发可点击任务。
3. lifecycle 映射：`before_agent_start`/`agent_start → started`；`agent_end` 检查最终 assistant `stopReason`，`error → failed`、`aborted → completed`（可在历史 UI 标为“已中止”）；**不可只按非空 `errorMessage` 判失败**，真实 `aborted` entry 同样会带该字段；`agent_settled → completed` 仅在本轮未失败时发出。选择 `agent_settled` 而非每次 `turn_end`，防止自动 retry/compact/follow-up 中途闪成 completed。
4. Pi 核心无 permission popup，任意第三方 extension 的 tool gate 也没有统一稳定 payload，因此本期不承诺 `blocked`。状态 UI 要显示 Pi 只提供 started/completed/failed；后续若 Pi 官方增加 permission lifecycle 再扩展。
5. `turn.rs` 的 agent allowlist、signal 读取/验证、`TurnHookInstallResult`、`TurnHookStatus`、all-installed 判断、`api.ts` 类型、turnHookStatus store、Settings hooks 卡片和四语描述均加入 Pi。检测条件是 settings 引用 + extension 内容/marker + 当前 signal path 都匹配；`--no-extensions` 或用户禁用时如实显示未启用。
6. 安装、更新、禁用/卸载保持幂等，配置写入前后做并发版本检查；extension 捕获所有写入错误，永远不阻塞 Pi。测试 global settings 空/已有 arrays/重复条目/用户禁用/坏 JSON/多 app 实例/ephemeral session。
7. Pi 运行状态进入既有 tab state、live notification、desktop pet task。`App.vue::openDesktopPetSession` 的独立 allowlist 改为 capability/统一 source 校验，使 Pi JSONL path 可被 `turn::resolve_desktop_pet_session` 找回。Pi 不需要 Codex asar 宠物素材。

### 阶段 8：worktree、文档和交付

1. 在真实临时 git worktree 中启动 Pi，确认 header.cwd 是 worktree 路径。只有验证成功，才将 Pi 设为 `worktree=true` 并同步加入前端 `App.vue::WORKTREE_AGENTS` 和 Rust `agent_supports_worktrees()`；两端缺一都会导致项目树或删除流程不一致。
2. worktree 删除前停止 Pi TUI/terminal tab，清理该 cwd 下的 Pi JSONL（当前产品的 worktree 删除是 hard delete，需在确认框精确计数并不误称可恢复），随后再移除物理 worktree。不要扫描/删除同一目录名但 header.cwd 不匹配的 session。
3. 更新 `README.md`、`README.zh-CN.md`、`README.ja.md` 的 supported CLI 数量、功能表和 Pi 限制；更新 en/zh/zh-TW/ja 的 stats scope、hooks 描述、“four/five CLI”固定文字和 Pi 相关错误/设置文案。
4. 增加开发者文档：session root precedence、v1-v3 tree 算法、cost source 语义、app-managed extension 安全模型、fixtures、兼容性策略和手动回归命令。

## 与现有 agent 关联的遗漏清单

| 关联点 | 当前代码的独立耦合 | Pi 计划要求 |
| --- | --- | --- |
| Agent 类型与持久化 | `Agent` union、`ALL_AGENTS`、`LaunchArgs`、enabled/default/reset、statsScope、export history | 全部加 Pi 和旧 localStorage migration。 |
| Stats | `stats/stream.rs` scope 白名单、StatsView `asAgent` | 支持 pi/all/visible scope，且区分 branch display 与全树计费。 |
| Tray | `TRAY_AGENT_NAMES` 和 macOS card 色/名称 | 加 Pi、可见性同步和空活动卡。 |
| Worktree | `WORKTREE_AGENTS` + `agent_supports_worktrees()` | 先实测 cwd，再同步维护两份 allowlist。 |
| 状态 | `turn.rs` 固定四 agent schema + CJS relay | 使用 Pi extension relay，补 Rust/API/Settings/UI/locale。 |
| Desktop pet | `openDesktopPetSession` 独立 agent allowlist | 让 Pi file session 可打开。 |
| CLI | `CliSpec`、前端 CLI unions、labels/URLs、upgrade all | 诊断 Pi；禁止自动 `pi update`。 |
| Export | 通用 `Msg[]` 是线性、HTML avatar 分支 | branch-aware HTML/MD + full-tree Pi JSON。 |
| Pricing | `PricingView` 固定 provider family | 不新增 Pi family；使用 Pi recorded cost/strict provider catalog。 |
| OAuth quota | `usage_api.rs` 固定 Anthropic | Pi 显式排除，不读 auth。 |
| GUI Chat | Claude/Codex 专属进程和协议 | Pi `guiChat=false`，不接入。 |

## 测试、手动回归与完成定义

### 自动化测试

- Rust：root precedence、path containment、v1/v2/v3 header、title/session_info、**物理最后 entry** active leaf、multi-branch traversal、user/custom `/tree` 目标与 viewer-only location 的差异、metadata leaf、cycle/dangling parent/duplicate id/self-parent、message/tool/bash/custom/compaction parsing、高密度 tool-loop（不假设角色交替、`toolCallId` 配对、顶层 bashExecution）、多 block image/text toolResult、可变 details、aborted assistant 的 `errorMessage`/thinking/usage、`firstKeptEntryId`/`retainedTail` 版本化 fixture、active-file append snapshot/retry、tree search hit 定位、append-only rename、file trash/restore、full-tree usage/cost de-dup（含 retainedTail 不重复）、strict fallback、stats stream、tray、worktree、Pi extension settings merge/status/signal lifecycle。
- TypeScript/Vitest：Agent exhaustive maps、settings migration/reset、new-menu 不显示 GUI Chat、CLI card 无自动 upgrade、Stats scope、export history/avatar、branch selector/search locator、非默认 Viewer branch 启动 terminal 的提示、hidden custom message 不进常规导出、Pi full-tree JSON export、desktop-pet open target。
- 回归：全部 Claude/Codex/Kimi/Grok/agy/opencode 测试、TypeScript typecheck、Rust tests、前端 build 通过。

### 手动回归

1. 用 `PI_CODING_AGENT_DIR` 和 `PI_CODING_AGENT_SESSION_DIR` 指向临时 fixture root，检查发现、项目归属、标题、空 session、损坏/软链文件和全局搜索。
2. 打开线性、多 branch、metadata-tip 与 user-message search-hit fixture：默认显示物理最后完整 entry 的 parent chain；选择其他 leaf/中间节点只切换 viewer 内容。随后重开原生 Pi，验证它仍取文件物理最后 entry，而不会读取 Viewer 的临时选择；选择 user/custom message 时额外验证原生 `/tree` 会回填编辑器而 Viewer 不会伪造该行为。
3. 以 `firstKeptEntryId` 与 `retainedTail` compaction fixture 分别核对：历史视图完整、有效 context 仅作版本化诊断、retainedTail 不出现在普通搜索/导出且不增加调用/费用。
4. 执行 rename、soft delete、restore、permanent delete；在原生 `pi --session <absolute path>` 中验证名称/恢复，且并发运行 Pi 时不会重写其他 entries。外部 Pi mtime 持续变化、内嵌 Pi 运行、重复 id/环文件均必须禁用写操作并给出原因。
5. 手工汇总 assistant/summary usage 与 Statistics/Tray 比较，确认 Pi `cost.total` 不被二次加价、未定价 provider 不产生虚假美元、放弃 branch 的真实花费仍统计。
6. 新建/恢复 Pi 于内嵌和外部 terminal，验证 cwd、特殊路径、extra args、缺失 CLI 和延迟落盘；从非默认 Viewer location 启动时确认提示和实际恢复位置一致。
7. 打开正在连续追加的高密度工具会话，验证刷新最终收敛为同一完整 snapshot，不出现缺少 toolResult、重复 usage 或搜索/导出与会话视图不同 branch 的中间状态。安装 extension 后覆盖正常完成、provider error、abort（即使 entry 有 `errorMessage`）、自动 retry/follow-up、`--no-extensions`、`--no-session`，确认 tab/notification/pet 状态及不阻塞 Pi。
8. 在 git worktree 中创建 Pi session，验证项目分组、删除确认计数、tab 停止与文件清理。

### 完成定义

- Pi 会话可被安全发现、按真实 cwd 分组、以原生默认 leaf（物理最后完整 entry）正确阅读，并可只读查看/搜索/导出其他 branch 或中间节点；不会把 Viewer 的选择伪装成 Pi 已持久化的运行位置或线性历史。
- 所有已发生的 Pi 调用在全树统计中至多记一次，Pi 记录成本、严格模型价格、估算和未知成本在 UI 中可区分。
- 重命名、回收站、终端、状态 extension、托盘、宠物、worktree、持久化、文档及本地化均通过对应测试。
- 不读取凭证、不上传 session、不自动升级 Pi、不把 Pi 的多 provider 模型或第三方 extensions 误表示为本应用能完整控制的能力。
