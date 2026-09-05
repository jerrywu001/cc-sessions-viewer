// 全局 TUI tabs 管理 —— 把"嵌入终端"提到 App 顶层，让多个 PTY 同时存活、互不打扰。
//
// 设计：
//   - 一个模块级的 reactive `tabs` 列表 + `activeUiId`，全应用唯一来源。
//   - 每个 tab 持有自己的 `Terminal` 实例 + 一个 detached <div> 容器；切 tab 只是
//     把 container 再 attach 到可见的 slot，xterm 内部 scrollback / 光标位置全程不丢。
//   - PTY 字节流通过 Tauri 全局事件 `pty://data` 广播，每个 tab 自己装一个 listener
//     按 `payload.id === ptyId` 过滤；listen 是 N 路独立订阅，互不抢消息。
//   - Terminal / FitAddon / HTMLDivElement 都用 `markRaw()` 包一层，避免 Vue 反应
//     式代理穿透到 xterm 内部 —— xterm 自己管 DOM mutation，不希望被劫持。
//
// 生命周期：
//   - openOrFocusTui  → 同会话已开则 focus；否则 new Terminal + new container +
//                       ptySpawn，把 tab push 进 tabs，set active。
//   - closeTab        → 卸 listener、dispose Terminal、kill PTY、splice。
//   - PTY 自身 exit   → 标记 status='exited'，不立刻移除（用户可以看到完整收尾再手动关）。
//
// 不持久化：刷新 webview = 全部 tabs 没了（PTY 进程被 kill）。这是预期 —— 应用
// 重启相当于关掉所有"窗口"，跟系统终端语义一致。

const _isMac = /Mac/i.test(navigator.platform)
const _isWindows = /Win/i.test(navigator.platform)

import { markRaw, nextTick, reactive, ref, watch } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { Agent, SessionMeta } from './types'
import { backgroundImagePath, theme, launchArgs, useReclaude } from './settings'
import { t } from './i18n'
import { humanizeTerminalSessionError } from './sessionError'
import { panes, focusPane, ensureLayout, activeUiId } from './panes'
import * as api from './api'
import {
  applyPendingTurnState,
  applyTurnSignal,
  applyTerminalInputState,
  clearLocalWorkingTurn,
  createTerminalInputState,
  markSessionActivity,
  rememberPendingTurnState,
  sessionPathsEqual,
  setProcessState,
  setTurnState,
  shouldTerminalInputStartTurn,
  type TerminalInputState,
  type TerminalProcessState,
  type TerminalTurnSignalSource,
  type TerminalTurnState,
} from './tabStatus'
import {
  buildTerminalSelectionDeleteSequence,
  shouldHandleTerminalSelectionDelete,
  type TerminalSelectionRange,
} from './terminalSelectionDelete'
import { installTerminalScrollbackProtection } from './terminalScrollback'
import {
  pasteIntentForEvent,
  readClipboardImages,
  runEmbeddedCliPaste,
} from './embeddedCliPaste'

export type {
  TerminalProcessState,
  TerminalTurnEventState,
  TerminalTurnSignalSource,
  TerminalTurnState,
} from './tabStatus'

/// 处理纯 shell tab 的 DOM paste 事件（含图片）。内嵌 CLI 由快捷键控制器处理。
/// Mac 上图片保存为临时文件后把路径粘贴到终端。
async function _handleTerminalPaste(term: Terminal, e: ClipboardEvent) {
  const items = e.clipboardData?.items
  if (!items) return
  for (const item of Array.from(items)) {
    if (item.kind === 'file' && item.type.startsWith('image/')) {
      e.preventDefault()
      e.stopImmediatePropagation()
      const file = item.getAsFile()
      if (!file) continue
      const buf = await file.arrayBuffer()
      const b64 = btoa(String.fromCharCode(...new Uint8Array(buf)))
      const path = await api.saveClipboardImage(b64, file.type)
      term.paste(path)
      return
    }
  }
}

type TerminalKeyEvent = Pick<
  KeyboardEvent,
  'type' | 'key' | 'ctrlKey' | 'shiftKey' | 'altKey' | 'metaKey'
>

export function shouldCopyWindowsTerminalSelection(
  ev: TerminalKeyEvent,
  hasSelection: boolean,
  platform = navigator.platform,
): boolean {
  return (
    /Win/i.test(platform) &&
    hasSelection &&
    ev.type === 'keydown' &&
    ev.key.toLowerCase() === 'c' &&
    ev.ctrlKey &&
    !ev.shiftKey &&
    !ev.altKey &&
    !ev.metaKey
  )
}

export function shouldBlinkTerminalCursor(agent: Agent, platform = navigator.platform): boolean {
  return !(/Win/i.test(platform) && agent === 'codex')
}

export function shouldUseStableTerminalCursor(
  agent: Agent,
  turnState: TerminalTurnState,
  isShell = false,
  platform = navigator.platform,
): boolean {
  return /Win/i.test(platform) && agent === 'codex' && !isShell && turnState === 'working'
}

export function terminalPtyColumns(
  agent: Agent,
  cols: number,
  platform = navigator.platform,
): number {
  if (!/Win/i.test(platform)) return cols
  if (agent === 'pi') return Math.max(20, cols - 2)
  if (agent === 'kimicode') return Math.max(20, cols - 1)
  return cols
}

function isCapsLockEvent(event: KeyboardEvent): boolean {
  // Chromium on macOS may expose Caps Lock as keyCode 20 while IME composition
  // is active, with an empty or non-standard `key`/`code` value.
  return event.key === 'CapsLock' || event.code === 'CapsLock' || event.keyCode === 20
}

export function shouldBufferTerminalImeSwitch(
  event: KeyboardEvent,
  platform = navigator.platform,
): boolean {
  if (isCapsLockEvent(event)) return true
  // Windows Microsoft Pinyin can use a bare Shift to switch Chinese/English.
  return /Win/i.test(platform)
    && (event.key === 'Shift' || event.code === 'ShiftLeft' || event.code === 'ShiftRight' || event.keyCode === 16)
    && !event.ctrlKey
    && !event.altKey
    && !event.metaKey
}

/**
 * 中文输入法在切换到英文时会结束当前 composition，并在同一轮事件循环里同时
 * 触发 xterm 的 composition 提交和 insertText。xterm 6 会把二者都作为 terminal data
 * 发出，导致预编辑文本（如 `grok`）重复写入 PTY。只在 compositionend 的下一轮事件
 * 循环中忽略第二次完全相同的数据，正常选词、英文输入及粘贴均不受影响。
 */
export function createTerminalImeInputDeduper() {
  // Caps Lock / Windows Shift 输入法切换有时会让 xterm 在 compositionend 后延迟一小段时间再发第二份
  // 数据；0ms 只覆盖同一轮事件，实际仍可能出现 `grokgrok`。
  const IME_DEDUP_WINDOW_MS = 120
  let awaitingCompositionInput = false
  let forwardedData: string | undefined
  let clearTimer: ReturnType<typeof setTimeout> | undefined
  let capsBuffer = ''
  let capsTimer: ReturnType<typeof setTimeout> | undefined
  let flushHandler: ((data: string) => void) | undefined

  const clear = () => {
    awaitingCompositionInput = false
    forwardedData = undefined
    clearTimer = undefined
  }

  const flushCapsBuffer = () => {
    if (capsTimer !== undefined) clearTimeout(capsTimer)
    capsTimer = undefined
    const data = normalizeCompositionData(capsBuffer)
    capsBuffer = ''
    if (data) flushHandler?.(data)
  }

  const normalizeCompositionData = (data: string) => {
    if (!/^[A-Za-z](?:[ A-Za-z]*[A-Za-z])?$/.test(data)) return data
    const compact = data.replace(/\s+/g, '')
    const half = compact.length / 2
    return half > 0 && Number.isInteger(half) && compact.slice(0, half) === compact.slice(half)
      ? compact.slice(0, half)
      : compact
  }

  return {
    setFlushHandler(handler: (data: string) => void) {
      flushHandler = handler
    },
    onInputMethodSwitch() {
      if (capsTimer !== undefined) clearTimeout(capsTimer)
      capsBuffer = ''
      capsTimer = setTimeout(flushCapsBuffer, IME_DEDUP_WINDOW_MS)
    },
    onCompositionEnd() {
      if (clearTimer !== undefined) clearTimeout(clearTimer)
      awaitingCompositionInput = true
      forwardedData = undefined
      // 覆盖 xterm 自己的延迟提交，以及 Caps Lock 触发的紧邻第二次提交。
      clearTimer = setTimeout(clear, IME_DEDUP_WINDOW_MS)
    },
    consume(data: string): string | null {
      if (capsTimer !== undefined) {
        capsBuffer += data
        return null
      }
      const normalized = normalizeCompositionData(data)
      if (!awaitingCompositionInput) return normalized
      if (forwardedData === undefined) {
        forwardedData = normalized
        // 只修复输入法合成串中的分隔空格（如 `g ro k`）；普通英文输入仍原样透传。
        return normalized
      }
      const isDuplicate = normalized === forwardedData
      clear()
      return isDuplicate ? null : normalized
    },
  }
}

async function copyTerminalSelectionText(text: string) {
  try {
    await navigator.clipboard?.writeText?.(text)
  } catch {
    /* Clipboard write can be denied by the webview; still swallow Ctrl+C so it never kills the PTY. */
  }
}

function handleWindowsTerminalSelectionCopy(term: Terminal, ev: KeyboardEvent): boolean {
  if (!shouldCopyWindowsTerminalSelection(ev, term.hasSelection())) {
    return false
  }
  ev.preventDefault()
  ev.stopImmediatePropagation()
  const text = term.getSelection()
  if (text) void copyTerminalSelectionText(text)
  return true
}

export interface TerminalSelectionDeleteTarget {
  hasSelection(): boolean
  getSelection(): string
  getSelectionPosition(): TerminalSelectionRange | undefined
  getActiveCursorRow(): number
  clearSelection(): void
  input(data: string, wasUserInput?: boolean): void
}

export function handleWindowsTerminalSelectionDelete(
  target: TerminalSelectionDeleteTarget,
  inputState: TerminalInputState,
  event: KeyboardEvent,
  canEdit: boolean,
  platform = navigator.platform,
): boolean {
  if (
    !canEdit ||
    !shouldHandleTerminalSelectionDelete(
      event,
      target.hasSelection(),
      platform,
    )
  ) {
    return false
  }

  event.preventDefault()
  event.stopImmediatePropagation()
  const sequence = buildTerminalSelectionDeleteSequence(
    inputState,
    target.getSelection(),
    target.getSelectionPosition(),
    target.getActiveCursorRow(),
  )
  if (sequence) {
    target.clearSelection()
    target.input(sequence, true)
  }
  return true
}

