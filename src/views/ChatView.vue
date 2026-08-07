<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch, defineAsyncComponent } from 'vue'
import { useVirtualizer } from '@tanstack/vue-virtual'
import type { Agent, Msg, SessionMeta, Block, ChatQuestionRequest } from '../types'
import { renderText, renderTextUncached, formatTime, formatElapsedSeconds, isCaveatOnlyMsg, isAskUserQuestionInstructionOnlyMsg, parseSystemEvent, cleanMetaText, metaKindIsPre, parseMetaFields, parseTeammateMessage, stripImagePlaceholders, parseFileRef } from '../format'
import type { MetaField } from '../format'
import { prettifyAndHighlightJson } from '../jsonHighlight'
import { renderAllMermaid, resetMermaidForTheme } from '../mermaid'
import { renderAllMath } from '../mathRender'
import { highlightAllCodeBlocks, rehighlightAllCodeBlocks } from '../shikiHighlight'
import { decorateCodeBlocks } from '../codeCopy'
import { theme, showToolCalls } from '../settings'
import { t } from '../i18n'
import ToolResult from '../components/ToolResult.vue'
import CollapsibleBox from '../components/CollapsibleBox.vue'
import ContextWindowCard from '../components/ContextWindowCard.vue'
import { parseContextUsage, type ContextUsage } from '../contextUsage'
// 图片灯箱只在点开预览时用 —— 懒加载，不进首屏（vue-easy-lightbox 不算大但能省一点）。
const VueEasyLightbox = defineAsyncComponent(() => import('vue-easy-lightbox'))
import { highlightDiff, looksLikeDiff } from '../diffHighlight'
import { renderCodexApplyPatchHtml } from '../codexApplyPatch'
import { codexPluginLinkForUri, renderCodexPluginLinkHtml, renderCodexPluginMentionHtmlText } from '../codexPluginMentions'
import { isFileChangeResult, isFileMutatingToolName, shouldAttachToolResult, shouldPreferToolResult } from '../toolResultRouting'
import {
  search,
  searchCount,
  searchIndex,
  searchScope,
  setSearchNavigator,
  toolsCollapsed,
} from '../chatToolbar'
import {
  IconArrowLeft,
  IconRefresh,
  IconTrash,
  IconRestore,
  IconPlay,
  IconFolder,
  IconArrowUp,
  IconArrowDown,
  IconChevronRight,
  IconAtom,
  IconTerminal,
  IconInfo,
  IconBell,
  IconPencil,
  IconCopy,
  IconCheck,
  IconDownload,
  IconMarkdown,
  IconHtml,
  IconJson,
  IconChart,
  IconFold,
  IconUnfold,
  IconLocate,
  IconEyeOff,
  IconEye,
  IconWrench,
  IconChat,
  IconReader,
  IconStop,
  fileIconFor,
  agentIcons,
} from '../components/icons'
import ChatComposer from '../components/ChatComposer.vue'
import ChatPermissionPrompt from '../components/ChatPermissionPrompt.vue'
import ChatQuestionPrompt from '../components/ChatQuestionPrompt.vue'
import {
  now as chatNow,
  interruptChat,
  respondPermission,
  respondQuestion,
  type ChatSession,
} from '../chatSessions'
import type { PermissionChoice } from '../chatPermission'
import type { QuestionSelection } from '../chatQuestion'
import { parseQuestionRequest } from '../chatQuestion'
import { openPathExternal, agentChatSlashCommands } from '../api'
import { convertFileSrc } from '@tauri-apps/api/core'
import { showTooltipFor, hideTooltip } from '../tooltip'
import { chatSupported } from '../chatComposerOptions'
import {
  chatHistoryEntryFromMsg,
  countChatHistoryEntriesBefore,
  type ChatHistoryEntry,
} from '../chatInputHistory'
import GitBranchControl from '../components/GitBranchControl.vue'

const props = defineProps<{
  agent: Agent
  session: SessionMeta
  messages: Msg[]
  /** 会话来自回收站 —— 只读查看，隐藏 重命名/恢复终端/删除/导出/统计 等操作。 */
  trashed?: boolean
  /** Live tail 状态：后端正在追这条 JSONL；为 true 时显示 "● Live" 徽章。 */
  live?: boolean
  /** Resume CLI 时使用的 cwd。空字符串时禁用「在窗口内对话」按钮。 */
  cwd?: string
  /** 非空 = live GUI chat 模式：底部挂 ChatComposer、隐藏只读专属操作按钮，
   *  `messages` 由父组件传入活跃会话的 reactive `msgs`。 */
  liveSession?: ChatSession | null
  /** live 模式下是否有「来源只读会话」可回看 —— 有才显示头部「切到 read」按钮。 */
  hasReadView?: boolean
}>()

const emit = defineEmits<{
  back: []
  refresh: []
  delete: []
  /** 入口 2：让父组件 openOrFocusTui，开（或聚焦已有）一个 TUI tab。 */
  resumeHere: []
  /** 入口 3：把当前只读会话就地切到 GUI chat 模式。 */
  switchToChat: []
  /** live chat 头部：切回来源会话的只读详情（read 模式），进程不停。 */
  switchToRead: []
  rename: []
  /** `/fork` 指令：把当前 live chat clone 成独立新会话并切过去（仅 live 模式、Claude）。 */
  fork: []
  /** 取消后编辑原提问：保留当前分支，另开只继承该提问之前上下文的新分支。 */
  editCancelled: [entry: ChatHistoryEntry, messageIndex: number, priorUserTurns: number]
  reveal: []
  copyId: []
  exportMd: []
  exportHtml: []
  exportJson: []
  restore: []
  /** 打开会话统计页 —— 原本住在 ChatTopbar 里，现挪进 chat-head 减少
   *  topbar + chat-head 两排 icon-only 按钮重叠的扫描负担。 */
  openSessionStats: []
  archive: []
  /** 头部星标：把当前会话收藏 / 取消收藏到「Views」历史。 */
}>()

// GUI live chat 的全局停止快捷键：监听 ChatView 本身，避免依赖输入框是否实际收到 keydown。
// 与底部停止按钮共用 interruptChat，非 GUI/read-only 视图不会生效。
function onGuiEscape(e: KeyboardEvent) {
  const session = props.liveSession
  const target = e.target
  if (
    e.key !== 'Escape' ||
    e.repeat ||
    e.isComposing ||
    !(target instanceof HTMLTextAreaElement) ||
    !target.classList.contains('cc-textarea') ||
    !session ||
    session.turnState !== 'running'
  ) return
  e.preventDefault()
  e.stopPropagation()
  void interruptChat(session)
}

/** live 模式当前轮已运行秒数（由 chatSessions 的模块时钟驱动）。 */
const runningElapsedSec = computed(() => {
  const s = props.liveSession
  if (!s || s.turnState !== 'running') return 0
  return Math.max(0, Math.floor((chatNow.value - s.turnStartedAt) / 1000))
})
const runningElapsedLabel = computed(() => formatElapsedSeconds(runningElapsedSec.value))
type FinishedTurnFeedback = {
  sessionUiId: number
  outcome: 'completed' | 'cancelled' | 'failed'
  elapsed: string
  key: number
}

const finishedTurnFeedback = ref<FinishedTurnFeedback | null>(null)
let finishedTurnTimer = 0

function clearFinishedTurnFeedback() {
  window.clearTimeout(finishedTurnTimer)
  finishedTurnFeedback.value = null
}

/** 当前轮结束后保留一小段成功/失败/取消反馈，避免运行状态行瞬间消失。 */
watch(
  () => [props.liveSession?.uiId ?? null, props.liveSession?.turnState ?? 'idle'] as const,
  ([uiId, state], [previousUiId, previousState]) => {
    const session = props.liveSession
    if (!session || uiId !== previousUiId) {
      clearFinishedTurnFeedback()
      return
    }
    if (state === 'running') {
      clearFinishedTurnFeedback()
      return
    }
    if (previousState !== 'running' || state !== 'idle') return
    const outcome = session.lastTurnOutcome ?? 'completed'
    finishedTurnFeedback.value = {
      sessionUiId: session.uiId,
      outcome,
      elapsed: formatElapsedSeconds(Math.ceil(session.lastTurnMs / 1000)),
      key: Date.now(),
    }
    window.clearTimeout(finishedTurnTimer)
    finishedTurnTimer = window.setTimeout(() => {
      finishedTurnFeedback.value = null
    }, 1050)
  },
)

const runningToolActivity = computed(() => {
  const session = props.liveSession
  if (!session || session.turnState !== 'running' || showToolCalls.value) return null
  // 交互式确认卡片本身已经说明“需要用户操作”，不要用旧的工具名抢占状态行。
  if (session.pendingPermissions.length || session.pendingQuestions.length) return null
  const activity = session.toolActivity
  if (activity && (activity.phase === 'calling' || chatNow.value - activity.updatedAt <= 1800)) {
    return activity
  }
  // 完成/失败反馈过期后，恢复仍在运行的并行工具中最新的一项。
  return session.toolActivities[session.toolActivities.length - 1] ?? activity ?? null
})

const runningToolStale = computed(() => {
  const activity = runningToolActivity.value
  return !!activity && activity.phase !== 'calling' && chatNow.value - activity.updatedAt > 1800
})

const runningToolLabel = computed(() => {
  const activity = runningToolActivity.value
  if (!activity) return ''
  const target = activity.summary.command
    ? `: ${activity.summary.command}`
    : activity.summary.target
      ? ` ${activity.summary.target}`
      : ''
  if (activity.summary.kind === 'callTool') {
    const name = activity.summary.target || activity.toolName
    if (runningToolStale.value) return t('chat.running.toolLastAction', { name })
    if (activity.phase === 'calling') return t('chat.running.toolCalling', { name })
    if (activity.phase === 'failed') return t('chat.running.toolFailed', { name })
    return t('chat.running.toolResult', { name })
  }
  const action = t(`chat.running.action.${activity.summary.kind}`)
  if (runningToolStale.value) return t('chat.running.toolLastActionAction', { action, target })
  if (activity.phase === 'calling') return t('chat.running.toolCallingAction', { action, target })
  if (activity.phase === 'failed') return t('chat.running.toolFailedAction', { action, target })
  return t('chat.running.toolResultAction', { action, target })
})

const runningToolKey = computed(() => {
  const activity = runningToolActivity.value
  return activity
    ? `${activity.phase}:${runningToolStale.value ? 'stale' : 'fresh'}:${activity.toolId ?? activity.toolName}:${activity.summary.kind}:${activity.summary.target ?? ''}:${activity.summary.command ?? ''}:${activity.updatedAt}`
    : ''
})

const runningFeedback = computed(() => {
  const session = props.liveSession
  if (!session) return null
  if (session.turnState === 'running') return { kind: 'running' as const }
  const finished = finishedTurnFeedback.value
  if (finished?.sessionUiId !== session.uiId) return null
  return {
    kind: finished.outcome,
    elapsed: finished.elapsed,
    key: finished.key,
  }
})

const finishedTurnLabel = computed(() => {
  const feedback = runningFeedback.value
  if (!feedback || feedback.kind === 'running') return ''
  if (feedback.kind === 'failed') return t('chat.running.failed')
  if (feedback.kind === 'cancelled') return t('chat.running.cancelled')
  return t('chat.running.completed')
})

/** 进行中状态动词：网络重试时 → 「请求失败 · 重试中 (n/N)」；流式 thinking → 「思考中」；
 *  否则「处理中」（参考 Claude 客户端）。 */
const runningVerb = computed(() => {
  const r = props.liveSession?.retry
  if (r) {
    return r.attempt && r.max
      ? t('chat.running.retryingN', { n: r.attempt, max: r.max })
      : t('chat.running.retrying')
  }
  if (props.liveSession?.pendingPermissions.length || props.liveSession?.pendingQuestions.length) {
    return t('chat.running.waitingForInput')
  }
  return props.liveSession?.live?.kind === 'thinking' ? t('chat.running.thinking') : t('chat.running.working')
})

function toggleTools() {
  toolsCollapsed.value = !toolsCollapsed.value
}

// Resume 按钮是否可用：回收站、缺 session id、缺 cwd 时禁用。
const sessionArchived = computed(() => !!props.session.codexArchived)
const canResumeHere = computed(
  () => !props.trashed && !sessionArchived.value && !!props.session.id && !!props.cwd,
)

// 只读页可临时覆盖全局「显示工具调用」设置。该值不落盘，切换到另一会话时也会复位，
// 方便用户偶尔检查某一段工具过程而不打扰平时紧凑的阅读视图。
const readToolCallsOverride = ref<boolean | null>(null)
const readToolCallsVisible = computed(() =>
  props.liveSession ? showToolCalls.value : readToolCallsOverride.value ?? showToolCalls.value,
)
function toggleReadToolCalls() {
  readToolCallsOverride.value = !readToolCallsVisible.value
}
watch(
  () => `${props.session.id}:${props.session.path}`,
  () => { readToolCallsOverride.value = null },
)

