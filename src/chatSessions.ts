// 全局 GUI chat 会话管理 —— 程序化聊天（stream-json 管道子进程）的前端状态层。
//
// 类比 `terminals.ts`（TUI tabs），但简单得多：没有 xterm，只有一份 reactive `Msg[]`
// 由 `agent-chat://event` 事件累积；`agent-chat://result` 推进 turn 门控；`sendPrompt`
// 把用户消息写进子进程 stdin。渲染完全复用 ChatView 的 `Block` 气泡。
//
// 事件路由：后端事件是全局广播且带 `chatId`，这里在模块加载时**一次性**装好 5 个
// listener，按 chatId 分派到对应会话。由于 `agentChatStart` 解析出 chatId 之前子进程
// 可能已经吐出 system/init —— listener 在 start 之前就已 attach（Tauri 不丢已 attach 的
// 事件），并用 `pendingByChatId` 缓冲「mapping 注册前」到达的事件，注册后回放。
//
// webview 刷新时后端进程不杀 —— 前端通过 reconnectChats() 重连。

import { reactive, ref } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import * as api from './api'
import { defaultModel, defaultEffort, defaultPermissionMode, effectiveEffort, sanitizeModel } from './chatComposerOptions'
import {
  rememberedChatEffort,
  rememberedChatModel,
  rememberedChatPermissionMode,
  rememberChatGuiPreference,
} from './chatGuiPreferences'
import { buildPermissionDecision, isExitPlanModeRequest, type PermissionChoice } from './chatPermission'
import {
  buildQuestionCancelDecision,
  buildQuestionDecision,
  type QuestionSelection,
} from './chatQuestion'
import { useReclaude } from './settings'
import { bumpUsage } from './usage'
import { markProjectsDirty } from './projectsRefresh'
import type { ChatHistoryEntry } from './chatInputHistory'
import { codexPluginMentionTextElements, expandCodexPluginMentionsForPrompt } from './codexPluginMentions'
import { isFileChangeResult } from './toolResultRouting'
import { summarizeTool, type ToolSummary } from './liveToolSummary'
import type {
  Agent,
  Block,
  ChatDelta,
  ChatDeltaPayload,
  ChatEventPayload,
  ChatExitPayload,
  ChatImageAttachment,
  ChatFileAttachment,
  ChatTextElement,
  ChatInitPayload,
  ChatPermissionPayload,
  ChatPermissionRequest,
  ChatQuestionPayload,
  ChatQuestionRequest,
  ChatProcessModel,
  ChatResultPayload,
  ChatStderrPayload,
  ChatTurnState,
  CodexRuntimeInfo,
  Msg,
  UsageSummary,
} from './types'

// Claude Code 给 /context、/compact、/model 等本地命令的 synthetic 记录打的「伪模型」标记 ——
// 它不是真实模型，绝不能让它流进底栏模型选择器（模型只应在用户手动调整时变动）。
const SYNTHETIC_MODEL = '<synthetic>'
const LOCAL_CODEX_PLAN_REQUEST_PREFIX = 'local-codex-plan-'

/** 一条待发消息（带可选图片 + 文件附件），形参与 sendPrompt 对齐 —— 出队时原样转发。 */
export interface QueuedMessage {
  id: number
  text: string
  images: ChatImageAttachment[]
  files: ChatFileAttachment[]
}

/** 最近一次工具事件的紧凑状态。只保存状态行需要的摘要，不保存完整工具输入或输出。 */
export interface LiveToolActivity {
  toolName: string
  toolId?: string
  summary: ToolSummary
  phase: 'calling' | 'result' | 'failed'
  updatedAt: number
}

export type ChatTurnOutcome = 'completed' | 'cancelled' | 'failed'

export interface ChatSession {
  /** 本地稳定 id（v-for / 选中用），与后端 chatId 是两套号。 */
  uiId: number
  /** 后端 chat 子进程 id —— start 解析完成前为 null。 */
  chatId: number | null
  agent: Agent
  /** 所属侧栏项目 key（= ProjectInfo.dirName）。 */
  projectKey: string
  cwd: string
  /** 续聊的源 session id；新开会话在 init 事件里回填。 */
  sessionId: string
  title: string
  /** 会话创建时间（续聊=原会话 created；新开=起聊时刻）。供 ChatView 头部「created」显示。 */
  createdAt?: string
  /** 由事件累积的对话消息，直接喂给 ChatView。 */
  msgs: Msg[]
  /** 新分支首次显示时回填到输入框的一次性草稿；ChatComposer 消费后清空。 */
  initialDraft?: ChatHistoryEntry
  /** 本轮问答状态。 */
  turnState: ChatTurnState
  /** 本轮开始时间戳（ms）；turnState='running' 时配合 `now` 算耗时。 */
  turnStartedAt: number
  /** 上一轮耗时（ms），结束后固定显示。 */
  lastTurnMs: number
  /** 进程生命周期。 */
  status: 'spawning' | 'running' | 'exited' | 'error'
  /** 最近一次 result 的 token 用量。 */
  usage?: UsageSummary
  /** 最近一条 assistant 记录的模型全名（如 "claude-opus-4-8"）—— §10.5 上下文窗口换算用。 */
  lastModel?: string
  /** Claude init 的 apiKeySource：'none' = 订阅/OAuth 登录（5h/周限额生效）；其它值
   *  = API key 计费（不受 5h/周窗口约束）→ 前端隐藏限额角标。undefined = 还没拿到 init。 */
  apiKeySource?: string
  errorMessage?: string
  /** stderr 诊断行（封顶，排障用）。 */
  stderrTail: string[]
  /**
   * 网络不稳/重试状态：running 期间 CLI 往 stderr 写退避/瞬时错误时置位，状态行据此显示
   * 「请求失败 · 重试中 (n/N)」替代纯耗时；收到下一条 event/delta（有进展）即清空。
   * null = 正常；`{}` = 重试中但拿不到次数；`{attempt,max}` = 带次数。
   */
  retry?: { attempt?: number; max?: number } | null
  /**
   * 正在流式生成的「进行中」文本块（仅 Claude --include-partial-messages）。
   * 与 `msgs` 解耦：每 token 只动这个小对象 → 只重渲染流式气泡，不触发整列表
   * mermaid/高亮重算（见 §10.6 perf 注）。权威 assistant 记录到达即清空（onMsg）。
   */
  live?: { kind: string; text: string } | null
  /** 当前/最近一次工具调用的轻量活动信息；只供运行状态行展示。 */
  toolActivity: LiveToolActivity | null
  /** 并行工具调用的未完成集合；状态行只展示其中最新的一项。 */
  toolActivities: LiveToolActivity[]
  /** 本轮结束原因，供 UI 在状态行消失前播放一次完成/失败/取消反馈。 */
  lastTurnOutcome: ChatTurnOutcome | null
  // ---- §10.2/10.3/10.4 切换器：当前选择（底栏 picker 改它，懒生效）----
  /** 权限模式：Claude 默认 bypassPermissions；Codex 默认 fullAccess。 */
  permissionMode: string
  /** 模型（别名 / 全名）；undefined = 用 CLI/配置默认。 */
  model?: string
  /** reasoning effort 档；undefined = 默认。 */
  effort?: string
  /** 该 agent 的进程模型（start 回填）：决定切设置走 restart 还是下轮 flag。 */
  processModel?: ChatProcessModel
  /** 当前**运行中的长驻进程**实际生效的设置（restart 检测用）。one-shot 不看它。 */
  applied?: { permissionMode: string; model?: string; effort?: string }
  /** 前端主动 stop/restart 旧进程时暂时屏蔽那次 exit，避免把新进程会话误标为 ended。 */
  suppressNextExit?: boolean
  /**
   * 待处理的交互式工具权限请求（Claude `--permission-prompt-tool stdio`）。CLI 在工具被
   * 门控时发来，ChatView 据此弹「允许 Claude 运行 X？」对话框。用户应答 / 该轮结束即出队。
   * 通常一次一个，但模型可连发多个工具调用，故用数组（FIFO）。
   */
  pendingPermissions: ChatPermissionRequest[]
  /**
   * 待处理的结构化提问（Claude `AskUserQuestion`，同走 `--permission-prompt-tool stdio`）。
   * 模型提问时 CLI 发来，ChatView 据此弹「选择题」卡片。用户作答 / 取消 / 该轮结束即出队。
   * 同样用数组（模型可一次提多组），但实际通常一次一个。
   */
  pendingQuestions: ChatQuestionRequest[]
  /**
   * 待发消息队列：一轮进行中（或队列非空）时回车不立即发送，而是入队，待本轮 `result`
   * 结束后按 FIFO 逐条出队发送。队列项不进 `msgs`（尚未发出），单独渲染为「待发」行，
   * 发出前可移除；带完整图片 / 文件附件。三种进程模型（长驻 / one-shot）通吃。
   */
  queue: QueuedMessage[]
  /** 已通过 `turn/steer` 提交给当前 Codex turn 的队列项。当前 turn 结束后自动清空。 */
  submittedQueue: QueuedMessage[]
  /**
   * drainQueue 正在发起一条发送（含 restart-with-resume 的异步窗口，此间 turnState 仍 idle）——
   * 守护并发出队，避免同一轮起两条。仅 drainQueue 内部读写，不参与渲染。
   */
  pendingSend?: boolean
  /** 正在等待 app-server 确认的引导项。每项独立，不阻塞用户继续引导其他待发消息。 */
  pendingSteerIds: number[]
}

