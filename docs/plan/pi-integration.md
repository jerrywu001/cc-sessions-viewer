# Pi Agent 接入开发计划

## 目标、结论与边界

将 Pi (`pi`) 接入 Sessions Viewer，作为新的本地 CLI agent，支持 Pi 会话的发现、阅读、树分支浏览、搜索、重命名、回收站恢复、统计计费、内嵌/外部终端、CLI 检查、运行状态、工作树、托盘、桌面宠物和文档。

沿用本轮既定范围：**不接入 GUI Chat**。Pi 虽提供 JSON/RPC mode，但本次不启动 `agent_chat`、不实现 Composer、模型/思考等级控件、图片附件、权限对话或 RPC 交互。新建和恢复均在原生 Pi TUI 中完成。

Pi 不应被当作单一模型供应商。它是可接入多 provider 的 harness：本机样本实际混有 `deepseek` 与 `github-copilot` provider。因此它有统计和已记录成本能力，但不新增一个伪造的“Pi 模型价格表 family”。

## 当前实现状态（2026-08-22）

- Pi 的基础接入已经可用：会话发现、默认/terminal branch lineage 阅读、消息/工具/图片/skill 渲染、重命名、回收站、统计、CLI/终端和状态 relay 已接入，并通过 Rust 全量测试与前端构建。
- 阶段 0、1、2、3、4、5、6、7、8 的主要代码路径已完成；阶段 2 仍保留“无独立树画布、搜索按会话聚合”的产品边界。不能把这些边界说成无损的 entry 级浏览器。具体状态见下方“当前未完成与已知偏差”。
- 阶段 6 已覆盖 Pi CLI 检查、版本检测/升级、绝对 JSONL path 恢复、header.cwd 校验、内嵌/外部 terminal 和 per-agent extra args。Pi 安装入口使用官方 curl installer（该 installer 最终仍安装 npm 包）；升级按实际二进制路径选择同前缀 npm/bun/brew，非这些包管理器时回退官方 `pi update self`。
- 阶段 7 已接入 app-managed global extension、原子 settings 合并、生命周期 relay、tab/notification/desktop pet 状态和 Settings hooks 状态卡片。`--no-session` 不产生可点击任务，extension 错误不会阻塞 Pi。
- 阶段 8 已完成真实临时 git worktree 启动、header.cwd 校验、session 清理和 worktree 移除验证；Pi 的 `worktree` capability 已与 Rust/App 实现统一。
- 会话列表 token 角标已接通 Pi persisted usage：后端汇总 JSONL 中 assistant、compaction 和 branch summary 的 usage；前端修复 IntersectionObserver 初始化时序，确保可见卡片实际发起 `session_usage` 请求。
- 已对默认 Pi session root 做成本对账：当前 4 个 JSONL、236 次 assistant 调用的 `usage.cost.total` 合计为 `$0.20186679`，与 input/output/cacheRead 分项之和一致。统计页显示的是 Pi 写入的 API 成本，不是 provider 最终账单或人民币消费；若外部账单更高，应继续向 provider 账单或未被默认 session root 发现的会话核对。
- 明确不在本轮范围：Pi GUI Chat/RPC、Pi `/share` 上传、Pi account quota、第三方 extension 协议和伪 sub-agent/permission gate。

### 当前未完成与已知偏差

以下项目是本次核对后确认仍有边界的功能，不能在发布说明中夸大为 entry 级完整实现：

| 项目 | 当前实际状态 | 影响 |
| --- | --- | --- |
| Viewer 全树/分支浏览 | 已提供只读 `session_tree` Tauri API，返回 entry、parent、children、kind、timestamp、ordinal、terminal；Pi 详情页对多个 terminal leaf 提供 branch picker，选中 lineage 保存在 view tab/localStorage。仍没有独立树状画布，也未暴露 v1 synthetic id 作为持久位置。 | Viewer 可以在页面内切换 B/C 等终端分支；切换只读 JSONL，不写 Pi 文件。`/tree` 仍需在 Pi 终端测试。 |
| Pi 全树搜索 | Pi 搜索会枚举 `session_tree` 的 terminal leaf，按各自 lineage 扫描用户正文；`SearchHit` 携带命中 leaf id，Viewer 打开时恢复该 branch。 | 已能命中放弃分支并定位到对应 lineage；仍未提供 entry 级多命中列表，搜索结果仍按会话聚合。 |
| 分支感知导出 | Markdown/HTML 使用当前 tab 选中的 lineage；Pi JSON 使用稳定快照生成 `cc-session-viewer-pi-export` 包络，包含 header、全部有效 entries、selected leaf、Pi schema 和 renderer version。 | 已能导出当前分支的 Markdown/HTML，以及可审计的 Pi v3 full-tree JSON；JSON 是 Viewer 包络，不是可直接交给 Pi 的原始 JSONL 文件。 |
| 非默认分支恢复提示 | 选中非默认 terminal leaf 后点击恢复，会先弹窗说明 Pi CLI 只能从文件物理最后 entry 恢复；确认后才启动终端。 | 不会把 Viewer 临时 leaf 误传给 Pi `--session`，用户能明确知道恢复位置会变化。 |
| capability 声明 | `AGENT_META.pi` 已统一为 `worktree=true`、`hooks=true`，与 Rust/App 的 worktree 和 app-managed extension relay 实现一致。 | 设置和入口不再误报 Pi 不支持这些能力；Pi hooks 仍是 extension relay，不是 Claude 风格静态 hook 文件。 |
| 分支/导出相关测试 | 已有 parser fixture 和默认 leaf 测试，但没有前端 branch selector、全树 SearchHit、Pi full-tree JSON export 的实现与测试。 | 阶段 2/5 的完成定义尚不能声称全部满足。 |

