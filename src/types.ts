export type Agent = 'claude' | 'codex' | 'agy' | 'opencode' | 'grok' | 'kimicode' | 'pi'

export interface ProjectInfo {
  dirName: string
  displayPath: string
  sessionCount: number
  lastModified: number
  /** 项目目录当前是否仍存在于磁盘上 */
  exists: boolean
  bookmarked?: boolean
  parentDirName?: string
  worktreeName?: string
}

export interface SessionMeta {
  id: string
  fileName: string
  path: string
  title: string
  cwd?: string
  created?: string
  modified: number
  size: number
  messageCount: number
  /** Pi-only: terminal branches and complete persisted tree entries. */
  piBranchCount?: number
  piEntryCount?: number
  codexAppListRank?: number | null
  codexAppListScanned: number
  codexAppFirstPageSize: number
  codexAppFirstPagePosition: number
  codexInternal: boolean
  codexArchived: boolean
}

export interface PiTreeNode {
  id: string
  parentId?: string | null
  children: string[]
  kind: string
  timestamp?: string | null
  ordinal: number
  terminal: boolean
}

export interface SessionPage {
  total: number
  sessions: SessionMeta[]
}

export type BlockKind = 'text' | 'thinking' | 'tool_use' | 'tool_result' | 'image' | 'file'

export interface DiffLine {
  kind: 'ctx' | 'add' | 'del'
  oldNo: number | null
  newNo: number | null
  text: string
}

export interface DiffHunk {
  oldStart: number
  newStart: number
  lines: DiffLine[]
}

export interface GitCommit {
  hash: string
  author: string
  date: string
  message: string
}

export interface GitFileStatus {
  path: string
  status: string
}

export interface GitRepositoryState {
  branch: string | null
  branches: string[]
  changeCount: number
}

export interface GitDiffFile {
  path: string
  additions: number
  deletions: number
  status: string
}

export interface Block {
  kind: BlockKind
  text?: string
  toolName?: string
  toolInput?: string
  toolId?: string
  isError: boolean
  filePath?: string
  fileChangeType?: 'add' | 'update' | 'delete' | string
  /** file 块：该路径是目录（GUI chat 的「Add folder」附件）。决定 chip 用文件夹图标 +
   *  「打开文件夹」提示，而非文件图标 +「打开文件」。 */
  isDir?: boolean
  diff?: DiffHunk[]
  imageSrc?: string
  /** 历史剪贴板图片路径存在但本地临时文件已被清理。仍按图片布局显示裂图占位。 */
  imageUnavailable?: boolean
  /** Chat 粘贴图片在正文中的 token；历史/本地回显用来保持图片绑定。 */
  inlinePlaceholder?: string
  /** Pi todo tool result's structured current task list. */
  piTodoSummary?: PiTodoSummary
}

export interface PiTodoSummary {
  completed: number
  total: number
  tasks: Array<{
    subject: string
    status: string
  }>
}

export interface Msg {
  uuid?: string
  role: 'user' | 'assistant'
  timestamp?: string
  /** Live Chat 中从本轮用户输入到该条模型/工具消息到达的耗时（仅前端运行时字段）。 */
  executionMs?: number
  model?: string
  sidechain: boolean
  blocks: Block[]
  /** 系统注入的 `type:"user"` 记录归类（compact / meta / task-notification /
   *  system / command-output）。后端 claude 源填充；其它 agent 不填 → undefined。
   *  非空时前端把这条渲染成低调的「系统」块，而非「Me」气泡。 */
  metaKind?: string
}

/** 全局搜索的命中条目（与 Rust 端 SearchHit 同形）。 */
export type SearchField = 'title' | 'id' | 'path' | 'text'

