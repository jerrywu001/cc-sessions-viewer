import { ref, computed, watch, watchEffect } from 'vue'
import { convertFileSrc } from '@tauri-apps/api/core'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import type { Agent, StatsRange, StatsScope } from './types'

export type Lang = 'en' | 'zh' | 'zh-TW' | 'ja'
export type Theme = 'light' | 'dark' | 'system' | 'codex' | 'dracula'

const LANG_KEY = 'lang'
const THEME_KEY = 'theme'
const PREFS_KEY = 'projPrefs:v1'
const PROJECT_ORDER_KEY = 'projectOrder:v1'
const STATS_SCOPE_KEY = 'statsScope:v1'
const STATS_RANGE_KEY = 'statsRange:v1'
const EXTERNAL_TERMINAL_KEY = 'useExternalTerminal:v1'
const AUTO_RESTORE_TERMINAL_TABS_KEY = 'autoRestoreTerminalTabs:v1'
const TERMINAL_APP_KEY = 'terminalApp:v1'
const CODEX_SHOW_INTERNAL_KEY = 'codexShowInternalSessions:v1'
const CODEX_SHOW_ARCHIVED_KEY = 'codexShowArchivedSessions:v1'
const LAUNCH_ARGS_KEY = 'launchArgs:v1'
const FONT_SCALE_KEY = 'fontScale:v1'
const FONT_FAMILY_KEY = 'fontFamily:v1'
const ENABLED_AGENTS_KEY = 'enabledAgents:v1'
const QUICK_OPEN_KEY = 'quickOpenTarget:v1'
const USE_RECLAUDE_KEY = 'useReclaude:v1'
const SHOW_TOOL_CALLS_KEY = 'showToolCalls:v1'
const EXPORT_SHOW_MESSAGE_TIME_KEY = 'exportShowMessageTime:v1'
const LEGACY_EXPORT_HTML_SHOW_MESSAGE_TIME_KEY = 'exportHtmlShowMessageTime:v1'
const CHAT_SPACING_KEY = 'chatSpacing:v1'
const SHOW_CHAT_RAIL_KEY = 'showChatRail:v1'
const CHAT_RAIL_COUNT_KEY = 'chatRailCount:v1'
const BACKGROUND_IMAGE_PATH_KEY = 'backgroundImagePath:v1'
const BACKGROUND_IMAGE_OPACITY_KEY = 'backgroundImageOpacity:v1'
const BACKGROUND_BORDER_OPACITY_KEY = 'backgroundBorderOpacity:v1'

/**
 * 根据浏览器/系统语言探测默认语言。
 * 匹配优先级：zh-Hant / zh-TW / zh-HK → zh-TW；其他 zh-* → zh；ja* → ja；其余 → en。
 * 仅在用户未显式设置（localStorage 无值）时生效。
 */
function detectSystemLang(): Lang {
  const candidates = (navigator.languages && navigator.languages.length
    ? navigator.languages
    : [navigator.language]) as string[]
  for (const raw of candidates) {
    if (!raw) continue
    const tag = raw.toLowerCase()
    if (tag.startsWith('zh')) {
      if (tag.includes('hant') || tag.includes('-tw') || tag.includes('-hk') || tag.includes('-mo')) {
        return 'zh-TW'
      }
      return 'zh'
    }
    if (tag.startsWith('ja')) return 'ja'
    if (tag.startsWith('en')) return 'en'
  }
  return 'en'
}

export const lang = ref<Lang>(
  (localStorage.getItem(LANG_KEY) as Lang | null) ?? detectSystemLang(),
)
function readTheme(): Theme {
  const v = localStorage.getItem(THEME_KEY)
  return v === 'light' || v === 'dark' || v === 'system' || v === 'codex' || v === 'dracula'
    ? v
    : 'system'
}
export const theme = ref<Theme>(readTheme())
export type TerminalApp = 'terminal' | 'iterm2' | 'ghostty' | 'cmux' | 'warp'