function selectionDeleteTarget(term: Terminal): TerminalSelectionDeleteTarget {
  return {
    hasSelection: () => term.hasSelection(),
    getSelection: () => term.getSelection(),
    getSelectionPosition: () => term.getSelectionPosition(),
    // xterm exposes the selection model's raw zero-based buffer row here.
    getActiveCursorRow: () =>
      term.buffer.active.baseY + term.buffer.active.cursorY,
    clearSelection: () => term.clearSelection(),
    input: (data, wasUserInput) => term.input(data, wasUserInput),
  }
}

export interface TerminalTab {
  /** 本地稳定 id，供 v-for 用；和后端 pty id 是两套号 */
  uiId: number
  /** 后端 PTY id —— spawn 完成前是 null */
  ptyId: number | null
  agent: Agent
  /** 所属侧栏项目的 key（= ProjectInfo.dirName）。tab 只在 (agent, projectKey)
   *  匹配当前侧栏选中项时显示在 strip 里；切别的项目 PTY 不杀，只是临时隐藏。 */
  projectKey: string
  /** 所属分屏格子 id（见 panes.ts）。多 pane 时 strip 只显示 paneId 匹配的 tab。 */
  paneId: number
  sessionId: string
  sessionPath: string
  title: string
  cwd: string
  createdAt: number
  /* xterm 实例 —— 用 markRaw() 防 Vue 代理 */
  term: Terminal
  fitAddon: FitAddon
  /** xterm 真正渲染所在的 <div>；切 tab 时把它从 slot 里挪走 / 挪回，不 dispose Terminal */
  container: HTMLDivElement
  unlistenData: UnlistenFn | null
  unlistenExit: UnlistenFn | null
  onDataDisp: { dispose: () => void } | null
  lastSyncedCols: number
  lastSyncedRows: number
  inputState: TerminalInputState
  stableCursorX?: number
  stableCursorRowFromBottom?: number
  /** 进程生命周期：只描述 PTY/CLI 进程本身，不代表本轮回答是否完成。 */
  processState: TerminalProcessState
  /** 本轮问答状态：完成/阻塞/错误只能由 agent/session 的明确信号推进。 */
  turnState: TerminalTurnState
  turnStateSource: TerminalTurnSignalSource | null
  turnStateUpdatedAt: number
  /** Grok promptId for rejecting delayed hook reports from an older turn. */
  turnSignalId?: string | null
  lastOutputAt: number
  pendingAnsiBytes: Uint8Array | null
  lastSessionActivityAt: number
  /** 兼容旧调用点；语义等同于 processState 的旧命名。 */
  status: 'spawning' | 'running' | 'exited' | 'error'
  errorMessage?: string
  exitCode?: number
  /** true = 纯 shell tab（不跑 agent CLI），不需要 turn watch 等 agent 逻辑。 */
  isShell?: boolean
  /** 用户手动重命名过 —— reconcile 时保留此标题，不被 session 标题覆盖。 */
  userRenamed?: boolean
}

export const tabs = ref<TerminalTab[]>([])
// activeUiId 现在是「聚焦 pane 的 activeUiId」投影（真身在 panes.ts）。从这里 re-export
// 以保持 TerminalStrip / App.vue 既有 import 路径不变。
export { activeUiId }
let nextUiId = 1

// ============================ 持久化（懒恢复） ============================
// 关窗 / 隐藏时把所有活跃 tab 的元数据存进 localStorage；重启后以 "saved"
// 状态恢复到 strip 上（仅画 pill，不创建 xterm / PTY）。用户点击时才水合。

const SAVED_TABS_KEY = 'savedTabs:v1'
const SAVED_NAV_KEY = 'savedNav:v1'
const SAVED_VIEWS_KEY = 'savedViews:v1'
const SAVED_ACTIVE_TUI_KEY = 'savedActiveTui:v1'

export interface SavedTab {
  agent: Agent
  projectKey: string
  sessionId: string
  sessionPath: string
  title: string
  cwd: string
  createdAt?: number
  isShell?: boolean
  userRenamed?: boolean
  /** 上次退出时所属分屏格子 id。恢复时若该 pane 不在了就兜底到主 pane（见 TerminalStrip）。 */
  paneId?: number
}

export interface SavedNav {
  agent: Agent
  activeDir: string | null
  /** 退出时如果在终端 tab 上，记录它的 sessionPath 以便重启时自动水合 */
  activeSessionPath: string | null
  /** 退出时的视图状态：'list' | 'tui' | 'view'（聊天详情）| 'welcome'（没选项目） */
  view: 'list' | 'tui' | 'view' | 'welcome'
  /** 退出时活跃 tab 没有 sessionPath（shell / 未匹配新会话），记录它在 savedTabs 中的索引 */
  activeSavedIndex?: number
}

// 每个项目最近打开的 View（会话详情 + read/chat 子模式）。和 savedNav 分开存：
// savedNav 只记「重启时停在哪个项目 / 哪个视图」，这里记「每个项目各自开着哪条 View」，
// 这样切到任意项目（含重启后第一次点）都能恢复它自己的 View tab，而不只是上次激活的那个。
export interface SavedView {
  agent: Agent
  dir: string
  session: SessionMeta
  mode: 'read' | 'chat'
}

function normalizeStoredAgent(agent: unknown): Agent | null {
  if (agent === 'kimi') return 'kimicode'
  return agent === 'claude' || agent === 'codex' || agent === 'agy' || agent === 'opencode' || agent === 'grok' || agent === 'kimicode' || agent === 'pi'
    ? agent
    : null
}

export function loadSavedViews(): SavedView[] {
  try {
    const raw = localStorage.getItem(SAVED_VIEWS_KEY)
    if (!raw) return []
    const arr = JSON.parse(raw)
    if (!Array.isArray(arr)) return []
    return arr.flatMap((v: any): SavedView[] => {
      const agent = normalizeStoredAgent(v?.agent)
      return agent && v.dir && v.session?.path ? [{ ...v, agent } as SavedView] : []
    })
  } catch {
    return []
  }
}

export function persistViews(views: SavedView[]) {
  try {
    localStorage.setItem(SAVED_VIEWS_KEY, JSON.stringify(views))
  } catch {
    /* 配额满 / 隐私模式：丢了也只是少恢复 View，无所谓 */
  }
}

export interface SavedActiveTui {
  agent: Agent
  dir: string
  sessionPath: string
  isShell?: boolean
}

export function loadSavedActiveTui(): SavedActiveTui[] {
  try {
    const raw = localStorage.getItem(SAVED_ACTIVE_TUI_KEY)
    if (!raw) return []
    const arr = JSON.parse(raw)
    if (!Array.isArray(arr)) return []
    return arr.flatMap((v: any): SavedActiveTui[] => {
      const agent = normalizeStoredAgent(v?.agent)
      return agent && v.dir ? [{ ...v, agent } as SavedActiveTui] : []
    })
  } catch {
    return []
  }
}

export function persistActiveTui(entries: SavedActiveTui[]) {
  try {
    localStorage.setItem(SAVED_ACTIVE_TUI_KEY, JSON.stringify(entries))
  } catch { /* ignore */ }
}

export const savedTabs = ref<SavedTab[]>(loadSavedTabs())

function loadSavedTabs(): SavedTab[] {
  try {
    const raw = localStorage.getItem(SAVED_TABS_KEY)
    if (!raw) return []
    const arr = JSON.parse(raw)
    if (!Array.isArray(arr)) return []
    const filtered = arr.flatMap((t: any): SavedTab[] => {
      const agent = normalizeStoredAgent(t?.agent)
      return agent && t.cwd ? [{ ...t, agent } as SavedTab] : []
    })
    for (let i = 0; i < filtered.length; i++) {
      if (!filtered[i].createdAt) filtered[i].createdAt = i + 1
    }
    return filtered
  } catch {
    return []
  }
}

export function loadSavedNav(): SavedNav | null {
  try {
    const raw = localStorage.getItem(SAVED_NAV_KEY)
    if (!raw) return null
    const v = JSON.parse(raw)
    const agent = normalizeStoredAgent(v?.agent)
    return agent ? { ...v, agent } as SavedNav : null
  } catch {
    return null
  }
}

export function persistTabState(nav: SavedNav) {
  const live: SavedTab[] = tabs.value
    .filter((t) => t.isShell || (t.sessionId && t.sessionPath) || isTabProcessAlive(t))
    .map((t) => ({
      agent: t.agent,
      projectKey: t.projectKey,
      sessionId: t.sessionId,
      sessionPath: t.sessionPath,
      title: t.title,
      cwd: t.cwd,
      createdAt: t.createdAt,
      paneId: t.paneId,
      ...(t.isShell ? { isShell: true } : {}),
      ...(t.userRenamed ? { userRenamed: true } : {}),
    }))
  // 合并：live tabs + 还没被水合的 saved tabs（避免切项目后丢掉未点过的恢复 tab）
  // 有 sessionPath 的按 path 去重；没有 sessionPath 的（shell / 未匹配新会话）
  // 需要检查是否已经在 live 中（按引用比较 savedTabs 条目是否仍存在）。
  const livePaths = new Set(live.filter((t) => t.sessionPath).map((t) => t.sessionPath))
  // 有 path 的按 path 去重；无 path 的（shell / 未匹配新会话）保留在 kept，
  // 水合过的已由 removeSavedTab 从 savedTabs 移除，不会重复。
  const kept = savedTabs.value.filter((s) =>
    s.sessionPath ? !livePaths.has(s.sessionPath) : true,
  )
  const all = [...live, ...kept]
  localStorage.setItem(SAVED_TABS_KEY, JSON.stringify(all))
  localStorage.setItem(SAVED_NAV_KEY, JSON.stringify(nav))
}

export function removeSavedTab(target: string | SavedTab) {
  if (typeof target === 'string') {
    savedTabs.value = savedTabs.value.filter((t) => t.sessionPath !== target)
  } else {
    const idx = savedTabs.value.indexOf(target)
    if (idx >= 0) savedTabs.value.splice(idx, 1)
  }
}

// 改 saved tab 的标题。和 removeSavedTab 同构：有 sessionPath 按它匹配，
// shell saved tab（无 path）按引用匹配。标记 userRenamed，水合后不被 session 标题覆盖。
export function renameSavedTab(target: string | SavedTab, title: string) {
  const apply = (t: SavedTab): SavedTab => ({ ...t, title, userRenamed: true })
  savedTabs.value = savedTabs.value.map((t) =>
    (typeof target === 'string' ? t.sessionPath === target : t === target)
      ? apply(t)
      : t,
  )
}

export function clearSavedTabs() {
  savedTabs.value = []
  localStorage.removeItem(SAVED_TABS_KEY)
}

export function clearAllTabs() {
  const ids = tabs.value.map((t) => t.uiId)
  for (const id of ids) closeTab(id)
  savedTabs.value = []
  localStorage.removeItem(SAVED_TABS_KEY)
  localStorage.removeItem(SAVED_NAV_KEY)
  localStorage.removeItem(SAVED_VIEWS_KEY)
  localStorage.removeItem(SAVED_ACTIVE_TUI_KEY)
  activeUiId.value = null
}

// ============================ 主题 ============================