function shortId(id: string): string {
  if (!id) return ''
  return id.length > 8 ? id.slice(0, 8) : id
}

function isToolOnly(m: Msg): boolean {
  return m.role === 'user' && m.blocks.every((b) => b.kind === 'tool_result' || (b.kind === 'image' && b.toolId))
}

function isAskUserQuestion(b: Block): boolean {
  return b.kind === 'tool_use' && b.toolName === 'AskUserQuestion'
}

/** AskUserQuestion 是 Claude 发起的工具调用。即使遇到旧 transcript 把这类
 * block 放在 user 外壳里，也按 assistant 视觉归属，避免把模型提问显示成「Me」。 */
function effectiveRole(m: Msg): Msg['role'] {
  return m.role === 'assistant' || m.blocks.some(isAskUserQuestion) ? 'assistant' : m.role
}

function askUserQuestionRequest(b: Block): ChatQuestionRequest | null {
  if (!isAskUserQuestion(b)) return null
  return parseQuestionRequest(b.toolInput ?? '', b.toolId ?? 'history-question')
}

function toolLabel(b: Block): string {
  if (b.kind === 'tool_use') return t('tool.call', { name: b.toolName ?? '' })
  return ''
}

/** thinking block 本身没有耗时字段。用其消息时间与前一条带时间戳的消息相减，
 *  作为可解释的近似值；过短或过长的间隔不显示，避免把排队/离线时长误称为思考时间。 */
function thinkingElapsedLabel(messageIndex: number): string | null {
  const current = Date.parse(props.messages[messageIndex]?.timestamp ?? '')
  if (!Number.isFinite(current)) return null

  for (let i = messageIndex - 1; i >= 0; i -= 1) {
    const previous = Date.parse(props.messages[i]?.timestamp ?? '')
    if (!Number.isFinite(previous)) continue
    const elapsedMs = current - previous
    if (elapsedMs < 400 || elapsedMs > 5 * 60_000) return null
    return formatElapsedSeconds(Math.max(1, Math.round(elapsedMs / 1000)))
  }
  return null
}

function thinkingLabel(messageIndex: number): string {
  const elapsed = thinkingElapsedLabel(messageIndex)
  return elapsed
    ? t('tool.thinkingDone', { duration: elapsed })
    : t('tool.thinkingDoneNoDuration')
}

function isCodexInlineCodeToolUse(b: Block): boolean {
  return props.agent === 'codex' && b.kind === 'tool_use' && b.toolName === 'apply_patch' && !b.isError
}

function renderNumberedCodeHtml(html: string): string {
  const lines = html.split('\n')
  return lines
    .map((line, i) => {
      const content = line || '&nbsp;'
      return `<div class="inline-code-line"><span class="inline-code-no">${i + 1}</span><span class="inline-code-text">${content}</span></div>`
    })
    .join('')
}

function toolUseCodeHtml(b: Block): string {
  const input = b.toolInput ?? ''
  if (isCodexInlineCodeToolUse(b)) {
    const rendered = renderCodexApplyPatchHtml(input, props.cwd)
    if (rendered) return rendered
  }
  const highlighted = looksLikeDiff(input)
    ? highlightDiff(input)
    : prettifyAndHighlightJson(input)
  return renderNumberedCodeHtml(highlighted)
}

function isCodexApplyPatchStructured(b: Block): boolean {
  return isCodexInlineCodeToolUse(b) && !!renderCodexApplyPatchHtml(b.toolInput ?? '', props.cwd)
}

function toolUseCodeClass(b: Block): string[] {
  if (isCodexApplyPatchStructured(b)) return ['codex-apply-patch']
  return ['code-block', looksLikeDiff(b.toolInput ?? '') ? 'lang-diff' : 'lang-json']
}

// 搜索范围分类 —— 给 .msg-row / tool_use <details> 打 data-search-scope，
// applySearch 沿祖先链找最近的 scope 决定是否收录该文本节点。
//   'user' / 'assistant'：用户消息 / 助手文本（含 thinking）
//   'tools-edit'：文件改动型工具（与 'agent' 选项合并）
//   'tools-other'：其它工具调用（与 'tools' 选项匹配）
function rowScope(m: Msg): string {
  if (isToolOnly(m)) return m.blocks.some((b) => isFileChangeResult(b)) ? 'tools-edit' : 'tools-other'
  // 系统注入块不是用户 prose 也不是助手回复 —— 给个独立 scope，只在「全部」筛选下命中。
  if (m.metaKind) return 'meta'
  return effectiveRole(m)
}
function toolUseScope(b: Block): string {
  return isFileMutatingToolName(b.toolName) ? 'tools-edit' : 'tools-other'
}

const resultByToolId = computed(() => {
  const map = new Map<string, Block>()
  for (const m of props.messages) {
    for (const b of m.blocks) {
      if (b.kind === 'tool_result' && b.toolId) {
        const previous = map.get(b.toolId)
        if (shouldPreferToolResult(b, previous)) map.set(b.toolId, b)
      }
    }
  }
  return map
})

const toolUseById = computed(() => {
  const map = new Map<string, Block>()
  for (const m of props.messages) {
    for (const b of m.blocks) {
      if (b.kind === 'tool_use' && b.toolId) map.set(b.toolId, b)
    }
  }
  return map
})

const inlinedResultIds = computed(() => {
  const set = new Set<string>()
  for (const m of props.messages) {
    for (const b of m.blocks) {
      if (
        b.kind === 'tool_use' &&
        b.toolId &&
        resultByToolId.value.has(b.toolId) &&
        !shouldAttachToolResult(b, resultByToolId.value.get(b.toolId))
      ) {
        set.add(b.toolId)
      }
    }
  }
  return set
})

function inlinedResultFor(b: Block): Block | undefined {
  if (b.kind !== 'tool_use' || !b.toolId) return undefined
  if (!inlinedResultIds.value.has(b.toolId)) return undefined
  return resultByToolId.value.get(b.toolId)
}

function isInlinedResult(b: Block): boolean {
  return b.kind === 'tool_result' && !!b.toolId && inlinedResultIds.value.has(b.toolId)
}

// 文件改动型 tool_result（structuredPatch diff）整块挂回发起它的 assistant tool_use 行内：
// 渲染在气泡下方、时间行上方，让「Tool call · Edit + File change + 时间」成为同一条消息的
// 整体（之前 diff 自成一行、时间夹在调用卡与 diff 之间，看着像断开两段）。其独立 tool-only
// 行随之隐藏。与 inlinedResult* 互斥：非文件改动型仍内嵌进调用卡。
const attachedResultIds = computed(() => {
  const set = new Set<string>()
  for (const m of props.messages) {
    for (const b of m.blocks) {
      if (
        b.kind === 'tool_use' &&
        b.toolId &&
        resultByToolId.value.has(b.toolId) &&
        shouldAttachToolResult(b, resultByToolId.value.get(b.toolId))
      ) {
        set.add(b.toolId)
      }
    }
  }
  return set
})

function attachedResultFor(b: Block): Block | undefined {
  if (b.kind !== 'tool_use' || !b.toolId) return undefined
  if (!attachedResultIds.value.has(b.toolId)) return undefined
  return resultByToolId.value.get(b.toolId)
}

function isAttachedResult(b: Block): boolean {
  return b.kind === 'tool_result' && !!b.toolId && attachedResultIds.value.has(b.toolId)
}

/** Codex apply_patch 的输入已经渲染成带路径和行级 diff 的文件变更卡。其成功回执通常只是
 * “Done” 一类协议确认，继续放在卡片下方只会制造一个无信息量的 Tool result。失败回执仍要
 * 保留，以免吞掉诊断信息；无法解析为 patch 的旧记录也保留原结果作为兜底。 */
function shouldShowAttachedResult(b: Block): boolean {
  const result = attachedResultFor(b)
  if (!result) return false
  return result.isError || !isCodexApplyPatchStructured(b)
}

function shouldHideToolResult(b: Block): boolean {
  if (b.kind !== 'tool_result' || !b.toolId) return false
  const sourceTool = toolUseById.value.get(b.toolId)
  if (sourceTool && isAskUserQuestion(sourceTool)) return true
  return !!sourceTool && isCodexInlineCodeToolUse(sourceTool)
}

/** 文件变更结果不受「显示工具调用」开关影响。除了自身的 structured diff / filePath 外，
 * 也兜住 Write/Edit/apply_patch 等已知写文件工具返回的简短成功结果。 */
function isAlwaysVisibleToolResult(b: Block): boolean {
  if (b.kind !== 'tool_result') return false
  const sourceTool = b.toolId ? toolUseById.value.get(b.toolId) : undefined
  return isFileChangeResult(b) || !!sourceTool && shouldAttachToolResult(sourceTool, b)
}

/** 普通过程性工具默认隐藏；AskUserQuestion 是用户交互卡，始终可见。Codex apply_patch
 * 的输入本身可能是唯一的结构化 patch，保留它作为文件变更内容的兜底展示。 */
function shouldShowToolUse(b: Block): boolean {
  if (b.kind !== 'tool_use') return false
  if (isAskUserQuestion(b)) return true
  if (readToolCallsVisible.value) return true
  return isCodexInlineCodeToolUse(b)
}

/** 已内联或已挂到文件变更卡的结果不在原位置重复渲染。其余普通结果受开关控制，
 * 文件结果则强制保留。 */
function shouldShowToolResult(b: Block): boolean {
  if (b.kind !== 'tool_result') return false
  if (shouldHideToolResult(b) || isInlinedResult(b) || isAttachedResult(b)) return false
  return readToolCallsVisible.value || isAlwaysVisibleToolResult(b)
}

function askUserQuestionResult(b: Block): Block | undefined {
  if (!isAskUserQuestion(b) || !b.toolId) return undefined
  return resultByToolId.value.get(b.toolId)
}

function rowHasContent(m: Msg): boolean {
  // Local-command caveat user messages are pure plumbing — hide the row entirely.
  if (isCaveatOnlyMsg(m)) return false
  // The actual AskUserQuestion card is rendered on Claude's side; the explicit
  // tool-test instruction is protocol scaffolding and would only duplicate it.
  if (isAskUserQuestionInstructionOnlyMsg(m)) return false
  return m.blocks.some((b) => {
    if (b.kind === 'text' || b.kind === 'thinking' || b.kind === 'file' || b.kind === 'image') return true
    if (b.kind === 'tool_use') return shouldShowToolUse(b) || !!attachedResultFor(b)
    return shouldShowToolResult(b)
  })
}

// ---- 图片：缩略图浮在气泡上方（参考 Claude 客户端），不进灰底气泡 ----
function imageSrcUrl(src: string): string {
  if (!src) return ''
  if (src.startsWith('data:') || src.startsWith('http:') || src.startsWith('https:')) {
    return src
  }
  try {
    return convertFileSrc(src)
  } catch (e) {
    console.error('convertFileSrc error:', e)
    return src
  }
}