/** 全局搜索的范围：只看会话 ID，或看标题 + 用户消息正文（不含 ID）。 */
export type SearchScope = 'id' | 'keyword'
export interface SearchHit {
  projectKey: string
  projectDisplay: string
  session: SessionMeta
  matchedField: SearchField
  /** 命中片段：title/id/path 等于原值；text 上是带前后文（带省略号）的小段。 */
  snippet: string
  /** 文本命中所在消息的索引（read_session 返回的数组下标）；metadata 命中为 undefined。 */
  matchMsgIndex?: number
  /** 文本命中所在消息的 uuid（若 agent 写了）；前端定位时优先用 uuid 兜底。 */
  matchMsgUuid?: string
  /** Pi-only terminal entry containing the text hit. */
  piLeafId?: string
}

/** 单个会话的 token 用量；与 Rust 端 UsageSummary 同形。
 *  `cacheCreation1hInputTokens` 是 `cacheCreationInputTokens` 的子集（1-hour tier），
 *  cost 公式额外按 1× 5min 价位再算一遍（合计 2×），别在 UI 上把它加进 total。 */
export interface UsageSummary {
  inputTokens: number
  outputTokens: number
  cacheCreationInputTokens: number
  cacheCreation1hInputTokens: number
  cacheReadInputTokens: number
  reasoningOutputTokens: number
  total: number
}

/** 统计 dashboard：单个项目的聚合（与 Rust ProjectStats 同形）。 */
export interface ProjectStats {
  dirName: string
  displayPath: string
  sessionCount: number
  messageCount: number
  callCount: number
  usage: UsageSummary
  costUsd: number
  lastModified: number
}

/** 统计 dashboard：某一天（UTC）的活动量。 */
export interface DailyActivity {
  date: string // YYYY-MM-DD
  sessionCount: number
  messageCount: number
  callCount: number
  tokens: number
  costUsd: number
}

/** Top Sessions 排行里的一条。 */
export interface SessionStat {
  agent: Agent
  sessionId: string
  path: string
  projectDisplay: string
  title: string
  lastModified: number
  callCount: number
  usage: UsageSummary
  costUsd: number
}

/** By Model 排行里的一条。 */
export interface ModelStat {
  model: string
  label: string
  callCount: number
  usage: UsageSummary
  costUsd: number
  /** 价格表未命中的真实 API 调用数；0 也可能是已知免费，不能只看 costUsd 判断。 */
  unpricedCallCount: number
  /** 使用 Grok 官方旗舰价格兜底估算的真实 API 调用数。 */
  estimatedCallCount: number
  /** 0..=1。cache_read / (input + cache_read + cache_creation)。 */
  cacheHitRate: number
}

/** By Tool / By Shell / By MCP 共用 name+count 对。 */
export interface NamedCount {
  name: string
  count: number
}

/** By Activity 一行：分类 key + 调用 / 成本。`key` 对应 stats.activity.* 翻译。 */
export interface ActivityStat {
  key: string
  turnCount: number
  callCount: number
  costUsd: number
}

/** 统计范围筛选 —— 前端 dropdown 切换。 */
export type StatsScope = 'all' | Agent

/** 时间范围筛选。`custom:start:end` 使用本地日期（YYYY-MM-DD），end 按整日包含。 */
export type StatsPresetRange = 'today' | 'days7' | 'days30' | 'month' | 'months3' | 'months6'
export type StatsRange = StatsPresetRange | `custom:${string}:${string}`

/** 流式统计的完整结果（与 Rust AgentStats 同形）。`scope` 标识维度。 */
export interface AgentStats {
  scope: 'all' | Agent | string
  sessionCount: number
  messageCount: number
  callCount: number
  daysActive: number
  usage: UsageSummary
  costUsd: number
  /** 未命中价格表的真实 API 调用数；costUsd 只包含已知价格部分。 */
  unpricedCallCount: number
  /** 使用 Grok 官方旗舰价格兜底估算的真实 API 调用数。 */
  estimatedCallCount: number
  /** Pi persisted `usage.cost.total` calls. */
  recordedCallCount: number
  /** Calls priced by the strict provider/model catalog fallback. */
  catalogCallCount: number
  cacheHitRate: number
  /** 按 cost_usd 降序的项目列表。 */
  projects: ProjectStats[]
  /** 按日期升序的日活时间轴（稀疏，没活动的天不出现）。 */
  dailyActivity: DailyActivity[]
  /** 按 cost_usd 降序的 Top 10 会话。 */
  topSessions: SessionStat[]
  /** 按 cost_usd 降序的模型排行。 */
  byModel: ModelStat[]
  /** 按调用次数降序的工具排行。 */
  byTool: NamedCount[]
  /** 按调用次数降序的 shell 主命令排行。 */
  byShell: NamedCount[]
  /** 按调用次数降序的 MCP server 排行。 */
  byMcp: NamedCount[]
  /** 按 cost_usd 降序的活动分类排行。 */
  byActivity: ActivityStat[]
}

