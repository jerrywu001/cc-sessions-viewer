<script setup lang="ts">
// GUI chat 输入框 —— 视觉 / 交互参考 Claude 桌面客户端。
//   · 大号圆角多行输入，右侧内嵌 发送(↵)/停止(□) 按钮
//   · 图片：粘贴(⌘V) / 拖拽 / "+" 选择 → 上方缩略图附件行，可单独移除
//   · 行首 "/" 调出可过滤的指令浮层（MVP 内置一份常用命令）
//   · 底栏：左 权限模式 chip + "+" 附件；右 模型名 + running spinner
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch, type Component } from 'vue'
import { t } from '../i18n'
import * as api from '../api'
import {
  canSteerQueued,
  clearChat,
  enqueuePrompt,
  interruptChat,
  now,
  removeQueued,
  steerQueued,
  type ChatSession,
  type QueuedMessage,
} from '../chatSessions'
import { buildChatHistory, type ChatHistoryEntry } from '../chatInputHistory'
import { parseChatSlashAction } from '../chatSlashActions'
import { systemSlashCommands } from '../chatSystemCommands'
import { openSideChat } from '../sideChat'
import { openCodexSideChat } from '../codexSideChat'
import { formatElapsedSeconds } from '../format'
import { showTooltipFor, hideTooltip } from '../tooltip'
import type { ChatImageAttachment, ChatFileAttachment, SlashCommand, ProjectFileEntry } from '../types'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import type { UnlistenFn } from '@tauri-apps/api/event'
import {
  IconPlus, IconSend, IconStop, IconClose, IconFolder, IconPaperclip, IconSlashSquare, IconSkill,
  IconArrowUp, IconChevronRight, IconZap, IconCornerDownLeft,
  fileIconFor,
} from './icons'
import ChatModeMenu from './ChatModeMenu.vue'
import ChatModelMenu from './ChatModelMenu.vue'
import ChatEffortSlider from './ChatEffortSlider.vue'
import GitBranchControl from './GitBranchControl.vue'
import AutoModeConfirmModal from './AutoModeConfirmModal.vue'
import {
  hasModelChoice,
  modelSupportsEffort,
  autoPickModel,
  fallbackPermissionMode,
  fallbackEffort,
  type ModelMenuOptions,
} from '../chatComposerOptions'
import { isAutoModeConfirmed, rememberAutoModeConfirmed } from '../autoMode'
import {
  usedContextTokens,
  contextWindowFor,
  contextPercent,
  formatTokensShort,
} from '../chatContext'
import {
  usage,
  usageWindows,
  usageLevel,
  formatRemaining,
  nowMs,
  startUsagePolling,
  stopUsagePolling,
} from '../usage'
import {
  codexPluginMentionRanges,
} from '../codexPluginMentions'
import { rememberChatGuiPreference } from '../chatGuiPreferences'

const props = defineProps<{ session: ChatSession }>()
// 客户端 slash 指令里需要 ChatView / App 出手的几个（展开右上角导出菜单、打开重命名框、
// fork 会话）上抛给父组件；/model、/clear、/btw、/side 在 composer 内部就地处理，不走 emit。
const emit = defineEmits<{
  openExport: []
  rename: []
  fork: []
  archive: []
}>()
const claudeHasCustomBaseUrl = ref(false)
const claudeAliasTargets = ref<Record<string, string | undefined>>({})
// init 事件回来前对鉴权方式的预判（后端 runtime_info 判：钥匙串有订阅凭证 → 'none'）。
// 进会话即拿，让官方订阅用户立刻看到 effort + 限额，而不是等首轮 init 才显形。
const claudeRuntimeApiKeySource = ref<string | undefined>(undefined)
// settings.json 的全局 effortLevel。transcript 不记录 effort、CLI 不带 --effort 即用它，
// 故续聊未改档前 effort 选择器展示它（真实生效默认），而不是滑杆假定的 levels[0]。
const claudeRuntimeEffortLevel = ref<string | undefined>(undefined)
// Codex 是否通过第三方 API key / 自定义端点使用（config.toml 有 model_provider）。
// 若是，隐藏仅官方订阅可用的模型（如 GPT-5.3-Codex-Spark）。
const codexUsingApiKey = ref(false)
// runtime info（含是否走 alias）是否已就位。非 claude/codex 没有这步，直接视为已就位。
const runtimeLoaded = ref(props.session.agent !== 'claude' && props.session.agent !== 'codex')

// ⌘U / Ctrl+U 全局唤起文件选择器 —— 挂在 window 上（而非 textarea），未聚焦输入框时也响应。
const isMac = /Mac/i.test(navigator.platform)
function onGlobalKeydown(e: KeyboardEvent) {
  if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && (e.key === 'u' || e.key === 'U')) {
    if (ended.value) return
    e.preventDefault()
    void pickFilesOrPhotos()
  }
}

// 占位符顺带提示上传快捷键（mac ⌘U / 其它 Ctrl+U）。
const composerPlaceholder = computed(() =>
  t('chat.composer.placeholder', { key: isMac ? '⌘U' : 'Ctrl+U' }),
)

// 系统文件拖拽：Tauri 默认 dragDropEnabled，OS 拖入的文件不会走 HTML5 drop（拿不到 File），
// 而是发 webview 级 drag-drop 事件（带文件**路径**）。会话窗口开着时把拖入的文件按和选择器
// 同样的规则入框（图片→缩略图、其它→@path chip）。一次只有一个 live chat composer 挂载。
const dragOver = ref(false)
let dropUnlisten: UnlistenFn | null = null

// 进入 live chat 即订阅账号额度轮询，离开退订（引用计数，多个 composer 共享一个定时器）。
onMounted(() => {
  startUsagePolling()
  window.addEventListener('keydown', onGlobalKeydown)
  void getCurrentWebview()
    .onDragDropEvent((e) => {
      if (ended.value) {
        dragOver.value = false
        return
      }
      const p = e.payload
      if (p.type === 'enter' || p.type === 'over') {
        dragOver.value = true
      } else if (p.type === 'drop') {
        dragOver.value = false
        for (const path of p.paths) void addPath(path)
      } else {
        dragOver.value = false // leave / cancel
      }
    })
    .then((un) => {
      dropUnlisten = un
    })
  if (props.session.agent === 'claude') {
    void api
      .claudeRuntimeInfo()
      .then((info) => {
        claudeHasCustomBaseUrl.value = !!info.hasCustomBaseUrl
        claudeAliasTargets.value = info.aliasTargets ?? {}
        claudeRuntimeApiKeySource.value = info.apiKeySource || undefined
        claudeRuntimeEffortLevel.value = info.effortLevel || undefined
      })
      .catch(() => {
        claudeHasCustomBaseUrl.value = false
        claudeAliasTargets.value = {}
        claudeRuntimeApiKeySource.value = undefined
        claudeRuntimeEffortLevel.value = undefined
      })
      .finally(() => {
        runtimeLoaded.value = true
      })
  }
  if (props.session.agent === 'codex') {
    void api
      .codexRuntimeInfo()
      .then((info) => {
        codexUsingApiKey.value = info.usesApiKey
      })
      .catch(() => {
        codexUsingApiKey.value = false
      })
      .finally(() => {
        runtimeLoaded.value = true
      })
  }
  const initialDraft = takeInitialDraft()
  if (initialDraft) {
    exitHistory()
    applyHistoryEntry(initialDraft)
  }
  focusInput()
})
onBeforeUnmount(() => {
  stopUsagePolling()
  window.removeEventListener('keydown', onGlobalKeydown)
  if (mentionTimer !== null) clearTimeout(mentionTimer)
  dropUnlisten?.()
})

const text = ref('')
const images = ref<ChatImageAttachment[]>([])
const files = ref<ChatFileAttachment[]>([]) // 非图片附件（文件/文件夹）→ 发送时 @path
const drafts = new Map<number, ChatHistoryEntry>()

// ↑/↓ 历史回填（参考 Claude 客户端）：把本会话用户发过的消息抽成可翻列表。
const promptHistory = computed<ChatHistoryEntry[]>(() => buildChatHistory(props.session.msgs))
// 当前翻到第几条（0 = 最旧，length-1 = 最新）；null = 没在浏览历史（编辑草稿态）。
const histPos = ref<number | null>(null)
// 进入浏览前的草稿快照，↓ 越过最新一条时还原。
let histDraft: ChatHistoryEntry | null = null
const historyHint = computed(() =>
  histPos.value === null
    ? ''
    : t('chat.composer.history', { n: histPos.value + 1, total: promptHistory.value.length }),
)
// 切换会话时按 uiId 保存并恢复草稿；各会话的文字和附件互不串用。
watch(() => props.session.uiId, (uiId, previousUiId) => {
  drafts.set(previousUiId, {
    text: text.value,
    images: images.value.map((image) => ({ ...image })),
    files: files.value.map((file) => ({ ...file })),
  })
  const draft = drafts.get(uiId) ?? takeInitialDraft()
  text.value = draft?.text ?? ''
  images.value = draft?.images.map((image) => ({ ...image })) ?? []
  files.value = draft?.files.map((file) => ({ ...file })) ?? []
  exitHistory()
  stash.value = null
  nextTick(autosize)
  focusInput()
})
// Ctrl+S stash：暂存输入框内容，下一条消息开始发送的瞬间恢复。
const stash = ref<{ text: string; images: ChatImageAttachment[]; files: ChatFileAttachment[] } | null>(null)
watch(() => props.session.turnState, (cur, prev) => {
  if (prev === 'idle' && cur === 'running' && stash.value) {
    if (!text.value.trim() && images.value.length === 0 && files.value.length === 0) {
      text.value = stash.value.text
      images.value = stash.value.images
      files.value = stash.value.files
      stash.value = null
      nextTick(autosize)
      focusInput()
    }
  }
})

const plusMenuOpen = ref(false) // "+" 弹出菜单（Add files or photos / Add folder）
const taEl = ref<HTMLTextAreaElement>()
const wrapEl = ref<HTMLElement>() // 输入框容器（@ 浮层定位的相对锚点）
const modelMenuRef = ref<{ openMenu: () => void } | null>(null) // `/model` 程序化展开底部模型面板
const previewSrc = ref('') // 点击缩略图后的大图预览（空 = 不显示）

const running = computed(() => props.session.turnState === 'running')
const ended = computed(
  () => props.session.status === 'exited' || props.session.status === 'error',
)
const canSend = computed(
  () =>
    !running.value &&
    !ended.value &&
    (!!text.value.trim() || images.value.length > 0 || files.value.length > 0),
)

/** running 时的耗时秒数（读模块时钟 now 驱动）。 */
const elapsedSec = computed(() => {
  if (!running.value) return 0
  return Math.max(0, Math.floor((now.value - props.session.turnStartedAt) / 1000))
})
const elapsedLabel = computed(() => formatElapsedSeconds(elapsedSec.value))

/** 网络重试态：有 retry 时状态行显示「请求失败 · 重试中 (n/N)」替代纯耗时。 */
const retryLabel = computed(() => {
  const r = props.session.retry
  if (!r) return ''
  return r.attempt && r.max
    ? t('chat.running.retryingN', { n: r.attempt, max: r.max })
    : t('chat.running.retrying')
})