export const useExternalTerminal = ref(localStorage.getItem(EXTERNAL_TERMINAL_KEY) === '1')
export const autoRestoreTerminalTabs = ref(localStorage.getItem(AUTO_RESTORE_TERMINAL_TABS_KEY) === '1')
export const terminalApp = ref<TerminalApp>(
  (localStorage.getItem(TERMINAL_APP_KEY) as TerminalApp | null) ?? 'terminal',
)
export const codexShowInternalSessions = ref(localStorage.getItem(CODEX_SHOW_INTERNAL_KEY) === '1')
export const codexShowArchivedSessions = ref(localStorage.getItem(CODEX_SHOW_ARCHIVED_KEY) !== '0')

export const useReclaude = ref(localStorage.getItem(USE_RECLAUDE_KEY) === '1')
export function setUseReclaude(v: boolean) {
  useReclaude.value = v
  localStorage.setItem(USE_RECLAUDE_KEY, v ? '1' : '0')
}

/** 是否渲染 Bash / Read / Grep 等过程性工具调用。默认关闭；文件改动和需要用户操作的
 * 交互式工具（AskUserQuestion、权限确认、Codex request_user_input）始终可见。 */
export const showToolCalls = ref(localStorage.getItem(SHOW_TOOL_CALLS_KEY) === '1')
export function setShowToolCalls(v: boolean) {
  showToolCalls.value = v
  localStorage.setItem(SHOW_TOOL_CALLS_KEY, v ? '1' : '0')
}

/** 导出默认附带创建和消息时间；关闭后只影响导出文件，不影响详情页。 */
const storedExportShowMessageTime =
  localStorage.getItem(EXPORT_SHOW_MESSAGE_TIME_KEY) ??
  localStorage.getItem(LEGACY_EXPORT_HTML_SHOW_MESSAGE_TIME_KEY)
export const exportShowMessageTime = ref(storedExportShowMessageTime !== '0')
export function setExportShowMessageTime(v: boolean) {
  exportShowMessageTime.value = v
  localStorage.setItem(EXPORT_SHOW_MESSAGE_TIME_KEY, v ? '1' : '0')
}

// Backward-compatible aliases for callers using the old HTML-specific name.
export const exportHtmlShowMessageTime = exportShowMessageTime
export const setExportHtmlShowMessageTime = setExportShowMessageTime

export const showChatRail = ref(localStorage.getItem(SHOW_CHAT_RAIL_KEY) !== '0')
export function setShowChatRail(v: boolean) {
  showChatRail.value = v
  localStorage.setItem(SHOW_CHAT_RAIL_KEY, v ? '1' : '0')
}

export type ChatRailCount = number
const CHAT_RAIL_COUNT_DEFAULT = 41
const CHAT_RAIL_COUNT_MIN = 21
const CHAT_RAIL_COUNT_MAX = 71

function isChatRailCount(value: number): value is ChatRailCount {
  return Number.isInteger(value) && value >= CHAT_RAIL_COUNT_MIN && value <= CHAT_RAIL_COUNT_MAX
}

function readChatRailCount(): ChatRailCount {
  const value = Number(localStorage.getItem(CHAT_RAIL_COUNT_KEY))
  return isChatRailCount(value) ? value : CHAT_RAIL_COUNT_DEFAULT
}

export const chatRailCount = ref<ChatRailCount>(readChatRailCount())
export function setChatRailCount(value: number) {
  const next = isChatRailCount(value) ? value : CHAT_RAIL_COUNT_DEFAULT
  chatRailCount.value = next
  localStorage.setItem(CHAT_RAIL_COUNT_KEY, String(next))
}

export type ChatSpacing = number

const CHAT_SPACING_DEFAULT = 100
const CHAT_SPACING_MIN = 30
const CHAT_SPACING_MAX = 150
const CHAT_SPACING_STEP = 2

function isChatSpacing(value: number): value is ChatSpacing {
  return Number.isInteger(value)
    && value >= CHAT_SPACING_MIN
    && value <= CHAT_SPACING_MAX
    && (value - CHAT_SPACING_MIN) % CHAT_SPACING_STEP === 0
}

function readChatSpacing(): ChatSpacing {
  const value = Number(localStorage.getItem(CHAT_SPACING_KEY))
  return isChatSpacing(value) ? value : CHAT_SPACING_DEFAULT
}