/** 流式推送的进度负载。`partial` 是到目前为止的累计快照，前端直接替换。 */
export interface StatsProgress {
  requestId: number
  processed: number
  total: number
  partial: AgentStats
}

export interface StatsDone {
  requestId: number
  stats: AgentStats
}

export interface StatsError {
  requestId: number
  error: string
}

// ============================ GUI chat（程序化聊天）============================

/** 一轮问答的运行状态。 */
export type ChatTurnState = 'idle' | 'running'

/** 输入框里的图片附件（粘贴 / 拖拽 / 选择）。`dataUrl` 供预览与本地回显，
 *  `data` 是去掉 `data:` 前缀的纯 base64，发送给后端时用。 */
export interface ChatImageAttachment {
  dataUrl: string
  mediaType: string
  data: string
  /** 文件名（来自文件选择/拖拽；粘贴的截图回退 image.png）。仅前端展示用。 */
  name?: string
  /** 原始磁盘路径（文件选择器 / 拖拽得到）。粘贴板截图无此字段。
   *  Codex 等 OneShot agent 用 `@"path"` 引用本地文件而非传 base64。 */
  sourcePath?: string
  /** 粘贴到正文中的稳定占位符（例如 `[Image #1]`）。普通附件没有此字段。 */
  inlinePlaceholder?: string
}

/** agent_chat_send 透传给后端的图片输入（与 Rust ChatImageInput 同形）。 */
export interface ChatImageInput {
  mediaType: string
  data: string
  /** Claude stdin 用来把图片内容块放回正文语义位置；Codex 会忽略。 */
  placeholder?: string
}

export interface ChatTextElement {
  type: 'mention'
  byteRange: {
    start: number
    end: number
  }
  path: string
  name: string
  placeholder?: string
}

/**
 * 非图片附件（文件 / 文件夹）。由系统选择器选出，发送时以 `@"path"` 追加到 prompt，
 * 让 agent 自己按路径读取。`isDir` 仅影响 chip 图标（文件夹用 folder 图标）。
 */
export interface ChatFileAttachment {
  path: string
  name: string
  isDir: boolean
}

/** GUI chat `@` 文件浮层的一条目录/文件项（与 Rust ProjectFileEntry 同形）。
 *  `relPath` 相对会话 cwd（`/` 分隔）；`name` 是末段名字；`isDir` 决定图标 + 钻取行为。 */
export interface ProjectFileEntry {
  relPath: string
  name: string
  isDir: boolean
  /** 仅目录有意义：是否含可见子项。空目录 = false → 不显示「进入」chevron、禁用下钻。 */
  hasChildren: boolean
}

/** GUI chat `/` 浮层的一条可用项（命令 / 技能，与 Rust SlashCommand 同形）。 */
export interface SlashCommand {
  /** 调用 token（无前导 `/`）：命令命名空间名 / 技能名。 */
  name: string
  /** 展示名：命令 = `/name`；技能 = 美化后的 Title Case。 */
  title: string
  description: string
  /** 分组 + 图标依据。`system` = 前端注入的客户端内置指令（不来自磁盘扫描）。 */
  kind: 'command' | 'skill' | 'system'
  /** 来源类别：user → UI 显示「Personal」；project / plugin → 用 originName；system → 无角标。 */
  origin: 'user' | 'project' | 'plugin' | 'system'
  /** 项目名 / 插件名（user 来源省略）。 */
  originName?: string
  /** 命令 `argument-hint`（如 `[--wait] [--base <ref>]`）：选中后在输入框作为 ghost 参数提示。 */
  argumentHint?: string
}