function claudeApiKeyDisablesEffort(s: Pick<ChatSession, 'agent' | 'apiKeySource'>): boolean {
  return s.agent === 'claude' && typeof s.apiKeySource === 'string' && s.apiKeySource !== '' && s.apiKeySource !== 'none'
}

function sessionEffectiveEffort(s: Pick<ChatSession, 'agent' | 'model' | 'effort' | 'apiKeySource'>): string | undefined {
  if (claudeApiKeyDisablesEffort(s)) return undefined
  return effectiveEffort(s.agent, s.model, s.effort)
}

async function codexRuntimeForAgent(agent: Agent): Promise<CodexRuntimeInfo | null> {
  if (agent !== 'codex') return null
  return api
    .codexRuntimeInfo()
    .catch(() => null)
}

function isFreshCodexChat(opts: StartChatOptions): boolean {
  return opts.agent === 'codex' && !opts.sessionId && !opts.forkSessionId && !opts.fork
}

function initialChatModel(agent: Agent, model: string | undefined, codexRuntime: CodexRuntimeInfo | null): string | undefined {
  if (agent === 'codex' && codexRuntime?.usesApiKey) return model ?? codexRuntime.model
  return sanitizeModel(agent, model) ?? defaultModel(agent)
}

function initialChatEffort(agent: Agent, effort: string | undefined, codexRuntime: CodexRuntimeInfo | null): string | undefined {
  if (agent === 'codex' && codexRuntime?.usesApiKey) return effort ?? codexRuntime.effort
  return effort ?? defaultEffort(agent)
}

function initialLastModel(
  agent: Agent,
  preloadMsgs: Msg[] | undefined,
  codexRuntime: CodexRuntimeInfo | null,
): string | undefined {
  const model = lastAssistantModel(preloadMsgs)
  if (agent === 'codex' && codexRuntime?.usesApiKey) return codexRuntime.model || model
  return sanitizeModel(agent, model)
}

export function chatEffectiveEffortForTest(
  s: Pick<ChatSession, 'agent' | 'model' | 'effort' | 'apiKeySource'>,
): string | undefined {
  return sessionEffectiveEffort(s)
}

export const chatSessions = ref<ChatSession[]>([])
export const activeChatUiId = ref<number | null>(null)
/** 模块级时钟 —— 任一会话 running 时每 250ms 跳一次，驱动「✳ 4s」计时显示。 */
export const now = ref<number>(0)
let nextUiId = 1
let nextQueueId = 1

// ============================ 事件路由 ============================

const sessionsByChatId = new Map<number, ChatSession>()
const pendingByChatId = new Map<number, Array<(s: ChatSession) => void>>()

function routeOrBuffer(chatId: number, apply: (s: ChatSession) => void) {
  const s = sessionsByChatId.get(chatId)
  if (s) {
    apply(s)
    return
  }
  const buf = pendingByChatId.get(chatId) ?? []
  buf.push(apply)
  pendingByChatId.set(chatId, buf)
}

function registerChat(chatId: number, s: ChatSession) {
  sessionsByChatId.set(chatId, s)
  const buf = pendingByChatId.get(chatId)
  if (buf) {
    for (const fn of buf) fn(s)
    pendingByChatId.delete(chatId)
  }
}

const STDERR_TAIL_MAX = 50

// 网络重试/瞬时错误信号（来自 CLI stderr）。命中 → 状态行显示「重试中」。宽松匹配以兼容
// 各 agent / 版本的措辞差异；只在 running 期间生效，且任何「有进展」事件都会清掉。
const RETRY_RE =
  /\b(retry(?:ing)?|reconnect(?:ing)?|overloaded|rate.?limit|request failed|api error|service unavailable|connection (?:error|reset|timed?\s?out)|ECONNRESET|ETIMEDOUT|ENOTFOUND|EAI_AGAIN|socket hang up|fetch failed)\b/i
// 重试次数：兼容「(4/10)」与「4 of 10」两种写法；抓不到则只显示通用「重试中」。
const RETRY_COUNT_RE = /\b(\d+)\s*(?:\/|of)\s*(\d+)\b/i

/**
 * 解析一条 CLI stderr：命中网络重试/瞬时错误信号则返回重试态（带次数填 `attempt`/`max`，
 * 否则空对象 `{}`），未命中返回 `null`。纯函数，导出供测试。
 */
export function parseRetryLine(line: string): { attempt?: number; max?: number } | null {
  if (!RETRY_RE.test(line)) return null
  const m = RETRY_COUNT_RE.exec(line)
  return m ? { attempt: Number(m[1]), max: Number(m[2]) } : {}
}

let listenersInstalled = false
const unlistens: UnlistenFn[] = []

async function ensureListeners(): Promise<void> {
  if (listenersInstalled) return
  listenersInstalled = true
  unlistens.push(
    await listen<ChatEventPayload>('agent-chat://event', (e) =>
      routeOrBuffer(e.payload.chatId, (s) => onMsg(s, e.payload.msg)),
    ),
    await listen<ChatInitPayload>('agent-chat://init', (e) =>
      routeOrBuffer(e.payload.chatId, (s) => onInit(s, e.payload)),
    ),
    await listen<ChatResultPayload>('agent-chat://result', (e) =>
      routeOrBuffer(e.payload.chatId, (s) => onResult(s, e.payload)),
    ),
    await listen<ChatDeltaPayload>('agent-chat://delta', (e) =>
      routeOrBuffer(e.payload.chatId, (s) => onDelta(s, e.payload.delta)),
    ),
    await listen<ChatExitPayload>('agent-chat://exit', (e) =>
      routeOrBuffer(e.payload.chatId, (s) => onExit(s, e.payload)),
    ),
    await listen<ChatStderrPayload>('agent-chat://stderr', (e) =>
      routeOrBuffer(e.payload.chatId, (s) => onStderr(s, e.payload)),
    ),
    await listen<ChatPermissionPayload>('agent-chat://permission', (e) =>
      routeOrBuffer(e.payload.chatId, (s) => onPermission(s, e.payload.request)),
    ),
    await listen<ChatQuestionPayload>('agent-chat://question', (e) =>
      routeOrBuffer(e.payload.chatId, (s) => onQuestion(s, e.payload.request)),
    ),
  )
}

function onMsg(s: ChatSession, msg: Msg) {
  // stream-json 事件没有 JSONL 那样的顶层 timestamp，后端 record_to_msg 给出的是
  // null/undefined → 前端 formatTime(null) 会渲染成「1970-01-01 08:00」。这里补上
  // 「此刻」（消息刚到达的时间），让 live 气泡显示真实时间。
  if (!msg.timestamp) msg.timestamp = new Date().toISOString()
  // Codex 的 item.completed 不带 model 字段 → 用会话当前选中的模型回填，
  // 让气泡显示模型标签（与 read 模式一致）、也让 lastModel 有值。
  if (!msg.model && msg.role === 'assistant' && s.model) msg.model = s.model
  // 记下模型全名（assistant 记录带 model）→ §10.5 上下文窗口换算。
  // `<synthetic>`（本地命令的 synthetic 记录）不是真实模型，跳过 —— 否则 /context、
  // /compact 等会话消息会把底栏模型选择器从真实模型带歪成「未知模型」。
  // 底栏模型选择器读 lastModel 兜底 → 这里也要 sanitize：旧 transcript 的 assistant 记录可能
  // 是已下架的模型（如 gpt-5.3-codex），回放时若原样写进 lastModel 会把选择器带到幽灵模型上。
  // 注意只 sanitize lastModel，不动 msg.model —— 气泡徽标要保留"这条历史消息真实用的模型"。
  if (msg.model && msg.model !== SYNTHETIC_MODEL) s.lastModel = sanitizeModel(s.agent, msg.model)
  updateToolActivity(s, msg)
  // 权威记录到达 → 当前块定稿，清掉流式预览（避免预览与真气泡并存）。
  clearLivePreview(s)
  s.retry = null // 有权威输出 = 网络恢复，撤掉「重试中」。
  if (shouldDropDuplicatePatchOutputForTest(s.msgs, msg)) return
  if (mergeToolUpdate(s, msg)) return
  // stream-json 的每个 assistant / tool_result(user) 事件就是一条完整气泡。
  // **重建数组**（而非 push）：ChatView 的 mermaid / 代码高亮 watcher 按引用比较
  // `props.messages`，只有引用变化才会重跑 —— 与只读模式 reassign chatMsgs 一致。
  s.msgs = [...s.msgs, msg]
}