// ---------- §10.2/10.3/10.4 底栏切换器 ----------
// 改的只是 session 上的当前选择（懒生效）：one-shot（Codex）下一轮带新 flag 即生效；
// 长驻（Claude）由下一次 sendPrompt 检测到变更后 restart-with-resume。t() 让 label 随
// 语言 / session 选择响应式刷新。
const agent = computed(() => props.session.agent)
const slashInsertChar = computed(() => agent.value === 'codex' ? '$' : '/')
// 拖拽投放区配色：用 agent 自己的品牌色 token（--brand-claude / -codex），
// 而非随窗口失焦变灰的 --brand。从 Finder 拖文件进来时本窗口处于 .is-blurred（--brand
// 被降级成 --text-mute 灰），用 raw token 才能始终保持橘/绿/蓝，不会先灰后橘地闪。
const dropBrand = computed(() => `var(--brand-${props.session.agent})`)
// 权威值（init 给的 session.apiKeySource）优先；没来之前用 runtime 预判兜底。
const effectiveApiKeySource = computed(
  () => props.session.apiKeySource ?? claudeRuntimeApiKeySource.value,
)
const usingApiKey = computed(() => {
  const src = effectiveApiKeySource.value
  return typeof src === 'string' && src !== '' && src !== 'none'
})
const usingCustomClaudeEndpoint = computed(
  () => props.session.agent === 'claude' && claudeHasCustomBaseUrl.value,
)
const claudeAliasMode = computed(
  () =>
    agent.value === 'claude' &&
    (effectiveApiKeySource.value !== 'none' || usingCustomClaudeEndpoint.value),
)
const modelMenuOptions = computed<ModelMenuOptions>(() => ({
  claudeAliasMode: claudeAliasMode.value,
  claudeAliasTargets: claudeAliasTargets.value,
  codexApiKeyMode: codexUsingApiKey.value,
}))
const showModelPicker = computed(() => hasModelChoice(agent.value, modelMenuOptions.value))
// 生效中的模型：用户没显式选过时（新会话 session.model=undefined）回落到运行时实际模型
// （lastModel，来自 assistant 记录）。模型菜单展示名、effort 档位（含 Opus 4.7/4.8 在 max
// 之后那一格 ultracode）都以它为准 —— 否则刚进会话没选过模型时滑杆拿不到模型、显示不出
// ultracode，非得手动切一下模型才出来。
const effectiveModel = computed(() => props.session.model ?? props.session.lastModel)

// 全新 claude 会话默认选中一个明确的「标准上下文」模型。否则 session.model 为空 → 后端不带
// --model → CLI 回落到 settings 里的默认模型（常被映射成 1M 上下文，需额度）→ 首条消息直接
// 「API Error: Usage credits required for 1M context」。等 runtime info 就位后再选，alias 模式
// 选别名（opus…，让 settings 映射接管），订阅模式选完整 id（claude-opus-5）。续聊（有历史
// 或已选过模型）不强选 —— 模型随历史/用户选择。
watch(
  [runtimeLoaded, () => props.session.model, () => props.session.lastModel],
  () => {
    if (props.session.agent !== 'claude' || !runtimeLoaded.value) return
    if (props.session.model || props.session.lastModel || props.session.msgs.length > 0) return
    const first = autoPickModel(agent.value, modelMenuOptions.value)
    if (first) onPickModel(first)
  },
  { immediate: true },
)
// effort 是「按模型」的能力：Haiku 不支持 effort，选中它就不展示滑杆（对齐 Claude 客户端）。
const showEffortPicker = computed(() =>
  (agent.value !== 'claude' || effectiveApiKeySource.value === 'none') &&
  !usingCustomClaudeEndpoint.value &&
  !usingApiKey.value &&
  modelSupportsEffort(agent.value, effectiveModel.value),
)

// 切到 auto（自动）模式前的二次确认门控：本工作区还没确认过就先弹框，确认后才真正生效
// 并记住该工作区（之后不再追问）。其它模式直接切。
const askAutoMode = ref(false)
function onPickPermission(v: string) {
  if (
    v === 'auto' &&
    props.session.permissionMode !== 'auto' &&
    !isAutoModeConfirmed(props.session.cwd)
  ) {
    askAutoMode.value = true
    return
  }
  props.session.permissionMode = v
  rememberChatGuiPreference(props.session.agent, { permissionMode: props.session.permissionMode })
}
function confirmAutoMode() {
  rememberAutoModeConfirmed(props.session.cwd)
  props.session.permissionMode = 'auto'
  rememberChatGuiPreference(props.session.agent, { permissionMode: props.session.permissionMode })
  askAutoMode.value = false
}
function cancelAutoMode() {
  askAutoMode.value = false
}
function onPickModel(v: string) {
  const model = v || undefined
  props.session.model = model
  // 新模型若不支持当前权限模式（如 Haiku 不支持 auto），自动回退到可用模式。
  props.session.permissionMode = fallbackPermissionMode(agent.value, props.session.permissionMode, model)
  // 当前 effort 档在新模型下不存在（如从 4.8 的 ultracode 切到 Sonnet）→ 退到最高可用档。
  props.session.effort = fallbackEffort(props.session.effort, props.session.agent, model)
  rememberChatGuiPreference(props.session.agent, {
    permissionMode: props.session.permissionMode,
    model: props.session.model,
    effort: props.session.effort,
  })
}
function onPickEffort(v: string) {
  props.session.effort = v || undefined
  rememberChatGuiPreference(props.session.agent, { effort: props.session.effort })
}

// ---------- §10.5 上下文窗口 + 限额指示 ----------
const ctxUsed = computed(() => usedContextTokens(props.session.usage))
const ctxWindow = computed(() =>
  contextWindowFor(
    props.session.agent,
    props.session.lastModel ?? props.session.model,
    ctxUsed.value,
  ),
)
const ctxPercent = computed(() => contextPercent(ctxUsed.value, ctxWindow.value))
// 常驻显示：只要有已知窗口就一直显示（首轮前为 0%），不再随 usage 有无而闪烁。
const showContext = computed(() => ctxWindow.value > 0)
const ctxTooltip = computed(
  () =>
    `${t('chat.composer.context.label')}: ${formatTokensShort(ctxUsed.value)} / ${formatTokensShort(
      ctxWindow.value,
    )} (${ctxPercent.value}%)`,
)

// 限额：走 OAuth 用量接口（src/usage.ts 轮询），每个窗口带精确利用率 + 重置时间，不受
// 「越过阈值才上报」限制，故 5h / 周能随时精确显示（context 在外层最先 → 5h → 周）。
// Claude 的 5h/周额度只对订阅/OAuth（apiKeySource === 'none'）成立。API key 计费一律不显示，
// 避免把第三方/API-key 会话误判成订阅。init 没回来前用 runtime 预判兜底（effectiveApiKeySource），
// 官方订阅一进会话即显示；真判不出（预判也未知）时仍保守隐藏。
const showRateLimits = computed(
  () =>
    props.session.agent === 'claude' &&
    effectiveApiKeySource.value === 'none' &&
    !usingCustomClaudeEndpoint.value,
)
function rlResetText(iso: string | undefined): string {
  if (!iso) return ''
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return ''
  const now = new Date()
  const hh = String(d.getHours()).padStart(2, '0')
  const mm = String(d.getMinutes()).padStart(2, '0')
  // 重置往往隔天（尤其周限额）→ 跨天则带上 月/日。
  return d.toDateString() === now.toDateString()
    ? `${hh}:${mm}`
    : `${d.getMonth() + 1}/${d.getDate()} ${hh}:${mm}`
}
function rlTypeLabel(key: 'five_hour' | 'seven_day'): string {
  return key === 'five_hour' ? t('chat.composer.limit.fiveHour') : t('chat.composer.limit.weekly')
}
const rateBadges = computed(() => {
  if (!showRateLimits.value || usingApiKey.value) return []
  const now = nowMs.value // 读响应式心跳 → 倒计时每跳重算（纯前端，零网络）。
  return usageWindows(usage.value).map((w) => {
    const label = rlTypeLabel(w.key)
    const remaining = formatRemaining(w.resetsAt, now) // 紧凑倒计时：4h30m / 2d6h / 45m
    const reset = rlResetText(w.resetsAt)
    return {
      key: w.key,
      // 「<窗口> <百分比>% · <倒计时>」，对齐 claude-hud；倒计时缺失则省略。
      text: remaining ? `${label} ${w.percent}% · ${remaining}` : `${label} ${w.percent}%`,
      level: usageLevel(w.percent),
      // 悬浮显示绝对重置时刻（与行内相对倒计时互补）。
      tooltip: reset ? t('chat.composer.limit.resets', { time: reset }) : `${label} ${w.percent}%`,
    }
  })
})

// 点击输入框空白处（非按钮/缩略图）→ 聚焦文本框，像原生输入框一样。
function onWrapClick(e: MouseEvent) {
  if ((e.target as HTMLElement).closest('button')) return
  if ((e.target as HTMLElement).closest('.cc-thumb')) return
  if (!ended.value) taEl.value?.focus()
}

// 打开会话 / 切换会话时自动聚焦输入框（像 Claude 客户端，进来就能直接打字）。
// 已结束的会话 textarea 是 disabled，聚焦无意义；nextTick 确保 DOM（切换后的新 textarea）已就绪。
function focusInput() {
  if (ended.value) return
  void nextTick(() => taEl.value?.focus())
}

// 大图预览：Esc 关闭。
function onPreviewKey(e: KeyboardEvent) {
  if (e.key === 'Escape' && previewSrc.value) previewSrc.value = ''
}
window.addEventListener('keydown', onPreviewKey)
onBeforeUnmount(() => window.removeEventListener('keydown', onPreviewKey))

// ---------- 自适应高度 ----------
// 新版 WebView2 原生支持 textarea 按内容伸缩；支持时避免每次输入都走
// `height = auto -> scrollHeight` 这条同步布局路径。旧运行时保留 JS fallback。
const nativeFieldSizing = typeof CSS !== 'undefined' && CSS.supports?.('field-sizing', 'content') === true
function autosize() {
  if (nativeFieldSizing) return
  const el = taEl.value
  if (!el) return
  el.style.height = 'auto'
  el.style.height = `${Math.min(el.scrollHeight, 220)}px`
}

// ---------- slash 指令浮层（§10.1 动态发现）----------
// 列表 = 前端注入的「System」内置指令（chatSystemCommands）+ 后端磁盘扫描出的命令/技能
// （项目/用户/插件）。扫描列表**不含 TUI 内置指令**（headless 下会报「not available」），故
// /export、/clear、/model 等系统指令靠前端这份补上。选中后按 `/<name>` 透传：系统指令在提交
// 时由 chatSlashActions 拦截分派，其余命令交给 CLI 展开。浮层按 kind 分组（System / Commands /
// Skills），每行 = 图标 + 展示名 + 截断描述 + 右侧来源角标。
type SlashItem = SlashCommand
const slashCommands = ref<SlashItem[]>([])

async function loadSlashCommands() {
  // System 组恒在最前；扫描失败也至少留下系统指令可用。
  const system = systemSlashCommands(props.session.agent)
  try {
    const scanned = await api.agentChatSlashCommands(props.session.agent, props.session.cwd)
    slashCommands.value = [...system, ...scanned]
  } catch {
    slashCommands.value = system
  }
}
// 进入会话 / 切换会话时拉一次。
watch(
  () => [props.session.agent, props.session.cwd],
  () => void loadSlashCommands(),
  { immediate: true },
)