function imageBlocks(m: Msg): Block[] {
  return m.blocks.filter((b) => b.kind === 'image' && b.imageSrc)
}
// 带图消息的正文要滤掉 [Image #n] 占位符（缩略图已单独展示）；无图消息原样渲染，
// 免得误删正文里对图片的文字引用。
function bubbleText(m: Msg, raw: string): string {
  let text = imageBlocks(m).length ? stripImagePlaceholders(raw) : raw
  if (m.blocks.some(isCodexPluginFileBlock)) text = text.replace(/^\[\s*/, '')
  return text
}
// 用户消息以 /命令 开头时，把开头那段 /token 标蓝（对齐输入框命令高亮 + 官方客户端渲染）。
// 只处理开头：renderText 把首段文本包成 <div class="text-run">，正则只命中首段开头的 /token
// （[^\s<]+ 到空白或下一个标签为止），不碰正文里别处的斜杠；非 / 开头的消息原样返回。
function renderBubble(m: Msg, raw: string): string {
  const text = bubbleText(m, raw)
  let html = renderText(text)
  if (effectiveRole(m) === 'user' && /^[/$]\S/.test(text)) {
    html = html.replace(
      /(<div class="text-run">)([/$][^\s<]+)/,
      '$1<span class="cmd-name">$2</span>',
    )
  }
  if (effectiveRole(m) === 'user' && props.agent === 'codex' && text.includes('@')) {
    html = renderCodexPluginMentionHtmlText(html)
  }
  return injectCmdDesc(html)
}

// ---- 命令描述：hover 蓝色命令 token 时显示其描述 ----
// 描述来自扫描当前项目可用的 commands/skills（与输入框 slash 列表同源；codex 暂为空）。
// 内置命令（/config、/clear 等）不在扫描列表里 → 无描述、hover 即无提示。
const cmdDesc = ref<Record<string, string>>({})
async function loadCmdDescriptions() {
  const cwd = props.cwd || props.session.cwd || ''
  try {
    const list = await agentChatSlashCommands(props.agent, cwd)
    const map: Record<string, string> = {}
    for (const c of list) if (c.description) map[c.name] = c.description
    cmdDesc.value = map
  } catch {
    cmdDesc.value = {}
  }
}
watch(
  () => [props.agent, props.cwd, props.session.path],
  () => void loadCmdDescriptions(),
  { immediate: true },
)
// 给已识别命令的 cmd-name 注入 data-cmd-desc（hover 读它显示 tip）。覆盖 <code>（转录
// <command-name>）与 <span>（纯文本开头 token）两种载体；无描述的不加。读 cmdDesc 建立响应依赖，
// 描述异步加载完后气泡自动重渲染。
function injectCmdDesc(html: string): string {
  const map = cmdDesc.value
  if (!html.includes('cmd-name') || !Object.keys(map).length) return html
  return html.replace(
    /<(code|span)([^>]*\bcmd-name\b[^>]*)>([/$][^<\s]+)<\/\1>/g,
    (full, tag, attrs, name) => {
      const desc = map[name.slice(1)]
      return desc ? `<${tag}${attrs} data-cmd-desc="${escapeAttr(desc)}">${name}</${tag}>` : full
    },
  )
}
function escapeAttr(s: string): string {
  const t = s.length > 200 ? s.slice(0, 200) + '…' : s
  return t
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
}
// ---- 文件附件：文件 chip 浮在气泡上方（参考目标样式），点击用系统默认程序打开 ----
// 栅格图扩展名：这类附件已作为缩略图展示，同条消息里不再重复出文件 chip。
const IMAGE_CHIP_EXTS = new Set([
  'png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'heic', 'heif', 'avif', 'tiff', 'ico',
])
function isImagePath(path: string): boolean {
  return IMAGE_CHIP_EXTS.has(path.split('.').pop()?.toLowerCase() ?? '')
}
function fileBlocks(m: Msg): Block[] {
  const hasThumbs = m.blocks.some((b) => b.kind === 'image' && b.imageSrc)
  const files = m.blocks.filter(
    (b) => b.kind === 'file' && b.filePath && !isCodexPluginFileBlock(b) && !(hasThumbs && isImagePath(b.filePath)),
  )
  return files
}
function isCodexPluginFileBlock(b: Block): boolean {
  return !!(b.kind === 'file' && b.filePath && codexPluginLinkForUri(b.filePath))
}
function codexPluginFileHtml(b: Block): string {
  return b.filePath ? renderCodexPluginLinkHtml('', b.filePath, escapeHtml) ?? '' : ''
}
// 文件名 = 路径 basename（兼容 `/` 与 `\`）；空则回退整段路径。
function fileName(path: string): string {
  const norm = path.replace(/[/\\]+$/, '')
  const base = norm.split(/[/\\]/).pop() ?? ''
  return base || path
}
function openFile(b: Block) {
  if (!b.filePath) return
  void openPathExternal(b.filePath, props.cwd || props.session?.cwd).catch(() => {})
}

// `/context` 报告（`## Context Usage` markdown）有两条来源：① live stream —— 一条
// assistant 消息（model `<synthetic>`），正文是裸 markdown；② 刷新/离线回看 —— 落盘的
// system/local_command 记录被还原成 command-output 折叠块，正文外面包着 <local-command-stdout>。
// 两者都先经 cleanMetaText 去壳（裸文本则原样返回），命中 `## Context Usage` 就升级成可折叠的
// ContextWindowCard。先用 startsWith 廉价过滤，再 parseContextUsage 严格确认；按文本缓存避免重解析。
const ctxUsageCache = new Map<string, ContextUsage | null>()
function contextUsageOf(m: Msg): ContextUsage | null {
  if (effectiveRole(m) !== 'assistant' && m.metaKind !== 'command-output') return null
  // 廉价子串过滤在前（每行 class 绑定都会调本函数）；命中的极少数块才去壳 + 严格确认结构。
  const b = m.blocks.find((x) => x.kind === 'text' && x.text && x.text.includes('## Context Usage'))
  if (!b?.text) return null
  const txt = cleanMetaText(b.text)
  if (!txt.startsWith('## Context Usage')) return null
  if (!ctxUsageCache.has(txt)) ctxUsageCache.set(txt, parseContextUsage(txt))
  return ctxUsageCache.get(txt)!
}

// 气泡是否还有非图片 / 非文件正文 —— 纯图片 / 纯文件消息不渲染空灰泡，只留上方缩略图 / chip。
function hasBubbleBody(m: Msg): boolean {
  return m.blocks.some((b) => {
    if (b.kind === 'image') return false
    if (b.kind === 'file') return isCodexPluginFileBlock(b)
    if (b.kind === 'text') return bubbleText(m, b.text ?? '').trim().length > 0
    return true
  })
}


// ---- 消息右键隐藏 ----
// 按 session path 在 localStorage 中存一组隐藏的消息标识（uuid 或索引）。
// 隐藏状态纯前端，不修改 JSONL 文件，三个 agent 通用。
function hiddenStorageKey(): string {
  return `hidden:${props.session.path}`
}
function loadHiddenSet(): Set<string> {
  try {
    const raw = localStorage.getItem(hiddenStorageKey())
    return raw ? new Set(JSON.parse(raw)) : new Set()
  } catch {
    return new Set()
  }
}
function saveHiddenSet(set: Set<string>) {
  if (set.size === 0) {
    localStorage.removeItem(hiddenStorageKey())
  } else {
    localStorage.setItem(hiddenStorageKey(), JSON.stringify([...set]))
  }
}

const hiddenIds = ref<Set<string>>(new Set())
const showHidden = ref(false)

function msgKey(m: Msg, idx: number): string {
  return m.uuid || `idx:${idx}`
}

function isHidden(m: Msg, idx: number): boolean {
  return hiddenIds.value.has(msgKey(m, idx))
}

const hiddenCount = computed(() => hiddenIds.value.size)

function toggleHideMsg(m: Msg, idx: number) {
  const key = msgKey(m, idx)
  const set = new Set(hiddenIds.value)
  if (set.has(key)) {
    set.delete(key)
  } else {
    set.add(key)
  }
  hiddenIds.value = set
  saveHiddenSet(set)
}

// 当前悬停的消息键。用 JS 状态而非纯 CSS :hover 来驱动操作行的显隐——live chat 流式
// 重渲染时 Chromium 的 :hover 伪类可能「粘」在旧行上，导致多行操作行同时常亮、移走也不
// 收（用户反馈：hover A 再 hover 别的都不自动隐藏）。改成 mouseenter/leave 维护单一键，
// 任一时刻只有一行 row-active，互斥且确定性收起。
const hoveredKey = ref<string | null>(null)

// 最近一轮被取消时，只允许编辑该轮最后一条真实用户输入。中断标记、系统注入记录和
// assistant/tool 消息都会被 chatHistoryEntryFromMsg 排除；普通完成轮次不显示入口。
const cancelledPrompt = computed<{ index: number; entry: ChatHistoryEntry } | null>(() => {
  const session = props.liveSession
  if (
    !session ||
    session.status !== 'running' ||
    session.turnState !== 'idle' ||
    session.lastTurnOutcome !== 'cancelled'
  ) return null
  for (let index = props.messages.length - 1; index >= 0; index -= 1) {
    const entry = chatHistoryEntryFromMsg(props.messages[index])
    if (entry) return { index, entry }
  }
  return null
})

function editCancelledPrompt() {
  const prompt = cancelledPrompt.value
  if (!prompt) return
  emit(
    'editCancelled',
    prompt.entry,
    prompt.index,
    countChatHistoryEntriesBefore(props.messages, prompt.index),
  )
}

// ---- 消息悬停操作：复制原文 ----
// 复制成功后短暂把对应消息的图标切成对勾（按 msgKey 记一个键，避免影响其它消息）。
const copiedMsgKey = ref<string | null>(null)
let copiedResetTimer = 0
function copyMsg(m: Msg, idx: number) {
  const text = m.blocks
    .filter((b) => b.kind === 'text')
    .map((b) => b.text ?? '')
    .join('\n\n')
    .trim()
  if (!text) return
  void navigator.clipboard?.writeText(text)
  const key = msgKey(m, idx)
  copiedMsgKey.value = key
  window.clearTimeout(copiedResetTimer)
  copiedResetTimer = window.setTimeout(() => {
    if (copiedMsgKey.value === key) copiedMsgKey.value = null
  }, 1200)
}

// 切换会话时重新加载隐藏集合
watch(
  () => props.session.path,
  () => {
    hiddenIds.value = loadHiddenSet()
    showHidden.value = false
  },
  { immediate: true },
)

// 隐藏消息现由气泡下方的悬停操作行接管；右键恢复浏览器默认行为（选中复制等），
// 不再弹自定义菜单。

const assistantName = computed(() =>
  props.agent === 'codex' ? 'Codex' : props.agent === 'agy' ? 'agy' : props.agent === 'opencode' ? 'opencode' : 'Claude',
)

function formatModelName(modelName: string): string {
  if (!modelName) return ''
  return modelName.replace(/\s*\([^)]*\)$/, '').trim()
}

function systemEventLabel(m: Msg): string | null {
  const ev = parseSystemEvent(m)
  if (!ev) return null
  if (ev.kind === 'rename') return t('chat.systemEvent.rename', { name: ev.name })
  if (ev.kind === 'interrupt') return t('chat.systemEvent.interrupted')
  return null
}

// 系统注入的 user 记录（metaKind）—— 后端 claude 源打的标记。映射到本地化标题，
// 前端据此把它们渲染成低调的「系统」块而非「Me」气泡。
const META_KIND_KEY: Record<string, string> = {
  compact: 'chat.metaKind.compact',
  meta: 'chat.metaKind.meta',
  'task-notification': 'chat.metaKind.taskNotification',
  system: 'chat.metaKind.system',
  'command-output': 'chat.metaKind.commandOutput',
  'teammate-message': 'chat.metaKind.teammateMessage',
}
function metaKindLabel(kind: string | undefined): string {
  if (!kind) return ''
  return t(META_KIND_KEY[kind] ?? 'chat.metaKind.system')
}

/** 命令输出和系统说明与 thinking 同属低优先级过程信息：沿用轻量折叠样式，
 *  但以终端 / 信息 / 提醒图标明确区分来源。Task notification 与 Context Summary
 *  同属系统过程信息，也保持这套无边框的轻量折叠样式。 */
function usesLightweightMetaStyle(kind: string | undefined): boolean {
  return kind === 'command-output' || kind === 'meta' || kind === 'compact' || kind === 'task-notification'
}

function lightweightMetaIcon(kind: string | undefined) {
  if (kind === 'task-notification') return IconBell
  return kind === 'command-output' ? IconTerminal : IconInfo
}

// 该消息的 metaKind 是否以等宽 <pre> 呈现（undefined-safe 包装，便于模板调用）。
function metaIsPre(m: Msg): boolean {
  return !!m.metaKind && metaKindIsPre(m.metaKind)
}
// metaKind 块里每个文本块的渲染：command-output / 通知类去壳 + ANSI 后以等宽
// <pre> 原样呈现；compact / meta 本身是 markdown，走常规 renderText。
function metaBlockHtml(kind: string | undefined, text: string): string {
  if (kind && metaKindIsPre(kind)) return cleanMetaText(text)
  return renderText(text)
}
// 把 metaKind 正文解析成 key/value 字段供模板格式化渲染：先试通用 <tag>value</tag>
// 结构（任务通知），再试 teammate-message 结构（多 agent 协作消息）。都不匹配
// （命令输出等纯文本）返回 null，交给 <pre> / markdown 分支。
function metaFieldsOf(text: string): MetaField[] | null {
  return parseMetaFields(text) ?? parseTeammateMessage(text)
}

const stats = computed(() => {
  const u = props.messages.filter(
    (m) =>
      effectiveRole(m) === 'user' &&
      !m.metaKind &&
      !isToolOnly(m) &&
      !isCaveatOnlyMsg(m) &&
      !isAskUserQuestionInstructionOnlyMsg(m) &&
      !systemEventLabel(m),
  ).length
  const a = props.messages.filter((m) => effectiveRole(m) === 'assistant').length
  return { u, a }
})

const lightboxVisible = ref(false)
const lightboxImgs = ref<string[]>([])
const lightboxIndex = ref(0)
// 同一条消息的多张图片整组进灯箱，点哪张就从哪张开始，可左右翻看。
function openLightbox(imgs: string[], index = 0) {
  lightboxImgs.value = imgs
  lightboxIndex.value = index
  lightboxVisible.value = true
}