/** 仅供单测驱动真实消息归并和紧凑工具状态的入口。 */
export function chatOnMsgForTest(s: ChatSession, msg: Msg) {
  onMsg(s, msg)
}

function toolForResult(s: ChatSession, msg: Msg, toolId?: string): Block | undefined {
  if (toolId) {
    for (let bi = msg.blocks.length - 1; bi >= 0; bi -= 1) {
      const b = msg.blocks[bi]
      if (b.kind === 'tool_use' && b.toolId === toolId) return b
    }
    for (let mi = s.msgs.length - 1; mi >= 0; mi -= 1) {
      const prior = s.msgs[mi]
      for (let bi = prior.blocks.length - 1; bi >= 0; bi -= 1) {
        const b = prior.blocks[bi]
        if (b.kind === 'tool_use' && b.toolId === toolId) return b
      }
    }
  }
  return undefined
}

function sameToolActivity(a: LiveToolActivity, b: LiveToolActivity): boolean {
  if (a.toolId || b.toolId) return !!a.toolId && a.toolId === b.toolId
  return a.toolName === b.toolName
}

function addActiveTool(s: ChatSession, activity: LiveToolActivity): void {
  s.toolActivities = [
    ...s.toolActivities.filter((item) => !sameToolActivity(item, activity)),
    activity,
  ]
}

function removeActiveTool(s: ChatSession, activity: LiveToolActivity): void {
  s.toolActivities = s.toolActivities.filter((item) => !sameToolActivity(item, activity))
}

/** 记录 tool_use / tool_result，让 GUI 在隐藏工具卡片后仍能说明正在执行什么。
 *  不尝试伪造命令内部百分比：协议能可靠提供的只有 calling / result / failed 三种阶段。 */
function updateToolActivity(s: ChatSession, msg: Msg) {
  let sawToolEvent = false
  for (const b of msg.blocks) {
    if (b.kind === 'tool_use') {
      sawToolEvent = true
      const activity: LiveToolActivity = {
        toolName: b.toolName || 'Tool',
        ...(b.toolId ? { toolId: b.toolId } : {}),
        summary: summarizeTool(b.toolName, b.toolInput, b.filePath),
        phase: 'calling',
        updatedAt: Date.now(),
      }
      addActiveTool(s, activity)
      s.toolActivity = activity
      continue
    }
    if (b.kind === 'tool_result') {
      sawToolEvent = true
      const source = toolForResult(s, msg, b.toolId)
      const toolName = b.toolName || source?.toolName || 'Tool'
      const activity: LiveToolActivity = {
        toolName,
        ...(b.toolId ? { toolId: b.toolId } : {}),
        summary: summarizeTool(toolName, source?.toolInput, b.filePath || source?.filePath),
        phase: b.isError ? 'failed' : 'result',
        updatedAt: Date.now(),
      }
      removeActiveTool(s, activity)
      s.toolActivity = activity
    }
  }
  // 模型重新开始输出 prose / thinking 后，结束仍在调用中的旧状态；完成/失败反馈交给
  // ChatView 的短时效逻辑保留一小段时间，避免结果文案刚到就被下一条文本吞掉。
  if (
    !sawToolEvent
    && s.toolActivity?.phase === 'calling'
    && msg.blocks.some((b) => b.kind === 'text' || b.kind === 'thinking')
  ) {
    s.toolActivities = []
    s.toolActivity = null
  }
}

function messageText(msg: Msg): string {
  return msg.blocks
    .filter((b) => b.kind === 'text' && b.text)
    .map((b) => b.text!)
    .join('\n')
}

function looksLikeCodexPlanMessage(msg: Msg): boolean {
  if (msg.role !== 'assistant') return false
  if (!msg.blocks.length) return false
  if (msg.blocks.some((b) => b.kind !== 'text')) return false
  const text = messageText(msg).trim()
  if (!text) return false
  return (
    /proposed\s+plan/i.test(text) ||
    /(?:^|\n)##\s+Summary\b/i.test(text) ||
    /(?:^|\n)##\s+Test\s+Plan\b/i.test(text) ||
    /(?:^|\n)\s*-\s+\[[ x]\]\s+/.test(text)
  )
}

function latestCodexPlanMessageIndex(s: ChatSession): number {
  for (let i = s.msgs.length - 1; i >= 0; i -= 1) {
    const msg = s.msgs[i]
    if (looksLikeCodexPlanMessage(msg)) return i
    if (msg.role === 'user' && !msg.metaKind) break
  }
  return -1
}

function maybePromptForCodexPlanImplementation(s: ChatSession): void {
  if (s.agent !== 'codex' || s.permissionMode !== 'plan') return
  if (s.pendingQuestions.length) return
  const planIndex = latestCodexPlanMessageIndex(s)
  if (planIndex < 0) return
  const requestId = `${LOCAL_CODEX_PLAN_REQUEST_PREFIX}${s.uiId}-${planIndex}`
  s.pendingQuestions = [
    ...s.pendingQuestions,
    {
      requestId,
      keepAfterTurn: true,
      localCodexPlanPrompt: true,
      questions: [{
        header: 'Plan',
        question: 'Implement this plan?',
        allowOther: false,
        options: [
          { label: 'Yes, implement this plan', description: 'Switch to coding.' },
          { label: 'No, stay in Plan mode', description: 'Keep planning.' },
        ],
      }],
    },
  ]
}

export function shouldDropDuplicatePatchOutputForTest(existing: Msg[], incoming: Msg): boolean {
  if (!incoming.blocks.length) return false
  if (!incoming.blocks.every((b) => b.kind === 'tool_result')) return false
  if (!incoming.blocks.every(isPlainPatchSuccessResult)) return false
  return existing
    .slice(-8)
    .some((msg) => msg.blocks.some((b) => b.kind === 'tool_result' && isFileChangeResult(b)))
}

function isPlainPatchSuccessResult(b: Block): boolean {
  if (isFileChangeResult(b)) return false
  const text = b.text ?? ''
  if (!text.includes('Success. Updated the following files:')) return false
  return /(?:^|\n)\s*[AMD]\s+.+/.test(text)
}

function mergeToolUpdate(s: ChatSession, msg: Msg): boolean {
  if (msg.role !== 'assistant') return false
  const incomingToolIds = new Set(
    msg.blocks
      .filter((b) => (b.kind === 'tool_use' || b.kind === 'tool_result') && b.toolId)
      .map((b) => b.toolId!),
  )
  if (!incomingToolIds.size) return false

  for (let i = s.msgs.length - 1; i >= 0; i -= 1) {
    const existing = s.msgs[i]
    if (existing.role !== 'assistant') continue
    const hasSameTool = existing.blocks.some(
      (b) => (b.kind === 'tool_use' || b.kind === 'tool_result') && b.toolId && incomingToolIds.has(b.toolId),
    )
    if (!hasSameTool) continue

    const retainedBlocks = existing.blocks.filter(
      (b) => !((b.kind === 'tool_use' || b.kind === 'tool_result') && b.toolId && incomingToolIds.has(b.toolId)),
    )
    const merged: Msg = {
      ...existing,
      model: msg.model ?? existing.model,
      timestamp: existing.timestamp ?? msg.timestamp,
      blocks: [...retainedBlocks, ...msg.blocks],
    }
    s.msgs = [
      ...s.msgs.slice(0, i),
      merged,
      ...s.msgs.slice(i + 1),
    ]
    return true
  }
  return false
}

/**
 * token 级流式增量（Claude / Codex）。只对 `text` 块做打字机预览（thinking / tool_use 块
 * 不预览 —— 交给随后的权威 assistant 记录定稿）。只动 `s.live` 这个小对象，不碰 `s.msgs`，
 * 故不会触发整列表重渲染。delta 先在非响应式缓冲区合并，每 50ms 最多提交一次，避免
 * token 频率直接变成 Markdown 重算、DOM 替换和自动滚动频率。
 */