export const chatSpacing = ref<ChatSpacing>(readChatSpacing())

function doApplyChatSpacing(value: ChatSpacing) {
  document.documentElement.style.setProperty('--chat-spacing-scale', String(value / 100))
}

export function setChatSpacing(value: ChatSpacing) {
  const next = isChatSpacing(value) ? value : CHAT_SPACING_DEFAULT
  chatSpacing.value = next
  localStorage.setItem(CHAT_SPACING_KEY, String(next))
  doApplyChatSpacing(next)
}

export function applyChatSpacing() {
  doApplyChatSpacing(chatSpacing.value)
}

doApplyChatSpacing(chatSpacing.value)

// ---------- 自定义背景 ----------
// 只持久化用户选择的文件路径。Tauri asset protocol 已授权本地资源路径，
// convertFileSrc 会为各平台生成可由 WebView 加载的安全 URL。
export type BackgroundImageOpacity = number
export type BackgroundBorderOpacity = number

const BACKGROUND_IMAGE_OPACITY_DEFAULT = 40
const BACKGROUND_IMAGE_OPACITY_MIN = 0
const BACKGROUND_IMAGE_OPACITY_MAX = 100
/* border-strong 会在此值基础上额外增加 18%，故上限保留余量。 */
const BACKGROUND_BORDER_OPACITY_DEFAULT = 26
const BACKGROUND_BORDER_OPACITY_MIN = 0
const BACKGROUND_BORDER_OPACITY_MAX = 80

function readBackgroundImagePath(): string | null {
  const path = localStorage.getItem(BACKGROUND_IMAGE_PATH_KEY)?.trim()
  return path || null
}

function isBackgroundImageOpacity(value: number): value is BackgroundImageOpacity {
  return Number.isInteger(value)
    && value >= BACKGROUND_IMAGE_OPACITY_MIN
    && value <= BACKGROUND_IMAGE_OPACITY_MAX
}

function readBackgroundImageOpacity(): BackgroundImageOpacity {
  const stored = localStorage.getItem(BACKGROUND_IMAGE_OPACITY_KEY)
  if (stored === null) return BACKGROUND_IMAGE_OPACITY_DEFAULT
  const value = Number(stored)
  return isBackgroundImageOpacity(value) ? value : BACKGROUND_IMAGE_OPACITY_DEFAULT
}

function isBackgroundBorderOpacity(value: number): value is BackgroundBorderOpacity {
  return Number.isInteger(value)
    && value >= BACKGROUND_BORDER_OPACITY_MIN
    && value <= BACKGROUND_BORDER_OPACITY_MAX
}

function readBackgroundBorderOpacity(): BackgroundBorderOpacity {
  const stored = localStorage.getItem(BACKGROUND_BORDER_OPACITY_KEY)
  if (stored === null) return BACKGROUND_BORDER_OPACITY_DEFAULT
  const value = Number(stored)
  return isBackgroundBorderOpacity(value) ? value : BACKGROUND_BORDER_OPACITY_DEFAULT
}

export const backgroundImagePath = ref<string | null>(readBackgroundImagePath())
export const backgroundImageOpacity = ref<BackgroundImageOpacity>(readBackgroundImageOpacity())
export const backgroundBorderOpacity = ref<BackgroundBorderOpacity>(readBackgroundBorderOpacity())
export const backgroundIsVideo = computed(() =>
  backgroundImagePath.value?.toLowerCase().endsWith('.mp4') ?? false,
)

function doApplyBackgroundImage(path: string | null, opacity: BackgroundImageOpacity) {
  const root = document.documentElement
  const enabled = Boolean(path)
  const isVideo = path?.toLowerCase().endsWith('.mp4') ?? false
  root.classList.toggle('has-custom-background', enabled)
  root.classList.toggle('has-custom-image-background', enabled && !isVideo)
  root.classList.toggle('has-custom-video-background', enabled && isVideo)
  root.style.setProperty('--custom-background-opacity', `${opacity}%`)
  if (path && !isVideo) {
    root.style.setProperty('--custom-background-image', `url(${JSON.stringify(convertFileSrc(path))})`)
  } else {
    root.style.removeProperty('--custom-background-image')
  }
}