const scrollEl = ref<HTMLElement>()
const vlistEl = ref<HTMLElement>()

// ============================ 虚拟滚动 ============================
// 长会话（几千条）以前一次性把所有 .msg-row 挂进 DOM（几万节点）+ 首屏同步跑满 markdown /
// Shiki / mermaid → 打开卡、滚动卡。改用 @tanstack/vue-virtual：只挂「可见窗口 + overscan
// 缓冲」的行,行高用 measureElement（ResizeObserver）动态测量 —— 代码高亮/图片/mermaid 异步
// 撑高后自动重测、自动校正滚动位置,正是「快速滚动不留大片空白」的关键。
//
// scrollMargin：列表并非贴着滚动容器顶端（.chat-scroll 有 28px padding-top）,把这段偏移告诉
// 虚拟器,scrollToIndex 才对得准。挂载后量一次。
const listScrollMargin = ref(0)
function measureListMargin() {
  const s = scrollEl.value
  const v = vlistEl.value
  if (!s || !v) return
  listScrollMargin.value = v.getBoundingClientRect().top - s.getBoundingClientRect().top + s.scrollTop
}

const rowVirtualizer = useVirtualizer(
  computed(() => ({
    count: props.messages.length,
    getScrollElement: () => scrollEl.value ?? null,
    // 粗估行高（未测量的行用它撑起滚动条几何）；测到真高后自动替换。取接近真实均值,减少大跳
    // 滚动时估算与实际的落差 —— 落差大 → 已测区间修正累积高度 → 滚动位置跳动 + 视口留缝。
    // 实测本类会话约 40% 消息被 v-show 隐藏(0 高),含 0 的均值≈110,故取 110 而非视觉行高。
    estimateSize: () => 110,
    // overscan：视口上下各多渲染的行数 —— 缓冲区,快速滚动时先垫住,避免白板。给大一点。
    overscan: 20,
    getItemKey: (i: number) => props.messages[i]?.uuid ?? i,
    scrollMargin: listScrollMargin.value,
    // ⚠️ 用 offsetHeight，不要用 getBoundingClientRect().height。本 app 的 body 有 CSS zoom(≈0.9)：
    // gBCR 返回的是**视觉坐标**(已 ×0.9)，而虚拟器定位用的 translateY / scrollTop / scrollHeight 都在
    // **布局坐标**。用 gBCR 会让每行高度被低估 ~10% → 行与行按 0.9 倍间距铺 → 整列消息逐行重叠 ~10%
    // (用户看到「消息错位」)。offsetHeight 是布局坐标、且本就是整数(无小数抖动,不会触发 RO 死循环)，
    // 两个问题一起解决。
    measureElement: (el: Element) => (el as HTMLElement).offsetHeight,
  })),
)
const virtualRows = computed(() => rowVirtualizer.value.getVirtualItems())
const totalSize = computed(() => rowVirtualizer.value.getTotalSize())
// 每个可见行绑到它,TanStack 用 data-index 认领并挂 ResizeObserver 动态测高。
function measureRow(el: unknown) {
  if (el instanceof Element) rowVirtualizer.value.measureElement(el)
}

// 自定义 rAF 平滑滚动：原生 behavior:'smooth' 在长会话里会随距离把动画拉长，
// 每帧又触发大段 reflow，所以 420 条消息时就会卡。这里用固定时长 + ease-out，
// 并在用户滚动/再次点击时打断。
let scrollRAF = 0
let pinRAF = 0
function cancelScroll() {
  if (scrollRAF) {
    cancelAnimationFrame(scrollRAF)
    scrollRAF = 0
  }
  if (pinRAF) {
    cancelAnimationFrame(pinRAF)
    pinRAF = 0
  }
}

// live GUI chat 专用：把视口「钉」在底部一小段时间。进入会话（预载历史）/ 新消息
// 到达后，代码高亮 / DiffBlock / 图片会异步把内容撑高 —— 单次 scrollToBottom 会
// 因为 scrollHeight 还在涨而停在半路。这里每帧重读 scrollHeight 跳到底，持续 ms 毫秒，
// 直到高度稳定。用户一旦主动滚动（wheel/touch）立即放手，绝不和用户抢滚动条。
function pinToBottomFor(ms: number) {
  const el = scrollEl.value
  if (!el) return
  cancelScroll()
  const until = performance.now() + ms
  const release = () => {
    cancelScroll()
    el.removeEventListener('wheel', release)
    el.removeEventListener('touchmove', release)
  }
  const stick = () => {
    const e = scrollEl.value
    if (!e) {
      pinRAF = 0
      return
    }
    e.scrollTop = e.scrollHeight
    if (performance.now() < until) {
      pinRAF = requestAnimationFrame(stick)
    } else {
      pinRAF = 0
      el.removeEventListener('wheel', release)
      el.removeEventListener('touchmove', release)
    }
  }
  el.addEventListener('wheel', release, { passive: true, once: true })
  el.addEventListener('touchmove', release, { passive: true, once: true })
  pinRAF = requestAnimationFrame(stick)
}
function smoothScrollTo(target: number) {
  const el = scrollEl.value
  if (!el) return
  cancelScroll()
  const start = el.scrollTop
  const dest = Math.max(0, Math.min(target, el.scrollHeight - el.clientHeight))
  const dist = dest - start
  if (Math.abs(dist) < 2) {
    el.scrollTop = dest
    return
  }
  // 距离越长动画稍微拉长一点，但封顶 360ms，避免长会话拖沓
  const duration = Math.min(360, 180 + Math.abs(dist) * 0.05)
  const t0 = performance.now()
  // easeOutCubic
  const ease = (p: number) => 1 - Math.pow(1 - p, 3)
  const step = (now: number) => {
    const p = Math.min(1, (now - t0) / duration)
    el.scrollTop = start + dist * ease(p)
    if (p < 1) {
      scrollRAF = requestAnimationFrame(step)
    } else {
      scrollRAF = 0
    }
  }
  // 用户主动滚动则中断
  const onUserScroll = () => {
    cancelScroll()
    el.removeEventListener('wheel', onUserScroll)
    el.removeEventListener('touchmove', onUserScroll)
  }
  el.addEventListener('wheel', onUserScroll, { passive: true, once: true })
  el.addEventListener('touchmove', onUserScroll, { passive: true, once: true })
  scrollRAF = requestAnimationFrame(step)
}
function scrollToTop() {
  smoothScrollTo(0)
}
function scrollToBottom() {
  if (props.liveSession) {
    // 直播：列表底部还挂着「流式行 / 运行状态行」等非虚拟项,按 scrollHeight 贴底才包含它们。
    pinToBottomFor(300)
    return
  }
  // 只读：末行高度多为估算,直接 scrollHeight 会落不到真底 —— 交给虚拟器逐帧测量对齐到末行。
  const n = props.messages.length
  if (n > 0) rowVirtualizer.value.scrollToIndex(n - 1, { align: 'end' })
}

// 跳转到某条消息：滚到对应 .msg-row，触发一次 .msg-flash 闪烁动画。
// 全局搜索点击命中后被 App.vue 通过 defineExpose 调用。idx 与 uuid 双兜底
// —— uuid 在场用 uuid 找（更稳，能扛重排），否则按 data-msg-idx 找。
//
// 长会话的滚动「不准」问题：
//   1) chatMsgs 被赋值后，巨型 v-for 要一两帧才把 .msg-row 真正挂上 DOM；
//   2) 挂上之后，里头的代码高亮 / DiffBlock / 图片还会异步把内容塞进去，
//      命中行的 offsetTop 会继续往下推。
// 应对：先「等 row 出现」最多 ~500ms；找到之后启动一个 rAF 循环，每帧重读
// offsetTop 让滚动追上后涨的高度；动画窗口（~360ms）结束后再校准 ~1.2s。
// 任何 wheel / pointerdown / keydown 都立即让位，绝不和用户抢滚动条。
const flashIdx = ref<number | null>(null)
let flashTimer = 0
let flashStickCleanup: (() => void) | null = null
function cancelFlashStick() {
  flashStickCleanup?.()
}
function flashMessage(idx: number, uuid?: string) {
  const sa = scrollEl.value
  if (!sa) return
  // uuid 优先（能扛重排）；否则按下标。定位到 messages 里的真实下标交给虚拟器。
  let realIdx = idx
  if (uuid) {
    const found = props.messages.findIndex((m) => m.uuid === uuid)
    if (found >= 0) realIdx = found
  }
  if (realIdx < 0 || realIdx >= props.messages.length) return

  cancelFlashStick()
  cancelScroll()

  // 虚拟器直接滚到目标行并居中。动态高度下,目标行挂载/测量、以及其上方行被 Shiki/图片
  // 撑高后偏移会变 —— 用一个短 rAF 循环反复 scrollToIndex 把它稳定到位;用户一动即让位。
  rowVirtualizer.value.scrollToIndex(realIdx, { align: 'center' })
  let userBailed = false
  const onUserInput = () => {
    userBailed = true
  }
  sa.addEventListener('wheel', onUserInput, { passive: true })
  sa.addEventListener('pointerdown', onUserInput, { passive: true })
  sa.addEventListener('keydown', onUserInput)
  const t0 = performance.now()
  let raf = 0
  const tick = () => {
    if (userBailed || performance.now() - t0 > 1200) return cleanup()
    rowVirtualizer.value.scrollToIndex(realIdx, { align: 'center' })
    raf = requestAnimationFrame(tick)
  }
  const cleanup = () => {
    if (raf) cancelAnimationFrame(raf)
    sa.removeEventListener('wheel', onUserInput)
    sa.removeEventListener('pointerdown', onUserInput)
    sa.removeEventListener('keydown', onUserInput)
    flashStickCleanup = null
  }
  flashStickCleanup = cleanup
  raf = requestAnimationFrame(tick)

  // 闪烁：先清状态,下一帧再写,确保 CSS 动画从头跑。
  flashIdx.value = null
  requestAnimationFrame(() => {
    flashIdx.value = realIdx
    window.clearTimeout(flashTimer)
    flashTimer = window.setTimeout(() => {
      flashIdx.value = null
    }, 1400)
  })
}
// ============================ Live tail: 自动跟随 + "N 条新" pill ============================
//
// 设计：当后端 emit session:append 后，App.vue 把新 Msg 推进 messages，
// 然后调 onLiveAppend(n) 让本组件决定怎么回应：
//   - 用户当前接近底部（100px 以内）→ 自动平滑滚到底，pill 不出现；
//   - 否则 → 在 pill 上累加 N，用户点 pill 才滚到底。
//
// 切换会话 / 关闭后重新打开同一会话时，watch(session.path) 把 newCount 归零，
// 避免把上一条会话的"未读"带到下一条。
const newCount = ref(0)
// 100px 阈值：比 atBottom 用的 8px 宽松得多，鼓励"贴着底"的常态自动跟随。
const FOLLOW_THRESHOLD = 100
function isNearBottom(): boolean {
  const el = scrollEl.value
  if (!el) return true
  return el.scrollTop + el.clientHeight >= el.scrollHeight - FOLLOW_THRESHOLD
}

function onLiveAppend(addedCount: number) {
  if (addedCount <= 0) return
  if (isNearBottom()) {
    // 等新行布局完成再滚 —— 否则 scrollHeight 还是旧值。
    requestAnimationFrame(() => {
      scrollToBottom()
      newCount.value = 0
    })
  } else {
    newCount.value += addedCount
  }
}

function jumpToNewest() {
  newCount.value = 0
  scrollToBottom()
}

// 切换到不同会话 → 清掉"未读"计数。
watch(
  () => props.session?.path,
  () => {
    newCount.value = 0
  },
)

defineExpose({ flashMessage, onLiveAppend })

// 到顶 / 到底时分别隐藏对应方向的 FAB，留一点 8px 阈值避免抖动
const atTop = ref(true)
const atBottom = ref(true)
function updateEdges() {
  const el = scrollEl.value
  if (!el) return
  atTop.value = el.scrollTop <= 8
  atBottom.value = el.scrollTop + el.clientHeight >= el.scrollHeight - 8
  if (atBottom.value && newCount.value > 0) {
    newCount.value = 0
  }
}
let rafEdge = 0
function onScroll() {
  if (rafEdge) return
  rafEdge = requestAnimationFrame(() => {
    rafEdge = 0
    updateEdges()
    // 装饰(高亮/mermaid)由「真实滚动」驱动,而非 virtualRows 的响应式变化 —— 否则装饰改行高
    // → measureElement 重测 → virtualRows 变 → 又装饰,即使静止也空转打满 CPU。滚动停 = 不再装饰。
    scheduleDecorate()
  })
}
// 悬浮 tip：事件委托挂在滚动容器上（v-html 动态节点挂不上指令）。两类目标共用：
// 命令 token（.cmd-name，描述来自 data-cmd-desc）与文件引用（.file-ref，固定提示「打开文件」）。
// 只在「进入新目标」时触发，避免在同一元素内移动反复重置延时。
let cmdHoverEl: HTMLElement | null = null
function onCmdOver(e: MouseEvent) {
  const el = (e.target as HTMLElement)?.closest?.(
    '.cmd-name[data-cmd-desc], .file-ref[data-file-ref]',
  ) as HTMLElement | null
  if (el === cmdHoverEl) return
  cmdHoverEl = el
  if (!el) {
    hideTooltip()
    return
  }
  showTooltipFor(el, el.dataset.cmdDesc != null ? el.dataset.cmdDesc || '' : t('chat.file.open'))
}
function onCmdLeave() {
  cmdHoverEl = null
  hideTooltip()
}