### 已验证可用范围

- 可在 Viewer 中发现 Pi JSONL，按 header.cwd 分组，读取默认物理最后完整 entry 的 lineage。
- Pi assistant/toolResult/bashExecution、失败工具、diff、图片、skill 注入正文隐藏和用户追加内容均有解析或回归覆盖。
- 可执行 Pi session rename、soft delete/restore、CLI 检查、内嵌/外部 terminal、统计和 Pi recorded cost 展示。
- Pi `/tree` 可以在原生终端中创建同文件分支；Viewer 当前只读取最终默认分支，不提供树状页面浏览。

### 后续实现顺序

1. 先补 Pi tree read API（全树节点、terminal leaf、`PiViewLocation`）和会话内 branch picker；这是搜索、导出和恢复提示的共同基础。
2. 再让搜索结果携带 Pi entry/leaf 定位，并让 Viewer 按命中 lineage 打开会话。
3. 接入 branch-aware Markdown/HTML 与 Pi v3 full-tree JSON 导出，明确区分历史 lineage 和 native effective context。
4. 最后统一 `AGENT_META` 的 `worktree`/`hooks` capability、Settings 展示和对应回归测试。

## 已完成的真实数据与文档调研

### 本机安装与数据

| 项目 | 实测结果 |
| --- | --- |
| 二进制 | `/Users/wuchao/.nvm/versions/node/v22.21.1/bin/pi`（官方 curl installer 选择 nvm 前缀后生成的 npm 全局包 `dist/cli.js` 链接） |
| 版本 | `0.84.2`（当前本机验证版本） |
| 默认 agent 根 | `~/.pi/agent`；本机未设置 `PI_CODING_AGENT_DIR` / `PI_CODING_AGENT_SESSION_DIR`，全局 settings 也未设置 `sessionDir`。 |
| 本机会话 | 4 条：2 条位于 `~/.pi/agent/sessions/--Users-wuchao-apps-claude-session-viewer--/`，另有 1 条位于 `~/.pi/agent/sessions/--Users-wuchao-develop-flutter-sales-app--/`、1 条位于 `~/.pi/agent/sessions/--Users-wuchao-develop-flutter-sales-dev-app--/`；每条都是一个 `.jsonl` 文件。 |
| 实例版本 | 三条 header 均为 session format v3，均无 `parentSession`，均为线性树（无一父多子节点）。 |
| 较长样本 | 62 行：11 user、28 assistant、19 toolResult，调用了 `bash` / `edit` / `read` / `write`。 |
| 新增工具密集样本 | 这是一个分析期间仍在追加的活动会话：首次观察为 214 行，本次最后一致快照为 333 行（5 user、162 assistant、162 toolResult、1 `bashExecution`，以及各 1 条 `model_change` / `thinking_level_change`）；之后 3 秒未再变化。树仍线性且物理相邻 `parentId` 连续，但它证明一轮中可以连续出现大量 `assistant(toolUse) → toolResult`，完全不满足“user 与 assistant 交替”的假设。最终快照有 162 个 toolCall 与 162 个均带 `toolCallId` 的 toolResult；关联必须按 ID，不能按相邻顺序猜测。实际调用过 `bash`、Dart 和 `chrome-devtools_*` 等外部/MCP 工具。 |
| 内容与用量 | assistant 记录含 text、thinking、toolCall；`usage` 有 input/output/cacheRead/cacheWrite/totalTokens，部分含 reasoning；每次 assistant 都保存成本分项与 `cost.total`。 |
| 中止与工具结果 | 新样本有 1 条 `stopReason='aborted'` 的 assistant：只有 thinking、仍有 persisted usage，并有非空 `errorMessage`，但没有 `rawStopReason`。另有 `toolResult` 按顺序混合多个 text 与 image block（截图结果），其 `details` 是不固定的 object；实际见过 `server/tool`、`error/server`、diff/patch 以及空 object。toolResult 均未带 usage。 |
| 样本成本 | 目前默认 session root 下 4 条 session 的 persisted assistant `cost.total` 合计为 `$0.20186679`；统计页直接使用该字段，不用猜测价格替换。 |
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
| `/tree` 的可选项 | 原生树包含所有 entry 类型；默认 UI 只是隐藏 label/custom/model/thinking/session_info 等 bookkeeping。 | Pi parser 会保留并验证全树 entry，但当前没有 `PiTreeNode` API、branch picker 或 metadata filter；前端只能读取默认 leaf。 |
| 选择 user/custom message | `navigateTree()` 会把运行中 leaf 设为该 entry 的 `parentId`（根则为 `null`），把原 content 放回编辑器；用户下一次提交才从 parent 创建 child。 | Viewer 当前没有树节点选择，不模拟 native 的编辑器回填/leaf 后退，也不写入文件。 |
| 选择其他 entry | assistant、toolResult、bashExecution、compaction、branch_summary、custom、label、model/thinking change、session_info 等都可作为 native leaf。 | Viewer 当前没有按 entry 查看位置；默认读取仍从文件最后一个完整 non-header entry 回溯。 |
| 导航是否写盘 | 无摘要时仅调用 `branch()`/`resetLeaf()`，只改内存；退出后丢失。若用户选择摘要，Pi 在新的运行 leaf 追加 `branch_summary`。 | Viewer 没有 branch selector，也不会调用或仿造 `branch()`；重命名、删除、恢复仍与原生 `/tree` 分离。 |
| 重新打开已选分支 | `pi --session <file>` 不接受 tree entry id；重载后仍回到物理最后 entry。`/fork`/`/clone` 会新建 JSONL。 | Viewer 当前没有临时 branch location，因此也没有对应恢复提示；终端恢复始终按 Pi 默认 leaf。 |
| label 与标题 | `labelsById` 和 session name 都从**全文件**顺序解析最新状态：label entry 可挂在任意 branch，却作用于任意 target；最新 `session_info` 也可以不在当前路径。它们不是 branch-local 状态。 | 标签显示为“会话共享标注”，标题为“会话级标题”；不得把它们当 selected branch 的历史事实或据此推断 branch 名称。Viewer 本期不提供 Pi label 编辑。 |