export function setBackgroundImagePath(path: string | null) {
  const next = path?.trim() || null
  backgroundImagePath.value = next
  if (next) {
    localStorage.setItem(BACKGROUND_IMAGE_PATH_KEY, next)
  } else {
    localStorage.removeItem(BACKGROUND_IMAGE_PATH_KEY)
  }
  doApplyBackgroundImage(next, backgroundImageOpacity.value)
}

export function setBackgroundImageOpacity(value: BackgroundImageOpacity) {
  const next = isBackgroundImageOpacity(value) ? value : BACKGROUND_IMAGE_OPACITY_DEFAULT
  backgroundImageOpacity.value = next
  localStorage.setItem(BACKGROUND_IMAGE_OPACITY_KEY, String(next))
  doApplyBackgroundImage(backgroundImagePath.value, next)
}

function doApplyBackgroundBorderOpacity(value: BackgroundBorderOpacity) {
  document.documentElement.style.setProperty('--custom-background-border-opacity', `${value}%`)
}

export function setBackgroundBorderOpacity(value: BackgroundBorderOpacity) {
  const next = isBackgroundBorderOpacity(value) ? value : BACKGROUND_BORDER_OPACITY_DEFAULT
  backgroundBorderOpacity.value = next
  localStorage.setItem(BACKGROUND_BORDER_OPACITY_KEY, String(next))
  doApplyBackgroundBorderOpacity(next)
}

export function applyBackgroundImage() {
  doApplyBackgroundImage(backgroundImagePath.value, backgroundImageOpacity.value)
}

doApplyBackgroundImage(backgroundImagePath.value, backgroundImageOpacity.value)
doApplyBackgroundBorderOpacity(backgroundBorderOpacity.value)

// ---------- 双击 / 新建快捷键默认打开什么 ----------
// 双击 tab 条空白处、⌘N / ⌘T 默认都开「会话(session)」。这里让用户改成开
// 「终端(terminal, 纯 shell)」或「chat(GUI live chat, 等价右键 New chat)」。
// 选择了不支持 GUI Chat 的 agent（例如 Grok）时，调用方会显示明确提示。
export type QuickOpenTarget = 'session' | 'terminal' | 'chat'
function readQuickOpenTarget(): QuickOpenTarget {
  const v = localStorage.getItem(QUICK_OPEN_KEY)
  return v === 'session' || v === 'terminal' || v === 'chat' ? v : 'session'
}
export const quickOpenTarget = ref<QuickOpenTarget>(readQuickOpenTarget())
export function setQuickOpenTarget(v: QuickOpenTarget) {
  quickOpenTarget.value = v
  localStorage.setItem(QUICK_OPEN_KEY, v)
}

export type LaunchArgs = { claude: string; codex: string; agy: string; opencode: string; grok: string; kimicode: string; pi: string }
function readLaunchArgs(): LaunchArgs {
  try {
    const v = localStorage.getItem(LAUNCH_ARGS_KEY)
    if (v) {
      const parsed = JSON.parse(v) as Partial<LaunchArgs>
      return {
        claude: parsed.claude ?? '',
        codex: parsed.codex ?? '',
        agy: parsed.agy ?? '',
        opencode: parsed.opencode ?? '',
        grok: parsed.grok ?? '',
        kimicode: parsed.kimicode ?? (parsed as { kimi?: string }).kimi ?? '',
        pi: parsed.pi ?? '',
      }
    }
  } catch { /* ignore */ }
  return { claude: '', codex: '', agy: '', opencode: '', grok: '', kimicode: '', pi: '' }
}
export const launchArgs = ref<LaunchArgs>(readLaunchArgs())

export function setLaunchArgs(agent: keyof LaunchArgs, args: string) {
  launchArgs.value = { ...launchArgs.value, [agent]: args }
  localStorage.setItem(LAUNCH_ARGS_KEY, JSON.stringify(launchArgs.value))
}