function xtermTheme(isDark: boolean) {
  // xterm 的画布默认会用 theme.background 填满。启用自定义壁纸时改成
  // 透明，让终端 / Codex TUI 的空白区域能显示图片或视频；ANSI 显式设置
  // 背景色的内容仍维持原样，保证交互界面的信息层级。
  const hasCustomBackground = Boolean(backgroundImagePath.value)
  const background = hasCustomBackground ? 'rgba(0, 0, 0, 0)' : undefined
  if (theme.value === 'dracula') {
    return {
      background: background ?? '#282a36',
      foreground: '#f8f8f2',
      cursor: '#f8f8f2',
      cursorAccent: '#282a36',
      selectionBackground: 'rgba(255,255,255,0.18)',
      black: '#21222c',
      red: '#ff5555',
      green: '#50fa7b',
      yellow: '#f1fa8c',
      blue: '#bd93f9',
      magenta: '#ff79c6',
      cyan: '#8be9fd',
      white: '#f8f8f2',
      brightBlack: '#6272a4',
      brightRed: '#ff6e6e',
      brightGreen: '#69ff94',
      brightYellow: '#ffffa5',
      brightBlue: '#d6acff',
      brightMagenta: '#ff92df',
      brightCyan: '#a4ffff',
      brightWhite: '#ffffff',
    }
  }
  return isDark
    ? {
        background: background ?? '#0a0a0a',
        foreground: '#ededed',
        cursor: '#ededed',
        cursorAccent: '#0a0a0a',
        selectionBackground: 'rgba(255,255,255,0.18)',
        black: '#1f1f1f',
        red: '#ef4444',
        green: '#10b981',
        yellow: '#eab308',
        blue: '#4d8bf8',
        magenta: '#a855f7',
        cyan: '#06b6d4',
        white: '#e5e5e5',
        brightBlack: '#525252',
        brightRed: '#f87171',
        brightGreen: '#34d399',
        brightYellow: '#facc15',
        brightBlue: '#60a5fa',
        brightMagenta: '#c084fc',
        brightCyan: '#22d3ee',
        brightWhite: '#fafafa',
      }
    : {
        background: background ?? '#ffffff',
        foreground: '#171717',
        cursor: '#171717',
        cursorAccent: '#ffffff',
        // xterm 会先把选区和 theme.background 混合。壁纸模式的透明背景会让
        // 原本的黑色选区预混为纯黑，压住黑字；改成浅灰，让每次 xterm 重绘后
        // 写入的内联选区色仍清晰可读。
        selectionBackground: hasCustomBackground ? 'rgba(255,255,255,0.70)' : 'rgba(0,0,0,0.12)',
        black: '#171717',
        red: '#b91c1c',
        green: '#047857',
        yellow: '#a16207',
        blue: '#1d4ed8',
        magenta: '#7c3aed',
        cyan: '#0e7490',
        white: '#404040',
        brightBlack: '#525252',
        brightRed: '#dc2626',
        brightGreen: '#059669',
        brightYellow: '#ca8a04',
        brightBlue: '#2563eb',
        brightMagenta: '#9333ea',
        brightCyan: '#0891b2',
        brightWhite: '#111111',
      }
}

function systemDarkActive(): boolean {
  return window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false
}

function isDarkActive(): boolean {
  return theme.value === 'dark' || theme.value === 'dracula' || (theme.value === 'system' && systemDarkActive())
}

function terminalColorScheme(): 'light' | 'dark' {
  return isDarkActive() ? 'dark' : 'light'
}

const XTERM_COLOR_MODE_MASK = 0x3000000
const XTERM_COLOR_VALUE_MASK = 0xffffff
const XTERM_COLOR_MODE_P16 = 0x1000000
const XTERM_COLOR_MODE_P256 = 0x2000000
const XTERM_COLOR_MODE_RGB = 0x3000000
const XTERM_FG_INVERSE = 0x4000000

type XtermMutableCell = {
  fg: number
  bg: number
  getChars?: () => string
  getBgColorMode?: () => number
  getBgColor?: () => number
  getFgColorMode?: () => number
  getFgColor?: () => number
}

type XtermMutableBufferLine = {
  length: number
  isWrapped?: boolean
  translateToString?: (trimRight?: boolean, startColumn?: number, endColumn?: number) => string
  loadCell: (index: number, cell: XtermMutableCell) => XtermMutableCell
  setCell: (index: number, cell: XtermMutableCell) => void
}