历史路径与 Pi 实际喂给模型的 context 也必须分开：`getBranch()` 是 leaf 到 root 的完整 entry lineage；`buildContextEntries()` 会以路径上最后一个 compaction 为边界，省略已摘要的旧 entries。包内 `docs/session-format.md` 已描述新格式可带 `retainedTail`，而本机 0.84.2 runtime 的实现仍只使用 `firstKeptEntryId`；两种格式不能互相假定。主会话视图、搜索和 Markdown/HTML 导出使用 Viewer 当前选定 lineage；Pi full-tree JSON 另保留全部原始 entries，并标注 selected leaf。`retainedTail` 是已有 message 的物化 checkpoint，不是新请求、不可进入主 transcript/search、也不可重复计费。

v1 还没有 `id`/`parentId`：Pi 首次打开会随机生成 8-hex id、把线性关系写成 v2，再把 `hookMessage` 改为 v3 的 `custom`，并**重写原文件**。Viewer 对未迁移 v1 只能用 `v1:<physical-ordinal>` 作为本次响应内的临时查看 id，绝不可把它存成可恢复 terminal/搜索定位、也不可据它 append rename；文件 revision 改变后必须失效重读。v2 的 `hookMessage` 按 legacy custom 降级，不能误渲染为 user/assistant。这样既能只读历史，也不会与 Pi 的随机迁移 id 竞争。

## 最终功能范围

| 能力 | 设计结论 |
| --- | --- |
| 会话发现 | 扫描已解析的 Pi session roots 下的 JSONL，按 header.cwd 聚合项目；不依赖编码目录名作为 cwd 真值。 |
| 树状历史 | 读取完整扁平树并显示 terminal-branch picker；暂不提供独立树画布或 entry 级中间节点导航。 |
| 消息与工具 | 把当前选定 lineage 的 user/assistant/toolResult/built-in tool 归一为 `Msg[]`；搜索命中会携带 terminal leaf 定位。 |
| 删除、恢复、重命名 | 单文件回收站；重命名通过追加原生兼容的 `session_info` entry。 |
| 搜索与导出 | 搜索扫描所有 terminal lineage；Markdown/HTML 使用当前 branch；Pi JSON 导出 full-tree envelope（header、entries、selected leaf）。 |
| 统计计费 | 全树唯一 entries 的 persisted usage/cost，含 compaction/branch summary 的单独 usage；不把放弃分支的实际花费丢弃。 |
| 终端与 CLI | Pi 检查、诊断、内嵌/外部 terminal 新建及按绝对路径恢复。 |
| 状态与宠物 | 由 app 托管的 Pi global extension 写运行状态 relay；托盘和桌面宠物复用通用状态/会话解析。 |
| worktree | Rust/App 的 header.cwd 校验、分组和删除路径已接通，Pi capability 已声明为 `true`。 |
| 显式排除 | GUI Chat/RPC、Pi account quota、Pi `/share` 上传、任意第三方 extension 的自定义协议/伪 sub-agent。 |

## 能力声明与跨 agent 契约

前端新增 `Agent = 'pi'`，在 `AGENT_META` 声明：

```text
history=true, terminal=true, guiChat=false, worktree=true,
hooks=true, stats=true, pricing=true
```

说明：`hooks=true` 指 app-managed Pi global extension relay，不代表 Pi 存在 Claude 风格的静态 hook 文件。

`pricing=true` 仅表示 Pi persisted session usage 中可得到成本，**不表示** Pi 有自己的上游模型 catalog。PricingView 继续按实际 provider/model family（Claude/OpenAI/Grok/Gemini 等）组织参考价目；Pi session cost 首先来自其持久化 `usage.cost.total`，来源在 UI 中标记为“Pi recorded cost”。

将 Pi 排入统一的 agent 顺序（建议 Claude → Codex → Kimi → Pi → Grok → agy → opencode），并为新增用户选择明确默认可见策略。所有入口读取 capability，避免用新的 `agent === 'pi'` allowlist 取代通用逻辑。

后端新增 `src-tauri/src/agents/pi.rs` 实现 `SessionSource`，并只在 `agents/mod.rs::source()` 加 `"pi"` 分派。解析、路径、树和 Pi 特化的计费逻辑都应留在该模块，不能扩散到 `lib.rs`、`trash.rs` 或前端。

## 分阶段开发计划

### 阶段 0：fixture、版本兼容和 agent 影响面基线（已完成）