// ---------- Agent 显隐开关 ----------
// 只用 cc 的用户可以把 codex 关掉，让侧栏/主页的 agent 切换更清爽。
// 固定顺序决定旧配置超过上限时的保留优先级；至少保留一个启用，否则整个 app 无内容可看。
export const ALL_AGENTS: Agent[] = ['claude', 'codex', 'grok', 'kimicode', 'pi', 'agy', 'opencode']
export const MAX_ENABLED_AGENTS = 4
type EnabledAgents = Record<Agent, boolean>

let enabledAgentsWereTrimmed = false

function limitEnabledAgents(agents: EnabledAgents): EnabledAgents {
  let enabledCount = 0
  const limited = { ...agents }
  for (const agent of ALL_AGENTS) {
    if (!limited[agent]) continue
    if (enabledCount < MAX_ENABLED_AGENTS) {
      enabledCount += 1
    } else {
      limited[agent] = false
    }
  }
  return limited
}

function readEnabledAgents(): EnabledAgents {
  const defaults: EnabledAgents = { claude: true, codex: true, grok: true, kimicode: false, pi: true, agy: false, opencode: false }
  try {
    const v = localStorage.getItem(ENABLED_AGENTS_KEY)
    if (v) {
      const parsed = JSON.parse(v) as Partial<EnabledAgents>
      const merged: EnabledAgents = {
        claude: parsed.claude ?? true,
        codex: parsed.codex ?? true,
        grok: parsed.grok ?? true,
        kimicode: parsed.kimicode ?? (parsed as { kimi?: boolean }).kimi ?? false,
        agy: parsed.agy ?? false,
        opencode: parsed.opencode ?? false,
        pi: parsed.pi ?? true,
      }
      // 防御：localStorage 里若全是 false（脏数据/手改）就回退到默认开启项。
      if (ALL_AGENTS.some((a) => merged[a])) {
        const limited = limitEnabledAgents(merged)
        if (ALL_AGENTS.some((a) => limited[a] !== merged[a])) {
          enabledAgentsWereTrimmed = true
          localStorage.setItem(ENABLED_AGENTS_KEY, JSON.stringify(limited))
        }
        return limited
      }
    }
  } catch { /* ignore */ }
  return defaults
}

export const enabledAgents = ref<EnabledAgents>(readEnabledAgents())

/** 当前启用（可见）的 agent，按固定顺序；保证非空。 */
export const visibleAgents = computed<Agent[]>(() =>
  ALL_AGENTS.filter((a) => enabledAgents.value[a]),
)

/** 返回并清除旧配置因超出上限而被自动调整的提示标记。 */
export function consumeEnabledAgentsTrimmedNotice(): boolean {
  const wasTrimmed = enabledAgentsWereTrimmed
  enabledAgentsWereTrimmed = false
  return wasTrimmed
}

export function setAgentEnabled(a: Agent, enabled: boolean) {
  // 不允许关掉最后一个启用的 agent。
  if (!enabled && enabledAgents.value[a] && visibleAgents.value.length === 1) return
  // 上限同样在状态层保障，避免通过非设置页调用绕过。
  if (enabled && !enabledAgents.value[a] && visibleAgents.value.length >= MAX_ENABLED_AGENTS) return
  enabledAgents.value = { ...enabledAgents.value, [a]: enabled }
  localStorage.setItem(ENABLED_AGENTS_KEY, JSON.stringify(enabledAgents.value))
}

export type FontScale = number

const FONT_SCALE_DEFAULT = 14
const FONT_SCALE_MIN = 12
const FONT_SCALE_MAX = 18

function readFontScale(): FontScale {
  const raw = localStorage.getItem(FONT_SCALE_KEY)
  if (raw === 'small') return 13
  if (raw === 'normal') return FONT_SCALE_DEFAULT
  if (raw === 'large') return 15
  const n = Number(raw)
  return n >= FONT_SCALE_MIN && n <= FONT_SCALE_MAX ? n : FONT_SCALE_DEFAULT
}
export const fontScale = ref<FontScale>(readFontScale())

export function setFontScale(s: FontScale) {
  fontScale.value = s
  localStorage.setItem(FONT_SCALE_KEY, String(s))
}