function rgbFromPacked(value: number): [number, number, number] {
  return [(value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff]
}

function rgbToPacked(r: number, g: number, b: number): number {
  return ((r & 0xff) << 16) | ((g & 0xff) << 8) | (b & 0xff)
}

function isExtremeNeutralRgb(value: number): boolean {
  const [r, g, b] = rgbFromPacked(value)
  if (Math.max(r, g, b) - Math.min(r, g, b) > 3) return false
  return r >= 220 || r <= 48
}

function isNeutralPaletteIndex(value: number): boolean {
  return value === 0 || value === 7 || value === 8 || value === 15 || (value >= 232 && value <= 255)
}

function isNeutralForegroundCell(cell: XtermMutableCell): boolean {
  const mode = cell.getFgColorMode?.() ?? (cell.fg & XTERM_COLOR_MODE_MASK)
  const color = cell.getFgColor?.() ?? (cell.fg & XTERM_COLOR_VALUE_MASK)
  if (cell.fg & XTERM_FG_INVERSE) return true
  if (mode === 0) return true
  if (mode === XTERM_COLOR_MODE_RGB) return isExtremeNeutralRgb(color)
  if (mode === XTERM_COLOR_MODE_P16 || mode === XTERM_COLOR_MODE_P256) return isNeutralPaletteIndex(color)
  return false
}

function codexUserMessageThemeColors(): { bg: number; fg: number } {
  if (theme.value === 'dracula') {
    return { bg: rgbToPacked(0x44, 0x47, 0x5a), fg: rgbToPacked(0xf8, 0xf8, 0xf2) }
  }
  if (isDarkActive()) {
    return { bg: rgbToPacked(0x1f, 0x1f, 0x1f), fg: rgbToPacked(0xed, 0xed, 0xed) }
  }
  return { bg: rgbToPacked(0xf4, 0xf4, 0xf4), fg: rgbToPacked(0x17, 0x17, 0x17) }
}

function setCellCodexUserMessageColors(cell: XtermMutableCell, colors: { bg: number; fg: number }) {
  const hasText = (cell.getChars?.() ?? '') !== ''
  const shouldSetFg = hasText && isNeutralForegroundCell(cell)
  cell.fg &= ~XTERM_FG_INVERSE
  cell.bg = (cell.bg & ~(XTERM_COLOR_MODE_MASK | XTERM_COLOR_VALUE_MASK)) | XTERM_COLOR_MODE_RGB | colors.bg
  if (shouldSetFg) {
    cell.fg = (cell.fg & ~(XTERM_COLOR_MODE_MASK | XTERM_COLOR_VALUE_MASK)) | XTERM_COLOR_MODE_RGB | colors.fg
  }
}

function activeMutableXtermLines(term: Terminal): { length: number; get: (index: number) => XtermMutableBufferLine | undefined } | null {
  const core = (term as unknown as { _core?: unknown })._core as
    | {
        buffers?: { active?: { lines?: { length: number; get: (index: number) => XtermMutableBufferLine | undefined } } }
        _bufferService?: { buffer?: { lines?: { length: number; get: (index: number) => XtermMutableBufferLine | undefined } } }
      }
    | undefined
  return core?.buffers?.active?.lines ?? core?._bufferService?.buffer?.lines ?? null
}

function repairCodexUserMessageBufferColors(tab: TerminalTab): boolean {
  if (tab.agent !== 'codex' || tab.isShell) return false
  const lines = activeMutableXtermLines(tab.term)
  if (!lines) return false

  const cell = tab.term.buffer.active.getNullCell() as unknown as XtermMutableCell
  const targetRows = new Set<number>()
  let changed = false

  for (let y = 0; y < lines.length; y++) {
    const line = lines.get(y)
    if (!line) continue

    const text = line.translateToString?.(true) ?? ''
    const isUserMessageStart: boolean = text.trimStart().startsWith('›')
    if (!isUserMessageStart) continue

    let start = y
    const previousLine = lines.get(y - 1)
    const previousText = previousLine?.translateToString?.(true) ?? ''
    if (previousLine && previousText.trim() === '') {
      start = y - 1
    }

    let end = y
    while (end + 1 < lines.length && lines.get(end + 1)?.isWrapped) end++
    // Alt+Enter / Shift+Enter 多行：续行不是 wrapped，而是缩进（对齐在 `› ` 之后的两个空格）
    // 的内容行，中间还可能夹着用户敲的**空行**。逐段吞：跳过中间连续空行后，若后面还有
    // composer 续行，就把这些空行一并并入并继续；直到空行后接的是非 composer 内容（footer，
    // 通常顶格）才停。否则遇到第一个内部空行就 break 会把灰底截断 —— 空行及其之后全露白
    // （用户反馈的"空行背景断层"）。
    for (;;) {
      let probe = end + 1
      while (probe < lines.length && (lines.get(probe)?.translateToString?.(true) ?? '').trim() === '') probe++
      if (probe >= lines.length) break
      const probeText = lines.get(probe)?.translateToString?.(true) ?? ''
      // footer（`模型 · 路径`，带 ` · ` 中点分隔）是 composer 下方的**边界**：它同样缩进，
      // 光靠"缩进"分不清，必须显式识别并在此停住，否则会把 footer 也吞进灰底（用户反馈的
      // "太粗暴：footer 也被涵盖"）。
      const isFooter = probeText.includes(' · ')
      const isComposerCont =
        !isFooter &&
        (!!lines.get(probe)?.isWrapped ||
          (probeText.startsWith('  ') && !probeText.trimStart().startsWith('›')))
      if (!isComposerCont) break
      end = probe
      while (end + 1 < lines.length && lines.get(end + 1)?.isWrapped) end++
    }
    const contentEnd = end
    const nextLine = lines.get(contentEnd + 1)
    const nextText = nextLine?.translateToString?.(true) ?? ''
    if (nextLine && nextText.trim() === '') {
      end = contentEnd + 1
    }

    for (let target = start; target <= end; target++) targetRows.add(target)
  }

  if (targetRows.size === 0) return false

  const colors = codexUserMessageThemeColors()
  for (const y of targetRows) {
    const line = lines.get(y)
    if (!line) continue
    for (let x = 0; x < line.length; x++) {
      line.loadCell(x, cell)
      setCellCodexUserMessageColors(cell, colors)
      line.setCell(x, cell)
      changed = true
    }
  }

  return changed
}

function syncRepairCodexUserMessageColors(tab: TerminalTab) {
  if (tab.agent !== 'codex' || tab.isShell) return
  repairCodexUserMessageBufferColors(tab)
}

function codexComposerEndRow(tab: TerminalTab, promptRow: number): number {
  const buffer = tab.term.buffer.active
  let endRow = promptRow

  for (let row = promptRow + 1; row < tab.term.rows; row++) {
    const line = buffer.getLine(buffer.viewportY + row)
    if (!line) break
    const text = line.translateToString(true)
    if (text.trim() === '') continue
    if (text.includes(' · ')) break
    const continuation = line.isWrapped
      || (text.startsWith('  ') && !text.trimStart().startsWith('›'))
    if (!continuation) break
    endRow = row
  }

  return endRow
}

export function captureStableTerminalCursor(tab: TerminalTab, followInput = false) {
  const buffer = tab.term.buffer.active
  let cursorX = buffer.cursorX
  let cursorY = buffer.cursorY

  // Codex 提交后会清空 composer。优先固定到屏幕最下方的 `› ` 输入提示，避免把
  // 静态光标留在刚提交文本的末尾；找不到时才使用 xterm 当前光标坐标。
  for (let row = Math.max(0, tab.term.rows - 1); row >= 0; row--) {
    const text = buffer.getLine(buffer.viewportY + row)?.translateToString(true) ?? ''
    const trimmed = text.trimStart()
    if (!trimmed.startsWith('›')) continue
    const promptEnd = text.length - trimmed.length + 2
    const composerEndRow = codexComposerEndRow(tab, row)
    const cursorInComposer = buffer.cursorY >= row
      && buffer.cursorY <= composerEndRow
      && (buffer.cursorY !== row || buffer.cursorX >= promptEnd)

    if (followInput && cursorInComposer) {
      cursorX = buffer.cursorX
      cursorY = buffer.cursorY
    } else if (
      followInput
      && tab.stableCursorX !== undefined
      && tab.stableCursorRowFromBottom !== undefined
    ) {
      // 状态刷新会临时把真实光标挪出 composer；此时横纵坐标都保持上一次输入位置。
      cursorX = tab.stableCursorX
      cursorY = tab.term.rows - 1 - tab.stableCursorRowFromBottom
    } else {
      cursorX = promptEnd
      cursorY = row
    }
    break
  }

  tab.stableCursorX = Math.max(0, Math.min(tab.term.cols - 1, cursorX))
  const clampedCursorY = Math.max(0, Math.min(tab.term.rows - 1, cursorY))
  tab.stableCursorRowFromBottom = tab.term.rows - 1 - clampedCursorY
}

function syncStableTerminalCursorGeometry(tab: TerminalTab) {
  if (tab.stableCursorX === undefined || tab.stableCursorRowFromBottom === undefined) return
  const screen = tab.term.element?.querySelector<HTMLElement>('.xterm-screen')
  if (!screen || tab.term.cols <= 0 || tab.term.rows <= 0) return

  const cellWidth = screen.clientWidth / tab.term.cols
  const cellHeight = screen.clientHeight / tab.term.rows
  if (cellWidth <= 0 || cellHeight <= 0) return

  const row = Math.max(0, tab.term.rows - 1 - tab.stableCursorRowFromBottom)
  tab.container.style.setProperty('--terminal-stable-cursor-left', `${tab.stableCursorX * cellWidth}px`)
  tab.container.style.setProperty('--terminal-stable-cursor-top', `${row * cellHeight}px`)
  tab.container.style.setProperty('--terminal-stable-cursor-width', `${cellWidth}px`)
  tab.container.style.setProperty('--terminal-stable-cursor-height', `${cellHeight}px`)
}

function suppressRenderedTerminalCursor(tab: TerminalTab) {
  // xterm 6 的 cursor 颜色规则带动态 terminal id，CSS specificity 高于应用样式。
  // 渲染完成后同步移除 cursor 专用 class，保留该 cell 自身的文字/颜色 class。
  for (const cursor of tab.container.querySelectorAll<HTMLElement>('.xterm-cursor')) {
    cursor.classList.remove(
      'xterm-cursor',
      'xterm-cursor-blink',
      'xterm-cursor-block',
      'xterm-cursor-bar',
      'xterm-cursor-underline',
      'xterm-cursor-outline',
    )
  }
}

function syncStableTerminalCursor(tab: TerminalTab) {
  const active = shouldUseStableTerminalCursor(tab.agent, tab.turnState, !!tab.isShell)
  const wasActive = tab.container.classList.contains('terminal-stable-cursor')

  if (active) {
    if (tab.stableCursorX === undefined || tab.stableCursorRowFromBottom === undefined) {
      captureStableTerminalCursor(tab)
    }
    tab.container.classList.add('terminal-stable-cursor')
    suppressRenderedTerminalCursor(tab)
    if (!wasActive) applyTerminalTheme(tab)
    syncStableTerminalCursorGeometry(tab)
    return
  }

  tab.container.classList.remove('terminal-stable-cursor')
  tab.stableCursorX = undefined
  tab.stableCursorRowFromBottom = undefined
  if (wasActive) applyTerminalTheme(tab)
}

function applyTerminalTheme(tab: TerminalTab) {
  const dark = isDarkActive()
  const newTheme = xtermTheme(dark)
  if (tab.agent === 'codex' && !tab.isShell) tab.container.classList.add('terminal-codex-tui')
  if (shouldUseStableTerminalCursor(tab.agent, tab.turnState, !!tab.isShell)) {
    tab.container.style.setProperty('--terminal-stable-cursor-color', newTheme.cursor)
  }
  tab.term.options.theme = newTheme
  const repaired = repairCodexUserMessageBufferColors(tab)
  if (repaired) tab.term.clearTextureAtlas()
  tab.term.refresh(0, Math.max(0, tab.term.rows - 1))
}

export function refreshAllTerminalThemes() {
  for (const tab of tabs.value) {
    applyTerminalTheme(tab)
  }
}

watch([theme, backgroundImagePath], refreshAllTerminalThemes)
watch(
  () => tabs.value.map((tab) => `${tab.uiId}:${tab.turnState}`).join('|'),
  () => {
    for (const tab of tabs.value) syncStableTerminalCursor(tab)
  },
  { flush: 'sync' },
)
window.matchMedia?.('(prefers-color-scheme: dark)').addEventListener('change', () => {
  if (theme.value === 'system') refreshAllTerminalThemes()
})

// ============================ base64 双向 ============================
// btoa / atob 对多字节字符不友好，统一走 Uint8Array 转换 + 分块避免栈溢出。

function bytesToBase64(bytes: Uint8Array): string {
  let bin = ''
  const CHUNK = 0x8000
  for (let i = 0; i < bytes.length; i += CHUNK) {
    const sub = bytes.subarray(i, i + CHUNK)
    bin += String.fromCharCode.apply(null, sub as unknown as number[])
  }
  return btoa(bin)
}

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

function asciiFromBytes(bytes: Uint8Array, start: number, end: number): string {
  let out = ''
  for (let i = start; i < end; i++) out += String.fromCharCode(bytes[i])
  return out
}

type ColorScheme = 'light' | 'dark'
type Rgb = [number, number, number]

function isDarkAnsiColor(value: string): boolean {
  const n = Number(value)
  if (!Number.isInteger(n)) return false
  return n === 0 || n === 8 || (n >= 232 && n <= 244)
}

function isDarkRgb(r: string, g: string, b: string): boolean {
  const rv = Number(r)
  const gv = Number(g)
  const bv = Number(b)
  if (![rv, gv, bv].every((v) => Number.isInteger(v) && v >= 0 && v <= 255)) return false
  return rv * 0.299 + gv * 0.587 + bv * 0.114 < 96
}

function isLightAnsiColor(value: string): boolean {
  const n = Number(value)
  if (!Number.isInteger(n)) return false
  return n === 7 || n === 15 || (n >= 245 && n <= 255)
}

function isLightRgb(r: string, g: string, b: string): boolean {
  const rv = Number(r)
  const gv = Number(g)
  const bv = Number(b)
  if (![rv, gv, bv].every((v) => Number.isInteger(v) && v >= 0 && v <= 255)) return false
  return rv * 0.299 + gv * 0.587 + bv * 0.114 > 180
}

const XTERM_CUBE_STEPS = [0, 95, 135, 175, 215, 255]

function toByte(value: string): number | null {
  if (!/^\d{1,3}$/.test(value)) return null
  const n = Number(value)
  return n <= 255 ? n : null
}

// 只解析 16-255；0-15 由 xterm 主题调色板决定实际颜色，这里看不到也不该猜。
function ansi256ToRgb(value: string): Rgb | null {
  const n = toByte(value)
  if (n === null || n < 16) return null
  if (n >= 232) {
    const gray = 8 + (n - 232) * 10
    return [gray, gray, gray]
  }
  const i = n - 16
  return [
    XTERM_CUBE_STEPS[Math.floor(i / 36)],
    XTERM_CUBE_STEPS[Math.floor(i / 6) % 6],
    XTERM_CUBE_STEPS[i % 6],
  ]
}

function parseRgb(r: string, g: string, b: string): Rgb | null {
  const rgb = [toByte(r), toByte(g), toByte(b)]
  return rgb.every((v): v is number => v !== null) ? (rgb as Rgb) : null
}

function rgbToHsl([r, g, b]: Rgb): [number, number, number] {
  const rn = r / 255
  const gn = g / 255
  const bn = b / 255
  const max = Math.max(rn, gn, bn)
  const min = Math.min(rn, gn, bn)
  const l = (max + min) / 2
  const d = max - min
  if (d === 0) return [0, 0, l]
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
  const h =
    max === rn
      ? ((gn - bn) / d + (gn < bn ? 6 : 0)) / 6
      : max === gn
        ? ((bn - rn) / d + 2) / 6
        : ((rn - gn) / d + 4) / 6
  return [h, s, l]
}

function hslToRgb([h, s, l]: [number, number, number]): Rgb {
  if (s === 0) {
    const v = Math.round(l * 255)
    return [v, v, v]
  }
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s
  const p = 2 * l - q
  const channel = (t: number): number => {
    if (t < 0) t += 1
    if (t > 1) t -= 1
    if (t < 1 / 6) return p + (q - p) * 6 * t
    if (t < 1 / 2) return q
    if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6
    return p
  }
  return [
    Math.round(channel(h + 1 / 3) * 255),
    Math.round(channel(h) * 255),
    Math.round(channel(h - 1 / 3) * 255),
  ]
}

  // codex 的前景永远是一整套「深色主题」调色板 —— 它在 Windows 上既不发 OSC 11 查背景色
// （codex-cli 0.144.4 实测不查，见 src-tauri/examples/codex_color_probe.rs），也不认
// COLORFGBG，只能假定背景是深的。实测它只用这些前景：
//   正文 #cccccc / #bbbbbb、次要 #909090、暗与分隔线 #5a5a5a #2f2f2f #1f1f1f、
//   accent #abdfa7(浅绿) #f6e2b7(奶油)，外加一个走调色板的 38;5;6。
//
// 所以浅色主题要做的是「把这套深色主题翻成它的浅色孪生版」：HSL 里镜像亮度 (L → 1-L)，
// 色相和饱和度原样保留。整个明暗阶梯被完整翻转而不是压平：
//   · #cccccc(正文) → #333333，#1f1f1f(分隔线) → #e0e0e0 —— 该重的仍重、该轻的仍轻；
//   · #abdfa7 → #245820、#f6e2b7 → #483409 —— 只换明暗，不换颜色。
// 别改成「只把不可读的颜色夹到某个亮度」：那会让深色分隔线原样留成白底上的死黑粗线，
// 且把亮度不同的 accent 全夹到同一档，颜色互相糊成一片。
//
// 深色主题下前景一个都不用动：codex 的假设和真实背景一致，本来就是对的。
function mirrorLightness(rgb: Rgb): Rgb | null {
  const [h, s, l] = rgbToHsl(rgb)
  const mirrored = hslToRgb([h, s, 1 - l])
  return mirrored.every((c, i) => c === rgb[i]) ? null : mirrored
}

// 浅色主题抹掉深底，深色主题抹掉浅底。codex 只画一种背景（#292929），且从不用浅底配深字，
// 所以镜像前景不会把字弄没。
const BG16_TO_DEFAULT: Record<ColorScheme, string[]> = {
  light: ['40', '47', '100', '107'],
  dark: ['47', '107'],
}

function rewriteExtColor(
  params: string[],
  scheme: ColorScheme,
  mirrorFg: boolean,
  stripBackground: boolean,
): string[] {
  const [kind, mode, ...rest] = params
  if (kind === '48') {
    if (stripBackground) return ['49']
    const unreadable =
      mode === '5'
        ? scheme === 'light'
          ? isDarkAnsiColor(rest[0])
          : isLightAnsiColor(rest[0])
        : scheme === 'light'
          ? isDarkRgb(rest[0], rest[1], rest[2])
          : isLightRgb(rest[0], rest[1], rest[2])
    return unreadable ? ['49'] : params
  }
  // 前景 16 色（30-37/90-97、38;5;0-15）交给 xterm 主题调色板 —— 那里已按主题挑过可读值。
  if (!mirrorFg) return params
  const rgb = mode === '5' ? ansi256ToRgb(rest[0]) : parseRgb(rest[0], rest[1], rest[2])
  const mirrored = rgb && mirrorLightness(rgb)
  return mirrored ? ['38', '2', ...mirrored.map(String)] : params
}

function readExtColor(parts: string[], i: number): { params: string[]; next: number } | null {
  const len = parts[i + 1] === '5' ? 3 : parts[i + 1] === '2' ? 5 : 0
  if (len === 0 || i + len > parts.length) return null
  return { params: parts.slice(i, i + len), next: i + len }
}

function normalizeSgrSemicolon(
  params: string,
  scheme: ColorScheme,
  mirrorFg: boolean,
  stripBackground: boolean,
): string {
  const parts = params === '' ? ['0'] : params.split(';')
  const out: string[] = []

  for (let i = 0; i < parts.length; i++) {
    const part = parts[i] === '' ? '0' : parts[i]

    if (part === '7') {
      out.push(part)
      continue
    }
    // 扩展色必须整段消费：逐参数匹配会误伤自己的参数（`38;2;40;…` 里的 40 被当成黑底改写）。
    if (part === '38' || part === '48') {
      const ext = readExtColor(parts, i)
      if (ext) {
        out.push(...rewriteExtColor(ext.params, scheme, mirrorFg, stripBackground))
        i = ext.next - 1
        continue
      }
    }
    const code = Number(part)
    const isBackgroundCode =
      Number.isInteger(code) && ((code >= 40 && code <= 47) || (code >= 100 && code <= 107))
    if (stripBackground ? isBackgroundCode : BG16_TO_DEFAULT[scheme].includes(part)) {
      out.push('49')
      continue
    }

    out.push(part)
  }

  return out.join(';')
}

function normalizeSgrColon(
  params: string,
  scheme: ColorScheme,
  mirrorFg: boolean,
  stripBackground: boolean,
): string {
  return params.replace(
    /(^|;)(38|48):(?:5:(\d+)|2:(\d+):(\d+):(\d+))(?=;|$)/g,
    (match, sep: string, kind: string, idx?: string, r?: string, g?: string, b?: string) => {
      const ext = idx === undefined ? [kind, '2', r!, g!, b!] : [kind, '5', idx]
      const rewritten = rewriteExtColor(ext, scheme, mirrorFg, stripBackground)
      return rewritten === ext ? match : `${sep}${rewritten.join(':')}`
    },
  )
}

/// 造一个 codex PTY 字节流的 SGR 归一化器。
///
/// `codexPaintsDarkPalette` 表示「codex 认不出真实背景，正按深色主题出色」—— 只有这时
/// 才该镜像前景。这个前提是**平台相关**的，不能想当然全开：
///   · Windows：实测成立（codex-cli 0.144.4 不发 OSC 11、不认 COLORFGBG，
///     见 src-tauri/examples/codex_color_probe.rs）→ 传 true。
///   · mac/Linux：未验证。codex 二进制里带着 `ESC]11;?`，只是 Windows 上没发；若它在
///     这些平台能问出背景色，就会直接出浅色主题的色（深字），此时再镜像一次会把深字翻成
///     浅字、在白底上彻底看不见 —— 比原 bug 更糟。所以默认传 false 保持原样。
/// 想给 mac 打开前，先在 mac 上跑 codex_color_probe 确认它到底发的是哪套色。
///
/// 背景归一化和这个开关无关，两个平台一直都做（是历史行为）。
/// `stripBackground` 用于 CLI 用 ANSI 背景铺满整个 TUI 的情况；把所有显式背景
/// 恢复为终端默认背景，才能继承当前应用主题（或透出自定义壁纸）。
export function codexSgrNormalizer(
  scheme: ColorScheme,
  codexPaintsDarkPalette: boolean,
  stripBackground = false,
): (params: string) => string | null {
  const mirrorFg = scheme === 'light' && codexPaintsDarkPalette
  return (params: string) => {
    const normalized = normalizeSgrColon(
      normalizeSgrSemicolon(params, scheme, mirrorFg, stripBackground),
      scheme,
      mirrorFg,
      stripBackground,
    )
    return normalized === params ? null : normalized
  }
}

function findIncompleteCsiStart(bytes: Uint8Array): number {
  if (bytes.length > 0 && bytes[bytes.length - 1] === 0x1b) return bytes.length - 1
  for (let i = Math.max(0, bytes.length - 32); i < bytes.length - 1; i++) {
    if (bytes[i] !== 0x1b || bytes[i + 1] !== 0x5b) continue
    let complete = false
    for (let j = i + 2; j < bytes.length; j++) {
      if (bytes[j] >= 0x40 && bytes[j] <= 0x7e) {
        complete = true
        break
      }
    }
    if (!complete) return i
  }
  return -1
}

function concatBytes(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length + b.length)
  out.set(a)
  out.set(b, a.length)
  return out
}