const LIVE_DELTA_FLUSH_MS = 50
const pendingLiveDeltas = new WeakMap<ChatSession, { text: string; timer: number }>()

function cancelPendingLiveDelta(s: ChatSession) {
  const pending = pendingLiveDeltas.get(s)
  if (!pending) return
  window.clearTimeout(pending.timer)
  pendingLiveDeltas.delete(s)
}

function flushPendingLiveDelta(s: ChatSession) {
  const pending = pendingLiveDeltas.get(s)
  if (!pending) return
  window.clearTimeout(pending.timer)
  pendingLiveDeltas.delete(s)
  if (!pending.text) return
  const prev = s.live ?? { kind: 'text' as const, text: '' }
  s.live = { kind: 'text', text: prev.text + pending.text }
}

function queueLiveDelta(s: ChatSession, text: string) {
  const pending = pendingLiveDeltas.get(s)
  if (pending) {
    pending.text += text
    return
  }
  pendingLiveDeltas.set(s, {
    text,
    timer: window.setTimeout(() => flushPendingLiveDelta(s), LIVE_DELTA_FLUSH_MS),
  })
}

function clearLivePreview(s: ChatSession) {
  cancelPendingLiveDelta(s)
  s.live = null
}

function onDelta(s: ChatSession, d: ChatDelta) {
  s.retry = null // 收到 token 流 = 有进展，撤掉「重试中」。
  if (d.phase === 'start') {
    cancelPendingLiveDelta(s)
    // 仅文本块起预览；thinking / tool_use 不预览（authoritative 记录会补）。
    s.live = d.kind === 'text' ? { kind: 'text', text: '' } : null
    if (d.kind === 'text' && s.toolActivity?.phase === 'calling') {
      s.toolActivities = []
      s.toolActivity = null
    }
  } else if (d.phase === 'delta') {
    if (d.kind === 'text' && d.text) {
      queueLiveDelta(s, d.text)
    }
  } else if (d.phase === 'stop') {
    // 权威消息若稍晚到达，先把尾部 delta 完整展示出来；onMsg 随后负责清空预览。
    flushPendingLiveDelta(s)
  }
}

/** 仅供单测验证 delta 合并与收尾。 */
export function chatOnDeltaForTest(s: ChatSession, d: ChatDelta) {
  onDelta(s, d)
}

function onInit(s: ChatSession, p: ChatInitPayload) {
  if (p.sessionId && s.sessionId !== p.sessionId) {
    s.sessionId = p.sessionId
    markProjectsDirty()
  }
  // 只认权威 init 给的字符串 apiKeySource（'none' / 'ANTHROPIC_API_KEY' / …）。Claude 的
  // 同 `system` 类型还会发 hook_started / thinking_tokens 等事件，它们没有 apiKeySource
  // （→ null）；若用 `!== undefined` 判断会被这些 null 覆盖回去，导致订阅模式被误判成 API
  // key 而隐藏限额角标。故只在拿到真实字符串时才写入，null/undefined 一律忽略。
  if (typeof p.apiKeySource === 'string' && p.apiKeySource) {
    s.apiKeySource = p.apiKeySource
    if (claudeApiKeyDisablesEffort(s) && s.effort !== undefined) {
      s.effort = undefined
    }
  }
  if (s.status === 'spawning') s.status = 'running'
}

function onResult(s: ChatSession, p: ChatResultPayload) {
  if (p.usage) s.usage = p.usage
  clearLivePreview(s) // 一轮结束，兜底清掉残留预览。
  s.lastTurnOutcome = p.ok === false ? 'failed' : 'completed'
  endTurn(s)
  // 一轮结束 → 账号 5h/周额度刚被这次对话消耗、值会变 → 事件驱动强制刷新（慢轮询之外的实时补位）。
  bumpUsage()
  maybePromptForCodexPlanImplementation(s)
  // 本轮结束 → 若有待发消息，按序发下一条（type-while-running 队列）。
  if (!s.pendingQuestions.some((q) => q.keepAfterTurn)) drainQueue(s)
}

export function chatOnResultForTest(s: ChatSession, p: ChatResultPayload) {
  onResult(s, p)
}

function onExit(s: ChatSession, p: ChatExitPayload) {
  if (s.suppressNextExit) {
    s.suppressNextExit = false
    return
  }
  clearLivePreview(s)
  s.lastTurnOutcome = p.code === 0 ? 'completed' : 'failed'
  endTurn(s)
  if (p.code !== 0 && !s.errorMessage) {
    s.errorMessage = s.stderrTail.slice(-3).join('\n') || `exited (${p.code})`
  }
  // 自动重启：进程退出后尝试 restart-with-resume，保持 chat 可用。
  void autoRestart(s)
}

const restartCooldown = new WeakMap<ChatSession, number>()
async function autoRestart(s: ChatSession) {
  const now = Date.now()
  const last = restartCooldown.get(s) ?? 0
  if (now - last < 3000) {
    if (s.status !== 'error') s.status = 'exited'
    return
  }
  restartCooldown.set(s, now)
  const old = s.chatId
  if (old !== null) {
    sessionsByChatId.delete(old)
    pendingByChatId.delete(old)
  }
  try {
    const eff = sessionEffectiveEffort(s)
    const info = await api.agentChatStart(
      s.agent,
      s.projectKey,
      s.cwd,
      s.sessionId || undefined,
      s.permissionMode,
      s.model,
      eff,
      undefined,
      useReclaude.value,
    )
    s.chatId = info.chatId
    s.processModel = info.processModel
    s.applied = { permissionMode: s.permissionMode, model: s.model, effort: eff }
    s.errorMessage = undefined
    s.status = 'running'
    registerChat(info.chatId, s)
    drainQueue(s)
  } catch {
    if (s.status !== 'error') s.status = 'exited'
  }
}

function onStderr(s: ChatSession, p: ChatStderrPayload) {
  s.stderrTail.push(p.line)
  if (s.stderrTail.length > STDERR_TAIL_MAX) {
    s.stderrTail.splice(0, s.stderrTail.length - STDERR_TAIL_MAX)
  }
  // 网络不稳重试：CLI 把退避/瞬时错误写到 stderr。running 期间命中就置「重试中」，
  // 下一条 event/delta/result（有进展）会把它清掉。
  if (s.turnState === 'running') {
    const r = parseRetryLine(p.line)
    if (r) s.retry = r
  }
}

/** 交互式工具权限请求到达 —— 入队让 ChatView 弹框。同 requestId 重复到达去重（幂等）。 */
function onPermission(s: ChatSession, request: ChatPermissionRequest) {
  if (s.pendingPermissions.some((p) => p.requestId === request.requestId)) return
  s.pendingPermissions = [...s.pendingPermissions, request]
}

/** 结构化提问到达 —— 入队让 ChatView 弹选择题卡片。同 requestId 重复到达去重（幂等）。 */
function onQuestion(s: ChatSession, request: ChatQuestionRequest) {
  if (s.pendingQuestions.some((q) => q.requestId === request.requestId)) return
  s.pendingQuestions = [...s.pendingQuestions, request]
}

function appendInterruptedMarker(s: ChatSession) {
  s.msgs = [
    ...s.msgs,
    {
      role: 'user',
      sidechain: false,
      timestamp: new Date().toISOString(),
      blocks: [{ kind: 'text', text: '[Request interrupted by user]', isError: false }],
    },
  ]
}

// ============================ 计时器 ============================

let tick: number | null = null
function ensureTicking() {
  if (tick !== null) return
  now.value = Date.now()
  tick = window.setInterval(() => {
    now.value = Date.now()
    // 没有任何 running 会话时自动停表，省得空转。
    if (!chatSessions.value.some((c) => c.turnState === 'running')) {
      if (tick !== null) {
        clearInterval(tick)
        tick = null
      }
    }
  }, 250)
}

function startTurn(s: ChatSession) {
  s.turnState = 'running'
  s.turnStartedAt = Date.now()
  s.retry = null // 新一轮重置重试态。
  s.toolActivity = null
  s.toolActivities = []
  s.lastTurnOutcome = null
  s.pendingPermissions = [] // 新一轮不带上一轮残留的权限请求。
  s.pendingQuestions = [] // 同理，残留的提问也清掉。
  now.value = Date.now()
  ensureTicking()
}