/** GUI chat 的进程模型：长驻 stdin / Codex app-server 切设置需 restart-with-resume；
 *  one-shot resume 则切设置改下轮 flag 即生效。 */
export type ChatProcessModel = 'longLivedStdin' | 'oneShotResume' | 'codexAppServer'

/** agent_chat_start 的返回（与 Rust ChatStartInfo 同形）。 */
export interface ChatStartInfo {
  chatId: number
  processModel: ChatProcessModel
}

export interface RunningChatInfo {
  chatId: number
  agent: Agent
  projectKey: string
  cwd: string
  sessionId: string | null
  title?: string
  messages?: Msg[]
  turnState?: ChatTurnState
  turnStartedAtMs?: number | null
  permissionMode: string
  model: string | null
  effort: string | null
  processModel: string
}

export interface ReclaudeInfo {
  installed: boolean
  daemonRunning: boolean
  daemonPort: number | null
}

export interface ClaudeRuntimeInfo {
  hasCustomBaseUrl: boolean
  aliasTargets: {
    opus?: string
    sonnet?: string
    haiku?: string
    fable?: string
  }
  /** init 事件回来前对鉴权方式的预判：'none' = 订阅/OAuth；其它 = API key；缺省 = 判不出。 */
  apiKeySource?: string
  /** settings.json 的 `effortLevel`：用户全局 reasoning effort 默认档。CLI 不带 --effort
   *  时即用它 —— effort 选择器在用户未改档前展示这个「真实生效默认」，而非假的 levels[0]。 */
  effortLevel?: string
}

export interface CodexRuntimeInfo {
  /** true = 用户通过第三方 API key / 自定义端点使用 Codex（config.toml 有 model_provider）。 */
  usesApiKey: boolean
  /** ~/.codex/config.toml 顶层 model；自定义 provider 下用于显示/勾选，不强制下发。 */
  model?: string
  /** ~/.codex/config.toml 顶层 model_reasoning_effort；保留给 UI 展示/后续扩展。 */
  effort?: string
}

/** agent-chat://* 事件 payload（与 Rust 端同形）。 */
export interface ChatEventPayload { chatId: number; msg: Msg }
export interface ChatInitPayload { chatId: number; sessionId?: string; apiKeySource?: string }
export interface ChatResultPayload { chatId: number; ok: boolean; usage?: UsageSummary }
export interface ChatStderrPayload { chatId: number; line: string }
export interface ChatExitPayload { chatId: number; code: number }

/** token 级流式增量（Claude stream_event / Codex app-server delta）。 */
export interface ChatDelta {
  index: number
  /** 'start' | 'delta' | 'stop' —— 内容块生命周期。 */
  phase: string
  /** 块类型 text | thinking | tool_use（start 必有；delta 也带，前端兜底建块）。 */
  kind?: string
  /** 仅 delta：本次追加的文本片段。 */
  text?: string
}
export interface ChatDeltaPayload { chatId: number; delta: ChatDelta }

/** 交互式工具权限请求（Claude 控制协议 `can_use_tool`，与 Rust ChatPermissionRequest 同形）。
 *  `input` 是工具参数原文（Bash 的 `command`、文件工具的 `file_path` 等）；
 *  `permissionSuggestions` 是「始终允许」的规则建议（`addRules`，含 destination）。 */
export interface ChatPermissionRequest {
  requestId: string
  toolName: string
  input: unknown
  description?: string
  permissionSuggestions?: unknown
}
export interface ChatPermissionPayload { chatId: number; request: ChatPermissionRequest }

/** AskUserQuestion 的单个选项。`preview` 是可选的等宽预览内容（mock / 代码 / 配置），
 *  仅单选题用得上 —— 渲染成左列选项、右栏预览的并排布局。 */