1. 从本机三条 session 生成**脱敏** fixtures：v3 linear、assistant thinking/toolCall、toolResult、model/thinking changes 和 persisted usage/cost。新增一条高密度 tool-loop fixture（少量 user、连续大量 `assistant(toolUse) → toolResult`、顶层 `message.role='bashExecution'`、Dart/MCP 工具名、toolCall/toolResult 通过 `toolCallId` 配对而非位置配对、toolResult 的多个有序 text/image block、不同形状的 `details`、isError），以及一条含 `aborted + errorMessage + thinking + usage` 的 assistant。另手造 Pi 官方 schema 覆盖的 branched tree、session_info（含清空名）、跨 branch label、compaction（0.84 的 `firstKeptEntryId` 与文档的 `retainedTail`）、branch_summary、custom/custom_message、bashExecution、v1 无 id 的线性记录、v2 `hookMessage`、坏 JSON、dangling parent、循环/self-parent、重复 id、乱序 timestamp 与“物理最后 entry 非 assistant”的场景。
2. 固定支持 session versions 1–3：v1 linear 使用仅本次读取有效的 physical-ordinal synthetic id，并禁用 rename/持久 terminal leaf；v2 `hookMessage` 降级为 custom；v3 走原始 tree parser。Pi 原生迁移 v1 会随机生成 id 并重写文件，所以 viewer revision 变化必须清除旧 location/cache。超出已知版本以安全降级读取 header/可识别 messages，并在开发诊断中可见。
3. 建立新 agent 穷举清单：`src/types.ts`、`agentMeta.ts`、icons、settings、export history、StatsView；Rust `agents/mod.rs`、stats stream、tray、worktree、turn hook；CLI type unions/UI；四语 locale 和三种 README。
4. 记录 data-root precedence 与解析结果的只读诊断。不得读 `auth.json`，不得把 `models.json`、`models-store.json`、`trust.json` 的敏感字段复制到应用日志或 UI。

### 阶段 1：会话根、发现、项目聚合和元数据（已完成）

1. 实现 `pi_agent_dir()`：非空 `PI_CODING_AGENT_DIR` 优先，否则 `~/.pi/agent`。实现 `pi_session_root()`：非空 `PI_CODING_AGENT_SESSION_DIR` 优先，否则读取 agent 根的 `settings.json.sessionDir`（按 Pi 文档解析 `~`、绝对/相对路径），最终回退 `agent_dir/sessions`。
2. 只递归扫描解析后的 root 内常规 `.jsonl` 文件；拒绝 symlink、path traversal、root 外 canonical path 和超出资源上限的大文件。`--session-dir` 的一次性外部覆盖因为没有持久索引，文档中明确为“不可自动发现”。
3. 对每个文件只先读取 header：要求 `type='session'`、合法 id/timestamp、非空 cwd；以 header.cwd 正规化后作为 `ProjectInfo.dir_name` 和展示路径。编码目录名仅作候选/一致性诊断，不能替代 cwd。
4. `SessionMeta.id` 使用 header id，`path` 使用 JSONL 绝对路径，`fileName` 为文件名，created 使用 header timestamp，modified 使用文件 mtime。size 是单文件大小。
5. 标题优先最新 `session_info.name`（最新空名称意味着清除）→ Pi 默认恢复 lineage 的第一条用户可显示文本 → 文件名/日期。不得把 branch_summary/custom 内容误当标题。
6. `messageCount` 默认计 Pi 默认恢复 lineage 的可视 user/assistant message 数，同时暴露 `piBranchCount` 和 `piEntryCount`，使列表不会把“整棵树花费”误称为“当前对话条数”。
7. 新会话直到首个 assistant entry 才可能被 Pi 持久落盘；项目刷新沿用现有退避刷新机制，不能假定按下 `pi` 后立即有可扫描文件。
8. 对正在写入的会话，单次读取必须基于一次完整 bytes snapshot 建树；读取前后 file identity/size/mtime 改变时丢弃本轮结果并短暂退避重试。文件 watcher 对连续 append 做 debounce，半写入尾行只报“等待下一次刷新”，不得将不完整 `assistant/toolResult` 对展示、搜索或统计分别提交，从而制造相互矛盾的 session snapshot。

### 阶段 2：树解析、会话阅读、分支选择与搜索（主要功能完成）