const slashOpen = ref(false)
const slashIdx = ref(0)
const slashStart = ref(-1) // 触发用 `/` 在 text 中的下标（-1 = 未触发）
const slashQuery = ref('') // `/` 与光标之间的过滤词（保证不含空白）
let dismissedSlashToken: { at: number; query: string } | null = null // Esc 关闭后，抑制同一 token 被 keyup 立刻重开
// 过滤：按调用名或展示名子串匹配（不分大小写）；空 query 列全部。后端已按「命令在前、技能在后」排好序。
const slashMatches = computed<SlashItem[]>(() => {
  const q = slashQuery.value.toLowerCase()
  if (!q) return slashCommands.value
  return slashCommands.value.filter(
    (s) => s.name.toLowerCase().includes(q) || s.title.toLowerCase().includes(q),
  )
})
// 把扁平列表按 kind 切成段，供分组渲染。每段独立 DOM 容器 → sticky 段标题只在本段内吸顶，
// 滚到下一段时被推走（多个 top:0 sticky 共用一个容器会全部叠在顶部，就是之前的「漏光」）。
// 每项保留它在 slashMatches 里的全局下标 gi，让键盘高亮 / 选择仍走同一套扁平 slashIdx。
const slashGroups = computed(() => {
  const groups: { kind: string; items: { item: SlashItem; gi: number }[] }[] = []
  slashMatches.value.forEach((item, gi) => {
    const last = groups[groups.length - 1]
    if (last && last.kind === item.kind) last.items.push({ item, gi })
    else groups.push({ kind: item.kind, items: [{ item, gi }] })
  })
  return groups
})
// 分组段标题（系统 / 命令 / 技能）。
function slashGroupLabel(kind: string): string {
  if (kind === 'system') return t('chat.composer.slashGroup.system')
  if (kind === 'skill') return t('chat.composer.slashGroup.skill')
  return t('chat.composer.slashGroup.command')
}
// 右侧来源角标：user → 本地化「Personal」；project / plugin → 项目名 / 插件名。
function slashSource(s: SlashItem): string {
  // user → User；project → 统一显示「Project」（不显示具体项目名）；plugin → 插件名。
  if (s.origin === 'user') return t('chat.composer.slashSource.user')
  if (s.origin === 'project') return t('chat.composer.slashSource.project')
  return s.originName ?? ''
}
// 浮层滚动容器：↑/↓ 导航时让高亮项滚进视野（行为同 @ 浮层的 scrollActiveMention）。
const slashListEl = ref<HTMLElement>()
function scrollActiveSlash() {
  nextTick(() => {
    const row = slashListEl.value?.querySelector('.cc-slash-item.active') as HTMLElement | null
    if (row && typeof row.scrollIntoView === 'function') row.scrollIntoView({ block: 'nearest' })
  })
}
function moveSlash(delta: number) {
  const n = slashMatches.value.length
  if (!n) return
  slashIdx.value = (slashIdx.value + delta + n) % n
  scrollActiveSlash()
}

// ---------- 输入框命令高亮（输入完一条「已识别」的 /命令 后，命令 token 标蓝）----------
// textarea 无法只给部分文本上色 → 在其后叠一层等样式镜像 div（cc-highlight）渲染带色文本，
// textarea 自身文本透明、只留光标。中文 IME 合成期间关掉透明，否则合成中的文字会看不见。
const composing = ref(false)
const hlEl = ref<HTMLElement>()
const recognizedNames = computed(() => new Set(slashCommands.value.map((c) => c.name)))
// 开头是否为一条已识别的完整命令：`/<name>` 后跟空白或行尾，且 name 在扫描列表里。
const leadingCommand = computed(() => {
  const m = text.value.match(/^[/$](\S+)(?=\s|$)/)
  return m && recognizedNames.value.has(m[1]) ? m[1] : null
})
const pluginHighlightRanges = computed(() =>
  props.session.agent === 'codex' ? codexPluginMentionRanges(text.value) : [],
)
const highlightActive = computed(() =>
  (leadingCommand.value !== null || pluginHighlightRanges.value.length > 0) && !composing.value,
)
// 当前开头命令对应的扫描项（取 argument-hint）。
const leadingCommandObj = computed<SlashItem | null>(() => {
  const name = leadingCommand.value
  if (!name) return null
  return slashCommands.value.find((c) => c.name === name) ?? null
})
// argument-hint ghost：仅当命令已识别、且其后还没填任何参数（只剩空白）时才提示参数格式（对齐 Claude TUI）。
const leadingPrefix = computed(() => text.value[0] === '$' || text.value[0] === '/' ? text.value[0] : '/')
const argHintGhost = computed(() => {
  const cmd = leadingCommandObj.value
  if (!cmd?.argumentHint) return ''
  const rest = text.value.slice(`${leadingPrefix.value}${cmd.name}`.length)
  return rest.trim() === '' ? cmd.argumentHint : ''
})
function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}
function escapeAttr(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
}
// 镜像 HTML：开头 `/命令` 包成蓝色 span，其余原样（转义）；未填参数时再追加暗色 ghost 提示。
// ghost 只在镜像层渲染、不进 textarea 真实值，故不会被透传给 CLI。
const highlightHtml = computed(() => {
  const ranges: { start: number; end: number; html: string }[] = pluginHighlightRanges.value.map((range) => ({
    start: range.start,
    end: range.end,
    html: `<span class="cc-cmd cc-plugin-name" data-cmd-desc="${escapeAttr(range.plugin.description)}">${escapeHtml(text.value.slice(range.start, range.end))}</span>`,
  }))
  const cmd = leadingCommand.value
  let ghostHtml = ''
  if (cmd) {
    const token = `${leadingPrefix.value}${cmd}`
    ranges.push({
      start: 0,
      end: token.length,
      html: `<span class="cc-cmd">${escapeHtml(token)}</span>`,
    })
    const ghost = argHintGhost.value
    const rest = text.value.slice(token.length)
    ghostHtml = ghost
      ? `<span class="cc-arg-hint">${rest ? '' : ' '}${escapeHtml(ghost)}</span>`
      : ''
  }
  ranges.sort((a, b) => a.start - b.start || b.end - a.end)
  let out = ''
  let last = 0
  for (const range of ranges) {
    if (range.start < last) continue
    out += escapeHtml(text.value.slice(last, range.start))
    out += range.html
    last = range.end
  }
  out += escapeHtml(text.value.slice(last))
  return out + ghostHtml
})
function onCompositionStart() {
  composing.value = true
}
function onCompositionEnd() {
  composing.value = false
  // Vue 的 v-model 会在 compositionend 附近补发一次 input；统一进同一个 nextTick
  // 调度器，既确保最终值已经同步，又避免 compositionend + input 重复做布局和浮层检测。
  scheduleInputWork()
}
// 文本超 max-height 滚动时，镜像层跟随 textarea 一起滚，保持对齐。
function syncHlScroll() {
  if (hlEl.value && taEl.value) hlEl.value.scrollTop = taEl.value.scrollTop
}
function onHlHover(e: MouseEvent) {
  const cmd = (e.target as HTMLElement).closest?.('.cc-cmd') as HTMLElement | null
  if (cmd?.dataset.cmdDesc != null) {
    showTooltipFor(cmd, cmd.dataset.cmdDesc || '')
  } else if (cmd && leadingCommandObj.value?.description) {
    showTooltipFor(cmd, `${leadingPrefix.value}${leadingCommandObj.value.name} — ${leadingCommandObj.value.description}`)
  }
}
function onHlLeave() {
  hideTooltip()
}

/** 从光标处向前找触发用的 `/`：前面须为行首或空白，且 `/`→光标间无空白 —— 与 `@` 浮层
 *  同一套「任意位置词首触发」规则，故 `http://`、`a/b` 路径里的 `/` 不会误触发。 */
function activeSlash(): { at: number; query: string } | null {
  const el = taEl.value
  if (!el) return null
  const caret = el.selectionStart ?? text.value.length
  const head = text.value.slice(0, caret)
  const atSlash = head.lastIndexOf('/')
  const atDollar = head.lastIndexOf('$')
  const at = Math.max(atSlash, atDollar)
  if (at < 0) return null
  const between = head.slice(at + 1)
  if (/\s/.test(between)) return null
  const before = at > 0 ? head[at - 1] : ''
  if (before && !/\s/.test(before)) return null
  return { at, query: between }
}

function closeSlash() {
  slashOpen.value = false
  slashStart.value = -1
  slashQuery.value = ''
}

function dismissSlash() {
  const s = activeSlash()
  dismissedSlashToken = s ? { at: s.at, query: s.query } : null
  closeSlash()
}

function detectSlash() {
  const s = activeSlash()
  if (!s) {
    dismissedSlashToken = null
    if (slashOpen.value || slashStart.value >= 0) closeSlash()
    return
  }
  if (dismissedSlashToken?.at === s.at && dismissedSlashToken.query === s.query) return
  dismissedSlashToken = null
  slashStart.value = s.at
  slashQuery.value = s.query
  slashOpen.value = slashMatches.value.length > 0
  if (slashIdx.value >= slashMatches.value.length) slashIdx.value = 0
}

/** 选中指令：把触发处的 `/query` 段替换成 `/<name> `（保留前后文，光标落在插入串尾）。 */
function pickSlash(item: SlashItem) {
  const start = slashStart.value
  const prefix = item.origin === 'system' ? '/' : slashInsertChar.value
  const insert = `${prefix}${item.name} `
  if (start < 0) {
    text.value = insert
  } else {
    const end = start + 1 + slashQuery.value.length
    text.value = text.value.slice(0, start) + insert + text.value.slice(end)
  }
  const caret = (start < 0 ? 0 : start) + insert.length
  closeSlash()
  nextTick(() => {
    const el = taEl.value
    if (el) {
      el.focus()
      el.setSelectionRange(caret, caret)
    }
    autosize()
  })
}

// ---------- @ 浮层（引用项目文件 / 目录）----------
// 输入框**任意位置**键入 `@`（前面是行首或空白）即触发：浮层列出会话 cwd 下的目录/文件
// （空 query→顶层；裸 query→全工作区模糊搜索；带路径 query→逐级浏览）。↑/↓ 选择；↵/点击=「引用」加成 chip；
// →/chevron=「展开」目录继续下钻；Esc 关。chip 走与系统选择器附件同一条 `@"path"` 通道。
type FileMentionItem = ProjectFileEntry & { kind?: 'file' }
type MentionItem = FileMentionItem

const mentionOpen = ref(false)
const mentionItems = ref<MentionItem[]>([])
const mentionIdx = ref(0)
const mentionStart = ref(-1) // 触发用 `@` 在 text 中的下标
const mentionQuery = ref('')
const mentionListEl = ref<HTMLElement>() // 滚动容器（↑/↓ 时让高亮项跟随滚动进视野）
const mentionLeft = ref(0) // 浮层左偏移（px）—— 跟随 `@` 在输入框里的水平位置
const MENTION_POPUP_MAX_WIDTH = 680
const mentionSections = computed(() => {
  const groups: { key: string; title: string; rows: { item: MentionItem; index: number }[] }[] = []
  const files: { item: MentionItem; index: number }[] = []
  mentionItems.value.forEach((item, index) => {
    files.push({ item, index })
  })
  if (files.length) groups.push({ key: 'files', title: mentionPathLabel.value, rows: files })
  return groups
})
const activeMentionItem = computed(() => mentionItems.value[mentionIdx.value])