function doApplyZoom(size: number) {
  const zoom = size / FONT_SCALE_DEFAULT
  document.documentElement.style.setProperty('--app-zoom', String(zoom))
  // WKWebView 下 CSS `body.style.zoom < 1` 导致选区坐标偏移（文字选中"飘"、
  // 右键菜单消失）。改用 Tauri 原生 webview setZoom（= 浏览器级缩放），
  // 坐标系统由引擎管理，选区 / 右键菜单 / 鼠标事件全部正确。
  document.body.style.zoom = ''
  try {
    getCurrentWebview().setZoom(zoom).catch(() => {
      document.body.style.zoom = String(zoom)
    })
  } catch {
    document.body.style.zoom = String(zoom)
  }
}

export function applyFontScale() {
  doApplyZoom(fontScale.value)
}

doApplyZoom(fontScale.value)

const FONT_FAMILY_DEFAULT = ''

function readFontFamily(): string {
  return localStorage.getItem(FONT_FAMILY_KEY) ?? FONT_FAMILY_DEFAULT
}
export const fontFamily = ref<string>(readFontFamily())

export function setFontFamily(v: string) {
  fontFamily.value = v
  localStorage.setItem(FONT_FAMILY_KEY, v)
}

function doApplyFontFamily(v: string) {
  if (v) {
    document.documentElement.style.setProperty('font-family', v)
  } else {
    document.documentElement.style.removeProperty('font-family')
  }
}

export function applyFontFamily() {
  doApplyFontFamily(fontFamily.value)
}

doApplyFontFamily(fontFamily.value)

export function setLang(l: Lang) {
  lang.value = l
  localStorage.setItem(LANG_KEY, l)
}

export function setTheme(t: Theme) {
  theme.value = t
  localStorage.setItem(THEME_KEY, t)
}

export function setUseExternalTerminal(v: boolean) {
  useExternalTerminal.value = v
  localStorage.setItem(EXTERNAL_TERMINAL_KEY, v ? '1' : '0')
}

export function setAutoRestoreTerminalTabs(v: boolean) {
  autoRestoreTerminalTabs.value = v
  localStorage.setItem(AUTO_RESTORE_TERMINAL_TABS_KEY, v ? '1' : '0')
}

export function setTerminalApp(v: TerminalApp) {
  terminalApp.value = v
  localStorage.setItem(TERMINAL_APP_KEY, v)
}

/** 用户是否手动选过终端应用 */
export function hasTerminalAppPreference(): boolean {
  return localStorage.getItem(TERMINAL_APP_KEY) !== null
}

/** 首次启动时根据检测结果设默认值：有 cmux 就默认 cmux，否则 terminal */
export function applyTerminalDefault(available: string[]) {
  if (hasTerminalAppPreference()) return
  if (available.includes('cmux')) {
    terminalApp.value = 'cmux'
  }
}

export function setCodexShowInternalSessions(v: boolean) {
  codexShowInternalSessions.value = v
  localStorage.setItem(CODEX_SHOW_INTERNAL_KEY, v ? '1' : '0')
}

export function setCodexShowArchivedSessions(v: boolean) {
  codexShowArchivedSessions.value = v
  localStorage.setItem(CODEX_SHOW_ARCHIVED_KEY, v ? '1' : '0')
}

function systemDark(): boolean {
  return window.matchMedia('(prefers-color-scheme: dark)').matches
}

export function applyTheme() {
  const dark = theme.value === 'dark' || theme.value === 'dracula' || (theme.value === 'system' && systemDark())
  document.documentElement.classList.toggle('theme-dark', dark)
  document.documentElement.classList.toggle('theme-codex', theme.value === 'codex')
  document.documentElement.classList.toggle('theme-dracula', theme.value === 'dracula')
}

/** 原生窗口外观该钉到哪一态（同步标题栏 / 失焦红绿灯灰圈的对比）。
 *  'system' 返回 null —— 交还系统自动跟随，不固定，免得破坏 prefers-color-scheme。 */
export function nativeAppearance(t: Theme): 'dark' | 'light' | null {
  if (t === 'system') return null
  return t === 'dark' || t === 'dracula' ? 'dark' : 'light'
}

// 主题变化或系统外观变化时自动应用
watchEffect(applyTheme)
window
  .matchMedia('(prefers-color-scheme: dark)')
  .addEventListener('change', () => {
    if (theme.value === 'system') applyTheme()
  })