1. 读完整 JSONL 后建立保留物理 ordinal 的 `id → entry` map，并先验证每个 parentId、循环、重复 id、无 header、错误时间和损坏行。v1 在 map 前生成仅本次读取有效的 synthetic ordinal id，且不暴露为可持久化 leaf；v2 `hookMessage` 作 legacy custom。半写入的尾行和无法解析/缺必要 id 的 entry 仅作为诊断跳过；dangling parent 的完整 entry 保留为带诊断的孤立 root（与 Pi `getTree()` 一致），其 lineage 在该点停止。发现环时中止回溯，绝不无限循环。重复 id/self-parent/环和孤立 root 均进入只读降级：不提供 entry-targeted action 或 append rename，避免与 Pi 的“Map 后写覆盖前写”实现产生不可预测的目标。
2. `read_session(path, leafId?)` 的缺省 leaf 为文件最后一个完整有效 non-header entry，匹配本机 Pi 0.84.2 的 `_buildIndex()`；从该 leaf 按 `parentId` 回溯至 root 后 reverse，即 Pi 重开后的 active lineage。`leafId` 只能是合法、已验证的非 header entry；显式非法 id 返回可诊断错误。`session_tree` 提供完整扁平树；Viewer 读取 Pi 会话时加载树并将所选 terminal leaf 保存到 view tab，刷新/重启后按该位置读取。搜索扫描所有 terminal lineage，导出使用当前 leaf 或 full-tree 包络。
3. 已实现 `read_session(path, leafId?)` 的后端 lineage 读取、默认物理 leaf 规则、`PiTreeNode` Tauri API、branch picker 和只读查看位置持久化；尚无独立树状画布。
4. `message` 映射：
   - user 的 string 或 text/image contents → user Msg；图像只保留显示安全的本地 data/placeholder 策略，尺寸超限时不内联；
   - assistant text → text、thinking → thinking、toolCall → tool_use，保留 toolCall id/name/arguments，以及 `stopReason`、可选 `rawStopReason` 和 `errorMessage`。`aborted` assistant 即使没有最终 text，也须显示为已中止的历史 turn；不能因它有 `errorMessage` 或没有 `rawStopReason` 而丢弃 thinking/usage，亦不能把它伪装为 provider `error`；
   - toolResult 按 toolCallId 生成 tool_result，保留 isError 和**原始顺序**的多个 image/text block。缺失、重复或不匹配的 toolCallId 保留为未关联的诊断结果，绝不按相邻位置猜配；image 应走与用户图片相同的 MIME/解码/尺寸上限和 placeholder 策略。`details` 视为不透明、可选、形状不稳定的诊断 metadata（例如 MCP `server/tool`、error、diff/patch），仅做大小受限的安全键值展示或折叠；原始工具文字、截图数据和 details 不进入全文搜索、计费或调试日志，绝不因 tool name、details 或 arguments 执行、信任或走 `chrome-devtools_*` / `dart_*` 等特殊分支；
   - `bashExecution` 是独立的 `message.role`，映射为工具执行（command、output、exitCode/cancelled/truncated），避免丢掉 Pi 的 `!`/bash 记录；它既不应冒充 assistant text，也不应被要求拥有 `toolCallId` 或相邻 assistant；
   - custom_message 仅在 `display=true` 时渲染为明确的 extension/system message；`display=false` 只作为不可见 context metadata 计数，绝不将其正文放入普通 transcript、全文搜索或 Markdown/HTML；custom、label、model_change、thinking_level_change、session_info 作为 metadata，不冒充用户或 assistant；
   - compaction 与 branch_summary 显示低调摘要卡，不能把摘要再次算为用户消息。
5. 上述 parser 还需处理 extension 自定义 content、未知 role、非数组 content、半写入 JSONL 尾行和 future type；所有未知数据均安全忽略或降级为无执行能力的文本。
6. 全树搜索已实现会话级 branch 定位：Pi 逐个 terminal leaf 读取 lineage，只扫描用户正文，`SearchHit.piLeafId` 让 Viewer 打开后恢复命中分支。当前仍只返回每个会话第一条命中，不提供 entry 级多结果列表。

### 阶段 3：重命名、单文件回收站、恢复与目录安全（已完成）

1. `SessionStorageKind::File`：Pi 删除和恢复移动单一 JSONL，不存在 Kimi/Codex 那种全局 index、SQLite 或目录旁车需要同步。通用回收站已支持 file storage，但 Pi source 必须给出正确 `SessionStorageUnit` 和 `trash_title`。
2. `validate_restore_target` 仅允许恢复到当前解析的 Pi session root 之下的普通 `.jsonl`，对绝对/相对 metadata、软链、重复目的地、非 Pi header 和 custom root 做防御校验。
3. Pi 自带 picker 在有 `trash` CLI 时也会使用系统回收站；本应用移动到自己的 session-viewer trash 与 Pi 原生行为不同，但都是可恢复路径。UI/文档要说明两种回收站互不互相列出，绝不永久删除原生系统回收站项目。
4. 重命名不改文件名、header、用户正文或历史 `session_info`。追加一条 Pi v3 兼容 session_info：collision-checked 唯一 8-hex entry id、parentId=写前复读到的最后完整有效 non-header entry、ISO timestamp、去除 CR/LF 的 name；通过 append-only 写入，追加前后校验文件变化/冲突。空名称应追加显式清名而非拒绝。标题是全文件状态，故该 entry 即使最终落在非 Viewer 所选 branch，Pi 重载仍会采用它。
5. 活跃 Pi 进程也可能持有与磁盘默认 leaf 不同的**内存** leaf；Pi 没有跨进程 session lock。本应用已知的内嵌 Pi/tab 活跃时禁止 rename/delete/restore。对外部 Pi 仅能以 mtime 稳定窗口、写前 re-stat/reparse、单行完整 append 和失败重试做 best-effort，不能承诺消除 race：Pi 随后可能从旧内存 leaf append 成 sibling，或继续写入已被移走的 inode。发生风险时不修改内存 title，并明确提示用户先退出外部 Pi；严禁 temp rewrite 覆盖增长中的 JSONL。
6. 对 soft delete、restore、permanent delete、批量删除、已打开 tab/view/export-history 的清理以及外部 Pi 同时占用文件的恢复策略建回归测试。被判定树结构不安全（重复 id/环/无有效 header）的文件仍可只读导出/诊断，但不允许重命名或按 entry 定位的写操作。

### 阶段 4：全树统计、成本来源与托盘（已完成）