function endTurn(s: ChatSession) {
  if (s.turnState === 'running') {
    s.lastTurnMs = Date.now() - s.turnStartedAt
  }
  s.turnState = 'idle'
  s.retry = null // 一轮结束撤掉「重试中」。
  // 一轮结束（含 result / exit / 中断 / 清屏）→ 大多数未应答请求已失效；Codex app-server
  // requestUserInput 例外，它可能在 plan turn completed 后继续等待用户确认。
  if (s.pendingPermissions.length) s.pendingPermissions = []
  if (s.pendingQuestions.length) {
    const kept = s.pendingQuestions.filter((q) => q.keepAfterTurn)
    if (kept.length !== s.pendingQuestions.length) s.pendingQuestions = kept
  }
  // 已引导的消息属于当前 turn；无论完成、失败还是中断，turn 结束后都不再显示为“已提交”。
  if (s.submittedQueue.length) s.submittedQueue = []
}

// ============================ 消息队列 ============================
// type-while-running：一轮进行中时回车把消息入队，待本轮 result 结束后按序逐条发出
//（对齐 Claude CLI 的消息队列）。纯前端实现、后端零改动：sendPrompt 仍是「发一条」原语，
// 队列只决定何时调它，故长驻（Claude）与 one-shot（Codex）通吃。

/** 会话是否可发送（进程在、未退出 / 未出错）。 */
function chatUsable(s: ChatSession): boolean {
  return s.chatId !== null && s.status !== 'exited' && s.status !== 'error'
}

/**
 * 入队 / 直发一条用户消息（可带图片 + 文件附件）。统一入口：ChatComposer 回车走这里。
 * 空闲且无待发时 drainQueue 会立即发出（= 现状的即时发送）；正在生成 / 队列非空时则排队，
 * 待本轮 `result` 结束后按 FIFO 逐条发出。空消息（无文本 / 图片 / 文件）忽略。
 */
export function enqueuePrompt(
  session: ChatSession,
  text: string,
  images: ChatImageAttachment[] = [],
  files: ChatFileAttachment[] = [],
): void {
  if (!text.trim() && images.length === 0 && files.length === 0) return
  if (!chatUsable(session)) return
  session.queue = [
    ...session.queue,
    { id: nextQueueId++, text, images: [...images], files: [...files] },
  ]
  drainQueue(session)
}

/**
 * 出队下一条并发送 —— 入队后 / 每轮 `result` 结束后调用。仅当会话空闲、无正在发起的发送、
 * 队列非空且进程健康时才发；发出即进入下一轮（startTurn）。`pendingSend` 守护
 * restart-with-resume 的异步窗口（此间 turnState 仍 idle），避免同一轮并发起两条。
 */
function drainQueue(session: ChatSession): void {
  if (session.turnState !== 'idle' || session.pendingSend) return
  if (session.queue.length === 0 || !chatUsable(session)) return
  const [next, ...rest] = session.queue
  session.queue = rest
  session.pendingSend = true
  void sendPrompt(session, next.text, next.images, next.files).finally(() => {
    session.pendingSend = false
  })
}

/** 移除一条待发消息（用户在待发列表点 ×）。 */
export function removeQueued(session: ChatSession, id: number): void {
  if (session.queue.some((q) => q.id === id)) {
    session.queue = session.queue.filter((q) => q.id !== id)
  }
}

/** 当前会话是否能把一条待发消息追加给正在运行的 Codex app-server turn。 */
export function canSteerQueued(session: ChatSession, id?: number): boolean {
  return (
    canSteerCurrentTurn(session) &&
    session.queue.length > 0 &&
    (id === undefined || !session.pendingSteerIds.includes(id))
  )
}

/** `steerQueued` 已占用提交锁后，用此检查确认当前 turn 在异步准备附件期间仍可接收引导。 */
function canSteerCurrentTurn(session: ChatSession): boolean {
  return (
    session.agent === 'codex' &&
    session.processModel === 'codexAppServer' &&
    session.turnState === 'running' &&
    session.status === 'running' &&
    session.chatId !== null &&
    session.pendingPermissions.length === 0 &&
    session.pendingQuestions.length === 0 &&
    !settingsChanged(session)
  )
}

const DEFERRED_STEER_PREFIX = [
  '[Deferred follow-up from the user]',
  'Continue and fully complete the original task already in progress first.',
  'Do not interrupt it, change its direction, or begin the follow-up below yet.',
  'Only after the original task is complete, process this follow-up in the order received:',
  '<follow-up>',
].join('\n') + '\n'
const DEFERRED_STEER_SUFFIX = '\n</follow-up>'

/** `turn/steer` 原生语义会立刻重定向模型；将队列项标为延后 follow-up，保留当前任务优先级。 */
function deferredSteerPayload(payload: { sendText: string; textElements: ChatTextElement[] }) {
  const offset = new TextEncoder().encode(DEFERRED_STEER_PREFIX).length
  return {
    text: DEFERRED_STEER_PREFIX + payload.sendText + DEFERRED_STEER_SUFFIX,
    textElements: payload.textElements.map((element) => ({
      ...element,
      byteRange: {
        start: element.byteRange.start + offset,
        end: element.byteRange.end + offset,
      },
    })),
  }
}

/**
 * 把指定待发项主动引导至当前 Codex turn。app-server 接受前绝不移除队列项，因此请求失败后
 * 它仍会按原顺序等待本轮结束。传给 `turn/steer` 的是延后 follow-up，不会抢占当前任务。
 */
export async function steerQueued(session: ChatSession, id: number): Promise<void> {
  if (!canSteerQueued(session, id)) return
  const queued = session.queue.find((q) => q.id === id)
  if (!queued) return

  session.pendingSteerIds = [...session.pendingSteerIds, id]
  try {
    const payload = await preparePromptPayload(session, queued.text, queued.images, queued.files)
    // 图片临时落盘期间当前 turn 可能已结束；让队列留在原处，等待正常 FIFO 路径处理。
    if (!canSteerCurrentTurn(session) || !session.queue.some((q) => q.id === id)) return
    const steer = deferredSteerPayload(payload)
    await api.agentChatSteer(session.chatId!, steer.text, steer.textElements)

    session.queue = session.queue.filter((q) => q.id !== id)
    session.submittedQueue = [...session.submittedQueue, queued]
    appendLocalUserMessage(session, queued.text, queued.images, queued.files)
  } catch {
    // 仅 app-server 明确接受后才迁移队列项；失败时保持原队列，交给本轮完成后的 FIFO 处理。
  } finally {
    session.pendingSteerIds = session.pendingSteerIds.filter((pendingId) => pendingId !== id)
  }
}

/** 清空待发队列（中断 / 清屏 / 停止 / 进程退出时调用 —— 「停就是停」，可预测）。 */
function clearQueue(session: ChatSession): void {
  if (session.queue.length) session.queue = []
  if (session.submittedQueue.length) session.submittedQueue = []
  if (session.pendingSteerIds.length) session.pendingSteerIds = []
}

// ============================ 查找 ============================

export function findChatByUiId(uiId: number): ChatSession | null {
  return chatSessions.value.find((c) => c.uiId === uiId) ?? null
}

export function activeChat(): ChatSession | null {
  return activeChatUiId.value === null ? null : findChatByUiId(activeChatUiId.value)
}

/**
 * 合成 key（worktree:/bookmark:）被并入真实项目时，把仍挂在旧 key 上的 live chat 的 projectKey
 * 迁到新 key。两个作用：
 *  1. 后续 restart / interrupt / clear / fork / sideChat 传给后端的是真实 key，而非已失效的合成 key；
 *  2. projectKey 是响应式字段，ChatView watch 它的变化 → 迁移后强制重测虚拟列表几何，修复
 *     「worktree 首轮渲染空白、要再发一次才出来」（迁移中途 reflow 使虚拟器缓存的滚动几何过期）。
 */
export function migrateChatSessionsProjectKey(oldKey: string, newKey: string): void {
  if (oldKey === newKey) return
  for (const c of chatSessions.value) {
    if (c.projectKey === oldKey) c.projectKey = newKey
  }
}

/** 已为某 sessionPath（续聊源）开过的 live chat —— 入口 2/3 复用，避免重复开。 */
export function findChatBySourceSession(agent: Agent, sessionId: string): ChatSession | null {
  if (!sessionId) return null
  return (
    chatSessions.value.find((c) => c.agent === agent && c.sessionId === sessionId) ?? null
  )
}

// ============================ 开 / 发 / 停 ============================