// 镜像 div 量出 textarea 中某字符位置的像素坐标（标准做法：复制样式 + 同文到测量 span）。
const CARET_PROPS = [
  'boxSizing', 'width', 'height', 'overflowX', 'overflowY',
  'borderTopWidth', 'borderRightWidth', 'borderBottomWidth', 'borderLeftWidth',
  'paddingTop', 'paddingRight', 'paddingBottom', 'paddingLeft',
  'fontStyle', 'fontVariant', 'fontWeight', 'fontStretch', 'fontSize',
  'lineHeight', 'fontFamily', 'textAlign', 'textTransform', 'textIndent',
  'letterSpacing', 'wordSpacing', 'tabSize',
] as const
function caretLeft(el: HTMLTextAreaElement, pos: number): number {
  const div = document.createElement('div')
  const cs = getComputedStyle(el)
  const s = div.style
  s.position = 'absolute'
  s.visibility = 'hidden'
  s.whiteSpace = 'pre-wrap'
  s.wordWrap = 'break-word'
  s.overflow = 'hidden'
  for (const p of CARET_PROPS) (s as unknown as Record<string, string>)[p] = cs[p as keyof CSSStyleDeclaration] as string
  div.textContent = el.value.slice(0, pos)
  const span = document.createElement('span')
  span.textContent = el.value.slice(pos) || '.'
  div.appendChild(span)
  document.body.appendChild(div)
  const left = span.offsetLeft - el.scrollLeft
  document.body.removeChild(div)
  return left
}

/** 把浮层左偏移对齐到 `@` 的水平位置（相对输入框容器 padding 盒），并钳制不越出右沿。 */
function updateMentionPos() {
  const el = taEl.value
  const wrap = wrapEl.value
  if (!el || !wrap || mentionStart.value < 0) return
  const taRect = el.getBoundingClientRect()
  const wrapRect = wrap.getBoundingClientRect()
  const borderLeft = parseFloat(getComputedStyle(wrap).borderLeftWidth) || 0
  let left = taRect.left - wrapRect.left - borderLeft + caretLeft(el, mentionStart.value)
  if (!Number.isFinite(left)) left = 0
  const avail = wrap.clientWidth
  const popupW = Math.min(MENTION_POPUP_MAX_WIDTH, avail)
  mentionLeft.value = Math.max(0, Math.min(left, Math.max(0, avail - popupW)))
}
let mentionSeq = 0 // 异步请求竞态守卫（只认最新一次）
let mentionTimer: number | null = null
let mentionFetched: string | null = null // 已拉取过的 query，避免重复抖动
let dismissedMentionToken: { at: number; query: string } | null = null // Esc 关闭后，抑制同一 token 被 keyup 立刻重开

/** 从光标处向前找触发用的 `@`：前面须为行首或空白，且 `@`→光标间无空白。命中返回
 *  { at, query }，否则 null（避开 foo@bar / 已被空白结束的 token）。 */
function activeMention(): { at: number; query: string } | null {
  const el = taEl.value
  if (!el) return null
  const caret = el.selectionStart ?? text.value.length
  const head = text.value.slice(0, caret)
  const at = head.lastIndexOf('@')
  if (at < 0) return null
  const between = head.slice(at + 1)
  if (/\s/.test(between)) return null
  const before = at > 0 ? head[at - 1] : ''
  if (before && !/\s/.test(before)) return null
  return { at, query: between }
}

function closeMention() {
  mentionOpen.value = false
  mentionItems.value = []
  mentionStart.value = -1
  mentionQuery.value = ''
  mentionFetched = null
}

function dismissMention() {
  const m = activeMention()
  dismissedMentionToken = m ? { at: m.at, query: m.query } : null
  closeMention()
}

function detectMention() {
  const m = activeMention()
  if (!m) {
    dismissedMentionToken = null
    if (mentionOpen.value) closeMention()
    return
  }
  if (dismissedMentionToken?.at === m.at && dismissedMentionToken.query === m.query) return
  dismissedMentionToken = null
  mentionStart.value = m.at
  updateMentionPos() // @ 的水平位置可能因其前面文字增删而移动 → 每次都校准
  // query 没变且已展开 → 不重复请求（光标在 token 内左右移 / 方向键导航时避免抖动）。
  if (mentionOpen.value && m.query === mentionQuery.value && mentionFetched === m.query) return
  mentionQuery.value = m.query
  if (mentionTimer !== null) clearTimeout(mentionTimer)
  mentionTimer = window.setTimeout(() => void fetchMentions(m.query), 70)
}

async function fetchMentions(q: string) {
  const seq = ++mentionSeq
  try {
    const projectItems = await api.listProjectFiles(props.session.cwd, q, 200)
    if (seq !== mentionSeq) return // 过期请求丢弃
    const active = activeMention()
    if (!active) {
      closeMention()
      return
    }
    if (active.query !== q) return
    mentionFetched = q
    const items: MentionItem[] = projectItems.map((item) => ({ ...item, kind: 'file' as const }))
    mentionItems.value = items
    mentionOpen.value = true
    if (mentionIdx.value >= items.length || q !== mentionQuery.value) mentionIdx.value = 0
  } catch {
    closeMention()
  }
}

/** 让当前高亮行滚动进可视区（↑/↓ 跨过浮层视口时跟随滚动，避免看不到选中项）。 */
function scrollActiveMention() {
  nextTick(() => {
    const row = mentionListEl.value?.querySelector('.cc-mention-item.active') as HTMLElement | null
    if (row && typeof row.scrollIntoView === 'function') row.scrollIntoView({ block: 'nearest' })
  })
}

function moveMention(delta: number) {
  const n = mentionItems.value.length
  if (!n) return
  mentionIdx.value = (mentionIdx.value + delta + n) % n
  scrollActiveMention()
}

function addMentionRef(relPath: string, isDir: boolean) {
  const clean = relPath.replace(/\/+$/, '')
  if (!clean) return
  if (files.value.some((f) => f.path === clean)) return
  // 相对路径既当 path（发送时 @"relpath"，agent 按 cwd 解析）又当展示名（chip 显示完整相对路径）。
  files.value.push({ path: clean, name: clean, isDir })
}

/** 把 `@token` 段替换为 insert；keepOpen 时光标停在新串尾部并重新探测（钻取续列）。 */
function replaceMentionToken(insert: string, keepOpen: boolean) {
  const start = mentionStart.value
  if (start < 0) return
  const end = start + 1 + mentionQuery.value.length
  const head = text.value.slice(0, start)
  const tail = text.value.slice(end)
  text.value = head + insert + tail
  const caret = head.length + insert.length
  mentionFetched = null // token 变了，强制重拉
  nextTick(() => {
    const el = taEl.value
    if (el) {
      el.focus()
      el.setSelectionRange(caret, caret)
    }
    autosize()
    if (keepOpen) detectMention()
  })
}

/** 引用 = 把条目加成 chip（文件 / 目录皆可），并从输入里抹掉 `@token`。 */
function commitMention(item: MentionItem) {
  addMentionRef(item.relPath, item.isDir)
  const start = mentionStart.value
  const end = start + 1 + mentionQuery.value.length
  const head = text.value.slice(0, start)
  let tail = text.value.slice(end)
  // token 两侧都是空白时合并掉一个，避免留下双空格。
  if (head.endsWith(' ') && tail.startsWith(' ')) tail = tail.slice(1)
  const caret = head.length
  text.value = head + tail
  closeMention()
  nextTick(() => {
    const el = taEl.value
    if (el) {
      el.focus()
      el.setSelectionRange(caret, caret)
    }
    autosize()
  })
}

/** 是否可进入下一级（目录且含可见子项）。空目录没有下级 → 不显示 chevron / 不响应 →。 */
function canDrill(item: MentionItem): boolean {
  return item.isDir && item.hasChildren
}

/** 进入 = 目录下钻（token 变 `@dir/` 续列）；空目录 / 文件没有下级 → 等同引用。 */
function openMention(item: MentionItem) {
  if (canDrill(item)) replaceMentionToken(`@${item.relPath}/`, true)
  else commitMention(item)
}

function mentionIconFor(item: MentionItem): Component {
  return item.isDir ? IconFolder : fileIconFor(item.relPath)
}

/** ← 逐级返回上一层：把 query 弹掉最后一段（含尾斜杠）。`web/icons/`→`web/`→``。 */
function parentQuery(q: string): string {
  const t = q.replace(/\/+$/, '')
  const i = t.lastIndexOf('/')
  return i < 0 ? '' : t.slice(0, i + 1)
}
/** 当前是否已钻入某层子目录（query 含 `/`）→ 决定是否提示/响应 ← 返回。 */
const mentionDrilled = computed(() => mentionQuery.value.includes('/'))
/** 浮层顶部的面包屑：`项目名/当前目录/`（随 query 的目录段实时更新，末段过滤词不计入）。
 *  顶层时只显示项目名。给用户一个「我现在在树的哪一层」的实时定位。 */
const mentionPathLabel = computed(() => {
  const cwd = props.session.cwd.replace(/[/\\]+$/, '')
  const root = cwd.split(/[/\\]/).pop() || cwd
  const q = mentionQuery.value
  const i = q.lastIndexOf('/')
  const dir = i < 0 ? '' : q.slice(0, i + 1)
  return `${root}/${dir}`
})
/** 光标是否停在 `@token` 末尾（方向键钻取/返回只在末尾生效，token 中间编辑不拦截）。 */
function caretAtMentionEnd(): boolean {
  const caret = taEl.value?.selectionStart ?? -1
  return caret === mentionStart.value + 1 + mentionQuery.value.length
}

// 光标移动（方向键松开 / 点击）后重新探测，让 `@` / `/` 浮层在任意位置都能跟随光标。
const CARET_MOVE_KEYS = new Set([
  'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown',
  'Home', 'End', 'PageUp', 'PageDown',
])
function onCaretMove(e?: KeyboardEvent | MouseEvent) {
  // 普通文字输入后还会收到 keyup；input 已经处理过，不再重复测量 @ 坐标和过滤 / 列表。
  if (e?.type === 'keyup' && !CARET_MOVE_KEYS.has((e as KeyboardEvent).key)) return
  // 浏览历史回填态下不自动弹 `/` / `@` 浮层：回填进来的 `/context`、`@file` 等会让浮层张开，
  // 而浮层一开，↑/↓ 就被它抢去选菜单，历史从此翻不动（用户报的「走到 /context 上下键失效」）。
  // 用户真正动手编辑（onInput → exitHistory）即恢复检测。
  if (histPos.value !== null) return
  detectMention()
  detectSlash()
}
function onMentionBlur() {
  // 失焦延迟关闭，给浮层项点击留出时机（项用 mousedown.prevent 保焦，正常点项不触发）。
  window.setTimeout(() => {
    if (document.activeElement !== taEl.value) {
      if (mentionOpen.value) closeMention()
      if (slashOpen.value) closeSlash()
    }
  }, 120)
}

let inputWorkPending = false
function runInputWork() {
  autosize()
  detectSlash()
  detectMention()
}

function scheduleInputWork() {
  if (inputWorkPending) return
  inputWorkPending = true
  nextTick(() => {
    inputWorkPending = false
    if (composing.value) return
    runInputWork()
  })
}

function onInput(e: InputEvent) {
  exitHistory() // 用户一旦手动编辑就退出历史浏览，下次 ↑ 重新从最新一条开始
  // 中文 / 日文 IME 合成期间会连续派发 input；半成品不需要自动高度、命令或文件检测。
  if (e.isComposing || composing.value) return
  // compositionend 已经安排了最终值处理时，忽略 Vue 紧邻补发的 input；普通输入保持
  // 原有同步语义，避免改变 slash 命令与紧随其后的 Enter 之间的事件顺序。
  if (inputWorkPending) return
  runInputWork()
}