// 文件引用 code（形如 a/b.ts:12，见 format.ts 的 .file-ref）点击：在外部编辑器打开。
// 相对 / 部分路径按会话 cwd 解析；末尾 :行[:列] 拆出来传给后端 —— 装了支持跳行的编辑器就跳到该行。
function onContentClick(e: MouseEvent) {
  const el = (e.target as HTMLElement)?.closest?.('.file-ref[data-file-ref]') as HTMLElement | null
  if (!el) return
  const raw = el.dataset.fileRef || ''
  if (!raw) return
  e.preventDefault()
  const { path, line, col } = parseFileRef(raw)
  void openPathExternal(path, props.cwd || props.session?.cwd, line, col).catch((err) => {
    console.warn('[chat] open file ref failed:', err)
  })
}
onMounted(() => {
  document.addEventListener('keydown', onGuiEscape, true)
  scrollEl.value?.addEventListener('scroll', onScroll, { passive: true })
  scrollEl.value?.addEventListener('mouseover', onCmdOver)
  scrollEl.value?.addEventListener('mouseleave', onCmdLeave)
  scrollEl.value?.addEventListener('click', onContentClick)
  // 内容渲染完再算一次（长消息列表挂载后 scrollHeight 才稳定）
  requestAnimationFrame(updateEdges)
})
onUnmounted(() => {
  document.removeEventListener('keydown', onGuiEscape, true)
  scrollEl.value?.removeEventListener('scroll', onScroll)
  scrollEl.value?.removeEventListener('mouseover', onCmdOver)
  scrollEl.value?.removeEventListener('mouseleave', onCmdLeave)
  scrollEl.value?.removeEventListener('click', onContentClick)
  if (rafEdge) cancelAnimationFrame(rafEdge)
  cancelScroll()
  cancelFlashStick()
  window.clearTimeout(flashTimer)
})

// ============================ 顶栏功能：折叠工具 / 搜索 ============================

const innerEl = ref<HTMLElement>()

// ---- 折叠状态持久化（虚拟滚动友好）
//
// 虚拟滚动会卸载并重建 DOM 行，原生 <details open> 状态随之丢失。
// 用一个 reactive Set 保存哪些 details 被用户手动打开过（key = "msgIdx-blockIdx"），
// 模板用 :open + @toggle 双向同步。一键折叠/展开时 clear / fill 整个 Set。
const openDetails = reactive(new Set<string>())
const explicitDetails = reactive(new Set<string>())

function detailKey(mi: number, bi: number, suffix?: string): string {
  return suffix ? `${mi}-${bi}-${suffix}` : `${mi}-${bi}`
}
function isDetailOpen(mi: number, bi: number, suffix?: string): boolean | undefined {
  const key = detailKey(mi, bi, suffix)
  return openDetails.has(key) ? true : explicitDetails.has(key) ? false : undefined
}
function onDetailToggle(mi: number, bi: number, ev: Event) {
  const el = ev.target as HTMLDetailsElement
  const key = detailKey(mi, bi)
  explicitDetails.add(key)
  if (el.open) openDetails.add(key)
  else openDetails.delete(key)
}
function onResultToggle(mi: number, bi: number, open: boolean) {
  const key = detailKey(mi, bi, 'r')
  explicitDetails.add(key)
  if (open) openDetails.add(key)
  else openDetails.delete(key)
}

function sweepDetails(open: boolean) {
  if (open) {
    for (let mi = 0; mi < props.messages.length; mi++) {
      const m = props.messages[mi]
      if (m.metaKind) openDetails.add(detailKey(mi, -1))
      for (let bi = 0; bi < m.blocks.length; bi++) {
        const b = m.blocks[bi]
        if (b.kind === 'thinking' || b.kind === 'tool_use') {
          openDetails.add(detailKey(mi, bi))
          openDetails.add(detailKey(mi, bi, 'r'))
        }
        if (b.kind === 'tool_result') {
          openDetails.add(detailKey(mi, bi, 'r'))
        }
      }
    }
  } else {
    openDetails.clear()
    explicitDetails.clear()
  }
}
watch(toolsCollapsed, (v) => sweepDetails(!v))

// ---- 消息内搜索：DOM walker 把匹配文本包成 <mark class="search-hit">
//
// 不修改渲染管线（renderText 走 v-html），而是渲染完之后再扫一遍 DOM，
// 把所有匹配的纯文本节点替换成带 <mark> 的片段。然后维护一组 mark 元素
// 让 ↑/↓ 按钮 / Enter 键能在它们之间跳转。messages / search 变化时整体重做。

let marks: HTMLElement[] = []
let searchDebounce = 0

function unmarkAll() {
  const root = innerEl.value
  if (!root) return
  const list = root.querySelectorAll<HTMLElement>('mark.search-hit')
  list.forEach((m) => {
    const parent = m.parentNode
    if (!parent) return
    parent.replaceChild(document.createTextNode(m.textContent ?? ''), m)
    parent.normalize()
  })
  marks = []
}

// ---- 数据驱动搜索（虚拟滚动版）----
// 旧实现遍历整棵 DOM 打 <mark> 计数,但虚拟滚动下 DOM 里只有可见行 —— 搜不到窗口外的消息。
// 改为：命中「计数 / 定位」在 messages 数据上算（覆盖全部消息,含未挂载的）；DOM 打标只对
// 当前可见行做（滚到哪标到哪）。粒度为「消息级」：searchCount = 命中的消息条数,上一条/下一条
// 在命中消息间跳转,滚到该消息后再在其行内高亮、并把首个 mark 设为 current。
let searchHits: number[] = [] // 命中的 message 下标（升序）

function scopePasses(scope: string, filter: string): boolean {
  if (filter === 'all') return true
  if (filter === 'user') return scope === 'user'
  if (filter === 'agent') return scope === 'assistant' || scope === 'tools-edit'
  if (filter === 'tools') return scope === 'tools-other'
  return true
}
// 一条消息里各可搜片段的 (scope, text) —— 与模板给 DOM 打的 data-search-scope 对齐。
function messageSegments(m: Msg): { scope: string; text: string }[] {
  const rscope = rowScope(m)
  const segs: { scope: string; text: string }[] = []
  for (const b of m.blocks) {
    if (b.kind === 'tool_use') {
      const attached = attachedResultFor(b)
      if (!shouldShowToolUse(b)) {
        // 文件变更卡独立保留、tool_use 本身隐藏时，搜索仍应能找到文件路径和结果。
        if (attached?.text) segs.push({ scope: 'tools-edit', text: attached.text })
        if (attached?.filePath) segs.push({ scope: 'tools-edit', text: attached.filePath })
        continue
      }
      const r = inlinedResultFor(b) ?? attached
      const s = r && isFileChangeResult(r) ? 'tools-edit' : toolUseScope(b)
      segs.push({ scope: s, text: `${b.toolName ?? ''} ${b.toolInput ?? ''}` })
      if (r?.text) segs.push({ scope: s, text: r.text })
      if (r?.filePath) segs.push({ scope: s, text: r.filePath })
    } else if (b.kind === 'tool_result') {
      // AskUserQuestion 的协议回执已经并入 Claude 的提问卡；不再让隐藏的
      // user/tool_result 行单独制造一个搜索命中。
      if (shouldShowToolResult(b)) {
        segs.push({ scope: isToolOnly(m) ? 'tools-edit' : rscope, text: b.text ?? '' })
      }
    } else if (b.text) {
      segs.push({ scope: rscope, text: b.text })
    }
    if (b.filePath) segs.push({ scope: rscope, text: b.filePath })
  }
  return segs
}
function computeSearchHits() {
  const q = search.value.trim().toLowerCase()
  if (!q) {
    searchHits = []
    searchCount.value = 0
    searchIndex.value = 0
    return
  }
  const filter = searchScope.value
  const hits: number[] = []
  for (let i = 0; i < props.messages.length; i++) {
    const m = props.messages[i]
    if (!rowHasContent(m)) continue
    if (isHidden(m, i) && !showHidden.value) continue // 隐藏行 = display:none,不参与搜索（与旧行为一致）
    for (const seg of messageSegments(m)) {
      if (scopePasses(seg.scope, filter) && seg.text.toLowerCase().includes(q)) {
        hits.push(i)
        break
      }
    }
  }
  searchHits = hits
  searchCount.value = hits.length
  searchIndex.value = hits.length ? Math.min(searchIndex.value || 1, hits.length) : 0
}

// 对当前挂载的可见行打 <mark>（不改计数 / 当前项）。滚动进新行时由 scheduleDecorate 调用。
function markVisibleSearch(scroll = true) {
  unmarkAll()
  const root = innerEl.value
  const q = search.value.trim()
  if (!q || !root) return
  const lower = q.toLowerCase()
  const filter = searchScope.value
  const scopeOk = (parent: HTMLElement): boolean => {
    if (filter === 'all') return true
    const node = parent.closest<HTMLElement>('[data-search-scope]')
    const scope = node?.dataset.searchScope ?? null
    return scopePasses(scope ?? '', filter)
  }
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const txt = node.textContent
      if (!txt || !txt.toLowerCase().includes(lower)) return NodeFilter.FILTER_REJECT
      const parent = (node as Text).parentElement
      if (!parent) return NodeFilter.FILTER_REJECT
      const tag = parent.tagName
      if (tag === 'SCRIPT' || tag === 'STYLE') return NodeFilter.FILTER_REJECT
      if (!scopeOk(parent)) return NodeFilter.FILTER_REJECT
      return NodeFilter.FILTER_ACCEPT
    },
  })
  const targets: Text[] = []
  let n: Node | null
  while ((n = walker.nextNode())) targets.push(n as Text)
  const collected: HTMLElement[] = []
  for (const text of targets) {
    const s = text.data
    const lowerS = s.toLowerCase()
    const frag = document.createDocumentFragment()
    let cur = 0
    let idx = lowerS.indexOf(lower, cur)
    while (idx >= 0) {
      if (idx > cur) frag.appendChild(document.createTextNode(s.slice(cur, idx)))
      const mark = document.createElement('mark')
      mark.className = 'search-hit'
      mark.textContent = s.slice(idx, idx + lower.length)
      frag.appendChild(mark)
      collected.push(mark)
      cur = idx + lower.length
      idx = lowerS.indexOf(lower, cur)
    }
    if (cur < s.length) frag.appendChild(document.createTextNode(s.slice(cur)))
    text.parentNode?.replaceChild(frag, text)
  }
  marks = collected
  applyCurrentClass(scroll)
}

function currentHitMsgIndex(): number | null {
  if (searchIndex.value < 1 || searchIndex.value > searchHits.length) return null
  return searchHits[searchIndex.value - 1]
}
// 把当前命中消息行内的第一个 mark 设为 .current,展开其祖先 details,并滚到视区中部。
function applyCurrentClass(scroll = true) {
  marks.forEach((mk) => mk.classList.remove('current'))
  const mi = currentHitMsgIndex()
  if (mi == null) return
  const row = innerEl.value?.querySelector<HTMLElement>(`.msg-vrow[data-index="${mi}"]`)
  const first = row?.querySelector<HTMLElement>('mark.search-hit')
  if (!first) return
  first.classList.add('current')
  let p: HTMLElement | null = first.parentElement
  while (p) {
    if (p.tagName === 'DETAILS' && !(p as HTMLDetailsElement).open) {
      ;(p as HTMLDetailsElement).open = true
    }
    p = p.parentElement
  }
  if (scroll) first.scrollIntoView({ block: 'center' })
}