export interface ChatQuestionOption {
  label: string
  description?: string
  preview?: string
}
/** AskUserQuestion 的单条提问。`multiSelect` 为真时允许多选（答案逗号拼接）。 */
export interface ChatQuestionItem {
  question: string
  header?: string
  multiSelect?: boolean
  allowOther?: boolean
  options: ChatQuestionOption[]
}
/** 模型向用户提的结构化选择题（Claude `AskUserQuestion`，与 Rust ChatQuestionRequest 同形）。
 *  与工具权限同走 `can_use_tool` 控制协议，回写时把 `questions` 原样带回 `updatedInput`。 */
export interface ChatQuestionRequest {
  requestId: string
  questions: ChatQuestionItem[]
  /** Kimi 的后台 AskUserQuestion：即时回执不是最终答案。 */
  background?: boolean
  keepAfterTurn?: boolean
  localCodexPlanPrompt?: boolean
}
export interface ChatQuestionPayload { chatId: number; request: ChatQuestionRequest }

/** 单个额度窗口（与 Rust usage_api::UsageWindow 同形）。来自 OAuth 用量接口。 */
export interface UsageWindow {
  /** 利用率百分比 0–100。 */
  utilization: number
  /** ISO8601 重置时间（用 `new Date()` 解析）。 */
  resetsAt?: string
}
/** 账号额度快照（与 Rust usage_api::AccountUsage 同形）。 */
export interface AccountUsage {
  fiveHour?: UsageWindow | null
  sevenDay?: UsageWindow | null
  sevenDayOpus?: UsageWindow | null
  sevenDaySonnet?: UsageWindow | null
}

export interface TrashItem {
  trashFile: string
  agent: Agent
  projectLabel: string
  originalPath: string
  /** 回收站里可读取的 transcript 路径；Grok 指向会话目录内的 updates.jsonl。 */
  trashPath: string
  deletedAt: number
  title: string
  size: number
}

export interface TrayAgentSummary {
  agent: string
  todayTokens: number
  todayCost: number
  todayUnpricedCalls: number
  todayEstimatedCalls: number
  weekTokens: number
  weekCost: number
  weekUnpricedCalls: number
  weekEstimatedCalls: number
  monthTokens: number
  monthCost: number
  monthUnpricedCalls: number
  monthEstimatedCalls: number
  sessionCount: number
}

export interface TrayStats {
  agents: TrayAgentSummary[]
  totalTodayTokens: number
  totalTodayCost: number
  totalTodayUnpricedCalls: number
  totalTodayEstimatedCalls: number
  totalWeekTokens: number
  totalWeekCost: number
  totalWeekUnpricedCalls: number
  totalWeekEstimatedCalls: number
  totalMonthTokens: number
  totalMonthCost: number
  totalMonthUnpricedCalls: number
  totalMonthEstimatedCalls: number
}

// ---- CLI 环境检测 ----

export interface CliVersionInfo {
  cli: 'claude' | 'codex' | 'agy' | 'opencode' | 'grok' | 'kimi' | 'pi'
  npmPackage: string
  currentVersion: string | null
  latestVersion: string | null
  upgradable: boolean
  installed: boolean
  error: string | null
  health: CliHealthStatus | null
}

export interface CliHealthStatus {
  healthy: boolean
  summary: string | null
}

export interface CliInstallation {
  path: string
  version: string | null
  isDefault: boolean
  packageManager: string
  resolvedPath: string | null
}

export interface CliDiagnosisResult {
  cli: 'claude' | 'codex' | 'agy' | 'opencode' | 'grok' | 'kimi' | 'pi'
  binaryName: string
  installations: CliInstallation[]
  hasConflict: boolean
  error: string | null
}

export interface CliUpgradeResult {
  cli: 'claude' | 'codex' | 'agy' | 'opencode' | 'grok' | 'kimi' | 'pi'
  success: boolean
  newVersion: string | null
  error: string | null
}