1. `read_turns` 的统计路径扫描全树全部唯一 persisted entry id，而会话 UI 默认显示物理最后 lineage、也可切换 terminal branch。这保证 `/tree` 放弃的分支、`/fork` 前的实际调用和 compact/branch summary 的真实消耗不会从账目消失；同一 entry 因属于多个 lineage 也只能计一次。`retainedTail` 只是 compaction 内的 materialized context 副本，永不作为另一个 usage/call 计入。
2. 每个 assistant message 的 `usage.input`、`output`、`cacheRead`、`cacheWrite`、可选 `reasoning`、`totalTokens` 都按 entry id 仅记一次；这包括只有 thinking 的 `aborted` assistant。`totalTokens` 不与组成字段相加；它仅作展示/一致性校验。无 usage 的 assistant 仍可计消息/工具，不计 token/cost；toolResult（包括有 image/details 的结果）没有独立 usage 时也不得从相邻 assistant 复制或推算用量。
3. compaction、branch_summary 自己可携带生成摘要的 usage，作为独立 LLM call 统计；toolResult 的可选 nested usage 只有在与父 assistant usage 不同且有稳定 toolCallId/entry id 时才单独入账，防止重复计算。未知/未来 compaction 的 `retainedTail` 内 assistant usage 一律不扫描；它可能与 tree 中 persisted assistant 相同，也可能只是 checkpoint payload，缺少可安全去重的独立调用身份。
4. `usage.cost.total` 为有限且非负数时，优先作为 `cost_usd`，并在内部 `cost_source='pi_recorded'` 标记。样本已经证明该字段存在且非零。成本分项可用于校验，但不能与 total 叠加。
5. `usage.cost.total` 缺失、无效或 provider 明确为订阅/自定义且 Pi 记录为不可用时：仍统计 token/call，尝试现有严格 provider/model 定价；只有严格命中才估算美元，否则增加 `unpricedCallCount`。绝不使用 Claude/Codex/Grok 平均价或把 Pi 本身当 provider。
6. 扩展 `CallRecord`/aggregator/UI 文案以区分 `pi_recorded`、strict catalog、official estimate、unpriced 四类成本来源。避免现有“官方价格估算”提示把 Pi 记录成本误描述为外部账单。
7. `stats/stream.rs` 增加 `pi` 的 all / single / `all:<visible agents>` scope；`settings.readStatsScope()`、StatsView 的 `asAgent`、scope label 与 tests 同步更新。
8. 将 Pi 加入 `stats/tray.rs::TRAY_AGENT_NAMES`、enabled 过滤、名字、品牌色和测试，保证 Pi 被启用时在 Today/7d/30d 显示，即使没有本月调用。
9. 不为 `PricingView` 新增 `pi` family；如模型本身严格命中已存在 provider family，可按其 provider/model 归类。Pi 只是运行该模型的 agent，独立 family 会掩盖多 provider 真相。

成本口径核验结论：Pi JSONL 的 `usage.cost.total` 是当前 Viewer 能够可靠复现的持久化 API 成本。它与 Pi 记录的 input/output/cacheRead 分项相加一致；Viewer 不把该值换算成 provider 最终账单，也不根据 token 数二次加价。人民币实际扣款高于该值时，需核对 provider 账单、汇率/套餐费用，或检查是否存在使用一次性 `--session-dir` 写入的未被默认 root 自动发现的会话。

### 阶段 5：前端入口、持久化、导出与故意排除（主要功能完成）

1. 更新 Agent 联合、`AGENT_META`、agent icon/label、`ALL_AGENTS`、enabled-agent defaults/migration/reset、`LaunchArgs` 的读写/重置、export history 的 `VALID_AGENTS` 和 Stats scope 的 localStorage 校验。旧 localStorage 必须自动补 `{ pi: ... }`，不会在升级后使 settings 页面崩溃。
2. 侧栏、Welcome、NewMenu、全局搜索、回收站、Top Sessions、通知标题、view history、terminal persistence 走现有通用 Agent 路径；为 Pi 加品牌图标并使 HTML export assistant avatar 不是 Claude 冒充。
3. NewMenu 仅显示“新建会话（TUI）”和 terminal；quick open 设置为 chat 时在 Pi 上明确提示不支持或降级为 TUI。不得让 `chatSessions.ts` / `agent_chat.rs` / Composer / PermissionPrompt / model picker 尝试创建 Pi GUI session。
4. 当前导出使用当前 Viewer lineage 的通用 `Msg[]`；Pi JSON 另走后端稳定快照，输出 `cc-session-viewer-pi-export` 包络（header、all entries、selected leaf、Pi schema/renderer version）。该 JSON 是 Viewer 审计包，不直接作为 Pi session JSONL 恢复输入。
5. 继续使用应用自身导出，不调用 Pi `/export`；更不调用 `/share`（会上传 private GitHub gist，属于额外外部写入）。
6. Pi 没有单一 OAuth account quota。`usage.ts` / `usage_api.rs` 固定为 Anthropic API，Pi 会话上不显示 Claude 订阅额度，也不读取 Pi 的 `auth.json`。

### 阶段 6：CLI 检查、内嵌终端与外部终端（已完成）