function normalizeAnsiBackground(
  bytes: Uint8Array,
  pending: Uint8Array | null,
  sgrNormalizer: (params: string) => string | null,
): { bytes: Uint8Array; pending: Uint8Array | null } {
  bytes = pending ? concatBytes(pending, bytes) : bytes
  const incompleteStart = findIncompleteCsiStart(bytes)
  const source = incompleteStart >= 0 ? bytes.subarray(0, incompleteStart) : bytes
  const nextPending = incompleteStart >= 0 ? bytes.subarray(incompleteStart) : null
  let out: number[] | null = null
  let copiedUntil = 0

  for (let i = 0; i < source.length - 2; i++) {
    if (source[i] !== 0x1b || source[i + 1] !== 0x5b) continue

    let end = i + 2
    while (end < source.length && !(source[end] >= 0x40 && source[end] <= 0x7e)) end++
    if (end >= source.length) break
    if (source[end] !== 0x6d) {
      i = end
      continue
    }

    const normalized = sgrNormalizer(asciiFromBytes(source, i + 2, end))
    if (normalized === null) {
      i = end
      continue
    }

    if (out === null) out = []
    for (let j = copiedUntil; j < i; j++) out.push(source[j])
    out.push(0x1b, 0x5b)
    for (let j = 0; j < normalized.length; j++) out.push(normalized.charCodeAt(j))
    out.push(0x6d)
    copiedUntil = end + 1
    i = end
  }

  if (out === null) return { bytes: source, pending: nextPending }
  for (let i = copiedUntil; i < source.length; i++) out.push(source[i])
  return { bytes: new Uint8Array(out), pending: nextPending }
}

// ============================ 查询 ============================

const findTab = (uiId: number) => tabs.value.find((t) => t.uiId === uiId)
export const isTabProcessAlive = (tab: TerminalTab) =>
  tab.processState === 'spawning' || tab.processState === 'alive'
const findTabBySession = (path: string) =>
  path ? tabs.value.find((t) => t.sessionPath === path && !t.isShell && isTabProcessAlive(t)) : undefined

type SessionForTabSync = {
  path: string
  id: string
  modified: number
  title?: string
}

function applySessionToTab(tab: TerminalTab, session: SessionForTabSync) {
  tab.sessionPath = session.path
  tab.sessionId = session.id
  if (tab.userRenamed) {
    api.renameSession(tab.agent, session.path, tab.title).catch(() => {})
  } else if (session.title?.trim()) {
    tab.title = session.title
  }
  applyPendingTurnState(tab, activeUiId.value === tab.uiId)
}

/**
 * 创建 tab 时调用：快照当前已知 session paths，用于后续 reconcile 排除旧 session。
 */
const knownPathsAtTabCreation = new Map<number, Set<string>>()

export function snapshotKnownSessions(tabUiId: number, paths: string[]) {
  knownPathsAtTabCreation.set(tabUiId, new Set(paths))
}

/**
 * 新会话 tab 的 sessionPath/sessionId 在创建时都是空的（CLI 自己生成 id），
 * 等用户从 TUI 回到列表后，刷新出的 sessions 里会包含刚才创建的会话。
 * 此函数把空路径的 tab 与最新出现的 session 匹配上，后续 closeTabBySessionPath
 * 才能正确找到 tab 并关闭。
 *
 * 只匹配在 tab 创建之后新出现的 session（不在快照内），
 * 避免把一个正在运行的旧 session（mtime 持续更新）错误地绑到新 tab 上。
 */
export function reconcileNewTabs(
  projectKey: string,
  sessions: SessionForTabSync[],
  agent?: Agent,
) {
  const unmatched = tabs.value.filter(
    (t) =>
      !t.isShell &&
      t.sessionPath === '' &&
      t.projectKey === projectKey &&
      (!agent || t.agent === agent) &&
      isTabProcessAlive(t),
  )
  if (!unmatched.length) return

  const takenPaths = new Set(
    tabs.value.filter((t) => t.sessionPath !== '').map((t) => t.sessionPath),
  )

  for (const tab of unmatched) {
    const known = knownPathsAtTabCreation.get(tab.uiId)
    const available = sessions
      .filter((s) => !takenPaths.has(s.path) && (!known || !known.has(s.path)))
      .sort((a, b) => (b.modified ?? 0) - (a.modified ?? 0))

    const matchIdx = available.findIndex((s) => (s.modified ?? 0) >= tab.createdAt - 5000)
    const match = matchIdx >= 0 ? available[matchIdx] : undefined
    if (match) {
      takenPaths.add(match.path)
      knownPathsAtTabCreation.delete(tab.uiId)
      applySessionToTab(tab, match)
    }
  }
}