/** 清除应用级缓存（项目置顶/沉底、手动排序和终端偏好；会话 rename 直接写 JSONL，不走 cache） */
export function clearAppCache() {
  localStorage.removeItem(PREFS_KEY)
  localStorage.removeItem(PROJECT_ORDER_KEY)
  localStorage.removeItem(TERMINAL_APP_KEY)
  localStorage.removeItem(EXTERNAL_TERMINAL_KEY)
  localStorage.removeItem(LAUNCH_ARGS_KEY)
  terminalApp.value = 'terminal'
  useExternalTerminal.value = false
  launchArgs.value = { claude: '', codex: '', agy: '', opencode: '', grok: '', kimicode: '', pi: '' }
}

// ---------- Statistics 页的 scope / range 持久化 ----------
// 默认 all agents + 过去 3 个月；用户改完写回 localStorage，下次进入沿用上次选择。
// （之前默认是 "all"=全部时间，全盘扫成本巨大且基本没人关心 1 年前的；改成
// months3 后默认体验快得多，需要看更老的数据再手动切。）

function readStatsScope(): StatsScope {
  const v = localStorage.getItem(STATS_SCOPE_KEY)
  if (v === 'kimi') return 'kimicode'
  return v === 'claude' || v === 'codex' || v === 'grok' || v === 'kimicode' || v === 'pi' || v === 'opencode' || v === 'all'
    ? v as StatsScope
    : 'all'
}
function readStatsRange(): StatsRange {
  const v = localStorage.getItem(STATS_RANGE_KEY) || ''
  // 老用户 localStorage 里可能还存着 'all'（已废弃）—— 这里静默回退到 months6，
  // 后端 parse_range 也已经不认 'all'。
  return v === 'today'
    || v === 'days7'
    || v === 'days30'
    || v === 'month'
    || v === 'months3'
    || v === 'months6'
    || /^custom:\d{4}-\d{2}-\d{2}:\d{4}-\d{2}-\d{2}$/.test(v)
    ? v as StatsRange
    : 'months3'
}

export const statsScope = ref<StatsScope>(readStatsScope())
export const statsRange = ref<StatsRange>(readStatsRange())

watch(statsScope, (v) => localStorage.setItem(STATS_SCOPE_KEY, v))
watch(statsRange, (v) => localStorage.setItem(STATS_RANGE_KEY, v))

/** Restore every persisted preference owned by this module and clear app cache. */
export function resetSettings() {
  setLang(detectSystemLang())
  setTheme('system')
  setUseExternalTerminal(false)
  setAutoRestoreTerminalTabs(false)
  localStorage.removeItem(TERMINAL_APP_KEY)
  terminalApp.value = 'terminal'
  setCodexShowInternalSessions(false)
  setCodexShowArchivedSessions(true)
  setUseReclaude(false)
  setShowToolCalls(false)
  setShowChatRail(true)
  setChatRailCount(CHAT_RAIL_COUNT_DEFAULT)
  setChatSpacing(CHAT_SPACING_DEFAULT)
  setBackgroundImagePath(null)
  setBackgroundImageOpacity(BACKGROUND_IMAGE_OPACITY_DEFAULT)
  setBackgroundBorderOpacity(BACKGROUND_BORDER_OPACITY_DEFAULT)
  setQuickOpenTarget('session')
  launchArgs.value = { claude: '', codex: '', agy: '', opencode: '', grok: '', kimicode: '', pi: '' }
  localStorage.removeItem(LAUNCH_ARGS_KEY)
  enabledAgents.value = { claude: true, codex: true, grok: true, kimicode: false, pi: true, agy: false, opencode: false }
  localStorage.removeItem(ENABLED_AGENTS_KEY)
  setFontScale(FONT_SCALE_DEFAULT)
  applyFontScale()
  setFontFamily(FONT_FAMILY_DEFAULT)
  applyFontFamily()
  statsScope.value = 'all'
  statsRange.value = 'months3'
  localStorage.removeItem('settingsActiveTab:v1')
  clearAppCache()
}