1. 在 `cli_env.rs` 增加 `CliSpec { name: 'pi', binary: 'pi', npm_package: '@earendil-works/pi-coding-agent' }`。版本检测以 `pi --version`，latest 走 npm registry；安装入口使用官方 `curl -fsSL https://pi.dev/install.sh | sh`。升级不根据用户输入的安装命令猜测，而是解析 `pi` 的真实路径：nvm/fnm/volta/npm/bun/brew 使用对应包管理器，独立二进制才回退官方 `pi update self`。
2. CLI 环境诊断显示登录 shell 实际解析的 pi 路径、重复安装、版本、`PI_CODING_AGENT_DIR` / session root 的**路径级**健康检查以及 `pi --help` 支持关键参数；不打印 auth、provider key 或 settings 内可能敏感的 proxy/credential 内容。
3. Pi 官方提供 `pi update self`（以及 `pi update pi`）自更新命令。CLI UI 可在检测到新版本时执行升级；curl installer 由于落地为 npm 全局包，会使用真实二进制旁的 npm 前缀安装最新包；独立二进制才调用官方自更新回退。安装动作沿用官方 installer，不读取或修改 Pi 凭证文件。
4. 更新 `src/types.ts` 的 CLI 联合类型、`CliEnvironmentCheck.vue` 的 label、icon、官方 URL 与所有 diagnosis/install/upgrade store key。Pi 卡片支持刷新、诊断和一键升级。
5. `resume_command` 为 `AgentCommand::new('pi').arg('--session').arg(absolute_jsonl_path)`；新建为 `AgentCommand::new('pi')`。两者均允许当前每-agent extra args，但须按现有 POSIX/PowerShell quote 规则处理。Viewer 选中非默认 leaf 时，恢复前明确提示 Pi 会回到物理最后 entry；不会伪造 entry id 参数。
6. 内嵌 PTY 走通用流程，不复制 Codex 专属 Windows 粘贴/编辑器颜色/retry 逻辑；增加 macOS/Linux/Windows 的 new/resume、包含空格路径、额外参数和未安装 CLI 测试。
7. 外部 terminal 同走 `new_session` / `resume_session`。验证 cwd 设为 Pi header.cwd、绝对 session path 恢复正确、`PI_CODING_AGENT_DIR` 环境继承一致，且 UI 不把 `--continue` 错当指定 session 恢复。

### 阶段 7：Pi extension 状态 relay、桌面宠物与 hooks 设置（已完成）

1. Pi 没有 Kimi/Claude 风格静态 hooks。改为生成 app 托管的 global extension，例如 `~/.pi/agent/extensions/cc-sessions-viewer-turn-status.ts`，并原子合并 `~/.pi/agent/settings.json.extensions`，仅添加/更新本应用的绝对 extension path，保留未知 JSON 字段、packages、skills、prompts、themes 和用户 entries。
2. 扩展直接写应用既有 turn-signal JSONL，不经 stdin hook script。每条 payload 至少带 `{ agent:'pi', path: sessionFile, sessionId, cwd, state, source:'hook' }`；无 persistent session file（`--no-session`）时不发可点击任务。
3. lifecycle 映射：`before_agent_start`/`agent_start → started`；`agent_end` 检查最终 assistant `stopReason`，`error → failed`、`aborted → completed`（可在历史 UI 标为“已中止”）；**不可只按非空 `errorMessage` 判失败**，真实 `aborted` entry 同样会带该字段；`agent_settled → completed` 仅在本轮未失败时发出。选择 `agent_settled` 而非每次 `turn_end`，防止自动 retry/compact/follow-up 中途闪成 completed。
4. Pi 核心无 permission popup，任意第三方 extension 的 tool gate 也没有统一稳定 payload，因此本期不承诺 `blocked`。状态 UI 要显示 Pi 只提供 started/completed/failed；后续若 Pi 官方增加 permission lifecycle 再扩展。
5. `turn.rs` 的 agent allowlist、signal 读取/验证、`TurnHookInstallResult`、`TurnHookStatus`、all-installed 判断、`api.ts` 类型、turnHookStatus store、Settings hooks 卡片和四语描述均加入 Pi。检测条件是 settings 引用 + extension 内容/marker + 当前 signal path 都匹配；`--no-extensions` 或用户禁用时如实显示未启用。
6. 安装、更新、禁用/卸载保持幂等，配置写入前后做并发版本检查；extension 捕获所有写入错误，永远不阻塞 Pi。测试 global settings 空/已有 arrays/重复条目/用户禁用/坏 JSON/多 app 实例/ephemeral session。
7. Pi 运行状态进入既有 tab state、live notification、desktop pet task。`App.vue::openDesktopPetSession` 的独立 allowlist 改为 capability/统一 source 校验，使 Pi JSONL path 可被 `turn::resolve_desktop_pet_session` 找回。Pi 不需要 Codex asar 宠物素材。

### 阶段 8：worktree、文档和交付（已完成）

开发说明、兼容性策略和回归命令均已整理在本计划文档；本阶段仅剩真实 Pi worktree 回归和最终交付检查。

1. ✅ 在真实临时 git worktree 中启动 Pi，确认 header.cwd 是 worktree 路径；Pi 已同步加入前端 `App.vue::WORKTREE_AGENTS`、Rust `agent_supports_worktrees()` 和 capability metadata。
2. ✅ 删除验证覆盖显式 Pi JSONL 清理与 worktree 移除；产品删除流程仍会先停止 terminal/tab、按 header.cwd 计数并 hard-delete 会话，再移除物理 worktree。不会处理 header.cwd 不匹配的 session。
3. ✅ README 三语 supported CLI / Pi 限制、stats scope、hooks 文案和 Pi 相关设置文案已同步。
4. ✅ 开发者文档已补充：session root precedence、v1-v3 tree 算法、cost source 语义、app-managed extension 安全模型、fixtures、兼容性策略和手动回归命令。

## 与现有 agent 关联的遗漏清单