// ---------- ↑/↓ 历史回填 ----------
/** 退出历史浏览态（丢弃草稿快照），不动当前文本。 */
function exitHistory() {
  histPos.value = null
  histDraft = null
}
/** 把一条历史输入回填进输入框，光标移到末尾。 */
function applyHistoryEntry(e: ChatHistoryEntry) {
  text.value = e.text
  images.value = e.images.map((i) => ({ ...i }))
  files.value = e.files.map((f) => ({ ...f }))
  nextTick(() => {
    autosize()
    const el = taEl.value
    if (el) {
      const end = text.value.length
      el.setSelectionRange(end, end)
    }
  })
}

/** 取走新分支携带的一次性草稿，避免之后切回该会话时覆盖用户已编辑的内容。 */
function takeInitialDraft(): ChatHistoryEntry | null {
  const draft = props.session.initialDraft
  if (!draft) return null
  props.session.initialDraft = undefined
  return {
    text: draft.text,
    images: draft.images.map((image) => ({ ...image })),
    files: draft.files.map((file) => ({ ...file })),
  }
}
/** 光标在首行（之前没有换行）—— ↑ 才接管为「上一条历史」，否则放行让光标正常上移。 */
function caretOnFirstLine(): boolean {
  const el = taEl.value
  if (!el) return true
  const caret = el.selectionStart ?? 0
  return !text.value.slice(0, caret).includes('\n')
}
/** 光标在末行（之后没有换行）—— ↓ 才接管为「下一条历史」。 */
function caretOnLastLine(): boolean {
  const el = taEl.value
  if (!el) return true
  const caret = el.selectionEnd ?? text.value.length
  return !text.value.slice(caret).includes('\n')
}
/** ↑：回填上一条（更旧）历史。返回是否消费了按键。 */
function historyPrev(): boolean {
  const h = promptHistory.value
  if (!h.length) return false
  if (histPos.value === null) {
    histDraft = { text: text.value, images: images.value.map((i) => ({ ...i })), files: files.value.map((f) => ({ ...f })) }
    histPos.value = h.length - 1
  } else if (histPos.value > 0) {
    histPos.value -= 1
  } else {
    return true // 已在最旧一条：仍消费按键，不让光标乱跳
  }
  applyHistoryEntry(h[histPos.value])
  return true
}
/** ↓：回填下一条（更新）历史；越过最新一条则还原草稿。返回是否消费了按键。 */
function historyNext(): boolean {
  if (histPos.value === null) return false // 没在浏览 → 放行默认 ↓
  const h = promptHistory.value
  if (histPos.value < h.length - 1) {
    histPos.value += 1
    applyHistoryEntry(h[histPos.value])
  } else {
    const draft = histDraft
    exitHistory()
    applyHistoryEntry(draft ?? { text: '', images: [], files: [] })
  }
  return true
}

// ---------- 键盘 ----------
function deleteCodexPluginMentionAtCaret(key: 'Backspace' | 'Delete'): boolean {
  if (props.session.agent !== 'codex') return false
  const el = taEl.value
  if (!el || el.selectionStart !== el.selectionEnd) return false
  const caret = el.selectionStart
  for (const range of pluginHighlightRanges.value) {
    let start = range.start
    let end = range.end
    if (key === 'Backspace') {
      if (caret > start && caret <= end) {
        // inside the token or just after @Plugin
      } else if (caret === end + 1 && text.value[end] === ' ') {
        end += 1
      } else {
        continue
      }
    } else if (caret >= start && caret < end) {
      if (caret === start && text.value[end] === ' ') end += 1
    } else {
      continue
    }
    text.value = text.value.slice(0, start) + text.value.slice(end)
    nextTick(() => {
      el.selectionStart = el.selectionEnd = start
      autosize()
      detectSlash()
      detectMention()
    })
    return true
  }
  return false
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape' && mentionOpen.value) {
    e.preventDefault()
    e.stopPropagation()
    dismissMention()
    return
  }
  if (mentionOpen.value && mentionItems.value.length) {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      moveMention(1)
      return
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault()
      moveMention(-1)
      return
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      commitMention(mentionItems.value[mentionIdx.value])
      return
    }
    if (e.key === 'Tab') {
      e.preventDefault()
      openMention(mentionItems.value[mentionIdx.value])
      return
    }
    if (e.key === 'ArrowRight' && caretAtMentionEnd()) {
      // → 下一级：仅当高亮目录含子项时钻入；空目录 / 文件没有下级，放行（光标已在行尾）。
      const it = mentionItems.value[mentionIdx.value]
      if (it && canDrill(it)) {
        e.preventDefault()
        openMention(it)
        return
      }
    }
    if (e.key === 'ArrowLeft' && mentionDrilled.value && caretAtMentionEnd()) {
      // ← 返回上一级（逐级弹掉路径末段）；未钻入时放行，让 ← 正常移动光标。
      e.preventDefault()
      replaceMentionToken(`@${parentQuery(mentionQuery.value)}`, true)
      return
    }
  }
  if (slashOpen.value && slashMatches.value.length) {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      moveSlash(1)
      return
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault()
      moveSlash(-1)
      return
    }
    if ((e.key === 'Enter' && !e.shiftKey) || e.key === 'Tab') {
      // ↵ / Tab 都把高亮项应用到输入框（对齐 @ 浮层的 Tab 行为）。
      e.preventDefault()
      pickSlash(slashMatches.value[slashIdx.value])
      return
    }
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      dismissSlash()
      return
    }
  }
  if (
    e.key === 'Escape' &&
    e.target === taEl.value &&
    !e.repeat &&
    !e.shiftKey &&
    !e.metaKey &&
    !e.ctrlKey &&
    !e.altKey &&
    !e.isComposing
  ) {
    if (running.value) {
      e.preventDefault()
      e.stopPropagation()
      void interruptChat(props.session)
    }
    return
  }
  // 历史回填：浮层都关着、无修饰键时，↑/↓ 在首/末行回填上一条/下一条用户消息（参考 Claude 客户端）。
  if (!e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && !e.isComposing) {
    if (e.key === 'ArrowUp' && caretOnFirstLine() && historyPrev()) {
      e.preventDefault()
      return
    }
    if (e.key === 'ArrowDown' && caretOnLastLine() && historyNext()) {
      e.preventDefault()
      return
    }
  }
  // Ctrl+Del：删除光标所在行。（原为 Ctrl+D，与 Windows 分屏快捷键冲突）
  if (e.ctrlKey && !e.metaKey && !e.shiftKey && !e.altKey && e.key === 'Delete') {
    e.preventDefault()
    const el = taEl.value
    if (!el) return
    const val = el.value
    const pos = el.selectionStart
    let lineStart = val.lastIndexOf('\n', pos - 1) + 1
    let lineEnd = val.indexOf('\n', pos)
    if (lineEnd === -1) lineEnd = val.length
    else lineEnd += 1
    if (lineStart === lineEnd && lineStart > 0) lineStart--
    text.value = val.slice(0, lineStart) + val.slice(lineEnd)
    nextTick(() => {
      el.selectionStart = el.selectionEnd = Math.min(lineStart, text.value.length)
      autosize()
    })
    return
  }
  // Ctrl+S stash（非 Cmd+S）：暂存当前输入框内容，下一轮结束后恢复。
  if (e.ctrlKey && !e.metaKey && !e.shiftKey && !e.altKey && (e.key === 's' || e.key === 'S')) {
    e.preventDefault()
    if (text.value.trim() || images.value.length > 0 || files.value.length > 0) {
      stash.value = { text: text.value, images: [...images.value], files: [...files.value] }
      text.value = ''
      images.value = []
      files.value = []
      nextTick(autosize)
    }
    return
  }
  // Backspace 整体删除已识别的 slash command token（蓝色高亮部分）。
  // 光标在 token 范围内（含紧跟的空格）且无选区时，一次 Backspace 清掉整个 `/command `。
  if ((e.key === 'Backspace' || e.key === 'Delete') && !e.isComposing && deleteCodexPluginMentionAtCaret(e.key)) {
    e.preventDefault()
    return
  }
  if (e.key === 'Backspace' && !e.isComposing && leadingCommand.value) {
    const el = taEl.value
    if (el && el.selectionStart === el.selectionEnd) {
      const token = `${leadingPrefix.value}${leadingCommand.value}`
      const hasTrailingSpace = text.value[token.length] === ' '
      const tokenEnd = token.length + (hasTrailingSpace ? 1 : 0)
      if (el.selectionStart <= tokenEnd) {
        e.preventDefault()
        text.value = text.value.slice(tokenEnd)
        nextTick(() => {
          el.selectionStart = el.selectionEnd = 0
          autosize()
        })
        return
      }
    }
  }
  if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
    e.preventDefault()
    submit()
  }
}

// ---------- 图片附件 ----------
function readFile(file: File): Promise<ChatImageAttachment | null> {
  return new Promise((resolve) => {
    const reader = new FileReader()
    reader.onload = () => {
      const dataUrl = String(reader.result || '')
      const comma = dataUrl.indexOf(',')
      if (comma < 0) return resolve(null)
      resolve({
        dataUrl,
        mediaType: file.type || 'image/png',
        data: dataUrl.slice(comma + 1),
        name: file.name || 'image.png',
      })
    }
    reader.onerror = () => resolve(null)
    reader.readAsDataURL(file)
  })
}

async function addFiles(files: FileList | File[]) {
  for (const f of Array.from(files)) {
    if (!f.type.startsWith('image/')) continue
    const att = await readFile(f)
    if (att) images.value.push(att)
  }
}

function onPaste(e: ClipboardEvent) {
  const items = e.clipboardData?.items
  if (!items) return
  const imgs = Array.from(items).filter((it) => it.kind === 'file' && it.type.startsWith('image/'))
  if (!imgs.length) return
  e.preventDefault()
  const files = imgs.map((it) => it.getAsFile()).filter((f): f is File => !!f)
  void addFiles(files)
}

function removeImage(i: number) {
  images.value.splice(i, 1)
}

// ---------- 文件 / 文件夹附件（系统选择器 → @path） ----------
// Claude 视觉接口只认 png/jpg/gif/webp —— 这几类才嵌成 base64 图片块（缩略图 + 视觉）；
// 其它图片格式（svg/heic…）和所有文档都当普通文件挂 @path，由 agent 自己读。
const VISION_EXTS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp'])
function extOf(p: string): string {
  const name = p.replace(/[/\\]+$/, '')
  const slash = Math.max(name.lastIndexOf('/'), name.lastIndexOf('\\'))
  const dot = name.lastIndexOf('.')
  return dot > slash + 1 ? name.slice(dot + 1).toLowerCase() : ''
}
function baseName(p: string): string {
  return p.replace(/[/\\]+$/, '').split(/[/\\]/).pop() || p
}
function addFileRef(path: string, isDir: boolean) {
  if (files.value.some((f) => f.path === path)) return // 去重
  files.value.push({ path, name: baseName(path), isDir })
}
async function addPath(path: string) {
  // 拖拽进来的路径可能是文件夹（系统选择器走 pickFolder 已知 isDir，但拖拽只给路径）。
  // 先 stat 一次：是目录就按文件夹 chip 收，绝不当文件 / 图片读。
  try {
    if (await api.pathIsDir(path)) {
      addFileRef(path, true)
      return
    }
  } catch {
    // stat 失败就按文件继续（下面的逻辑兜底）。
  }
  if (VISION_EXTS.has(extOf(path))) {
    try {
      const { mediaType, data } = await api.readFileBase64(path)
      images.value.push({ dataUrl: `data:${mediaType};base64,${data}`, mediaType, data, name: baseName(path), sourcePath: path })
      return
    } catch {
      // 读取失败就退化成普通文件引用
    }
  }
  addFileRef(path, false)
}
async function pickFilesOrPhotos() {
  plusMenuOpen.value = false
  const sel = await openDialog({ multiple: true, directory: false })
  if (!sel) return
  for (const p of Array.isArray(sel) ? sel : [sel]) await addPath(p)
}
async function pickFolder() {
  plusMenuOpen.value = false
  const sel = await openDialog({ directory: true, multiple: false })
  if (typeof sel === 'string') addFileRef(sel, true)
}
// 往输入框塞一个 "/" 并唤起指令浮层。`/` 必须处在词首（行首或空白后）才会触发浮层，
// 故已有文字且结尾非空白时补一个空格再放 `/`（空框直接 "/"，结尾已是空白则直接接 "/"）。
function pickSlashCommands() {
  plusMenuOpen.value = false
  const cur = text.value
  if (!cur.trim()) text.value = '/'
  else if (!/\s$/.test(cur)) text.value = `${cur} /`
  else text.value = `${cur}/`
  nextTick(() => {
    const el = taEl.value
    if (el) {
      el.focus()
      const end = text.value.length
      el.setSelectionRange(end, end)
    }
    autosize()
    detectSlash()
  })
}
function removeFile(i: number) {
  files.value.splice(i, 1)
}