// 跳到第 k 个命中消息（1-based,环绕）：虚拟器滚过去,等行挂载后标记 + 高亮当前。
function gotoHit(k: number) {
  if (!searchHits.length) return
  const idx = ((k - 1 + searchHits.length) % searchHits.length) + 1
  searchIndex.value = idx
  const mi = searchHits[idx - 1]
  rowVirtualizer.value.scrollToIndex(mi, { align: 'center' })
  // 用 setTimeout 而非 rAF 轮询等待目标行挂载：窗口被遮挡(visibilityState=hidden)时 rAF 会
  // 被暂停,marking 就永远不跑;Vue 的渲染调度不依赖 rAF,所以 setTimeout 能在隐藏时照常收敛。
  let tries = 0
  const settle = () => {
    const row = innerEl.value?.querySelector(`.msg-vrow[data-index="${mi}"]`)
    if (!row && tries++ < 20) {
      setTimeout(settle, 16)
      return
    }
    markVisibleSearch() // 内部会 applyCurrentClass 高亮当前命中
  }
  setTimeout(settle, 16)
}

// 重新计算命中集并跳到第一个（输入框变更 / 切换范围时）。
function applySearch() {
  computeSearchHits()
  if (searchHits.length) gotoHit(1)
  else unmarkAll()
}

function navigateMatches(dir: 1 | -1) {
  if (searchHits.length === 0) return
  gotoHit(searchIndex.value + dir)
}

watch(search, () => {
  // 短文本输入会快速变更，debounce 避免每按一键都重写一遍 DOM
  window.clearTimeout(searchDebounce)
  searchDebounce = window.setTimeout(applySearch, 120)
})

// 切换搜索范围时立即重做（不 debounce —— 是离散操作）
watch(searchScope, () => {
  if (search.value) applySearch()
})

// 消息变化（切换会话 / 刷新）后重新建立标记 + 重新 sweep 折叠态 + 渲染 mermaid 占位符
// live GUI chat 自动跟随：必须在「消息变化导致 DOM 撑高之前」判断用户是否贴底，
// 否则新消息一来 scrollHeight 立刻变大、isNearBottom 误判为 false，就再也不跟随了。
// flush:'pre' 的 watcher 在 re-render 前跑，此刻 scrollTop/scrollHeight 还是旧值。
// 对当前挂在 DOM 里的可见行补做高亮 / mermaid / 装饰 / 折叠态。虚拟滚动下只有窗口内的行
// 存在,这些函数都跳过已处理节点（:not([data-shiki]) 等）,所以每次只碰新进入的行,成本≈仅可见行。
function decorateVisible() {
  const root = innerEl.value ?? null
  renderAllMermaid(root)
  renderAllMath(root)
  highlightAllCodeBlocks(root)
  decorateCodeBlocks(root)
}
// 滚动使可见窗口频繁变化 —— 用 rAF 合并,一帧最多跑一次。
//
// ⚠️ 关键：只在「可见行的下标区间」真正变化时才装饰。否则会陷入死循环：装饰（高亮/mermaid）
// 改了行高 → measureElement 的 ResizeObserver 重测 → 虚拟器调整 start → virtualRows 变（对象
// 变了但可见行没变）→ watch 又触发装饰 …… 每帧空转把 CPU 打满。用首/末下标当指纹,重排但下标
// 区间不变就跳过,反馈环即断。
let decorateRAF = 0
let lastVisibleKey = ''
function scheduleDecorate() {
  const rows = virtualRows.value
  const key = rows.length ? `${rows[0].index}-${rows[rows.length - 1].index}` : ''
  if (key === lastVisibleKey) return
  lastVisibleKey = key
  if (decorateRAF) return
  decorateRAF = requestAnimationFrame(() => {
    decorateRAF = 0
    decorateVisible()
    if (search.value) markVisibleSearch(false)
  })
}
// 注意：不要 watch(virtualRows) 来触发装饰 —— 会形成 装饰→改行高→重测→virtualRows 变→装饰
// 的死循环。装饰改由 onScroll（真实滚动）驱动;静止时不跑,反馈环天然断开。

let wasAtBottomBeforeUpdate = true
watch(
  () => props.messages,
  () => {
    if (props.liveSession) wasAtBottomBeforeUpdate = isNearBottom()
  },
)
watch(
  () => props.messages,
  () => {
    lastVisibleKey = '' // 消息集变了,让下次滚动重新装饰
    nextTick(() => {
      decorateVisible()
      if (search.value) applySearch()
      // 聊天进行中：变化前贴底 → 钉到最新消息（钉一小段，扛住高亮/图片异步撑高）。
      if (props.liveSession && wasAtBottomBeforeUpdate) {
        pinToBottomFor(220)
      }
    })
  },
  { flush: 'post' },
)

// §10.6 流式即时渲染：delta 已在 chatSessions 以 50ms 合并；这里仍即时形成 Markdown，
// 但不把每个不断增长的中间前缀写进定稿消息 LRU，避免长回答积累大量一次性 HTML。
const streamingHtml = computed(() => {
  const lv = props.liveSession?.live
  return lv ? renderTextUncached(lv.text) : ''
})

// §10.6 流式：合并后的预览更新时只贴底一次。异步高亮/图片只会出现在权威消息，仍由
// messages watcher 的 pinToBottomFor 兜住；无需每个流式批次都重启 120ms 的逐帧循环。
watch(
  () => props.liveSession?.live?.text,
  () => {
    if (!props.liveSession?.live) return
    if (isNearBottom()) {
      nextTick(() => {
        // turn 开始或权威消息的 pin 循环若仍在运行，它已经负责贴底，不重复写布局。
        if (pinRAF) return
        const el = scrollEl.value
        if (el) el.scrollTop = el.scrollHeight
      })
    }
  },
)

// 用户发起一轮（turnState idle→running 仅由 sendPrompt 触发）→ **无视当前滚动位置**，
// 强制跳到底部并跟随这一轮的最新消息（用户回显 + agent 回复）。这是聊天的标准行为：
// 发消息总该看到自己刚发的那条 + 随后的回答，哪怕之前滚到了中间。之后若用户手动上滑读
// 历史，isNearBottom() 转 false，message/delta watcher 自动停止跟随、不打扰。
watch(
  () => props.liveSession?.turnState,
  (ts, prev) => {
    if (ts === 'running' && prev !== 'running') {
      wasAtBottomBeforeUpdate = true
      nextTick(() => pinToBottomFor(500))
    }
  },
)

// 权限确认 / 结构化提问不是 messages 的一部分，前面的消息 watcher 不会触发。
// 这类卡片需要用户立即处理，所以一出现就强制带到视野底部。
watch(
  () => {
    const s = props.liveSession
    const permissions = s?.pendingPermissions ?? []
    const questions = s?.pendingQuestions ?? []
    const permTail = permissions.length ? permissions[permissions.length - 1].requestId : ''
    const questionTail = questions.length ? questions[questions.length - 1].requestId : ''
    return `${s?.pendingPermissions.length ?? 0}:${permTail}|${s?.pendingQuestions.length ?? 0}:${questionTail}`
  },
  (key, prev) => {
    if (!props.liveSession || key === prev || key === '0:|0:') return
    newCount.value = 0
    wasAtBottomBeforeUpdate = true
    nextTick(() => pinToBottomFor(700))
  },
  { flush: 'post' },
)

// 会话所属项目 key 迁移（worktree 首个会话落盘 → 合成 key 并入真实项目）时，外层布局会 reflow。
// 虚拟列表缓存的滚动几何随之失效，而它只在真实滚动时重算范围 → getVirtualItems() 变空 → 整列
// 消息渲染空白（用户反馈：codex worktree 首轮空白，要再发一次才出来）。这里在 key 变化后强制
// 重测虚拟器并（在底部时）重新钉底，逼它立刻重算可见行。
watch(
  () => props.liveSession?.projectKey,
  (key, prev) => {
    if (!key || key === prev) return
    nextTick(() => {
      rowVirtualizer.value.measure()
      if (isNearBottom()) pinToBottomFor(300)
    })
  },
)

// 主题切换：mermaid 不能运行时换色，要把已渲染节点 reset 再 redraw。
watch(theme, () => {
  nextTick(() => {
    resetMermaidForTheme(innerEl.value ?? null)
    renderAllMermaid(innerEl.value ?? null)
    rehighlightAllCodeBlocks(innerEl.value ?? null)
  })
})

onMounted(() => {
  setSearchNavigator(navigateMatches)
  document.addEventListener('click', onDocClick)
  // 初次挂载也跑一遍 —— 会话已经有 messages 时 watch 不会触发。
  nextTick(() => {
    measureListMargin() // 量列表相对滚动容器顶端的偏移,scrollToIndex 才对得准
    decorateVisible()
    // live GUI chat：进入时（含入口 3 切换 / 列表续聊的预载历史）钉到底部一会儿，
    // 露出最新上下文 + composer，扛住代码高亮/图片异步撑高。
    if (props.liveSession) {
      wasAtBottomBeforeUpdate = true
      pinToBottomFor(600)
    }
  })
})

// live ChatView 可能复用读模式的实例（同为 <ChatView> 组件），patch-in-place 时
// onMounted 不会重跑 —— 监听 liveSession 由空变有，确保「切换进 chat」也钉到底。
watch(
  () => props.liveSession,
  (ls) => {
    if (ls) {
      wasAtBottomBeforeUpdate = true
      nextTick(() => pinToBottomFor(600))
    }
  },
)
onUnmounted(() => {
  setSearchNavigator(null)
  window.clearTimeout(searchDebounce)
  window.clearTimeout(finishedTurnTimer)
  unmarkAll()
  document.removeEventListener('click', onDocClick)
})

// 导出下拉菜单：点空白处关闭。锚定到导出按钮容器，点容器内的项不触发关闭。
const exportMenuOpen = ref(false)
const exportMenuEl = ref<HTMLElement>()
function toggleExportMenu(e: Event) {
  e.stopPropagation()
  exportMenuOpen.value = !exportMenuOpen.value
  locateMenuOpen.value = false
}
// composer 的 `/model`/`/export` 等客户端指令：`/export` 展开右上角导出下拉（与点导出按钮等效）。
function openExportFromComposer() {
  exportMenuOpen.value = true
  locateMenuOpen.value = false
}

// ---- 定位下拉：列出所有用户提问，点击跳转到对应消息。
const locateMenuOpen = ref(false)
const locateMenuEl = ref<HTMLElement>()
const locateFilter = ref('')
const locateInputEl = ref<HTMLInputElement>()
interface PromptEntry {
  idx: number
  seq: number
  uuid?: string
  text: string
  time: string
}
const promptEntries = computed<PromptEntry[]>(() => {
  const entries: PromptEntry[] = []
  for (let i = 0; i < props.messages.length; i++) {
    const m = props.messages[i]
    if (
      m.role !== 'user' ||
      m.metaKind ||
      isToolOnly(m) ||
      isCaveatOnlyMsg(m) ||
      isAskUserQuestionInstructionOnlyMsg(m) ||
      systemEventLabel(m)
    ) continue
    const textBlock = m.blocks.find((b) => b.kind === 'text' && b.text)
    const raw = textBlock?.text ?? ''
    const plain = raw.replace(/<[^>]*>/g, '').trim()
    const text = plain.length > 80 ? plain.slice(0, 80) + '…' : plain || `#${entries.length + 1}`
    entries.push({ idx: i, seq: entries.length + 1, uuid: m.uuid, text, time: formatTime(m.timestamp) })
  }
  return entries
})
const filteredPromptEntries = computed(() => {
  const q = locateFilter.value.trim().toLowerCase()
  if (!q) return promptEntries.value
  return promptEntries.value.filter((e) => e.text.toLowerCase().includes(q))
})
function toggleLocateMenu(e: Event) {
  e.stopPropagation()
  locateMenuOpen.value = !locateMenuOpen.value
  exportMenuOpen.value = false
  if (locateMenuOpen.value) {
    locateFilter.value = ''
    nextTick(() => locateInputEl.value?.focus())
  }
}
function jumpToPrompt(entry: PromptEntry) {
  locateMenuOpen.value = false
  flashMessage(entry.idx, entry.uuid)
}
function highlightLocateText(text: string): string {
  const q = locateFilter.value.trim()
  if (!q) return escapeHtml(text)
  const escaped = escapeHtml(text)
  const qEscaped = escapeHtml(q)
  const re = new RegExp(`(${qEscaped.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`, 'gi')
  return escaped.replace(re, '<mark class="locate-hl">$1</mark>')
}
function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

function onDocClick(e: MouseEvent) {
  if (exportMenuOpen.value) {
    if (!(exportMenuEl.value && exportMenuEl.value.contains(e.target as Node))) {
      exportMenuOpen.value = false
    }
  }
  if (locateMenuOpen.value) {
    if (!(locateMenuEl.value && locateMenuEl.value.contains(e.target as Node))) {
      locateMenuOpen.value = false
    }
  }
}
</script>