export function syncTabTitlesFromSessions(
  agent: Agent,
  projectKey: string,
  sessions: SessionForTabSync[],
) {
  const byPath = new Map(sessions.map((s) => [s.path, s]))
  for (const tab of tabs.value) {
    if (tab.agent !== agent || tab.projectKey !== projectKey || !tab.sessionPath) continue
    if (tab.userRenamed) continue
    const session = byPath.get(tab.sessionPath)
    if (session?.title?.trim() && tab.title !== session.title) {
      tab.title = session.title
    }
  }
}

export function setTabTitleByUiId(uiId: number, title: string) {
  const tab = tabs.value.find((t) => t.uiId === uiId)
  if (tab) {
    tab.title = title.trim()
    tab.userRenamed = true
  }
}

export function syncTabTitleBySessionPath(agent: Agent, sessionPath: string, title: string) {
  const trimmed = title.trim()
  if (!trimmed) return
  for (const tab of tabs.value) {
    if (tab.agent === agent && tab.sessionPath === sessionPath) {
      tab.title = trimmed
    }
  }
}

function isTerminalCancelInput(data: string) {
  return data === '\x1b' || data === '\x03'
}

/**
 * codex resume 在 TUI 引导阶段常因「配置加载失败 / model provider 缺失 / 会话被占用」直接退出，
 * codex 打进 xterm 的原始报错（如 `Model provider \`aixj_vip\` not found`）用户往往看不懂
 * ——尤其是那些在旧 CLI / VSCode 扩展里用过、后来 provider 改名/删除的历史会话。
 * 这里从最近的 PTY 输出里识别这类错误，返回一句可操作的本地化提示；否则 null（不打扰
 * 其它退出场景）。只在 codex tab、非 0 退出码时调用。
 */