// ---------- 发送 / 停止 ----------
async function submit() {
  // running 时**不再拦截**：有内容就走 enqueuePrompt —— 空闲即发，运行中则入队（type-while-running）。
  if (ended.value) return
  if (!text.value.trim() && images.value.length === 0 && files.value.length === 0) return
  const body = text.value
  exitHistory()
  const action = parseChatSlashAction(body)
  const intercept =
    !!action &&
    !(action.kind === 'btw' && props.session.agent !== 'claude') &&
    !(action.kind === 'side' && props.session.agent !== 'codex') &&
    !(action.kind === 'fork' && props.session.agent !== 'claude') &&
    !(action.kind === 'archive' && props.session.agent !== 'codex') &&
    !(action.kind === 'model' && !showModelPicker.value)
  if (action && intercept) {
    text.value = ''
    images.value = []
    files.value = []
    closeSlash()
    nextTick(autosize)
    switch (action.kind) {
      case 'btw':
        openBtw(action.prompt)
        break
      case 'side':
        openCodexSide(action.prompt)
        break
      case 'export':
        emit('openExport')
        break
      case 'rename':
        emit('rename')
        break
      case 'fork':
        emit('fork')
        break
      case 'clear':
        void clearChat(props.session)
        break
      case 'model':
        modelMenuRef.value?.openMenu()
        break
      case 'plan':
        onPickPermission('plan')
        break
      case 'archive':
        emit('archive')
        break
    }
    return
  }
  const imgs = images.value
  const fls = files.value
  text.value = ''
  images.value = []
  files.value = []
  closeSlash()
  nextTick(autosize)
  enqueuePrompt(props.session, body, imgs, fls)
}

/** 打开 btw 侧聊：fork 主聊上下文（仅 Claude 主聊有 sessionId 时），可带首句提示词。 */
function openBtw(prompt?: string) {
  if (props.session.agent !== 'claude') return
  void openSideChat({
    projectKey: props.session.projectKey,
    cwd: props.session.cwd,
    forkSessionId: props.session.sessionId || undefined,
    model: props.session.model,
    effort: props.session.effort,
    prompt,
  })
}

/** 打开 Codex `/side`：app-server fork 出 ephemeral thread，不复用 Claude btw 的流程。 */
function openCodexSide(prompt?: string) {
  if (props.session.agent !== 'codex') return
  void openCodexSideChat({
    projectKey: props.session.projectKey,
    cwd: props.session.cwd,
    forkThreadId: props.session.sessionId || undefined,
    model: props.session.model,
    effort: props.session.effort,
    permissionMode: props.session.permissionMode,
    prompt,
  })
}

function onPrimary() {
  if (running.value) {
    void interruptChat(props.session)
  } else {
    void submit()
  }
}

/** 待发消息的单行预览：优先正文，纯附件（无正文）时回退为附件计数描述。 */
function queuedLabel(q: QueuedMessage): string {
  const body = q.text.trim()
  if (body) return body
  const parts: string[] = []
  if (q.images.length) parts.push(t('chat.composer.queue.nImages', { n: q.images.length }))
  if (q.files.length) parts.push(t('chat.composer.queue.nFiles', { n: q.files.length }))
  return parts.join(' · ')
}
</script>

<template>
  <div class="chat-composer" :class="{ 'drag-over': dragOver }" :style="{ '--cc-drop-brand': dropBrand }">
    <!-- 大图预览：点缩略图打开，点任意处 / Esc 关闭 -->
    <Teleport to="body">
      <div v-if="previewSrc" class="cc-preview" @click="previewSrc = ''">
        <img :src="previewSrc" alt="" @click.stop />
        <button class="cc-preview-x" v-tooltip="t('common.close')" @click="previewSrc = ''">
          <IconClose />
        </button>
      </div>
    </Teleport>

    <!-- 待发队列：Codex app-server 运行中可将指定项引导到当前 turn，其余仍按 result FIFO 发出。 -->
    <div v-if="session.queue.length || session.submittedQueue.length" class="cc-queue" role="list">
      <div
        v-for="q in session.submittedQueue"
        :key="q.id"
        class="cc-queue-item cc-queue-item-submitted"
        role="listitem"
        v-tooltip="t('chat.composer.queue.submittedHint')"
      >
        <IconArrowUp class="cc-queue-ic" />
        <span class="cc-queue-text">{{ queuedLabel(q) }}</span>
        <span v-if="q.text.trim() && (q.images.length || q.files.length)" class="cc-queue-attach">
          <IconPaperclip />{{ q.images.length + q.files.length }}
        </span>
      </div>
      <div
        v-for="q in session.queue"
        :key="q.id"
        class="cc-queue-item"
        role="listitem"
        v-tooltip="t('chat.composer.queue.hint')"
      >
        <IconCornerDownLeft class="cc-queue-ic" />
        <span class="cc-queue-text">{{ queuedLabel(q) }}</span>
        <span v-if="q.text.trim() && (q.images.length || q.files.length)" class="cc-queue-attach">
          <IconPaperclip />{{ q.images.length + q.files.length }}
        </span>
        <button
          v-if="canSteerQueued(session, q.id)"
          type="button"
          class="cc-queue-steer"
          :aria-label="t('chat.composer.queue.steer')"
          v-tooltip="t('chat.composer.queue.steerHint')"
          @click.stop="steerQueued(session, q.id)"
        >
          <IconArrowUp />
        </button>
        <button
          type="button"
          class="cc-queue-x"
          :disabled="session.pendingSteerIds.includes(q.id)"
          v-tooltip="t('chat.composer.queue.remove')"
          @click.stop="removeQueued(session, q.id)"
        >
          <IconClose />
        </button>
      </div>
    </div>

    <!-- 输入框：单个 div 容器 —— 框内含 slash 浮层 + 图片缩略图 + 文本行（图片在框内、不再单列在框外） -->
    <div ref="wrapEl" class="cc-input-wrap" :class="{ disabled: ended }" @click="onWrapClick">
      <!-- 系统文件拖入提示：盖在输入框内（inset:0，与输入框严丝合缝，复用其圆角与描边） -->
      <div v-if="dragOver" class="cc-drop-hint">
        <IconPaperclip class="cc-drop-ic" />
        <span>{{ t('chat.composer.dropHint') }}</span>
      </div>
      <!-- @ 浮层：Codex 插件 + 项目 cwd 下目录/文件。插件插入 @Plugin，文件引用为 chip。 -->
      <div v-if="mentionOpen" class="cc-mention" role="listbox" :style="{ left: mentionLeft + 'px' }">
        <div ref="mentionListEl" class="cc-mention-list">
          <div v-for="section in mentionSections" :key="section.key" class="cc-mention-section">
            <div class="cc-mention-path">{{ section.title }}</div>
            <div
              v-for="{ item: it, index: i } in section.rows"
              :key="it.relPath"
              class="cc-mention-item"
              :class="{ active: i === mentionIdx }"
              role="option"
              :aria-selected="i === mentionIdx"
              @mouseenter="mentionIdx = i"
              @mousedown.prevent
              @click="commitMention(it)"
            >
              <component :is="mentionIconFor(it)" class="cc-mention-ic" />
              <span class="cc-mention-main">
                <span class="cc-mention-nm">{{ it.isDir ? it.name + '/' : it.name }}</span>
                <span v-if="!mentionDrilled && it.relPath !== it.name" class="cc-mention-rel">{{ it.relPath }}</span>
              </span>
              <button
                v-if="canDrill(it)"
                class="cc-mention-open"
                v-tooltip="t('chat.composer.mention.open')"
                @mousedown.prevent
                @click.stop="openMention(it)"
              >
                <IconChevronRight />
              </button>
            </div>
          </div>
          <div v-if="!mentionItems.length" class="cc-mention-empty">
            {{ t('chat.composer.mention.empty') }}
          </div>
        </div>
        <div v-if="mentionItems.length" class="cc-mention-hint">
          <kbd>↵</kbd>{{ t('chat.composer.mention.attach') }}
          <template v-if="activeMentionItem && canDrill(activeMentionItem)"><kbd>→</kbd>{{ t('chat.composer.mention.open') }}</template>
          <template v-if="mentionDrilled"><kbd>←</kbd>{{ t('chat.composer.mention.back') }}</template>
        </div>
      </div>

      <!-- slash 指令浮层：每个 kind 一个独立段（sticky 段标题只在本段吸顶），行 = 图标 + 展示名 + 截断描述 + 来源角标 -->
      <div v-if="slashOpen" ref="slashListEl" class="cc-slash" role="listbox">
        <div v-for="grp in slashGroups" :key="grp.kind" class="cc-slash-section">
          <div class="cc-slash-group">{{ slashGroupLabel(grp.kind) }}</div>
          <button
            v-for="{ item, gi } in grp.items"
            :key="item.kind + ':' + item.name"
            class="cc-slash-item"
            :class="{ active: gi === slashIdx }"
            role="option"
            @mouseenter="slashIdx = gi"
            @mousedown.prevent
            @click="pickSlash(item)"
          >
            <component :is="item.kind === 'skill' ? IconSkill : IconSlashSquare" class="cc-slash-ic" />
            <span class="cc-slash-title">{{ item.title }}</span>
            <span class="cc-slash-desc">{{ item.description }}</span>
            <span v-if="slashSource(item)" class="cc-slash-src">{{ slashSource(item) }}</span>
          </button>
        </div>
      </div>

      <!-- ↑/↓ 历史回填提示：浏览历史消息时显示「History 当前/总数」（框内左上角） -->
      <div v-if="historyHint" class="cc-history-hint">{{ historyHint }}</div>

      <!-- 图片缩略图（框内顶部）：hover 显示文件名，点击预览 -->
      <div v-if="images.length" class="cc-attachments">
        <div
          v-for="(img, i) in images"
          :key="i"
          class="cc-thumb"
          v-tooltip="img.name || ''"
          @click="previewSrc = img.dataUrl"
        >
          <img :src="img.dataUrl" alt="" />
          <button
            class="cc-thumb-x"
            v-tooltip="t('chat.composer.removeImage')"
            @click.stop="removeImage(i)"
          >
            <IconClose />
          </button>
        </div>
      </div>

      <!-- 文件 / 文件夹附件 chip：图标 + 文件名（限宽省略号），可单独移除 -->
      <div v-if="files.length" class="cc-files">
        <div v-for="(f, i) in files" :key="f.path" class="cc-file-chip" v-tooltip="f.path">
          <component :is="f.isDir ? IconFolder : fileIconFor(f.path)" class="cc-file-ic" />
          <span class="cc-file-nm">{{ f.name }}</span>
          <button class="cc-file-x" v-tooltip="t('chat.composer.removeImage')" @click.stop="removeFile(i)">
            <IconClose />
          </button>
        </div>
      </div>

      <!-- 文本 + 内嵌发送/停止 -->
      <div class="cc-input-row">
        <!-- 文本框 + 命令高亮镜像层：镜像在底层渲染带色文本，textarea 文本透明、只留光标 -->
        <div class="cc-ta-wrap">
          <textarea
            ref="taEl"
            v-model="text"
            class="cc-textarea"
            :class="{ 'cc-textarea--hl': highlightActive }"
            rows="1"
            :placeholder="ended ? t('chat.composer.ended') : composerPlaceholder"
            :disabled="ended"
            @input="onInput"
            @keydown="onKeydown"
            @keyup="onCaretMove"
            @click="onCaretMove"
            @blur="onMentionBlur"
            @paste="onPaste"
            @compositionstart="onCompositionStart"
            @compositionend="onCompositionEnd"
            @scroll="syncHlScroll"
          />
          <div
            v-if="highlightActive"
            ref="hlEl"
            class="cc-highlight"
            :class="{ 'cc-highlight--hint': argHintGhost }"
            aria-hidden="true"
            v-html="highlightHtml"
            @mouseenter.self.stop
            @mouseover="onHlHover"
            @mouseleave="onHlLeave"
            @click="taEl?.focus()"
          ></div>
        </div>

        <button
          class="cc-primary"
          :class="{ running }"
          :disabled="!running && !canSend"
          v-tooltip="running ? t('chat.composer.stop') : t('chat.composer.send')"
          @click="onPrimary"
        >
          <component :is="running ? IconStop : IconSend" />
        </button>
      </div>
    </div>

    <!-- 底栏：左 权限 chip + 附件；右 running spinner + 模型 / effort -->
    <div class="cc-footer">
      <div class="cc-footer-left">
        <ChatModeMenu
          :agent="agent"
          :selected="session.permissionMode"
          :model="session.model"
          :disabled="ended"
          @pick="onPickPermission"
        />
        <div class="cc-attach">
          <button
            class="cc-attach-btn"
            :class="{ active: plusMenuOpen }"
            v-tooltip="t('chat.composer.attach')"
            @click="plusMenuOpen = !plusMenuOpen"
          >
            <IconPlus />
          </button>
          <template v-if="plusMenuOpen">
            <div class="cc-plus-backdrop" @click="plusMenuOpen = false" />
            <div class="cc-plus-menu" role="menu">
              <button class="cc-plus-item" role="menuitem" @click="pickFilesOrPhotos">
                <IconPaperclip class="cc-plus-ic" />
                <span class="cc-plus-label">{{ t('chat.composer.addFiles') }}</span>
                <kbd class="cc-plus-kbd">⌘U</kbd>
              </button>
              <button class="cc-plus-item" role="menuitem" @click="pickFolder">
                <IconFolder class="cc-plus-ic" />
                <span class="cc-plus-label">{{ t('chat.composer.addFolder') }}</span>
              </button>
              <button class="cc-plus-item" role="menuitem" @click="pickSlashCommands">
                <IconSlashSquare class="cc-plus-ic" />
                <span class="cc-plus-label">{{ t('chat.composer.slashCommands') }}</span>
              </button>
            </div>
          </template>
        </div>
        <!-- btw 侧聊：右上角浮框里顺手问一句（仅 Claude）。也可直接输入 `/btw 提示词`。 -->
        <button
          v-if="session.agent === 'claude'"
          class="cc-attach-btn cc-btw-btn"
          v-tooltip="t('chat.btw.open')"
          @click="openBtw()"
        >
          <IconZap />
        </button>
        <!-- Codex `/side`：独立的 ephemeral fork 浮层，不与 Claude `/btw` 共用状态。 -->
        <button
          v-if="session.agent === 'codex'"
          class="cc-attach-btn cc-side-btn"
          v-tooltip="t('chat.side.open')"
          @click="openCodexSide()"
        >
          <IconZap />
        </button>
        <GitBranchControl
          :cwd="session.cwd"
          :disabled="session.turnState !== 'idle'"
          menu-placement="above"
        />
      </div>
      <div class="cc-footer-right">
        <!-- 顺序：context → 5h → 周（用户要求）。context 占用 ≥70% 紫、≥90% 红。 -->
        <span
          v-if="showContext"
          class="cc-ctx"
          :class="{ warn: ctxPercent >= 70 && ctxPercent < 90, danger: ctxPercent >= 90 }"
          v-tooltip="ctxTooltip"
        >{{ ctxPercent }}%</span>
        <span
          v-for="b in rateBadges"
          :key="b.key"
          class="cc-ratelimit"
          :class="{ warn: b.level === 'warn', danger: b.level === 'danger' }"
          v-tooltip="b.tooltip"
        >{{ b.text }}</span>
        <span v-if="running" class="cc-running" :class="{ retrying: session.retry }">
          <span class="cc-star" :class="session.agent">✳</span>
          <span v-if="session.retry" class="cc-retry">{{ retryLabel }} · </span>{{ elapsedLabel }}
        </span>
        <ChatModelMenu
          v-if="showModelPicker"
          ref="modelMenuRef"
          :agent="session.agent"
          :selected="session.model"
          :display-value="effectiveModel"
          :menu-options="modelMenuOptions"
          @pick="onPickModel"
        />
        <ChatEffortSlider
          v-if="showEffortPicker"
          :agent="session.agent"
          :model="effectiveModel"
          :selected="session.effort"
          :default-level="claudeRuntimeEffortLevel"
          @pick="onPickEffort"
        />
      </div>
    </div>

    <AutoModeConfirmModal
      :show="askAutoMode"
      :cwd="session.cwd"
      @confirm="confirmAutoMode"
      @cancel="cancelAutoMode"
    />
  </div>