<template>
  <div class="chat-head">
    <button class="icon-btn" v-tooltip="t('chat.back')" @click="$emit('back')">
      <IconArrowLeft />
    </button>
    <div class="chat-head-info">
      <div class="t">
        <span class="t-text">{{ session.title }}</span>
        <button
          v-if="!trashed"
          class="title-rename-ic"
          v-tooltip="t('chat.action.rename')"
          @click="$emit('rename')"
        >
          <IconPencil />
        </button>
      </div>
      <div class="s">
        <span>{{
          t('chat.stats', {
            u: stats.u,
            a: stats.a,
            time: formatTime(session.created),
          })
        }}</span>
        <span
          v-if="live && !trashed"
          class="live-badge"
          v-tooltip="t('chat.live.tooltip')"
        >
          <span class="live-dot" />
          <span class="live-label">{{ t('chat.live') }}</span>
        </span>
        <span v-if="sessionArchived" class="codex-special-tag">
          {{ t('list.codex.archived') }}
        </span>
        <span v-if="session.id" class="session-id" v-tooltip="session.id">
          <span class="session-id-label">{{ t('session.id') }}</span>
          <span class="session-id-text">{{ shortId(session.id) }}</span>
          <button
            class="session-id-copy"
            v-tooltip="t('chat.action.copyId')"
            @click="$emit('copyId')"
          >
            <IconCopy />
          </button>
        </span>
        <GitBranchControl
          :cwd="cwd || session.cwd"
          :disabled="!!liveSession && liveSession.turnState !== 'idle'"
        />
      </div>
    </div>
    <!-- 会话统计 + 折叠 Tool calls：原本住在 ChatTopbar.ct-actions 里，
         与 chat-head 的 5 个会话级 icon 隔一行 40px topbar 在同一垂直线上。
         挪进 chat-head 后顶栏只剩 scope+search 一条横线。toolsCollapsed
         走 chatToolbar 模块 ref 共享，原 ChatTopbar 的对应按钮已删除。 -->
    <button
      v-if="!trashed && agent !== 'agy'"
      class="icon-btn"
      v-tooltip="t('chat.tb.sessionStats')"
      @click="$emit('openSessionStats')"
    >
      <IconChart />
    </button>
    <div ref="locateMenuEl" class="locate-menu-wrap">
      <button
        class="icon-btn"
        :class="{ active: locateMenuOpen }"
        v-tooltip="t('chat.tb.locate')"
        @click="toggleLocateMenu"
      >
        <IconLocate />
      </button>
      <div v-if="locateMenuOpen" class="locate-menu" role="menu">
        <div class="locate-menu-search">
          <input
            ref="locateInputEl"
            v-model="locateFilter"
            class="locate-search-input"
            :placeholder="t('chat.tb.locate.placeholder')"
            @keydown.escape.stop="locateMenuOpen = false"
          />
        </div>
        <div class="locate-menu-list">
          <button
            v-for="entry in filteredPromptEntries"
            :key="entry.idx"
            class="locate-menu-item"
            role="menuitem"
            @click="jumpToPrompt(entry)"
          >
            <span class="locate-item-idx">#{{ entry.seq }}</span>
            <span class="locate-item-text" v-html="highlightLocateText(entry.text)"></span>
            <span class="locate-item-time">{{ entry.time }}</span>
          </button>
          <div v-if="!filteredPromptEntries.length" class="locate-menu-empty">
            {{ t('chat.empty') }}
          </div>
        </div>
      </div>
    </div>
    <button
      class="icon-btn"
      v-tooltip="
        toolsCollapsed
          ? t('chat.tb.tools.expand')
          : t('chat.tb.tools.collapse')
      "
      @click="toggleTools"
    >
      <component :is="toolsCollapsed ? IconUnfold : IconFold" />
    </button>
    <button
      v-if="hiddenCount > 0"
      class="icon-btn"
      :class="{ active: showHidden }"
      v-tooltip="showHidden ? t('chat.action.hideHidden') : t('chat.action.showHidden')"
      @click="showHidden = !showHidden"
    >
      <component :is="showHidden ? IconEye : IconEyeOff" />
      <span class="hidden-badge">{{ hiddenCount }}</span>
    </button>
    <button
      v-if="!liveSession"
      class="icon-btn tool-calls-toggle"
      :class="{ active: readToolCallsVisible, 'is-hidden': !readToolCallsVisible }"
      v-tooltip="readToolCallsVisible ? t('chat.action.hideToolCalls') : t('chat.action.showToolCalls')"
      @click="toggleReadToolCalls"
    >
      <IconWrench />
    </button>
    <span class="chat-head-sep" />
    <!-- 在窗口内 resume（TUI）：仅只读详情。live chat 里已在对话中，无需再开 TUI tab。 -->
    <button
      v-if="!liveSession && !trashed"
      class="icon-btn"
      :class="{ disabled: !canResumeHere }"
      v-tooltip="canResumeHere ? t('chat.action.resumeHere') : t('chat.action.resumeUnavailable')"
      :disabled="!canResumeHere"
      @click="canResumeHere && $emit('resumeHere')"
    >
      <IconPlay />
    </button>
    <!-- 打开目录 / 导出：read 与 live chat 两种模式都需要。 -->
    <button
      v-if="!trashed && agent !== 'opencode'"
      class="icon-btn"
      v-tooltip="t('chat.action.reveal')"
      @click="$emit('reveal')"
    >
      <IconFolder />
    </button>
    <!-- 刷新：仅只读详情。live chat 是实时流，无需手动刷新。 -->
    <button
      v-if="!liveSession && !trashed"
      class="icon-btn"
      v-tooltip="t('chat.action.refresh')"
      @click="$emit('refresh')"
    >
      <IconRefresh />
    </button>
    <span v-if="!trashed" class="chat-head-sep" />
    <div v-if="!trashed" ref="exportMenuEl" class="export-menu-wrap">
      <button
        class="icon-btn"
        :class="{ active: exportMenuOpen }"
        v-tooltip:top="t('chat.tb.export.md') + ' / ' + t('chat.tb.export.html')"
        @click="toggleExportMenu"
      >
        <IconDownload />
      </button>
      <div v-if="exportMenuOpen" class="export-menu" role="menu">
        <button
          class="export-menu-item"
          role="menuitem"
          @click="exportMenuOpen = false; $emit('exportMd')"
        >
          <IconMarkdown />
          <span>{{ t('chat.tb.export.md') }}</span>
        </button>
        <button
          class="export-menu-item"
          role="menuitem"
          @click="exportMenuOpen = false; $emit('exportHtml')"
        >
          <IconHtml />
          <span>{{ t('chat.tb.export.html') }}</span>
        </button>
        <button
          class="export-menu-item"
          role="menuitem"
          @click="exportMenuOpen = false; $emit('exportJson')"
        >
          <IconJson />
          <span>{{ t('chat.tb.export.json') }}</span>
        </button>
      </div>
    </div>
    <!-- 删除：read 与 live chat 两种模式都有（chat 里删 = 软删并停掉当前会话）。 -->
    <button
      v-if="!trashed"
      class="icon-btn danger"
      v-tooltip="t('chat.action.delete')"
      @click="$emit('delete')"
    >
      <IconTrash />
    </button>
    <button
      v-if="!liveSession && trashed"
      class="icon-btn chat-restore-btn"
      v-tooltip="t('trash.restore')"
      @click="$emit('restore')"
    >
      <IconRestore />
    </button>
  </div>

  <div ref="scrollEl" class="chat-scroll" :class="{ 'has-composer': !!liveSession }">
    <div ref="innerEl" class="chat-inner">
      <!-- 虚拟滚动列表：只有可见窗口 + overscan 的行真正挂 DOM。外层 .chat-vlist 撑满整段
           虚拟总高（滚动条几何）；每个 .msg-vrow 绝对定位到自己的 translateY,并绑 measureRow
           动态测高。内层用单元素 v-for 把当前行的消息重新命名回 `m`,行体保持原样。 -->
      <div
        ref="vlistEl"
        class="chat-vlist"
        :style="{ height: totalSize + 'px', position: 'relative', width: '100%' }"
      >
      <div
        v-for="vr in virtualRows"
        :key="vr.index"
        :ref="measureRow"
        :data-index="vr.index"
        class="msg-vrow"
        :style="{ position: 'absolute', top: 0, left: 0, width: '100%', transform: `translateY(${vr.start - listScrollMargin}px)` }"
      >
      <template v-for="m in [messages[vr.index]]" :key="vr.index">
      <div
        v-show="rowHasContent(m) && (!isHidden(m, vr.index) || showHidden)"
        class="msg-row"
        :class="[
          contextUsageOf(m) ? 'assistant' : systemEventLabel(m) ? 'system' : m.metaKind ? 'meta' : isToolOnly(m) ? 'tool-only' : effectiveRole(m),
          { 'msg-flash': flashIdx === vr.index, 'msg-hidden': isHidden(m, vr.index) && showHidden, 'row-active': hoveredKey === msgKey(m, vr.index) },
        ]"
        :data-search-scope="rowScope(m)"
        :data-msg-idx="vr.index"
        :data-msg-uuid="m.uuid ?? ''"
        @mouseenter="hoveredKey = msgKey(m, vr.index)"
        @mouseleave="hoveredKey === msgKey(m, vr.index) && (hoveredKey = null)"
      >
        <!-- /context 的可视化面板：放在最前，无论它来自 live 的 assistant 消息还是离线回看的
             command-output 落盘记录，都升级成可折叠卡片（而非灰泡里的原始 markdown 表 / 命令输出框）。 -->
        <ContextWindowCard v-if="contextUsageOf(m)" :usage="contextUsageOf(m)!" />

        <!-- System events (e.g. /rename) render as a small centered line,
             not a "Me" bubble — they're meta facts, not user prose. -->
        <div v-else-if="systemEventLabel(m)" class="system-event">
          {{ systemEventLabel(m) }}
        </div>

        <!-- System-injected user records (compaction summary, skill injection,
             task notifications, command output). Not "Me" prose — render like an
             assistant turn. Command output / system notes use the same lightweight
             disclosure as thinking; other meta records retain their labeled card. -->
        <div v-else-if="m.metaKind" class="bubble meta-msg">
          <div class="role-tag">
            <span class="name">
              <component :is="agentIcons[agent]" class="agent-icon" :class="agent" />
              {{ assistantName }}
            </span>
          </div>
          <details
            :class="usesLightweightMetaStyle(m.metaKind) ? 'thinking-block meta-lightweight-block' : 'block-card'"
            :open="isDetailOpen(vr.index, -1)"
            @toggle="onDetailToggle(vr.index, -1, $event)"
          >
            <summary :class="usesLightweightMetaStyle(m.metaKind) ? 'thinking-summary' : 'block-summary'">
              <template v-if="usesLightweightMetaStyle(m.metaKind)">
                <component :is="lightweightMetaIcon(m.metaKind)" class="thinking-icon" aria-hidden="true" />
                <span class="thinking-label">{{ metaKindLabel(m.metaKind) }}</span>
                <span class="thinking-chev"><IconChevronRight /></span>
              </template>
              <template v-else>
                <span class="chev"><IconChevronRight /></span>
                <span class="label meta-summary-label">{{ metaKindLabel(m.metaKind) }}</span>
              </template>
            </summary>
            <div :class="usesLightweightMetaStyle(m.metaKind) ? 'thinking-content' : 'block-body'">
              <template v-for="(b, bi) in m.blocks" :key="bi">
                <template v-if="b.kind === 'text'">
                  <dl v-if="metaFieldsOf(b.text ?? '')" class="meta-fields">
                    <template v-for="(f, fi) in metaFieldsOf(b.text ?? '')!" :key="fi">
                      <dt class="meta-field-key">{{ f.key }}</dt>
                      <dd class="meta-field-val">{{ f.value }}</dd>
                    </template>
                  </dl>
                  <pre v-else-if="metaIsPre(m)">{{ cleanMetaText(b.text ?? '') }}</pre>
                  <div v-else class="text-run" v-html="metaBlockHtml(m.metaKind, b.text ?? '')" />
                </template>
              </template>
            </div>
          </details>
        </div>

        <div v-else-if="isToolOnly(m)" style="max-width: 86%; min-width: 0">
          <template v-for="(b, bi) in m.blocks" :key="bi">
            <ToolResult
              v-if="b.kind === 'tool_result' && shouldShowToolResult(b)"
              :block="b"
              :cwd="cwd"
              :persist-open="isDetailOpen(vr.index, bi, 'r')"
              @toggle="v => onResultToggle(vr.index, bi, v)"
            />
          </template>
        </div>

        <template v-else>
          <!-- 图片缩略图：浮在气泡上方、页面背景上（不进灰底气泡），小缩略图自适应比例、
               点击放大。参考 Claude 客户端：用户图片在「Me」气泡之上独立成排。 -->
          <div v-if="imageBlocks(m).length" class="msg-images">
            <button
              v-for="(b, bi) in imageBlocks(m)"
              :key="'img' + bi"
              type="button"
              class="msg-image-thumb"
              @click="openLightbox(imageBlocks(m).map((x) => imageSrcUrl(x.imageSrc!)), bi)"
            >
              <img :src="imageSrcUrl(b.imageSrc!)" loading="lazy" alt="" />
            </button>
          </div>

          <!-- 文件附件 chip：浮在气泡上方，点击用系统默认程序外部打开。 -->
          <div v-if="fileBlocks(m).length" class="msg-files">
            <button
              v-for="(b, bi) in fileBlocks(m)"
              :key="'file' + bi"
              type="button"
              class="msg-file-chip"
              v-tooltip="b.isDir ? t('chat.folder.open') : t('chat.file.open')"
              @click="openFile(b)"
            >
              <component :is="b.isDir ? IconFolder : fileIconFor(b.filePath!)" class="msg-file-icon" />
              <span class="msg-file-name">{{ fileName(b.filePath!) }}</span>
            </button>
          </div>

          <div v-if="hasBubbleBody(m)" class="bubble" :class="effectiveRole(m)">
          <div class="role-tag">
            <span class="name">
              <component
                v-if="effectiveRole(m) === 'assistant'"
                :is="agentIcons[agent]"
                class="agent-icon"
                :class="agent"
              />
              {{ effectiveRole(m) === 'user' ? t('chat.role.me') : assistantName }}
            </span>
            <span v-if="m.model" class="tool-chip">{{ formatModelName(m.model) }}</span>
            <span v-if="m.sidechain" class="sidechain-badge">
              {{ t('chat.badge.subtask') }}
            </span>
          </div>

          <CollapsibleBox :enabled="effectiveRole(m) === 'user'" :max-height="320">
            <template v-for="(b, bi) in m.blocks" :key="bi">
              <div v-if="b.kind === 'text'" class="text-run" v-html="renderBubble(m, b.text ?? '')" />
              <div v-else-if="isCodexPluginFileBlock(b)" class="text-run" v-html="codexPluginFileHtml(b)" />

              <!-- AskUserQuestion 是 Claude 的提问工具。历史回放也使用和 live 一致的
                   选择题视觉；对应的 user/tool_result 协议记录在下方被隐藏并合并进状态。 -->
              <ChatQuestionPrompt
                v-else-if="isAskUserQuestion(b) && askUserQuestionRequest(b)"
                :request="askUserQuestionRequest(b)!"
                :agent="agent"
                :readonly="true"
                :history-result="askUserQuestionResult(b)"
                :data-search-scope="toolUseScope(b)"
              />

              <!-- image 块已渲染在气泡上方，正文里跳过 -->

              <details
                v-else-if="b.kind === 'thinking'"
                class="thinking-block"
                :open="isDetailOpen(vr.index, bi)"
                @toggle="onDetailToggle(vr.index, bi, $event)"
              >
                <summary class="thinking-summary">
                  <IconAtom class="thinking-icon" aria-hidden="true" />
                  <span class="thinking-label">{{ thinkingLabel(vr.index) }}</span>
                  <span class="thinking-chev"><IconChevronRight /></span>
                </summary>
                <div class="thinking-content" v-html="renderText(b.text ?? '')" />
              </details>

              <div
                v-else-if="isCodexInlineCodeToolUse(b) && shouldShowToolUse(b)"
                class="inline-tool-code inline-tool-code-flat"
                :data-search-scope="toolUseScope(b)"
              >
                <div
                  :class="toolUseCodeClass(b)"
                  v-html="toolUseCodeHtml(b)"
                />
              </div>

              <details
                v-else-if="b.kind === 'tool_use' && shouldShowToolUse(b)"
                class="block-card"
                :class="{ 'in-user': effectiveRole(m) === 'user' }"
                :data-search-scope="toolUseScope(b)"
                :open="isDetailOpen(vr.index, bi)"
                @toggle="onDetailToggle(vr.index, bi, $event)"
              >
                <summary class="block-summary">
                  <span class="chev"><IconChevronRight /></span>
                  <span class="label">{{ toolLabel(b) }}</span>
                </summary>
                <div class="block-body">
                  <pre class="lang-json" v-html="prettifyAndHighlightJson(b.toolInput ?? '')" />
                  <ToolResult
                    v-if="inlinedResultFor(b)"
                    :block="inlinedResultFor(b)!"
                    :cwd="cwd"
                    :persist-open="isDetailOpen(vr.index, bi, 'r')"
                    @toggle="v => onResultToggle(vr.index, bi, v)"
                  />
                </div>
              </details>

              <ToolResult
                v-else-if="b.kind === 'tool_result' && shouldShowToolResult(b)"
                :block="b"
                :cwd="cwd"
                :in-user="effectiveRole(m) === 'user'"
                :persist-open="isDetailOpen(vr.index, bi, 'r')"
                @toggle="v => onResultToggle(vr.index, bi, v)"
              />
            </template>
          </CollapsibleBox>
          </div>

          <!-- 文件改动型 tool_result：作为本条消息的一部分整块展示在气泡下方、时间行上方，
               与发起它的「Tool call · Edit」同属一条消息（diff 自成卡片便于一眼看改动）。 -->
          <template v-for="(b, bi) in m.blocks" :key="'fc' + bi">
            <div
              v-if="b.kind === 'tool_use' && shouldShowAttachedResult(b)"
              class="attached-result"
              data-search-scope="tools-edit"
            >
              <ToolResult
                :block="attachedResultFor(b)!"
                :cwd="cwd"
                :persist-open="isDetailOpen(vr.index, bi, 'r')"
                @toggle="v => onResultToggle(vr.index, bi, v)"
              />
            </div>
          </template>

          <!-- 悬停操作行：与气泡平级、在「气泡下方」按行内排布（不再绝对定位贴边，
               故不会压到紧随其后的 tool-only 结果卡上）。时间统一藏起，hover 才露出。 -->
          <div class="msg-actions">
            <span class="msg-time">{{ formatTime(m.timestamp) }}</span>
            <button
              v-if="cancelledPrompt?.index === vr.index"
              class="msg-action-btn"
              type="button"
              :aria-label="t('chat.action.editMsg')"
              v-tooltip="t('chat.action.editMsg')"
              @click="editCancelledPrompt"
            >
              <IconPencil />
            </button>
            <button
              class="msg-action-btn"
              type="button"
              v-tooltip="copiedMsgKey === msgKey(m, vr.index) ? t('chat.action.copied') : t('chat.action.copyMsg')"
              @click="copyMsg(m, vr.index)"
            >
              <component :is="copiedMsgKey === msgKey(m, vr.index) ? IconCheck : IconCopy" />
            </button>
            <!-- 隐藏消息仅在只读详情里出现：直播 chat 进行中无需隐藏自己刚发的消息。 -->
            <button
              v-if="!liveSession"
              class="msg-action-btn"
              type="button"
              v-tooltip="isHidden(m, vr.index) ? t('chat.action.unhideMsg') : t('chat.action.hideMsg')"
              @click="toggleHideMsg(m, vr.index)"
            >
              <component :is="isHidden(m, vr.index) ? IconEye : IconEyeOff" />
            </button>
          </div>
        </template>
      </div>
      </template>
      </div>
      </div>

      <!-- §10.6 流式预览：正在生成的文本块打字机（仅 Claude 产 delta；Codex 永不出现）。
           权威 assistant 记录到达即清空 live、真气泡入列表 —— 同位替换，无闪。 -->
      <div v-if="liveSession && liveSession.live" class="msg-row assistant">
        <div class="bubble assistant">
          <div class="role-tag">
            <span class="name">
              <component :is="agentIcons[agent]" class="agent-icon" :class="agent" />
              {{ assistantName }}
            </span>
          </div>
          <!-- 即时 markdown（v-html）：与定稿气泡同款渲染，定稿时同位无缝替换。 -->
          <div class="text-run" v-html="streamingHtml" />
        </div>
      </div>
      <!-- live 模式：工具卡片隐藏时仍持续说明正在调用什么；整轮结束后短暂保留完成反馈。 -->
      <Transition name="chat-running-feedback">
        <div
          v-if="runningFeedback"
          class="chat-running-row"
          :class="[
            runningFeedback.kind === 'running' ? { retrying: liveSession?.retry } : `is-${runningFeedback.kind}`,
            { 'is-finished': runningFeedback.kind !== 'running' },
          ]"
        >
          <template v-if="runningFeedback.kind === 'running'">
            <component :is="agentIcons[agent]" class="chat-running-star" :class="agent" />
            <span class="chat-running-label" :data-text="runningVerb">{{ runningVerb }}</span>
            <span class="chat-running-time">{{ runningElapsedLabel }}</span>
            <Transition name="chat-running-tool" mode="out-in">
              <span
                v-if="runningToolLabel"
                :key="runningToolKey"
                class="chat-running-tool"
                :class="[runningToolActivity?.phase, { stale: runningToolStale }]"
              >
                <span class="chat-running-tool-dot" aria-hidden="true" />
                <span class="chat-running-tool-text">{{ runningToolLabel }}</span>
              </span>
            </Transition>
            <button
              v-if="liveSession"
              class="chat-running-stop"
              type="button"
              v-tooltip="t('chat.composer.stop')"
              @click="interruptChat(liveSession)"
            >
              <IconStop />
              <span>{{ t('chat.composer.stop') }}</span>
            </button>
          </template>
          <template v-else>
            <IconCheck v-if="runningFeedback.kind === 'completed'" class="chat-running-finish-icon" />
            <IconStop v-else class="chat-running-finish-icon" />
            <span class="chat-running-finish-label">{{ finishedTurnLabel }}</span>
            <span class="chat-running-time">{{ runningFeedback.elapsed }}</span>
          </template>
        </div>
      </Transition>

      <!-- live 模式：交互式工具权限对话框（Claude `--permission-prompt-tool stdio`）。
           CLI 门控某个工具时入队一条，用户在此放行 / 拒绝；应答或本轮结束即出队。 -->
      <ChatPermissionPrompt
        v-for="req in (liveSession ? liveSession.pendingPermissions : [])"
        :key="req.requestId"
        :request="req"
        :agent="agent"
        @choose="(c: PermissionChoice) => liveSession && respondPermission(liveSession, req, c)"
      />

      <!-- live 模式：模型向用户提的结构化选择题（Claude `AskUserQuestion`）。
           作答 / 取消都经 respondQuestion 回写控制协议；应答或本轮结束即出队。 -->
      <ChatQuestionPrompt
        v-for="q in (liveSession ? liveSession.pendingQuestions : [])"
        :key="q.requestId"
        :request="q"
        :agent="agent"
        @submit="(sel: QuestionSelection[]) => liveSession && respondQuestion(liveSession, q, sel)"
        @cancel="liveSession && respondQuestion(liveSession, q, null)"
      />

      <div v-if="!messages.length && !liveSession" class="empty" style="height: 200px">
        <div>{{ t('chat.empty') }}</div>
      </div>
    </div>
  </div>

  <div v-if="messages.length" class="scroll-fab" :class="{ 'has-composer': !!liveSession }">
    <!-- read ⇄ chat 切到对方模式的开关：两个方向共用底部同一个 FAB 位置，按模式二选一显示，
         图标就地在 book（→read）/ 气泡（→chat）间切换 —— 同位置即可，不需要过渡飞线动画。 -->
    <button
      v-if="liveSession && hasReadView"
      class="fab fab-accent"
      v-tooltip="t('chat.action.switchToRead')"
      @click="$emit('switchToRead')"
    >
      <IconReader />
    </button>
    <button
      v-else-if="!liveSession && !trashed && !sessionArchived && canResumeHere && chatSupported(agent)"
      class="fab fab-accent"
      v-tooltip="t('chat.action.switchToChat')"
      @click="$emit('switchToChat')"
    >
      <IconChat />
    </button>
    <div v-if="newCount > 0 || !atTop || !atBottom" class="scroll-arrow-stack">
      <button
        v-if="newCount > 0"
        class="new-pill"
        @click="jumpToNewest"
      >
        {{ t('chat.newMessages', { n: newCount }) }}
      </button>
      <button
        v-if="!atTop"
        class="fab"
        v-tooltip="t('chat.action.top')"
        @click="scrollToTop"
      >
        <IconArrowUp />
      </button>
      <button
        v-if="!atBottom"
        class="fab"
        v-tooltip="t('chat.action.bottom')"
        @click="scrollToBottom"
      >
        <IconArrowDown />
      </button>
    </div>
  </div>

  <!-- live GUI chat：底部输入框（Claude 客户端样式） -->
  <ChatComposer
    v-if="liveSession"
    :session="liveSession"
    @open-export="openExportFromComposer"
    @rename="$emit('rename')"
    @fork="$emit('fork')"
    @archive="$emit('archive')"
  />

  <VueEasyLightbox
    :visible="lightboxVisible"
    :imgs="lightboxImgs"
    :index="lightboxIndex"
    @hide="lightboxVisible = false"
  />
</template>