export interface StartChatOptions {
  agent: Agent
  projectKey: string
  cwd: string
  /** 续聊既有会话时给出；新开会话留空（init 事件回填）。 */
  sessionId?: string
  /**
   * 侧聊 fork 的源会话。它只传给后端，不能写进新 ChatSession 的 `sessionId`，否则在
   * app-server 回传新 thread id 前会错误地把主会话当成侧聊。
   */
  forkSessionId?: string
  /** 侧聊：从 `forkSessionId`（或 `sessionId`）派生独立会话。 */
  fork?: boolean
  /** Codex 专用：不将新 thread materialize 到磁盘。 */
  ephemeral?: boolean
  title: string
  /** 续聊既有会话时传原会话的 created；新开留空（startChat 用当前时刻）。 */
  created?: string
  permissionMode?: string
  /** 初始模型 / effort（可选）；缺省走 CLI 默认。 */
  model?: string
  effort?: string
  /** 续聊种子：原会话末尾的上下文用量，给上下文进度角标兜底，避免刚切过去显示 0%。
   *  首个 result 事件到达后会被真实 usage 覆盖。 */
  initialUsage?: UsageSummary
  /** 续聊既有会话时，预载该会话已有的消息当作历史 transcript 显示。
   *  `--resume` 只在后端续上下文、不会把历史作为事件重放，所以前端必须自己预载，
   *  否则切到 chat 后会是一片空白。新开会话留空。 */
  preloadMsgs?: Msg[]
  /** 新分支首次显示时回填到输入框的一次性草稿。 */
  initialDraft?: ChatHistoryEntry
  /** 开起来立刻发的第一句（可选）。 */
  initialPrompt?: string
  initialImages?: ChatImageAttachment[]
  /** session 对象一建好（已入列、status='spawning'）就同步回调，早于 `agentChatStart` 的
   *  await —— 调用方据此**立刻**把 chat tab 显示出来，不必等后端进程握手完成。Codex 走
   *  app-server，握手要几秒，若等 await 完再建 tab，右键新建后要干等一会才出来。session 是
   *  reactive，spawning→running 会自动反映到已显示的 ChatView。 */
  onReady?: (session: ChatSession) => void
}

/** 预载 transcript 末尾那条带 model 的 assistant 记录的模型全名（续聊时回填 lastModel）。 */
export function lastAssistantModel(msgs: Msg[] | undefined): string | undefined {
  if (!msgs) return undefined
  for (let i = msgs.length - 1; i >= 0; i--) {
    const m = msgs[i]
    // `<synthetic>` 是本地命令的伪模型，不是真实续聊模型 —— 跳过，继续往前找真实的那条。
    if (m.role === 'assistant' && m.model && m.model !== SYNTHETIC_MODEL) return m.model
  }
  return undefined
}

function normalizeRestoredMessages(
  msgs: Msg[] | undefined,
  fallbackModel?: string,
): Msg[] {
  const restoredAt = new Date().toISOString()
  return (msgs ?? []).map((msg) => {
    const normalized: Msg = { ...msg, blocks: msg.blocks ? [...msg.blocks] : [] }
    if (!normalized.timestamp) normalized.timestamp = restoredAt
    if (!normalized.model && normalized.role === 'assistant' && fallbackModel) {
      normalized.model = fallbackModel
    }
    return normalized
  })
}

/**
 * 起一个 GUI chat 会话：建 reactive session → 装 listener（若未装）→ `agentChatStart`
 * 拿 chatId → 注册路由 → 可选发首条消息。失败时 status='error'，会话仍留在列表里。
 */
export async function startChat(opts: StartChatOptions): Promise<ChatSession> {
  await ensureListeners()

  const uiId = nextUiId++
  const codexRuntime = isFreshCodexChat(opts) ? await codexRuntimeForAgent(opts.agent) : null
  const initialPermissionMode = opts.permissionMode ?? rememberedChatPermissionMode(opts.agent)
  const initialModel = initialChatModel(opts.agent, opts.model ?? rememberedChatModel(opts.agent), codexRuntime)
  const initialEffort = initialChatEffort(
    opts.agent,
    opts.effort ?? rememberedChatEffort(opts.agent, initialModel),
    codexRuntime,
  )
  const session = reactive<ChatSession>({
    uiId,
    chatId: null,
    agent: opts.agent,
    projectKey: opts.projectKey,
    cwd: opts.cwd,
    sessionId: opts.sessionId ?? '',
    title: opts.title,
    createdAt: opts.created ?? new Date().toISOString(),
    msgs: opts.preloadMsgs ? [...opts.preloadMsgs] : [],
    initialDraft: opts.initialDraft
      ? {
          text: opts.initialDraft.text,
          images: opts.initialDraft.images.map((image) => ({ ...image })),
          files: opts.initialDraft.files.map((file) => ({ ...file })),
        }
      : undefined,
    turnState: 'idle',
    turnStartedAt: 0,
    lastTurnMs: 0,
    status: 'spawning',
    queue: [],
    submittedQueue: [],
    pendingSteerIds: [],
    stderrTail: [],
    retry: null,
    live: null,
    toolActivity: null,
    toolActivities: [],
    lastTurnOutcome: null,
    pendingPermissions: [],
    pendingQuestions: [],
    permissionMode: initialPermissionMode,
    // 官方模型菜单下：会话起步带一个明确模型 + effort（用户可改）。
    // Codex 自定义 provider：用 ~/.codex/config.toml 的 model/effort 初始化 GUI，确保
    // UI、app-server 参数、全局配置三者一致；没有配置值时才保持 undefined。
    // sanitizeModel：旧会话记忆的模型可能已不在菜单里（如 gpt-5.3-codex），回退到该 agent
    // 的兜底；自定义 provider 保留原模型名，让反代/别名配置自行接管。
    model: initialModel,
    effort: initialEffort,
    // 续聊：从预载 transcript 末尾的 assistant 记录回填 lastModel，让模型在「进会话即显」
    // （而非等首轮回复后才由 onMsg 填上）。effort 不在 transcript 里 → 无法同样回填，
    // 由 composer 用运行时 settings.effortLevel 兜底显示真实生效默认。
    lastModel: initialLastModel(opts.agent, opts.preloadMsgs, codexRuntime),
    // 续聊种子：原会话末尾上下文规模，首个 result 到达前给进度角标兜底。
    usage: opts.initialUsage,
  }) as ChatSession
  chatSessions.value.push(session)
  activeChatUiId.value = uiId
  // tab 立刻可见：在 await 后端握手之前就把 reactive session 交回调用方建 tab。
  opts.onReady?.(session)

  try {
    // Haiku 等不支持 effort 的模型省掉 --effort（effectiveEffort → undefined）。
    const eff = sessionEffectiveEffort(session)
    const info = await api.agentChatStart(
      opts.agent,
      opts.projectKey,
      opts.cwd,
      opts.forkSessionId ?? opts.sessionId,
      session.permissionMode,
      session.model,
      eff,
      opts.fork,
      useReclaude.value,
      opts.preloadMsgs,
      opts.title,
      opts.ephemeral,
    )
    session.chatId = info.chatId
    session.processModel = info.processModel
    // 记下这套进程实际起在哪个设置上（restart 检测基线）。
    session.applied = {
      permissionMode: session.permissionMode,
      model: session.model,
      effort: eff,
    }
    registerChat(info.chatId, session)
    if (session.status === 'spawning') session.status = 'running'

    if (opts.initialPrompt || (opts.initialImages && opts.initialImages.length)) {
      await sendPrompt(session, opts.initialPrompt ?? '', opts.initialImages ?? [])
    }
  } catch (err) {
    session.status = 'error'
    session.errorMessage = String(err)
  }
  return session
}

/** 发送一条用户消息：本地立即回显成一条 user 气泡 → 置 running → 写进子进程 stdin。 */
export async function sendPrompt(
  session: ChatSession,
  text: string,
  images: ChatImageAttachment[] = [],
  files: ChatFileAttachment[] = [],
): Promise<void> {
  const trimmed = text.trim()
  if (!trimmed && images.length === 0 && files.length === 0) return
  if (session.chatId === null || session.status === 'exited' || session.status === 'error') {
    return
  }

  const { sendImages, sendText, textElements } = await preparePromptPayload(session, text, images, files)

  appendLocalUserMessage(session, text, images, files)

  // 长驻进程 / Codex app-server：模型/effort/权限在当前进程或 thread 上有粘性，若用户改了
  // 就先 restart-with-resume 换新实例再发；one-shot 才能靠本轮 agentChatSend 直接生效。
  if (requiresRestartForSettings(session) && settingsChanged(session)) {
    const ok = await restartChat(session)
    if (!ok) return // restart 失败：status 已置 error
  }

  const chatId = session.chatId
  if (chatId === null) return // restart 兜底：进程没起来就别发

  startTurn(session)
  try {
    await api.agentChatSend(
      chatId,
      sendText,
      sendImages.map((i) => ({ mediaType: i.mediaType, data: i.data })),
      session.model,
      sessionEffectiveEffort(session),
      session.permissionMode,
      textElements,
    )
  } catch (err) {
    session.lastTurnOutcome = 'failed'
    endTurn(session)
    session.status = 'error'
    session.errorMessage = String(err)
  }
}