</template>

<style scoped>
.chat-composer {
  position: relative;
  background: var(--bg);
  padding: 10px 22px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
/* 拖拽悬停态：输入框自身变成品牌色虚线投放区（边框即输入框边框，不会两层错位露灰边） */
.chat-composer.drag-over .cc-input-wrap {
  min-height: 92px;
  border-style: dashed;
  border-width: 1.5px;
  border-color: color-mix(in srgb, var(--cc-drop-brand) 60%, transparent);
  /* 立即呈现橘色虚线，不要从输入框默认的灰色描边过渡（否则拖入瞬间会闪一下灰→橘） */
  transition: none;
}
/* 提示遮罩盖满输入框内沿（inset:0 + 继承圆角），不透明品牌淡色挡住下面的文本/缩略图 */
.cc-drop-hint {
  position: absolute;
  inset: 0;
  z-index: 6;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 7px;
  border-radius: inherit;
  background: color-mix(in srgb, var(--cc-drop-brand) 9%, var(--surface));
  color: var(--text);
  font-size: 13px;
  font-weight: 500;
  pointer-events: none;
}
.cc-drop-ic {
  width: 22px;
  height: 22px;
  color: var(--cc-drop-brand);
}

/* ↑/↓ 历史回填提示：框内左上角的低调小字，等宽数字防止翻页时抖动 */
.cc-history-hint {
  font-size: 12px;
  line-height: 1;
  color: var(--text-mute);
  font-variant-numeric: tabular-nums;
  user-select: none;
}

/* 附件缩略图 */
.cc-attachments {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}
.cc-thumb {
  position: relative;
  width: 56px;
  height: 56px;
  border-radius: 8px;
  overflow: hidden;
  border: 1px solid var(--border);
  background: var(--surface);
  cursor: zoom-in;
}
.cc-thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.cc-thumb-x {
  position: absolute;
  top: 2px;
  right: 2px;
  width: 16px;
  height: 16px;
  border-radius: 999px;
  border: none;
  background: rgba(0, 0, 0, 0.6);
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0;
}
.cc-thumb-x :deep(svg) {
  width: 10px;
  height: 10px;
}

/* 大图预览遮罩（Teleport 到 body；fixed inset:0 在 zoom 下仍铺满视口） */
.cc-preview {
  position: fixed;
  inset: 0;
  z-index: 200;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.78);
  cursor: zoom-out;
  padding: 40px;
}
.cc-preview img {
  max-width: 92vw;
  max-height: 88vh;
  object-fit: contain;
  border-radius: 8px;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.5);
  cursor: default;
}
.cc-preview-x {
  position: fixed;
  top: 18px;
  right: 18px;
  width: 32px;
  height: 32px;
  border-radius: 999px;
  border: none;
  background: rgba(255, 255, 255, 0.16);
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.cc-preview-x:hover {
  background: rgba(255, 255, 255, 0.28);
}
.cc-preview-x :deep(svg) {
  width: 16px;
  height: 16px;
}

/* 待发队列：输入框上方的「将发送」消息行，每轮结束后按序逐条发出（type-while-running） */
.cc-queue {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-bottom: 8px;
}
.cc-queue-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 6px 6px 10px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--surface);
  color: var(--text-mute);
  font-size: 13px;
}
.cc-queue-item-submitted {
  border-style: dashed;
  opacity: 0.68;
}
.cc-queue-ic {
  flex: none;
  width: 13px;
  height: 13px;
  opacity: 0.65;
}
.cc-queue-text {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.cc-queue-attach {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 2px;
  font-size: 12px;
  opacity: 0.8;
  font-variant-numeric: tabular-nums;
}
.cc-queue-attach :deep(svg) {
  width: 12px;
  height: 12px;
}
.cc-queue-x {
  flex: none;
  width: 18px;
  height: 18px;
  padding: 0;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-mute);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.cc-queue-steer {
  flex: none;
  width: 22px;
  height: 18px;
  padding: 0;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-mute);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.cc-queue-steer:hover {
  background: var(--surface-hover);
  color: var(--text);
}
.cc-queue-steer :deep(svg) {
  width: 12px;
  height: 12px;
}
.cc-queue-x:hover {
  background: var(--surface-hover);
  color: var(--text);
}
.cc-queue-x:disabled {
  cursor: default;
  opacity: 0.45;
}
.cc-queue-x :deep(svg) {
  width: 12px;
  height: 12px;
}

/* 输入框：div 容器，纵向 [缩略图] + [文本行]；图片在框内 */
.cc-input-wrap {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 8px;
  border: 1px solid var(--border);
  border-radius: 14px;
  background: var(--surface);
  padding: 10px 10px 10px 14px;
  transition: border-color 0.15s;
  cursor: text;
}
/* focus 边框：用柔和的中性灰（border-strong），别用近黑的 accent */
.cc-input-wrap:focus-within {
  border-color: var(--border-strong);
}
.cc-input-wrap.disabled {
  opacity: 0.7;
  cursor: default;
}
.cc-input-row {
  display: flex;
  align-items: flex-end;
  gap: 8px;
}
/* 文本框包裹层：作为高亮镜像（absolute inset:0）的定位上下文，自身在行内仍占满弹性宽度 */
.cc-ta-wrap {
  position: relative;
  flex: 1;
  min-width: 0;
  display: flex;
}
.cc-textarea {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  resize: none;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-size: 14px;
  line-height: 1.5;
  max-height: 220px;
  field-sizing: content;
  padding: 2px 0;
  overflow-x: hidden;
}
.cc-textarea::placeholder {
  color: var(--text-mute);
}
/* 命令高亮：textarea 文本透明、只留光标；带色文本由下方镜像层渲染 */
.cc-textarea--hl {
  color: transparent;
  -webkit-text-fill-color: transparent;
  caret-color: var(--text);
}
/* 镜像层：与 textarea 同字体/字号/行高/内边距/换行规则，逐字符对齐；不接收指针事件 */
.cc-highlight {
  position: absolute;
  inset: 0;
  box-sizing: border-box;
  margin: 0;
  padding: 2px 0;
  font: inherit;
  font-size: 14px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  overflow-wrap: break-word;
  overflow: hidden;
  color: var(--text);
  pointer-events: none;
}
/* 命令 token：蓝色（深色主题用更亮的蓝以保证对比度）。
   :deep() 必需 —— 镜像层用 v-html 注入，注入节点拿不到 scoped 的 data-v 属性，普通 scoped 选择器选不中。 */
.cc-highlight :deep(.cc-cmd) {
  color: #2563eb;
  pointer-events: auto;
  cursor: default;
}
:root.theme-dark .cc-highlight :deep(.cc-cmd) {
  color: #60a5fa;
}
/* argument-hint ghost：暗色参数提示。仅在「命令已输入、参数未填」时出现，此时真实文本只有一行命令，
   故镜像层切 nowrap，让长提示停在同一行、右侧溢出裁切（对齐 TUI），而不是换行到被裁的第二行。 */
.cc-highlight--hint {
  white-space: nowrap;
}
.cc-highlight :deep(.cc-arg-hint) {
  color: var(--text-mute);
}
.cc-primary {
  flex: none;
  width: 30px;
  height: 30px;
  border-radius: 8px;
  border: none;
  background: transparent;
  color: var(--text);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: opacity 0.15s, background 0.15s;
}
/* 无填充背景；仅 hover 时浅灰 */
.cc-primary:hover:not(:disabled) {
  background: var(--surface-hover);
}
.cc-primary:disabled {
  opacity: 0.35;
  cursor: default;
}
.cc-primary.running {
  color: var(--text);
}
.cc-primary :deep(svg) {
  width: 15px;
  height: 15px;
}

/* slash 浮层：分组 + 每行 [图标][展示名][截断描述][来源角标] */
.cc-slash {
  position: absolute;
  left: 0;
  right: 0;
  bottom: calc(100% + 6px);
  max-height: 320px;
  overflow-y: auto;
  /* ↑/↓ scrollIntoView 时给 sticky 段标题留出高度，避免高亮项被标题盖住 */
  scroll-padding-top: 30px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 12px;
  /* 单层描边：用无 ring 的柔和投影（--shadow-md 自带 0 0 0 1px ring，叠加 border 会显成双框）。 */
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.18);
  /* 顶部不留 padding：让 sticky 段标题能贴着滚动区顶吸住，否则行会从这条留白里露出来 */
  padding: 0 4px 4px;
  z-index: 30;
}
/* 每个 kind 一个段：作为 sticky 段标题的「容器范围」，标题滚到段尾就被推走、不与下段叠加 */
.cc-slash-section {
  position: relative;
}
/* 段标题（Commands / Skills）：在本段内 sticky 吸顶 */
.cc-slash-group {
  position: sticky;
  top: 0;
  z-index: 1;
  padding: 8px 12px 5px;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-mute);
  /* 不透明底色：吸顶时盖住下方滚动经过的行 */
  background: var(--surface);
}
.cc-slash-item {
  width: 100%;
  display: grid;
  grid-template-columns: auto auto minmax(0, 1fr) auto;
  align-items: center;
  gap: 10px;
  padding: 7px 12px;
  border: none;
  background: transparent;
  border-radius: 8px;
  cursor: pointer;
  text-align: left;
  color: var(--text);
}
.cc-slash-item.active {
  background: var(--surface-hover);
}
.cc-slash-ic {
  flex: none;
  width: 16px;
  height: 16px;
  color: var(--text-dim);
}
.cc-slash-title {
  font-weight: 600;
  font-size: 13px;
  white-space: nowrap;
}
.cc-slash-desc {
  font-size: 12px;
  color: var(--text-mute);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}