| 关联点 | 当前代码的独立耦合 | Pi 计划要求 |
| --- | --- | --- |
| Agent 类型与持久化 | `Agent` union、`ALL_AGENTS`、`LaunchArgs`、enabled/default/reset、statsScope、export history | 全部加 Pi 和旧 localStorage migration。 |
| Stats | `stats/stream.rs` scope 白名单、StatsView `asAgent` | 支持 pi/all/visible scope，且区分 branch display 与全树计费。 |
| Tray | `TRAY_AGENT_NAMES` 和 macOS card 色/名称 | 加 Pi、可见性同步和空活动卡。 |
| Worktree | `WORKTREE_AGENTS` + `agent_supports_worktrees()` | 先实测 cwd，再同步维护两份 allowlist。 |
| 状态 | `turn.rs` 固定四 agent schema + CJS relay | 使用 Pi extension relay，补 Rust/API/Settings/UI/locale。 |
| Desktop pet | `openDesktopPetSession` 独立 agent allowlist | 让 Pi file session 可打开。 |
| CLI | `CliSpec`、前端 CLI unions、labels/URLs、upgrade all | 诊断 Pi；按实际安装方式升级。 |
| Export | 通用 `Msg[]` 是线性、HTML avatar 分支 | 当前 branch 的 HTML/MD + full-tree Pi JSON。 |
| Pricing | `PricingView` 固定 provider family | 不新增 Pi family；使用 Pi recorded cost/strict provider catalog。 |
| OAuth quota | `usage_api.rs` 固定 Anthropic | Pi 显式排除，不读 auth。 |
| GUI Chat | Claude/Codex 专属进程和协议 | Pi `guiChat=false`，不接入。 |

## 测试、手动回归与完成定义

### 自动化测试

- Rust：已覆盖 root precedence、path containment、v1/v2/v3 header、title/session_info、**物理最后 entry** active leaf、multi-branch parser、cycle/dangling parent/duplicate id/self-parent、message/tool/bash/custom/compaction parsing、高密度 tool-loop、图片/skill、aborted usage、append-only rename、file trash/restore、full-tree usage/cost de-dup、strict fallback、stats stream、tray、worktree、Pi extension settings merge/status lifecycle。
- 待补测试：前端 branch selector/search locator、非默认 Viewer branch 启动 terminal 提示、hidden custom message 导出、Pi full-tree JSON export 的独立 Vitest 覆盖；功能路径本身已实现。
- TypeScript/Vitest：已覆盖的 agent/settings/CLI/stats/export 基础回归继续保留；branch selector、search locator、恢复提示和 full-tree export 仍可继续补充独立 UI 测试。
- 回归：全部 Claude/Codex/Kimi/Grok/agy/opencode 测试、TypeScript typecheck、Rust tests、前端 build 通过。

### 手动回归

1. 用 `PI_CODING_AGENT_DIR` 和 `PI_CODING_AGENT_SESSION_DIR` 指向临时 fixture root，检查发现、项目归属、标题、空 session、损坏/软链文件和默认 lineage 搜索。
2. 打开线性和多 branch fixture：确认 Viewer 默认显示物理最后完整 entry 的 parent chain；使用 branch picker 切换 B/C terminal leaf，确认只显示对应 lineage；在原生 Pi 用 `/tree` 验证分支，重开原生 Pi 仍取文件物理最后 entry。
3. 以 `firstKeptEntryId` 与 `retainedTail` compaction fixture 分别核对：历史视图完整、有效 context 仅作版本化诊断、retainedTail 不出现在普通搜索/导出且不增加调用/费用。
4. 执行 rename、soft delete、restore、permanent delete；在原生 `pi --session <absolute path>` 中验证名称/恢复，且并发运行 Pi 时不会重写其他 entries。外部 Pi mtime 持续变化、内嵌 Pi 运行、重复 id/环文件均必须禁用写操作并给出原因。
5. 手工汇总 assistant/summary usage 与 Statistics/Tray 比较，确认 Pi `cost.total` 不被二次加价、未定价 provider 不产生虚假美元、放弃 branch 的真实花费仍统计。
6. 新建/恢复 Pi 于内嵌和外部 terminal，验证 cwd、特殊路径、extra args、缺失 CLI 和延迟落盘；选中非默认 Viewer location 时确认恢复提示，并验证 Pi 仍按物理最后 entry 恢复。
7. 打开正在连续追加的高密度工具会话，验证刷新最终收敛为同一完整 snapshot，不出现缺少 toolResult、重复 usage 或搜索/导出与会话视图不同 branch 的中间状态。安装 extension 后覆盖正常完成、provider error、abort（即使 entry 有 `errorMessage`）、自动 retry/follow-up、`--no-extensions`、`--no-session`，确认 tab/notification/pet 状态及不阻塞 Pi。
8. 在 git worktree 中创建 Pi session，验证项目分组、删除确认计数、tab 停止与文件清理。

### 完成定义

- 当前完成定义：Pi 会话可被安全发现、按真实 cwd 分组、以原生默认 leaf（物理最后完整 entry）正确阅读；不会把平行分支串成线性历史。Viewer 支持 terminal branch 的只读查看、全树搜索定位、当前 branch Markdown/HTML 和 full-tree JSON 导出；独立树画布与 entry 级多命中仍不在范围。
- 所有已发生的 Pi 调用在全树统计中至多记一次，Pi 记录成本、严格模型价格、估算和未知成本在 UI 中可区分。
- 重命名、回收站、终端、统计、托盘、基础状态 relay、worktree 路径、持久化、文档及本地化已覆盖；Pi capability 标记和 terminal branch 相关功能已统一。
- 不读取凭证、不上传 session、不把 Pi 的多 provider 模型或第三方 extensions 误表示为本应用能完整控制的能力。