/** 按当前 agent 的输入协议生成发送/引导共用的 app-server payload。 */
async function preparePromptPayload(
  session: ChatSession,
  text: string,
  images: ChatImageAttachment[],
  files: ChatFileAttachment[],
): Promise<{ sendImages: ChatImageAttachment[]; sendText: string; textElements: ChatTextElement[] }> {
  const trimmed = text.trim()
  const isStdinAgent = session.processModel === 'longLivedStdin'
  const isCodexAppServer = session.processModel === 'codexAppServer'
  let sendImages: ChatImageAttachment[] = images
  let sendText: string
  let textElements: ChatTextElement[] = []

  if (isStdinAgent) {
    // Claude（LongLived / stdin）：文件/文件夹用 @"path"，图片走 base64 参数。
    const refs = files.map((f) => `@"${f.path}"`).join(' ')
    sendText = [trimmed, refs].filter(Boolean).join(trimmed && refs ? ' ' : '')
  } else {
    // Codex（OneShot / AppServer）：客户端构造 Codex 专用消息格式，不依赖 server 解析 @"path"。
    //   文件/图片 → # Files mentioned by the user: 结构（server 会自动注入到 context）
    //   文件夹   → [name](path/) markdown 链接（内联到正文）
    //   图片     → 额外传 base64 给后端，由 codex_turn_params 放入 input_image
    const codexPrompt = isCodexAppServer ? trimmed : expandCodexPluginMentionsForPrompt(trimmed)
    const mentionedFiles: { name: string; path: string }[] = []
    const folderRefs: string[] = []

    for (const f of files) {
      if (f.isDir) {
        const name = f.name.replace(/[/\\]+$/, '').split(/[/\\]/).pop() || f.name
        const trailingSlash = f.path.endsWith('/') ? '' : '/'
        folderRefs.push(`[${name}](${f.path}${trailingSlash})`)
      } else {
        mentionedFiles.push({ name: f.name, path: f.path })
      }
    }

    // 图片：有 sourcePath 直接用；粘贴板截图存临时文件取路径。
    for (const img of images) {
      if (img.sourcePath) {
        mentionedFiles.push({ name: img.name || img.sourcePath.split('/').pop() || 'image', path: img.sourcePath })
      } else {
        try {
          const saved = await api.saveTempImage(img.data, img.mediaType)
          mentionedFiles.push({ name: saved.split('/').pop() || 'image', path: saved })
        } catch { /* 存盘失败 */ }
      }
    }

    if (mentionedFiles.length > 0) {
      // 构造 # Files mentioned 结构（与 Codex 官方客户端一致）
      let header = '\n# Files mentioned by the user:\n\n'
      for (const f of mentionedFiles) {
        header += `## ${f.name}: ${f.path}\n\n`
      }
      header += `## My request for Codex:\n`
      const userParts = [codexPrompt, ...folderRefs].filter(Boolean).join(' ')
      sendText = header + userParts + '\n'
      if (isCodexAppServer) textElements = codexPluginMentionTextElements(trimmed, new TextEncoder().encode(header).length)
    } else {
      sendText = [codexPrompt, ...folderRefs].filter(Boolean).join(' ')
      if (isCodexAppServer) textElements = codexPluginMentionTextElements(trimmed)
    }

    // Codex app-server 自己从 # Files mentioned 的路径读取文件/图片，不需要传 base64。
    sendImages = []
  }

  return { sendImages, sendText, textElements }
}

/** 本地立即回显一条已提交的用户消息（普通发送与引导共用）。 */
function appendLocalUserMessage(
  session: ChatSession,
  text: string,
  images: ChatImageAttachment[],
  files: ChatFileAttachment[],
): void {
  const trimmed = text.trim()
  const blocks: Block[] = []
  for (const img of images) {
    blocks.push({ kind: 'image', imageSrc: img.dataUrl, isError: false })
  }
  for (const f of files) {
    blocks.push({ kind: 'file', filePath: f.path, isDir: f.isDir, isError: false })
  }
  if (trimmed) {
    blocks.push({ kind: 'text', text: trimmed, isError: false })
  }
  // 重建数组（理由同 onMsg）；带上「此刻」时间戳，否则 user 气泡时间显示成「—」。
  session.msgs = [
    ...session.msgs,
    { role: 'user', sidechain: false, blocks, timestamp: new Date().toISOString() },
  ]
}

/** 运行中的进程/thread 实际生效的设置，与当前选择是否已不一致（需 restart 才能换）。 */
function settingsChanged(s: ChatSession): boolean {
  const a = s.applied
  if (!a) return false
  return (
    a.permissionMode !== s.permissionMode ||
    a.model !== s.model ||
    a.effort !== sessionEffectiveEffort(s)
  )
}

function requiresRestartForSettings(s: ChatSession): boolean {
  return s.processModel === 'longLivedStdin' || s.processModel === 'codexAppServer'
}

/**
 * §10.0 restart-with-resume：停掉旧长驻进程/app-server，用当前 model/effort/权限重起一个
 * `--resume` 既有 session 的新实例，热替换 `chatId` 并重注册路由（`msgs` 原样保留）。one-shot
 * agent 无需 restart（直接返回 true）。返回 false 表示 restart 失败（已置 error）。
 */
async function restartChat(s: ChatSession): Promise<boolean> {
  if (!requiresRestartForSettings(s) || s.chatId === null) return true
  const old = s.chatId
  try {
    sessionsByChatId.delete(old)
    await api.agentChatStop(old)
    // 有源 session id 就 --resume 续上下文；还没有（首轮 init 未回填）就全新起，
    // 反正此时也没历史可丢，新 flag 直接生效。
    const eff = sessionEffectiveEffort(s)
    const info = await api.agentChatStart(
      s.agent,
      s.projectKey,
      s.cwd,
      s.sessionId || undefined,
      s.permissionMode,
      s.model,
      eff,
      undefined,
      useReclaude.value,
    )
    s.chatId = info.chatId
    s.processModel = info.processModel
    s.applied = { permissionMode: s.permissionMode, model: s.model, effort: eff }
    registerChat(info.chatId, s)
    return true
  } catch (err) {
    s.status = 'error'
    s.errorMessage = String(err)
    return false
  }
}

/** 中止当前轮 / 结束会话进程，但**保留**会话与已有 transcript（不从列表移除）。
 *  MVP 没有「不杀进程的软中断」，stop = kill 子进程 → 会话进入 exited，输入禁用。 */
export async function stopChat(session: ChatSession): Promise<void> {
  clearQueue(session) // 停进程 → 待发队列作废。
  if (session.chatId !== null) {
    try {
      await api.agentChatStop(session.chatId)
    } catch {
      /* 幂等 */
    }
  }
  session.lastTurnOutcome = 'cancelled'
  endTurn(session)
  if (session.status !== 'error') session.status = 'exited'
}

/** 中断当前这一轮回复，但保留 chat 会话继续可发。Claude 映射到 CLI 的 Esc。 */
const interrupting = new WeakSet<ChatSession>()
export async function interruptChat(session: ChatSession): Promise<void> {
  if (session.chatId === null) return
  if (interrupting.has(session)) return
  interrupting.add(session)
  try {
    if (session.processModel === 'longLivedStdin') {
      const old = session.chatId
      try {
        appendInterruptedMarker(session)
        session.suppressNextExit = true
        sessionsByChatId.delete(old)
        pendingByChatId.delete(old)
        await api.agentChatStop(old)
        const eff = sessionEffectiveEffort(session)
        const info = await api.agentChatStart(
          session.agent,
          session.projectKey,
          session.cwd,
          session.sessionId || undefined,
          session.permissionMode,
          session.model,
          eff,
          undefined,
          useReclaude.value,
        )
        session.chatId = info.chatId
        session.processModel = info.processModel
        session.applied = {
          permissionMode: session.permissionMode,
          model: session.model,
          effort: eff,
        }
        registerChat(info.chatId, session)
        session.lastTurnOutcome = 'cancelled'
        endTurn(session)
        clearLivePreview(session)
        session.status = 'running'
        drainQueue(session)
        return
      } catch (err) {
        session.suppressNextExit = false
        session.errorMessage = String(err)
        session.lastTurnOutcome = 'failed'
        endTurn(session)
        // autoRestart in onExit will recover
        return
      }
    }
    await api.agentChatInterrupt(session.chatId)
    session.lastTurnOutcome = 'cancelled'
    endTurn(session)
    clearLivePreview(session)
    if (session.status === 'spawning') session.status = 'running'
    drainQueue(session)
  } catch (err) {
    session.status = 'error'
    session.errorMessage = String(err)
    session.lastTurnOutcome = 'failed'
    endTurn(session)
  } finally {
    interrupting.delete(session)
  }
}