export function codexResumeConfigHint(recentOutput: string): string | null {
  // 先剥掉 ANSI 转义，避免颜色码把 provider 名或关键字截断。
  const plain = recentOutput.replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, '')
  const providerMatch = plain.match(/Model provider\s*['"`]?([^'"`\n]+?)['"`]?\s*not found/i)
  if (providerMatch) {
    return t('tui.codexProviderMissing', { provider: providerMatch[1].trim() })
  }
  if (/failed to load configuration/i.test(plain)) {
    return t('tui.codexConfigError')
  }
  // 内嵌终端没有 Chat 那样的“重试”按钮，必须明确告诉用户释放外部占用后
  // 关闭当前终端 tab，再重新打开会话；否则用户只会看到 app-server 原始英文报错。
  if (/already has an active writer|active writer/i.test(plain)) {
    return t('tui.codexActiveWriter')
  }
  return null
}

function tabsBySession(agent: Agent, sessionPath: string, sessionId?: string) {
  if (!sessionPath && !sessionId) return []
  return tabs.value.filter(
    (tab) =>
      tab.agent === agent &&
      ((sessionPath && sessionPathsEqual(tab.sessionPath, sessionPath)) ||
        (!!sessionId && tab.sessionId === sessionId)) &&
      isTabProcessAlive(tab),
  )
}

export function markTabSessionActivity(agent: Agent, sessionPath: string) {
  const now = Date.now()
  for (const tab of tabsBySession(agent, sessionPath)) {
    tab.lastSessionActivityAt = now
    markSessionActivity(tab)
  }
}

export function markTabTurnStarted(
  agent: Agent,
  sessionPath: string,
  source: TerminalTurnSignalSource,
  signalId?: string,
  sessionId?: string,
) {
  const targets = tabsBySession(agent, sessionPath, sessionId)
  if (!targets.length) rememberPendingTurnState(agent, sessionPath, 'started', source, signalId)
  for (const tab of targets) {
    applyTurnSignal(tab, 'started', source, activeUiId.value === tab.uiId, signalId)
  }
}

export function markTabTurnCompleted(
  agent: Agent,
  sessionPath: string,
  source: TerminalTurnSignalSource,
  signalId?: string,
  sessionId?: string,
) {
  const targets = tabsBySession(agent, sessionPath, sessionId)
  if (!targets.length) rememberPendingTurnState(agent, sessionPath, 'completed', source, signalId)
  for (const tab of targets) {
    applyTurnSignal(tab, 'completed', source, activeUiId.value === tab.uiId, signalId)
  }
}

export function markTabTurnBlocked(
  agent: Agent,
  sessionPath: string,
  source: TerminalTurnSignalSource,
  signalId?: string,
  sessionId?: string,
) {
  const targets = tabsBySession(agent, sessionPath, sessionId)
  if (!targets.length) rememberPendingTurnState(agent, sessionPath, 'blocked', source, signalId)
  for (const tab of targets) {
    applyTurnSignal(tab, 'blocked', source, activeUiId.value === tab.uiId, signalId)
  }
}

export function markTabTurnFailed(
  agent: Agent,
  sessionPath: string,
  source: TerminalTurnSignalSource,
  signalId?: string,
  sessionId?: string,
) {
  const targets = tabsBySession(agent, sessionPath, sessionId)
  if (!targets.length) rememberPendingTurnState(agent, sessionPath, 'failed', source, signalId)
  for (const tab of targets) {
    applyTurnSignal(tab, 'failed', source, activeUiId.value === tab.uiId, signalId)
  }
}

export function markTabViewed(uiId: number) {
  const tab = findTab(uiId)
  if (tab?.turnState === 'review') {
    setTurnState(tab, 'idle', tab.turnStateSource ?? 'hook')
  }
}

export function activeTab(): TerminalTab | null {
  if (activeUiId.value === null) return null
  return findTab(activeUiId.value) ?? null
}

const embeddedCliPasteInFlight = new Set<number>()

/**
 * These CLIs recognize xterm's bracketed-paste image path and turn it into
 * their native image label. Other CLIs should receive a plain terminal input
 * so their own paste confirmation UI is not opened.
 */
export function shouldUseBracketedImagePaste(agent: Agent): boolean {
  return agent === 'claude'
    || agent === 'codex'
    || agent === 'grok'
    || agent === 'opencode'
}

function canPasteIntoEmbeddedCli(tab: TerminalTab): boolean {
  return (
    !tab.isShell
    && tab.ptyId !== null
    && tab.processState === 'alive'
    && tabs.value.some((candidate) => candidate.uiId === tab.uiId)
  )
}

/**
 * Start exactly one clipboard operation for an embedded CLI tab. Image paths
 * use the CLI-specific input channel so supported agents retain their native
 * image labels while the others avoid opening a paste confirmation UI.
 */
function startEmbeddedCliPaste(tab: TerminalTab, intent: 'image' | 'text' | 'unified'): boolean {
  if (!canPasteIntoEmbeddedCli(tab)) return false
  if (embeddedCliPasteInFlight.has(tab.uiId)) return true

  embeddedCliPasteInFlight.add(tab.uiId)
  void runEmbeddedCliPaste(
    intent,
    {
      paste: (value) => {
        if (canPasteIntoEmbeddedCli(tab)) tab.term.paste(value)
      },
      pasteImage: (value) => {
        if (!canPasteIntoEmbeddedCli(tab)) return
        if (shouldUseBracketedImagePaste(tab.agent)) tab.term.paste(value)
        else tab.term.input(value)
      },
    },
    {
      readImages: intent === 'text' || (_isMac && intent === 'image')
        ? undefined
        : readClipboardImages,
      readText: async () => {
        if (_isMac) {
          // Native access avoids WKWebView permission failures. If the command
          // is unavailable (for example while an older dev binary is running),
          // retain the browser clipboard as a compatibility fallback.
          try {
            const nativeText = await api.readMacosClipboardText()
            if (nativeText !== null && nativeText !== undefined) return nativeText
          } catch {
            // Fall through to navigator.clipboard below.
          }
        }
        return (await navigator.clipboard?.readText?.()) ?? ''
      },
    },
    {
      saveImage: api.saveClipboardImage,
      saveImagePath: _isMac && (intent === 'image' || intent === 'unified')
        ? api.saveMacosClipboardImage
        : undefined,
    },
  ).finally(() => {
    embeddedCliPasteInFlight.delete(tab.uiId)
  })
  return true
}

/**
 * Handles frontend Edit > Paste actions, including the Windows custom title
 * bar, when an embedded CLI is active.
 */
export function pasteIntoActiveTui(): boolean {
  const tab = activeTab()
  if (!tab) return false
  return startEmbeddedCliPaste(tab, 'unified')
}

// ============================ 开 / 关 / 切 ============================

export interface OpenTuiOptions {
  agent: Agent
  projectKey: string
  /** resume 模式必填；new-session 模式为空 —— 由 CLI 自己生成。 */
  sessionId: string
  /** resume 模式必填；new-session 模式为空 —— JSONL 还没存在。 */
  sessionPath: string
  title: string
  cwd: string
  /** new-session 模式：当前已知 session paths，用于 reconcile 排除旧 session。 */
  knownSessionPaths?: string[]
  /** 恢复持久化 tab 时保留原创建时间。 */
  createdAt?: number
  /** 用户手动重命名过 —— reconcile 时保留此标题。 */
  userRenamed?: boolean
}

/**
 * resume：同会话已有 tab 就 focus；否则新开一个 PTY + xterm 跑 `<cli> --resume <id>`。
 * new：每次都开新 tab，跑 `<cli>` (无 --resume)，CLI 自己生成新 session id；不挂 watcher
 * （没有 sessionPath 可监听）。失败时 tab.status = 'error' 但仍留在列表里。
 */
export async function openOrFocusTui(opts: OpenTuiOptions): Promise<void> {
  if (!opts.cwd) return
  const isNew = !opts.sessionId

  if (!isNew) {
    const existing = findTabBySession(opts.sessionPath)
    if (existing) {
      activateTabInPane(existing)
      return
    }
  }

  const term = markRaw(
    new Terminal({
      fontSize: 13,
      fontFamily:
        '"SF Mono", "Menlo", "Consolas", "Liberation Mono", "Courier New", monospace',
      cursorBlink: shouldBlinkTerminalCursor(opts.agent),
      convertEol: false,
      allowProposedApi: true,
      scrollback: 5000,
      theme: xtermTheme(isDarkActive()),
    }),
  )
  installTerminalScrollbackProtection(term, opts.agent, false)
  const fitAddon = markRaw(new FitAddon())
  term.loadAddon(fitAddon)

  const container = markRaw(document.createElement('div'))
  container.className =
    opts.agent === 'codex'
      ? 'terminal-host terminal-codex-tui'
      : opts.agent === 'grok'
        ? 'terminal-host terminal-grok-tui'
        : 'terminal-host'
  // 提示 xterm 即将 attach；真正的 open(container) 推迟到 slot 把 container 放入
  // 可见 DOM 树之后，否则在 detached 节点上 open 会拿不到尺寸。
  term.open(container)
  const uiId = nextUiId++
  if (isNew && opts.knownSessionPaths) {
    snapshotKnownSessions(uiId, opts.knownSessionPaths)
  }
  // ⚠️ 必须用 reactive() 包一层 —— 否则后面 `tab.status = 'running'` 改的是
  // raw 对象，Vue Proxy 收不到 set 通知，TerminalStrip 里 v-if="tab.status === 'spawning'"
  // 永远卡在转圈。markRaw 过的 term/fitAddon/container 会被 reactive() 跳过，不会被代理。
  const tab = reactive<TerminalTab>({
    uiId,
    ptyId: null,
    agent: opts.agent,
    projectKey: opts.projectKey,
    paneId: ensureLayout(opts.agent, opts.projectKey).focusedPaneId,
    sessionId: opts.sessionId,
    sessionPath: opts.sessionPath,
    title: opts.title,
    cwd: opts.cwd,
    createdAt: opts.createdAt ?? Date.now(),
    term,
    fitAddon,
    container,
    unlistenData: null,
    unlistenExit: null,
    onDataDisp: null,
    lastSyncedCols: 0,
    lastSyncedRows: 0,
    inputState: createTerminalInputState(),
    processState: 'spawning',
    turnState: 'unknown',
    turnStateSource: null,
    turnStateUpdatedAt: Date.now(),
    lastOutputAt: 0,
    lastSessionActivityAt: 0,
    pendingAnsiBytes: null,
    status: 'spawning',
  }) as TerminalTab
  if (opts.userRenamed) tab.userRenamed = true
  tabs.value.push(tab)
  activateTabInPane(tab)
  if (tab.agent === 'codex' && !tab.isShell) tab.container.classList.add('terminal-codex-tui')
  term.onWriteParsed(() => {
    if (!shouldUseStableTerminalCursor(tab.agent, tab.turnState, !!tab.isShell)) return
    captureStableTerminalCursor(tab, true)
    syncStableTerminalCursorGeometry(tab)
  })
  term.onRender(() => {
    if (shouldUseStableTerminalCursor(tab.agent, tab.turnState, !!tab.isShell)) {
      suppressRenderedTerminalCursor(tab)
    }
  })
  const imeInputDeduper = createTerminalImeInputDeduper()
  if (_isMac) {
    // xterm maps Ctrl+V to ^V before the browser paste event. Capture both
    // macOS shortcuts before xterm so neither path can invoke its native paste.
    tab.container.addEventListener('keydown', (event) => {
      const intent = pasteIntentForEvent(event, navigator.platform)
      if (!intent) return
      if (!startEmbeddedCliPaste(tab, intent)) return
      event.preventDefault()
      event.stopImmediatePropagation()
    }, true)
    // The macOS Edit > Paste command can dispatch a paste event without a
    // keydown in WebKit. Keep it on the same image-aware path as the keyboard
    // shortcut instead of letting xterm wrap the file path in bracketed paste.
    tab.container.addEventListener('paste', (event) => {
      if (!startEmbeddedCliPaste(tab, 'unified')) return
      event.preventDefault()
      event.stopImmediatePropagation()
    }, true)
  }
  term.textarea?.addEventListener('compositionstart', () => {
    tab.container.classList.add('terminal-ime-composing')
  })
  term.textarea?.addEventListener('compositionend', () => {
    tab.container.classList.remove('terminal-ime-composing')
    imeInputDeduper.onCompositionEnd()
  })
  term.attachCustomKeyEventHandler((ev) => {
    if (ev.type === 'keydown' && shouldBufferTerminalImeSwitch(ev)) imeInputDeduper.onInputMethodSwitch()
    if (handleWindowsTerminalSelectionCopy(term, ev)) return false
    if (
      handleWindowsTerminalSelectionDelete(
        selectionDeleteTarget(term),
        tab.inputState,
        ev,
        tab.ptyId !== null && tab.processState === 'alive',
      )
    ) {
      return false
    }
    // xterm turns Ctrl+V into the literal ^V control character before the
    // browser can dispatch a paste event. Intercept the shared paste shortcut
    // here as well as the macOS capture listener above; otherwise Windows and
    // Linux fall back to each CLI's own (inconsistent) Ctrl+V behavior.
    const pasteIntent = pasteIntentForEvent(ev, navigator.platform)
    if (pasteIntent && startEmbeddedCliPaste(tab, pasteIntent)) {
      ev.preventDefault()
      ev.stopImmediatePropagation()
      return false
    }
    if (ev.type !== 'keydown' || ev.altKey) return true
    const key = ev.key.toLowerCase()

    // Shift+Enter → 换行，在此直接拦截（不依赖 onData 的 shiftHeld 间接追踪 —— Windows
    // WebView2 上那条 onData 路径不可靠）。字节层分平台，因为 codex 的换行键解析不一样：
    //   · Mac/Linux：codex 认 \n(LF / Ctrl+J) 为换行，原始 PTY 直通即可。
    //   · Windows：codex 走 console-API 读按键事件、不解析 VT 转义，实测**只有 Alt+Enter
    //     (ESC+CR = \x1b\r) 被认作换行**（\n/Ctrl+J/kitty[13;2u] 都无效，见 codex#4401）。
    //     故把 Shift+Enter 映射成 Alt+Enter 的字节序列。
    if (key === 'enter' && ev.shiftKey && !ev.ctrlKey && !ev.metaKey) {
      if (tab.ptyId !== null && tab.processState === 'alive') {
        tab.inputState = applyTerminalInputState(tab.inputState, '\n').nextState
        const seq = _isWindows ? '\x1b\r' : '\n'
        api.ptyWrite(tab.ptyId, bytesToBase64(new TextEncoder().encode(seq))).catch(() => {})
      }
      return false
    }

    const mod = _isMac ? ev.metaKey : ev.ctrlKey
    const otherMod = _isMac ? ev.ctrlKey : ev.metaKey
    if (mod && !otherMod && !ev.shiftKey && (key === 'w' || key === 't' || key === 'r' || key === 'f')) {
      return false
    }

    return true
  })

  // 等 slot 把 container append 到可见 DOM 后再 fit + spawn —— 否则尺寸 = 0。
  // 两轮 rAF：一轮让 Vue 把 v-show 切完，一轮等浏览器布局稳定。
  await nextTick()
  await new Promise((r) => requestAnimationFrame(() => r(null)))
  await new Promise((r) => requestAnimationFrame(() => r(null)))

  try {
    fitAddon.fit()
  } catch {
    /* 容器仍可能没尺寸（用户极速切走），退到默认 80x24 由后端决定 */
  }
  const cols = term.cols || 80
  const rows = term.rows || 24
  tab.lastSyncedCols = cols
  tab.lastSyncedRows = rows

  let ptyId: number
  try {
    const extra = launchArgs.value[opts.agent as keyof typeof launchArgs.value] || ''
    const colorScheme = terminalColorScheme()
    const reclaude = opts.agent === 'claude' && useReclaude.value
    ptyId = isNew
      ? await api.ptySpawnNew(
          opts.agent,
          opts.cwd,
          terminalPtyColumns(opts.agent, cols),
          rows,
          extra,
          colorScheme,
          reclaude,
        )
      : await api.ptySpawn(
          opts.agent,
          opts.sessionId,
          opts.cwd,
          opts.sessionPath,
          terminalPtyColumns(opts.agent, cols),
          rows,
          extra,
          colorScheme,
          reclaude,
        )
  } catch (e) {
    setProcessState(tab, 'error')
    setTurnState(tab, 'error', 'pty-exit')
    tab.errorMessage = humanizeTerminalSessionError(e)
    term.write(`\r\n\x1b[31m[error] ${tab.errorMessage}\x1b[0m\r\n`)
    return
  }
  tab.ptyId = ptyId
  setProcessState(tab, 'alive')
  // codex resume 失败时给友好提示用：滚动保留最近一段 PTY 文本（仅 codex tab 需要）。
  // stream:true 让多字节字符跨块拼接不乱码；只留尾部 6KB，够覆盖启动阶段的报错。
  let recentCodexOutput = ''
  const hintDecoder = new TextDecoder('utf-8')

  // 后端 → xterm（按 id 过滤多 tab）
  tab.unlistenData = await listen<{ id: number; base64: string }>('pty://data', (e) => {
    if (e.payload.id !== ptyId) return
    tab.lastOutputAt = Date.now()
    const bytes = base64ToBytes(e.payload.base64)
    const customBackground = Boolean(backgroundImagePath.value)
    if (tab.agent === 'codex') {
      if (tab.agent === 'codex') {
        recentCodexOutput = (recentCodexOutput + hintDecoder.decode(bytes, { stream: true })).slice(-6000)
      }
      // 只有 Windows 上确认 codex 会误用深色调色板；mac/Linux 未验证，保持原样不镜像。
      const normalizer = codexSgrNormalizer(
        terminalColorScheme(),
        tab.agent === 'codex' && _isWindows,
        customBackground,
      )
      const normalized = normalizeAnsiBackground(bytes, tab.pendingAnsiBytes, normalizer)
      tab.pendingAnsiBytes = normalized.pending
      term.write(
        normalized.bytes,
        tab.agent === 'codex' ? () => syncRepairCodexUserMessageColors(tab) : undefined,
      )
    } else {
      tab.pendingAnsiBytes = null
      term.write(bytes)
    }
  })
  tab.unlistenExit = await listen<{ id: number; code: number }>('pty://exit', (e) => {
    if (e.payload.id !== ptyId) return
    setProcessState(tab, 'exited')
    if (tab.turnState === 'working') {
      setTurnState(tab, e.payload.code === 0 ? 'unknown' : 'error', 'pty-exit')
    }
    tab.exitCode = e.payload.code
    term.write(`\r\n\x1b[2m[process exited: ${e.payload.code}]\x1b[0m\r\n`)
    // 非 0 退出且是 codex：识别「配置/provider 加载失败」→ 补一行友好提示，并**保留 tab**
    // 让用户能看到报错（否则一闪而过就没了）。
    if (e.payload.code !== 0 && tab.agent === 'codex') {
      const hint = codexResumeConfigHint(recentCodexOutput)
      if (hint) term.write(`\r\n\x1b[33m${hint.replace(/\n/g, '\r\n')}\x1b[0m\r\n`)
      return
    }
    // 干净退出（如两次 Ctrl+C 正常退出会话）：会话 tab 随之关闭，不复用「保留在列表
    // 里等用户手动关」的旧语义。`closeTab` 内部对已关闭的 tab 幂等，重复 exit 事件安全。
    closeTab(tab.uiId)
  })

  // Shift 状态追踪：onData 不带修饰键信息，靠 keydown/keyup 标志判断 Shift+Enter→\n。
  let shiftHeld = false
  term.textarea?.addEventListener('keydown', (e) => { shiftHeld = e.shiftKey }, true)
  term.textarea?.addEventListener('keyup', () => { shiftHeld = false }, true)

  const writeTerminalInput = (rawData: string) => {
    if (tab.ptyId === null || tab.processState !== 'alive') return
    let data = rawData
    // Shift+Enter: xterm 发 \r，替换为 \n（与原生终端行为一致）。
    if (data === '\r' && shiftHeld) data = '\n'
    if (isTerminalCancelInput(data)) {
      clearLocalWorkingTurn(tab, activeUiId.value === tab.uiId)
      tab.inputState = createTerminalInputState()
    } else {
      const input = applyTerminalInputState(tab.inputState, data)
      if (
        tab.turnState !== 'blocked'
        && input.submittedLines.some((line) => shouldTerminalInputStartTurn(tab.agent, line))
      ) {
        if (shouldUseStableTerminalCursor(tab.agent, 'working', !!tab.isShell)) {
          captureStableTerminalCursor(tab)
        }
        setTurnState(tab, 'working', 'pty-input')
      }
      tab.inputState = input.nextState
    }
    const bytes = new TextEncoder().encode(data)
    api.ptyWrite(tab.ptyId, bytesToBase64(bytes)).catch(() => {})
  }
  imeInputDeduper.setFlushHandler(writeTerminalInput)

  // xterm → 后端（注：spawning / exited 时屏蔽，避免空 ptyId 或写已死管道）
  tab.onDataDisp = term.onData((data) => {
    if (tab.ptyId === null || tab.processState !== 'alive') return
    const imeData = imeInputDeduper.consume(data)
    if (imeData === null) return
    writeTerminalInput(imeData)
  })

  term.focus()
}

/** 开一个纯 shell tab（不跑任何 agent CLI），用于在项目目录里执行任意命令。 */
export async function openShellTab(opts: {
  agent: Agent
  projectKey: string
  title: string
  cwd: string
  createdAt?: number
}): Promise<void> {
  if (!opts.cwd) return

  const term = markRaw(
    new Terminal({
      fontSize: 13,
      fontFamily:
        '"SF Mono", "Menlo", "Consolas", "Liberation Mono", "Courier New", monospace',
      cursorBlink: true,
      convertEol: false,
      allowProposedApi: true,
      scrollback: 5000,
      theme: xtermTheme(isDarkActive()),
    }),
  )
  installTerminalScrollbackProtection(term, opts.agent, true)
  const fitAddon = markRaw(new FitAddon())
  term.loadAddon(fitAddon)

  const container = markRaw(document.createElement('div'))
  container.className = 'terminal-host'
  term.open(container)
  if (_isMac) container.addEventListener('paste', (e: Event) => _handleTerminalPaste(term, e as ClipboardEvent), true)

  const uiId = nextUiId++
  const tab = reactive<TerminalTab>({
    uiId,
    ptyId: null,
    agent: opts.agent,
    projectKey: opts.projectKey,
    paneId: ensureLayout(opts.agent, opts.projectKey).focusedPaneId,
    sessionId: '',
    sessionPath: '',
    title: opts.title,
    cwd: opts.cwd,
    createdAt: opts.createdAt ?? Date.now(),
    term,
    fitAddon,
    container,
    unlistenData: null,
    unlistenExit: null,
    onDataDisp: null,
    lastSyncedCols: 0,
    lastSyncedRows: 0,
    inputState: createTerminalInputState(),
    processState: 'spawning',
    turnState: 'unknown',
    turnStateSource: null,
    turnStateUpdatedAt: Date.now(),
    lastOutputAt: 0,
    lastSessionActivityAt: 0,
    pendingAnsiBytes: null,
    status: 'spawning',
    isShell: true,
  }) as TerminalTab
  tabs.value.push(tab)
  activateTabInPane(tab)
  const imeInputDeduper = createTerminalImeInputDeduper()
  term.attachCustomKeyEventHandler((ev) => {
    if (ev.type === 'keydown' && shouldBufferTerminalImeSwitch(ev)) imeInputDeduper.onInputMethodSwitch()
    if (handleWindowsTerminalSelectionCopy(term, ev)) return false
    if (ev.type !== 'keydown' || ev.altKey) return true
    const key = ev.key.toLowerCase()

    const mod = _isMac ? ev.metaKey : ev.ctrlKey
    const otherMod = _isMac ? ev.ctrlKey : ev.metaKey
    if (mod && !otherMod && !ev.shiftKey && (key === 'w' || key === 't' || key === 'r' || key === 'f')) {
      return false
    }
    return true
  })

  let shiftHeld = false
  term.textarea?.addEventListener('keydown', (e) => { shiftHeld = e.shiftKey }, true)
  term.textarea?.addEventListener('keyup', () => { shiftHeld = false }, true)

  await nextTick()
  await new Promise((r) => requestAnimationFrame(() => r(null)))
  await new Promise((r) => requestAnimationFrame(() => r(null)))

  try {
    fitAddon.fit()
  } catch { /* */ }
  const cols = term.cols || 80
  const rows = term.rows || 24
  tab.lastSyncedCols = cols
  tab.lastSyncedRows = rows

  let ptyId: number
  try {
    ptyId = await api.ptySpawnShell(opts.cwd, cols, rows, terminalColorScheme())
  } catch (e) {
    setProcessState(tab, 'error')
    setTurnState(tab, 'error', 'pty-exit')
    tab.errorMessage = String(e)
    term.write(`\r\n\x1b[31m[error] ${e}\x1b[0m\r\n`)
    return
  }
  tab.ptyId = ptyId
  setProcessState(tab, 'alive')

  tab.unlistenData = await listen<{ id: number; base64: string }>('pty://data', (e) => {
    if (e.payload.id !== ptyId) return
    tab.lastOutputAt = Date.now()
    tab.pendingAnsiBytes = null
    term.write(base64ToBytes(e.payload.base64))
  })
  tab.unlistenExit = await listen<{ id: number; code: number }>('pty://exit', (e) => {
    if (e.payload.id !== ptyId) return
    setProcessState(tab, 'exited')
    tab.exitCode = e.payload.code
    term.write(`\r\n\x1b[2m[process exited: ${e.payload.code}]\x1b[0m\r\n`)
  })

  term.textarea?.addEventListener('compositionend', () => imeInputDeduper.onCompositionEnd())

  const writeShellInput = (data: string) => {
    if (tab.ptyId === null || tab.processState !== 'alive') return
    if (data === '\r' && shiftHeld) data = '\n'
    const bytes = new TextEncoder().encode(data)
    api.ptyWrite(tab.ptyId, bytesToBase64(bytes)).catch(() => {})
  }
  imeInputDeduper.setFlushHandler(writeShellInput)

  tab.onDataDisp = term.onData((data) => {
    if (tab.ptyId === null || tab.processState !== 'alive') return
    const imeData = imeInputDeduper.consume(data)
    if (imeData === null) return
    writeShellInput(imeData)
  })

  term.focus()
}

/** 在 tab 自己的 pane 里激活它（并聚焦该 pane）。pane 已不存在则挂到当前聚焦 pane。 */
function activateTabInPane(tab: TerminalTab) {
  let pane = panes.get(tab.paneId)
  if (!pane) {
    tab.paneId = ensureLayout(tab.agent, tab.projectKey).focusedPaneId
    pane = panes.get(tab.paneId)
  }
  if (pane) {
    pane.activeUiId = tab.uiId
    focusPane(pane.id)
  }
}

/** 切换激活 tab。`null` = 隐藏聚焦 pane 的 TUI 层，露出 view（聊天/列表/统计/...）。 */
export function setActive(uiId: number | null) {
  if (uiId === null) {
    activeUiId.value = null
    return
  }
  const tab = tabs.value.find((t) => t.uiId === uiId)
  if (!tab) return
  activateTabInPane(tab)
  markTabViewed(uiId)
}

/** 书签合并到真实项目时，把旧 projectKey 的 tab 迁移到新 key，避免 strip 过滤丢失。 */
export function migrateTabsProjectKey(oldKey: string, newKey: string) {
  for (const tab of tabs.value) {
    if (tab.projectKey === oldKey) {
      tab.projectKey = newKey
    }
  }
}

/** 完全关闭一个 tab：kill PTY、dispose Terminal、移出列表。如果是当前 active 会自动落到邻居。 */
export function closeTabsByProject(projectKey: string) {
  const toClose = tabs.value.filter(t => t.projectKey === projectKey).map(t => t.uiId)
  for (const id of toClose) closeTab(id)
}

export function closeTabBySessionPath(sessionPath: string) {
  const tab = tabs.value.find(t => t.sessionPath === sessionPath)
  if (tab) closeTab(tab.uiId)
}

export function closeTab(uiId: number) {
  const idx = tabs.value.findIndex((t) => t.uiId === uiId)
  if (idx < 0) return
  const tab = tabs.value[idx]

  // splice + active fallback first → UI updates immediately
  tabs.value.splice(idx, 1)
  knownPathsAtTabCreation.delete(uiId)
  const pane = panes.get(tab.paneId)
  if (pane && pane.activeUiId === uiId) {
    // 落到同 pane 内的邻居 tab；该 pane 没别的 tab 了就露出 view 层
    const sameCtx = tabs.value.filter(
      (t) => t.agent === tab.agent && t.projectKey === tab.projectKey && t.paneId === tab.paneId,
    )
    pane.activeUiId = sameCtx[0]?.uiId ?? null
  }

  // heavy cleanup after reactive state is settled
  tab.onDataDisp?.dispose()
  tab.unlistenData?.()
  tab.unlistenExit?.()
  if (tab.ptyId !== null) {
    api.ptyKill(tab.ptyId).catch(() => {})
  }
  try {
    tab.term.dispose()
  } catch {
    /* 已经 dispose 过 */
  }
  if (tab.container.parentElement) {
    tab.container.parentElement.removeChild(tab.container)
  }
}

/**
 * 刷新指定 tab（默认当前 active）的尺寸：fit() 之后把新的 cols/rows 推给后端 PTY。
 * 外面用 ResizeObserver / 主题/侧栏切换后调用。失败时静默退出 —— 多数情况下是
 * tab 已经被关掉了，由后续的 close 流程负责清场。
 */
export function refit(uiId?: number) {
  const target = uiId !== undefined ? findTab(uiId) : activeTab()
  if (!target) return
  try {
    target.fitAddon.fit()
  } catch {
    return
  }
  syncStableTerminalCursorGeometry(target)
  const cols = target.term.cols
  const rows = target.term.rows
  if (
    target.ptyId !== null &&
    cols > 0 &&
    rows > 0 &&
    (cols !== target.lastSyncedCols || rows !== target.lastSyncedRows)
  ) {
    target.lastSyncedCols = cols
    target.lastSyncedRows = rows
    api.ptyResize(target.ptyId, terminalPtyColumns(target.agent, cols), rows).catch(() => {})
  }
}