/* 来源角标：右对齐灰字（Personal / 项目名 / 插件名） */
.cc-slash-src {
  font-size: 12px;
  color: var(--text-mute);
  white-space: nowrap;
  margin-left: auto;
}

/* @ 文件浮层（结构同 slash，多一列右对齐父目录灰字 + 底部按键提示）。
   单层描边：用 border + 无 ring 的柔和投影（--shadow-md 自带 0 0 0 1px ring，叠加 border
   会显成双框）；宽度收到 680px（窄窗时回落到容器宽）。 */
.cc-mention {
  position: absolute;
  left: 0; /* 默认值；实际由 :style 绑定的 mentionLeft 跟随 @ 的水平位置 */
  width: 680px;
  max-width: 100%;
  bottom: calc(100% + 6px);
  display: flex;
  flex-direction: column;
  max-height: 300px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 10px;
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.18);
  padding: 4px;
  z-index: 30;
}
/* 顶部面包屑：灰色小字 + 分隔线。direction:rtl + text-align:left → 路径过长时省略号落在
   左侧（保住最深一段，面包屑更有用）；纯 ASCII 路径在 WebKit 下方向正常。 */
.cc-mention-path {
  flex: none;
  position: sticky;
  top: 0;
  z-index: 1;
  padding: 5px 11px 6px;
  font-size: 11px;
  color: var(--text-mute);
  background: var(--surface);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  direction: rtl;
  text-align: left;
  border-bottom: 1px solid var(--border);
  margin-bottom: 2px;
}
.cc-mention-list {
  overflow-y: auto;
}
.cc-mention-section + .cc-mention-section {
  margin-top: 2px;
}
.cc-mention-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 10px;
  border-radius: 7px;
  cursor: pointer;
  color: var(--text);
}
.cc-mention-item.active {
  background: var(--surface-hover);
}
.cc-mention-ic {
  flex: none;
  width: 15px;
  height: 15px;
  color: var(--text-dim);
}
.cc-mention-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
}
.cc-mention-nm {
  width: 100%;
  min-width: 0;
  font-size: 13px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.cc-mention-rel {
  width: 100%;
  min-width: 0;
  font-size: 11px;
  color: var(--text-mute);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.cc-mention-empty {
  padding: 18px 12px;
  color: var(--text-mute);
  font-size: 12px;
  text-align: center;
}
.cc-mention-open {
  flex: none;
  width: 20px;
  height: 20px;
  border-radius: 6px;
  border: none;
  background: transparent;
  color: var(--text-mute);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.cc-mention-open:hover {
  background: var(--surface);
  color: var(--text);
}
.cc-mention-open :deep(svg) {
  width: 14px;
  height: 14px;
}
.cc-mention-hint {
  display: flex;
  align-items: center;
  gap: 5px;
  position: relative;
  padding: 7px 10px 3px;
  font-size: 11px;
  color: var(--text-mute);
  background: var(--surface);
  margin-top: 0;
}
.cc-mention-hint::before {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
  top: -10px;
  height: 10px;
  pointer-events: none;
  background: linear-gradient(to top, rgba(0, 0, 0, 0.035), transparent);
}
.cc-mention-hint kbd {
  font-family: inherit;
  font-size: 11px;
  color: var(--text-dim);
  background: var(--surface-hover);
  border-radius: 4px;
  padding: 1px 5px;
}

/* 底栏 */
.cc-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.cc-footer-left,
.cc-footer-right {
  display: flex;
  align-items: center;
  gap: 8px;
}
.cc-attach-btn {
  width: 24px;
  height: 24px;
  border-radius: 6px;
  border: none;
  background: transparent;
  color: var(--text-dim);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.cc-attach-btn:hover,
.cc-attach-btn.active {
  background: var(--surface-hover);
  color: var(--text);
}
.cc-attach-btn :deep(svg) {
  width: 16px;
  height: 16px;
}

/* 文件附件 chip 行 */
.cc-files {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.cc-file-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 220px;
  padding: 5px 6px 5px 10px;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: var(--surface);
}
.cc-file-ic {
  flex: none;
  width: 15px;
  height: 15px;
  color: var(--text-dim);
}
.cc-file-nm {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
  color: var(--text);
}
.cc-file-x {
  flex: none;
  width: 16px;
  height: 16px;
  border-radius: 999px;
  border: none;
  background: transparent;
  color: var(--text-mute);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.cc-file-x:hover {
  background: var(--surface-hover);
  color: var(--text);
}
.cc-file-x :deep(svg) {
  width: 12px;
  height: 12px;
}

/* "+" 弹出菜单 */
.cc-attach {
  position: relative;
  display: flex;
}
.cc-plus-backdrop {
  position: fixed;
  inset: 0;
  z-index: 40;
}
.cc-plus-menu {
  position: absolute;
  bottom: calc(100% + 6px);
  left: 0;
  z-index: 41;
  min-width: 232px;
  padding: 6px;
  border-radius: 12px;
  border: 1px solid var(--border);
  background: var(--surface);
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.18);
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.cc-plus-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text);
  font-size: 14px;
  text-align: left;
  cursor: pointer;
}
.cc-plus-item:hover {
  background: var(--surface-hover);
}
.cc-plus-ic {
  flex: none;
  width: 17px;
  height: 17px;
  color: var(--text-dim);
}
.cc-plus-label {
  flex: 1;
}
.cc-plus-kbd {
  flex: none;
  font-size: 12px;
  color: var(--text-mute);
  font-family: inherit;
}
.cc-running {
  font-size: 12px;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
}
/* 网络重试态：整行转琥珀色提醒（含星标），区别于正常处理中。 */
.cc-running.retrying,
.cc-running.retrying .cc-star {
  color: #d97706;
}
.cc-retry {
  font-weight: 500;
}
/* §10.5 上下文占用 % */
.cc-ctx {
  font-size: 12px;
  color: var(--text-mute);
  font-variant-numeric: tabular-nums;
  padding: 2px 4px;
  border-radius: 6px;
  cursor: default;
}
/* 占用 ≥70%：紫色提醒（纯文字色，无背景） */
.cc-ctx.warn {
  color: #7c3aed;
}
/* 占用 ≥90%：红色告警（纯文字色，无背景） */
.cc-ctx.danger {
  color: #d92d20;
}
/* 账号额度徽标（5h / 周，OAuth 用量接口）。文本「<窗口> <百分比>%」。
   与 .cc-ctx 同款样式：同字号、同阈值配色、纯文字色不带背景。 */
.cc-ratelimit {
  font-size: 12px;
  color: var(--text-mute);
  font-variant-numeric: tabular-nums;
  cursor: default;
  padding: 2px 4px;
  border-radius: 6px;
}
/* ≥70%：紫色提醒（同 .cc-ctx.warn） */
.cc-ratelimit.warn {
  color: #7c3aed;
}
/* ≥90%：红色告警（同 .cc-ctx.danger） */
.cc-ratelimit.danger {
  color: #d92d20;
}
/* ✳ 是 agent 品牌标记，用 agent 自己的色调，不跟随主题 --brand。 */
.cc-star {
  color: var(--brand-claude, #d97757);
  animation: cc-spin 1.4s linear infinite;
  display: inline-block;
}
.cc-star.codex {
  color: var(--brand-codex);
}
@keyframes cc-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