/**
 * `/clear`：清屏 + 重置上下文。先清空界面消息，再停掉当前 chat 进程、**不带**任何源
 * session 重起一个全新进程（空上下文、新 session id 由 init 回填）。磁盘上的旧 transcript
 * 不动，仍可在会话列表里续聊。两种进程模型通吃：长驻（Claude）换一个全新长驻进程；
 * one-shot（Codex）换一个 session_id 为空的新登记，下一轮即从零开始。
 */
export async function clearChat(session: ChatSession): Promise<void> {
  // 立即视觉清屏 —— 无论后续 restart 成败，界面与上下文角标都应清零。
  session.msgs = []
  clearLivePreview(session)
  session.toolActivity = null
  session.toolActivities = []
  session.lastTurnOutcome = null
  session.usage = undefined
  session.retry = null
  clearQueue(session) // 清屏 = 重置上下文 → 待发队列也清空。

  if (session.chatId === null || session.status === 'exited' || session.status === 'error') {
    // 进程已不在：纯视觉清屏即可（没有可重置的上下文）。
    return
  }
  const old = session.chatId
  try {
    session.suppressNextExit = true
    sessionsByChatId.delete(old)
    pendingByChatId.delete(old)
    await api.agentChatStop(old)
    const eff = sessionEffectiveEffort(session)
    const info = await api.agentChatStart(
      session.agent,
      session.projectKey,
      session.cwd,
      undefined,
      session.permissionMode,
      session.model,
      eff,
      undefined,
      useReclaude.value,
    )
    session.chatId = info.chatId
    session.processModel = info.processModel
    session.applied = {
      permissionMode: session.permissionMode,
      model: session.model,
      effort: eff,
    }
    session.sessionId = '' // 新进程的 init 会回填全新 session id
    registerChat(info.chatId, session)
    session.lastTurnOutcome = 'cancelled'
    endTurn(session)
    session.status = 'running'
  } catch (err) {
    session.suppressNextExit = false
    session.status = 'error'
    session.errorMessage = String(err)
    session.lastTurnOutcome = 'failed'
    endTurn(session)
  }
}

/** 关闭并回收一个 chat 会话：停进程、解路由、从列表移除。 */
export async function closeChat(uiId: number): Promise<void> {
  const idx = chatSessions.value.findIndex((c) => c.uiId === uiId)
  if (idx === -1) return
  const session = chatSessions.value[idx]
  if (session.chatId !== null) {
    sessionsByChatId.delete(session.chatId)
    pendingByChatId.delete(session.chatId)
    try {
      await api.agentChatStop(session.chatId)
    } catch {
      /* 幂等：已死的 id 也安全 */
    }
  }
  chatSessions.value.splice(idx, 1)
  if (activeChatUiId.value === uiId) {
    activeChatUiId.value = chatSessions.value.length ? chatSessions.value[0].uiId : null
  }
}

export function setActiveChat(uiId: number | null) {
  activeChatUiId.value = uiId
}

// ============================ 交互式工具权限 ============================

/**
 * 应答一个待处理的权限请求：构造 decision → 回写后端 → 出队。无论成败都先出队（让对话框
 * 立即消失，避免用户连点重复回写同一个 requestId）。回写失败置 error。
 */
export async function respondPermission(
  session: ChatSession,
  request: ChatPermissionRequest,
  choice: PermissionChoice,
): Promise<void> {
  session.pendingPermissions = session.pendingPermissions.filter(
    (p) => p.requestId !== request.requestId,
  )
  if (session.chatId === null) return
  try {
    await api.agentChatRespondPermission(
      session.chatId,
      request.requestId,
      buildPermissionDecision(request, choice),
    )
    maybeExitClaudePlanMode(session, request, choice)
  } catch (err) {
    session.status = 'error'
    session.errorMessage = String(err)
  }
}

function maybeExitClaudePlanMode(
  session: ChatSession,
  request: ChatPermissionRequest,
  choice: PermissionChoice,
) {
  if (session.agent !== 'claude' || session.permissionMode !== 'plan') return
  if (!isExitPlanModeRequest(request) || choice === 'deny') return
  const next = defaultPermissionMode('claude')
  session.permissionMode = next
  rememberChatGuiPreference('claude', { permissionMode: next })
}

/**
 * 回写一次结构化提问（AskUserQuestion）的应答 —— 先出队（无论成败，避免卡死的卡片），
 * 再把决定写回 CLI。`selections` 为 null = 用户点了取消（deny）；否则按选择构造 allow 决定。
 */
export async function respondQuestion(
  session: ChatSession,
  request: ChatQuestionRequest,
  selections: QuestionSelection[] | null,
): Promise<void> {
  session.pendingQuestions = session.pendingQuestions.filter(
    (q) => q.requestId !== request.requestId,
  )
  if (request.localCodexPlanPrompt || request.requestId.startsWith(LOCAL_CODEX_PLAN_REQUEST_PREFIX)) {
    if (codexPlanSelectionAcceptsImplementation(selections)) {
      maybeExitCodexPlanMode(session, selections)
      await sendPrompt(session, 'Implement the plan.')
    } else {
      drainQueue(session)
    }
    return
  }
  if (session.chatId === null) return
  const decision =
    selections === null
      ? buildQuestionCancelDecision()
      : buildQuestionDecision(request, selections)
  try {
    await api.agentChatRespondQuestion(session.chatId, request.requestId, decision)
    maybeExitCodexPlanMode(session, selections)
  } catch (err) {
    session.status = 'error'
    session.errorMessage = String(err)
  }
}

function codexPlanSelectionAcceptsImplementation(selections: QuestionSelection[] | null): boolean {
  if (!selections) return false
  return selections.some((selection) =>
    selection.labels.some((label) => {
      const normalized = label.trim().toLowerCase()
      return normalized.startsWith('yes') && (
        normalized.includes('implement') ||
        normalized.includes('coding') ||
        normalized.includes('编码') ||
        normalized.includes('执行')
      )
    }),
  )
}

function maybeExitCodexPlanMode(session: ChatSession, selections: QuestionSelection[] | null) {
  if (session.agent !== 'codex' || session.permissionMode !== 'plan' || !selections) return
  if (!codexPlanSelectionAcceptsImplementation(selections)) return
  const next = defaultPermissionMode('codex')
  session.permissionMode = next
  rememberChatGuiPreference('codex', { permissionMode: next })
}

/**
 * 页面刷新后重连后端仍存活的 chat 进程。
 * 返回 Map<projectKey, ChatSession> 供 App.vue 按项目恢复 liveChat。
 */
export async function reconnectChats(): Promise<ChatSession[]> {
  await ensureListeners()
  const running = await api.agentChatListRunning()
  const result: ChatSession[] = []
  for (const info of running) {
    const uiId = nextUiId++
    const model = sanitizeModel(info.agent as Agent, info.model ?? undefined) ?? defaultModel(info.agent as Agent)
    const effort = info.effort ?? defaultEffort(info.agent as Agent)
    const messages = normalizeRestoredMessages(info.messages, model)
    const session = reactive<ChatSession>({
      uiId,
      chatId: info.chatId,
      agent: info.agent as Agent,
      projectKey: info.projectKey,
      cwd: info.cwd,
      sessionId: info.sessionId ?? '',
      title: info.title ?? '',
      createdAt: new Date().toISOString(),
      msgs: messages,
      turnState: info.turnState ?? 'idle',
      turnStartedAt: info.turnStartedAtMs ?? (info.turnState === 'running' ? Date.now() : 0),
      lastTurnMs: 0,
      status: 'running',
      queue: [],
      submittedQueue: [],
      pendingSteerIds: [],
      stderrTail: [],
      retry: null,
      live: null,
      toolActivity: null,
      toolActivities: [],
      lastTurnOutcome: null,
      pendingPermissions: [],
      pendingQuestions: [],
      permissionMode: info.permissionMode,
      model,
      effort,
      lastModel: sanitizeModel(info.agent as Agent, lastAssistantModel(messages)),
      processModel: info.processModel as ChatProcessModel,
      applied: {
        permissionMode: info.permissionMode,
        model,
        effort,
      },
    }) as ChatSession
    chatSessions.value.push(session)
    registerChat(info.chatId, session)
    result.push(session)
  }
  return result
}
