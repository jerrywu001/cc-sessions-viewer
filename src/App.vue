<script setup lang="ts">
import { ref, shallowRef, computed, onMounted, onUnmounted, watch, nextTick, provide, defineAsyncComponent } from 'vue'
import type { Agent, ProjectInfo, SessionMeta, TrashItem, Msg, UsageSummary } from './types'
import * as api from './api'
import { shortName } from './format'
import { t } from './i18n'
import { convertFileSrc } from '@tauri-apps/api/core'
import {
  clearAppCache,
  codexShowArchivedSessions,
  codexShowInternalSessions,
  lang,
  setLang,
  setTheme,
  theme,
  nativeAppearance,
  useExternalTerminal,
  autoRestoreTerminalTabs,
  launchArgs,
  terminalApp,
  applyTerminalDefault,
  visibleAgents,
  quickOpenTarget,
  backgroundImagePath,
  backgroundIsVideo,
} from './settings'
import { focusSearchBox, navigate as chatNavigate, resetChatToolbar } from './chatToolbar'
import { focusTuiSearchBox } from './tuiToolbar'
import { emitMenuSync, installMenuRouter, type MenuHandlers } from './menu'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { resetTrashToolbar, exitSelectMode, selectedTrash } from './trashToolbar'
import {
  resetSessionsToolbar,
  sessionsFilterActive,
  selectedSessions,
  exitSessionSelectMode,
} from './sessionsToolbar'
import {
  exportMarkdown,
  exportHtml,
  exportJson,
  exportMarkdownToDir,
  exportHtmlToDir,
  exportJsonToDir,
  pickExportDir,
  batchExportFolderName,
  type ExportKind,
} from './export'
import { fly } from './fly'
import { recordRecent } from './recents'
import { recordExport, type ExportRecord } from './exportHistory'
import { globalSearchOpen, openGlobalSearch } from './globalSearch'
import { runBackgroundCheck } from './updateCheck'
import { refreshTurnHookStatus } from './turnHookStatus'
import {
  acknowledgeDesktopPetTask,
  resolveDesktopPetSession,
  restoreDesktopPet as restoreDesktopPetWindow,
  type DesktopPetResolvedSession,
  type DesktopTask,
} from './desktopPet'
import type { SearchHit } from './types'
import ChatSidePanel from './components/ChatSidePanel.vue'
import CodexSidePanel from './components/CodexSidePanel.vue'
import SettingsModal from './components/SettingsModal.vue'
import { IconSearch } from './components/icons'
import WindowsTitlebar, { type WindowMenuGroup } from './components/WindowsTitlebar.vue'
import ChatTopbar from './components/topbar/ChatTopbar.vue'
import TuiTopbar from './components/topbar/TuiTopbar.vue'
import TrashTopbar from './components/topbar/TrashTopbar.vue'
import SessionsTopbar from './components/topbar/SessionsTopbar.vue'
import TrashView from './views/TrashView.vue'
// 按需视图懒加载：StatsView 拖着重量级图表库 @antv/g2，PricingView / ExportHistoryView 也是
// 二级页面 —— 都不进首屏主包，进对应页面时再拉各自的 chunk。
const StatsView = defineAsyncComponent(() => import('./views/StatsView.vue'))
import Sidebar from './components/Sidebar.vue'
import SidebarTopbar from './components/SidebarTopbar.vue'
import PaneGrid from './components/PaneGrid.vue'
import ConfirmModal from './modals/ConfirmModal.vue'
import RenameModal from './modals/RenameModal.vue'
import GlobalSearchModal from './modals/GlobalSearchModal.vue'
const ExportHistoryView = defineAsyncComponent(() => import('./views/ExportHistoryView.vue'))
const PricingView = defineAsyncComponent(() => import('./views/PricingView.vue'))
import ProjectContextMenu from './modals/ProjectContextMenu.vue'
import WorktreeModal from './modals/WorktreeModal.vue'
import {
  clearPendingLiveNotification,
  enqueueLiveNotification,
} from './liveNotifications'
import {
  activeUiId,
  openOrFocusTui,
  openShellTab,
  setActive as setActiveTui,
  activeTab as currentActiveTab,
  closeTab,
  closeTabsByProject,
  closeTabBySessionPath,
  reconcileNewTabs,
  syncTabTitlesFromSessions,
  syncTabTitleBySessionPath,
  setTabTitleByUiId,
  isTabProcessAlive,
  markTabSessionActivity,
  markTabTurnStarted,
  markTabTurnCompleted,
  markTabTurnBlocked,
  markTabTurnFailed,
  markTabViewed,
  migrateTabsProjectKey,
  tabs as tuiTabs,
  persistTabState,
  loadSavedNav,
  loadSavedActiveTui,
  persistActiveTui,
  savedTabs,
  removeSavedTab,
  renameSavedTab,
  clearAllTabs,
  type TerminalTab,
  type SavedTab,
  type SavedNav,
  type SavedActiveTui,
} from './terminals'
import { sessionPathsEqual } from './tabStatus'
import {
  recordView,
  setViewTitle,
  removeViewEverywhere,
} from './viewHistory'
import { startChat, closeChat, reconnectChats, lastAssistantModel, migrateChatSessionsProjectKey, chatSessions, type ChatSession } from './chatSessions'
import type { ChatHistoryEntry } from './chatInputHistory'
import { sideChat, openSideChat, closeSideChat, closeAllSideChats, activeBtwSessionIds } from './sideChat'
import {
  activeCodexSideSessionIds,
  closeAllCodexSideChats,
  closeCodexSideChat,
  codexSideChat,
  openCodexSideChat,
} from './codexSideChat'
import {
  type ViewTab,
  viewTabs,
  activeViewTabId,
  activeViewTab,
  createViewTab,
  suppressActivation,
  findViewTab,
  removeViewTab,
  setActiveViewTab,
  visibleViewTabs,
  persistViewTabs,
  loadSavedViewTabs,
  clearSavedViewTabs,
  markViewTabsRestored,
  migrateViewTabsProjectKey,
  type SavedViewTab,
} from './viewTabs'
import {
  currentAgent as panesAgent,
  currentProjectKey as panesProject,
  panes,
  focusedPane,
  focusPane,
  currentLayout,
  currentPanes,
  paneCount,
  splitPane,
  closePane,
  persistLayouts,
  migratePaneProjectKey,
  type SplitDir,
} from './panes'
import { projectsDirty, markProjectsDirty } from './projectsRefresh'
import { paneViewsOf } from './paneRegistry'
import { PaneActionsKey, type PaneActions } from './paneActions'
import { chatSupported } from './chatComposerOptions'

// ---------- 状态 ----------
// 默认进首个可见 agent —— 用户若在设置里关掉了 claude，启动时就不该停在隐藏的 agent 上。
const agent = ref<Agent>(visibleAgents.value[0] ?? 'claude')
const projects = ref<ProjectInfo[]>([])
const activeDir = ref<string | null>(null)

// 内嵌 TUI 后台同步时上一次看到的会话总数（-1 = 尚无基线 / 刚切项目）。TUI 里跑出新会话
// 会让总数增长 → 据此触发侧栏计数重载（见 syncTuiTitlesNow）。与「上次同步值」比而不与侧栏
// 徽标比：list_projects / list_sessions 过滤口径可能不同，跟徽标比会常驻抖动。
let lastTuiSyncedTotal = -1

// 分屏当前视图 = 侧栏选中的 (agent, project)。panes 模块据此解出当前布局 / 聚焦 pane，
// 而 activeUiId / activeViewTabId 又是聚焦 pane 的投影，所以这一步是它们正确工作的前提。
watch([agent, activeDir], ([a, dir]) => {
  panesAgent.value = a
  panesProject.value = dir
  // 切项目 / 切 agent → 作废 TUI 会话总数基线，避免用上一个项目的总数误判「新增会话」。
  lastTuiSyncedTotal = -1
}, { immediate: true })
const showTrash = ref(false)
const showStats = ref(false)
const showExportHistory = ref(false)
const showPricing = ref(false)
const showSettings = ref(false)
const settingsTab = ref<'general' | 'theme' | 'advanced' | 'hooks' | 'pet' | 'cli' | 'shortcuts' | 'updates'>()
const sidebarOpen = ref(true)
const refreshing = ref(false)
const isWindows = /Win/i.test(navigator.platform)
type WindowCloseAction = 'tray' | 'exit'
const WINDOW_CLOSE_PREF_KEY = 'windowCloseAction:v1'
const windowClosePrompt = ref({ show: false, remember: false })
const windowCloseRunning = ref(false)
let windowCloseUnlisten: UnlistenFn | null = null
let beforeQuitUnlisten: UnlistenFn | null = null
function toggleSidebar() {
  sidebarOpen.value = !sidebarOpen.value
}

const SIDEBAR_WIDTH_KEY = 'sidebarWidth:v1'
const SIDEBAR_MIN_WIDTH = 220
const SIDEBAR_MAX_WIDTH = 420

function clampSidebarWidth(width: number): number {
  const viewportMax = Math.max(SIDEBAR_MIN_WIDTH, window.innerWidth - 360)
  return Math.round(Math.min(Math.max(width, SIDEBAR_MIN_WIDTH), SIDEBAR_MAX_WIDTH, viewportMax))
}

function loadSidebarWidth(): number {
  const raw = Number(localStorage.getItem(SIDEBAR_WIDTH_KEY))
  return clampSidebarWidth(Number.isFinite(raw) && raw > 0 ? raw : 248)
}

const sidebarWidth = ref(loadSidebarWidth())
const sidebarResizing = ref(false)
const appStyle = computed<Record<string, string>>(() => ({
  '--sidebar-w': `${sidebarWidth.value}px`,
}))
const backgroundVideoUrl = computed(() =>
  backgroundIsVideo.value && backgroundImagePath.value
    ? convertFileSrc(backgroundImagePath.value)
    : '',
)
let sidebarResizeStartX = 0
let sidebarResizeStartWidth = 0

function onSidebarResizePointerDown(e: PointerEvent) {
  e.preventDefault()
  sidebarResizing.value = true
  sidebarResizeStartX = e.clientX
  sidebarResizeStartWidth = sidebarWidth.value
  document.body.classList.add('is-sidebar-resizing')
  window.addEventListener('pointermove', onSidebarResizePointerMove)
  window.addEventListener('pointerup', onSidebarResizePointerUp, { once: true })
  window.addEventListener('pointercancel', onSidebarResizePointerUp, { once: true })
}

function onSidebarResizePointerMove(e: PointerEvent) {
  if (!sidebarResizing.value) return
  sidebarWidth.value = clampSidebarWidth(
    sidebarResizeStartWidth + e.clientX - sidebarResizeStartX,
  )
}

function onSidebarResizePointerUp() {
  if (!sidebarResizing.value) return
  sidebarResizing.value = false
  document.body.classList.remove('is-sidebar-resizing')
  localStorage.setItem(SIDEBAR_WIDTH_KEY, String(sidebarWidth.value))
  window.removeEventListener('pointermove', onSidebarResizePointerMove)
  window.removeEventListener('pointerup', onSidebarResizePointerUp)
  window.removeEventListener('pointercancel', onSidebarResizePointerUp)
}

function onWindowResize() {
  sidebarWidth.value = clampSidebarWidth(sidebarWidth.value)
}

const codexSessionOptions = computed(() => ({
  includeCodexInternal: codexShowInternalSessions.value,
  includeCodexArchived: codexShowArchivedSessions.value,
}))

function sessionListOptions() {
  if (agent.value === 'codex') return codexSessionOptions.value
  // opencode 有同款「归档」语义（session.time_archived）——挂在同一个偏好上。
  if (agent.value === 'opencode') return { includeCodexArchived: codexShowArchivedSessions.value }
  return undefined
}

/** 顶栏刷新：重新拉取项目 + 当前列表 + 当前打开的对话，全部静默，不动选中与滚动。 */
async function refreshAll() {
  if (refreshing.value) return
  refreshing.value = true
  const tasks: Promise<unknown>[] = []

  // 1. 项目列表（保留 activeDir）
  tasks.push(
    api.listProjects(agent.value, sessionListOptions()).then((p) => {
      applyProjects(p)
    }).catch(() => {}),
  )

  // 2. 当前列表（项目会话 or 回收站）
  if (showTrash.value) {
    tasks.push(
      api.listTrash().then((t) => {
        trash.value = t
      }).catch(() => {}),
    )
  } else if (activeDir.value) {
    const keepScroll = listScrollEl.value?.scrollTop ?? savedListScroll
    // 保留当前已加载数量，避免分页回退
    const n = Math.max(sessions.value.length, PAGE_SIZE)
    tasks.push(
      api
        .listSessions(agent.value, activeDir.value, 0, n, sessionListOptions())
        .then((page) => {
          sessions.value = page.sessions
          sessionTotal.value = page.total
          nextTick(() => {
            if (listScrollEl.value) listScrollEl.value.scrollTop = keepScroll
          })
        })
        .catch(() => {}),
    )
  }

  // 3. 当前打开的 session tab（如有）—— 静默替换 messages
  const curViewTab = activeViewTab.value
  if (curViewTab?.type === 'session' && curViewTab.session) {
    tasks.push(
      api
        .readSession(agent.value, curViewTab.session.path)
        .then((msgs) => {
          curViewTab.msgs = msgs
        })
        .catch(() => {}),
    )
  }

  try {
    await Promise.all(tasks)
  } finally {
    refreshing.value = false
  }
}
const sessions = shallowRef<SessionMeta[]>([])
const visibleSessions = computed(() => {
  const hiddenIds = new Set([...activeBtwSessionIds(), ...activeCodexSideSessionIds()])
  return hiddenIds.size ? sessions.value.filter(s => !hiddenIds.has(s.id)) : sessions.value
})
const sessionTotal = ref(0)
const loadingMore = ref(false)
const trash = shallowRef<TrashItem[]>([])
const loadingList = ref(false)

const PAGE_SIZE = 40

// openSession / liveChat / chatMsgs 从 activeViewTab 派生，保持模板层不变
const openSession = computed<SessionMeta | null>(() => {
  const tab = activeViewTab.value
  if (!tab) return null
  if (tab.type === 'session') return tab.session
  if (tab.type === 'chat') return tab.sourceSession
  return null
})
const liveChat = computed<ChatSession | null>(() => {
  const tab = activeViewTab.value
  return tab?.type === 'chat' ? tab.chatSession : null
})
const chatMsgs = computed<Msg[]>(() => {
  const tab = activeViewTab.value
  if (!tab) return []
  if (tab.type === 'session') return tab.msgs
  if (tab.type === 'chat') return tab.chatSession?.msgs ?? []
  return []
})
// 每个项目最近活跃的 TUI tab —— 切项目再切回来时据此恢复 TUI 层。
// 存 sessionPath（跨重启稳定）而非 uiId（重启后变）。
const activeTuiByProject = new Map<string, { uiId?: number; sessionPath: string; isShell?: boolean }>()
const activeViewByProject = new Map<string, { viewUiId: number | null; wasTui: boolean }>()
// 每个 agent 上次停留的项目 —— 左侧切 agent 再切回来时直接恢复到该项目（含其活跃 tab），
// 而不是回到欢迎页要求再点一次。undefined / null 表示上次停在欢迎页。
const lastDirByAgent = new Map<Agent, string | null>()
const viewKey = (a: string, dir: string) => a + ' ' + dir
const hydratingSavedTabs = new Set<string>()
function persistTuiMap() {
  const out: SavedActiveTui[] = []
  for (const [k, v] of activeTuiByProject) {
    const sep = k.indexOf(' ')
    if (sep < 0) continue
    out.push({ agent: k.slice(0, sep) as Agent, dir: k.slice(sep + 1), sessionPath: v.sessionPath, ...(v.isShell ? { isShell: true } : {}) })
  }
  persistActiveTui(out)
}

function savedTabKey(saved: SavedTab): string {
  return saved.sessionPath || [
    saved.agent,
    saved.projectKey,
    saved.isShell ? 'shell' : 'session',
    saved.sessionId,
    saved.cwd,
    String(saved.createdAt ?? 0),
  ].join('\n')
}

function removeSavedAfterHydrate(saved: SavedTab) {
  removeSavedTab(saved.sessionPath ? saved.sessionPath : saved)
}

async function hydrateSavedTabOnce(saved: SavedTab): Promise<boolean> {
  const key = savedTabKey(saved)
  if (hydratingSavedTabs.has(key)) return true
  hydratingSavedTabs.add(key)
  try {
    const ok = await hydrateSavedTab(saved)
    if (ok) removeSavedAfterHydrate(saved)
    return ok
  } finally {
    hydratingSavedTabs.delete(key)
  }
}

function projectLiveTuiTabs(dir: string): TerminalTab[] {
  return tuiTabs.value.filter(
    (t) => t.agent === agent.value && t.projectKey === dir && isTabProcessAlive(t),
  )
}

function findProjectSavedTui(
  dir: string,
  remembered?: { sessionPath: string; isShell?: boolean },
): SavedTab | undefined {
  const saved = savedTabs.value.filter((s) => s.agent === agent.value && s.projectKey === dir)
  if (!saved.length) return undefined
  if (remembered?.isShell) {
    const shell = saved.find((s) => s.isShell)
    if (shell) return shell
  } else if (remembered?.sessionPath) {
    const byPath = saved.find((s) => s.sessionPath === remembered.sessionPath)
    if (byPath) return byPath
  }
  return [...saved].sort((a, b) => (a.createdAt ?? 0) - (b.createdAt ?? 0))[0]
}

async function maybeActivateProjectTui(dir: string, remembered?: { uiId?: number; sessionPath: string; isShell?: boolean }): Promise<boolean> {
  // 1) 有对应「活着」的 tab（同会话内切项目 / 切 agent 时 PTY 仍在 tuiTabs）→ 直接激活，
  //    便宜、无进程重开、无闪烁，且**不受 autoRestore 开关限制** —— 激活态本就该恢复，
  //    与 chat/read/git view tab 的持久化对称（0.2.10 即如此，被懒恢复改动误伤）。
  const liveTabs = projectLiveTuiTabs(dir)
  const liveMatch = remembered?.uiId != null
    ? liveTabs.find((t) => t.uiId === remembered.uiId)
    : remembered?.isShell
      ? liveTabs.find((t) => t.isShell)
      : remembered?.sessionPath
        ? liveTabs.find((t) => t.sessionPath === remembered.sessionPath)
        : undefined
  if (liveMatch) {
    if (activeUiId.value !== liveMatch.uiId) setActiveTui(liveMatch.uiId)
    return true
  }
  // 2) 没有活着的对应 tab → 从 saved pill 重开进程。有明确 remembered 命中就重开「那一个」
  //    （对齐 0.2.10：刷新/重启后也恢复上次激活的终端）；只有在完全没 remembered、只能兜底取
  //    「第一个 pill」时才尊重 autoRestore 开关，避免切项目时误开一个不相干的终端。
  const saved = findProjectSavedTui(dir, remembered)
  if (!saved) return false
  const hasRemembered = remembered?.uiId != null || !!remembered?.isShell || !!remembered?.sessionPath
  if (!hasRemembered && !autoRestoreTerminalTabs.value) return false
  return hydrateSavedTabOnce(saved)
}
// 非空表示当前打开的会话来自回收站（只读查看）—— 详情页据此切换为「回收站模式」。
const openTrashItem = ref<TrashItem | null>(null)
// "● Live" 徽章：仅当会话**确实正在被写入**时为 true。
//   - 打开时 mtime 距今 < FRESH_MS → 视作"刚才还在跑"，先亮起来
//   - 收到 session:append 事件 → 文件真的有新增 → 亮起 / 续命
//   - 安静 STALE_MS 后自动熄灭 —— CLI 进程通常已结束
// 这与"是否在后端追这个文件"分离：watcher 对所有非回收站会话都开，
// 否则用户从终端 resume 一个老会话时我们就漏掉了。
// （"● Live" 徽章的实际渲染在 PaneContent 里按各自 pane 的 view tab 解出。）
const LIVE_FRESH_MS = 3 * 60 * 1000
const LIVE_STALE_MS = 2 * 60 * 1000
function clearLive() {
  const tab = activeViewTab.value
  if (!tab) return
  tab.liveTailing = false
  window.clearTimeout(tab.liveFadeTimer)
  tab.liveFadeTimer = 0
}

// 单会话统计目标。非空 → StatsView 切换到 session 模式，scope 锁定到这条 JSONL。
// 与 showStats=true 联用：全局统计时此值为 null，会话统计时填上 {agent, path, title}。
const sessionStatsTarget = ref<{ agent: Agent; path: string; title?: string } | null>(null)
// 单会话统计是从哪进入的：决定「返回」按钮往哪走。
//   'chat'   ← ChatTopbar 的统计按钮（关闭 → 回到原聊天）
//   'global' ← 全局 StatsView Top Sessions 行点击（关闭 → 回到全局 StatsView）
const sessionStatsFrom = ref<'chat' | 'global' | null>(null)

// 聚焦格子内部的 ChatView / SessionsView 实例 —— 每个 PaneContent 把自己登记进 paneRegistry，
// App 按聚焦 paneId 取，借此做 flashMessage / onLiveAppend / 列表滚动保存恢复。
const focusedPaneViews = computed(() => paneViewsOf(focusedPane.value?.id))
const chatViewRef = computed(() => focusedPaneViews.value?.chatView ?? null)
const sidebarRef = ref<InstanceType<typeof Sidebar> | null>(null)
const listScrollEl = computed<HTMLElement | undefined>(
  () => focusedPaneViews.value?.sessionsView?.scrollEl,
)
let savedListScroll = 0
const TUI_TITLE_SYNC_INTERVAL_MS = 4000
let tuiTitleSyncTimer = 0
let syncingTuiTitles = false

watch(openSession, (val, old) => {
  // 切换 / 关闭会话时把聊天页顶栏（搜索 / 折叠 / 等）状态归零，
  // 否则前一个会话的搜索词 / 折叠态会留到下一个，体验古怪。
  if (val?.path !== old?.path) resetChatToolbar()
  // 关闭会话即退出回收站模式 —— openTrashItem 永远不残留到下一次打开。
  if (!val) openTrashItem.value = null
  // 切到别的会话 / 关闭会话 → 立刻让后端停掉旧 watcher。
  // openChat 里会再起新的；openTrashSession / null 都不需要 watcher。
  if (val?.path !== old?.path) {
    clearLive()
    clearPendingLiveNotification()
    api.unwatchSession().catch(() => {})
    if (val?.path) {
      const tab = activeViewTab.value
      if (tab?.type === 'session') {
        api.watchSession(tab.agent, val.path).catch(() => {})
      }
    }
  }
  if (!val && old) {
    nextTick(() => {
      if (listScrollEl.value) listScrollEl.value.scrollTop = savedListScroll
    })
  }
})

// —— 分屏 ——
// 全局全区视图（stats/trash/…）接管主区时分屏格子不可见，此时不响应拆分/关闭快捷键。
const globalViewActive = computed(
  () => showStats.value || showTrash.value || showExportHistory.value || showPricing.value,
)

/** Cmd+D → 右侧新增一格（row），Cmd+Shift+D → 下方新增一格（col）。新格聚焦、无 tab（露项目主页）。 */
function splitFocusedPane(dir: SplitDir) {
  if (globalViewActive.value) return
  const p = focusedPane.value
  if (!p) return
  splitPane(p.id, dir)
}

/** 关闭聚焦格子（至少保留 1 格）。被关格子里的 tab 迁移到折叠后聚焦的邻格，PTY 不杀。 */
// 关闭并**释放**某个分屏格子里的所有 tab（kill PTY / dispose xterm / kill chat 子进程 /
// 丢弃 msgs），再收起空格子并把聚焦落到邻居。不迁移到邻居——迁移会让进程和 xterm 实例
// 常驻内存，关得越多越卡。
function closePaneFreeing(paneId: number) {
  if (currentPanes.value.length <= 1) return
  const tuiIds = tuiTabs.value.filter((t) => t.paneId === paneId).map((t) => t.uiId)
  const viewIds = viewTabs.value.filter((t) => t.paneId === paneId).map((t) => t.uiId)
  for (const id of tuiIds) closeTab(id)
  for (const id of viewIds) {
    const vt = viewTabs.value.find((t) => t.uiId === id)
    if (vt?.type === 'chat') closeLiveChat(id)
    else removeViewTab(id)
  }
  closePane(paneId)
  saveTabState()
}

// Cmd+Shift+W：关闭聚焦格子（快捷键 = power user，不二次确认）。
function closeFocusedPane() {
  if (globalViewActive.value) return
  const p = focusedPane.value
  if (!p) return
  closePaneFreeing(p.id)
}

// SessionsView 顶栏「退出分屏」按钮：二次确认后关闭指定格子（会丢失该格的终端/会话/聊天）。
function exitPane(paneId: number) {
  if (currentPanes.value.length <= 1) return
  ask({
    title: t('dialog.exitPane.title'),
    message: t('dialog.exitPane.body'),
    okText: t('dialog.exitPane.ok'),
    danger: true,
    onOk: () => closePaneFreeing(paneId),
  })
}

/**
 * Cmd+Alt+方向键 → 按空间方向把聚焦移到相邻格子。用实际 DOM 矩形而非树顺序，这样在
 * 混合 row/col 嵌套下也能正确选出「右边/下边」的那格。主轴距离为主、交叉轴距离加权次之。
 */
function focusPaneDir(dir: 'left' | 'right' | 'up' | 'down') {
  if (globalViewActive.value || currentPanes.value.length <= 1) return
  const cur = focusedPane.value
  if (!cur) return
  const els = [...document.querySelectorAll<HTMLElement>('.pane-grid .pane[data-pane-id]')]
  const rectOf = (id: number) => els.find((e) => e.dataset.paneId === String(id))?.getBoundingClientRect()
  const curRect = rectOf(cur.id)
  if (!curRect) return
  const cx = curRect.left + curRect.width / 2
  const cy = curRect.top + curRect.height / 2
  let best: { id: number; score: number } | null = null
  for (const p of currentPanes.value) {
    if (p.id === cur.id) continue
    const r = rectOf(p.id)
    if (!r) continue
    const dx = r.left + r.width / 2 - cx
    const dy = r.top + r.height / 2 - cy
    let primary: number, cross: number
    if (dir === 'right') { if (dx <= 1) continue; primary = dx; cross = Math.abs(dy) }
    else if (dir === 'left') { if (dx >= -1) continue; primary = -dx; cross = Math.abs(dy) }
    else if (dir === 'down') { if (dy <= 1) continue; primary = dy; cross = Math.abs(dx) }
    else { if (dy >= -1) continue; primary = -dy; cross = Math.abs(dx) }
    const score = primary + cross * 2
    if (!best || score < best.score) best = { id: p.id, score }
  }
  if (best) focusPane(best.id)
}

const activeProject = computed(() =>
  projects.value.find((p) => p.dirName === activeDir.value),
)
const activeAgentLabel = computed(() =>
  agent.value === 'codex' ? 'Codex' : agent.value === 'agy' ? 'agy' : agent.value === 'opencode' ? 'opencode' : 'Claude',
)
const topbarContextTitle = computed(() => {
  if (showStats.value) return t('sidebar.stats')
  if (showTrash.value) return t('sidebar.trash')
  if (showExportHistory.value) return t('sidebar.history')
  if (showPricing.value) return t('sidebar.pricing')
  return activeProject.value ? shortName(activeProject.value.displayPath) : activeAgentLabel.value
})
const topbarContextMeta = computed(() => {
  if (showStats.value || showTrash.value || showExportHistory.value || showPricing.value) {
    return activeAgentLabel.value
  }
  if (openSession.value || activeUiId.value !== null) return t('chat.tui.viewTab')
  if (activeProject.value) return t('chat.tui.listTab')
  return ''
})
// 详情页用的 agent：从 activeViewTab 的 trashAgent / importedAgent / agent 推导。
const chatAgent = computed<Agent>(
  () => activeViewTab.value?.trashAgent ?? activeViewTab.value?.importedAgent ?? activeViewTab.value?.agent ?? agent.value,
)

// ---------- 当前项目是否有 git 仓库 ----------
const projectHasGit = ref(false)
watch(activeProject, async (proj) => {
  if (!proj) { projectHasGit.value = false; return }
  projectHasGit.value = await api.gitHasRepo(proj.displayPath).catch(() => false)
}, { immediate: true })

// ---------- 项目置顶 / 沉底偏好（持久化到 localStorage）----------
type ProjState = 'pinned' | 'sunk'
const PREFS_KEY = 'projPrefs:v1'
const PROJECT_ORDER_KEY = 'projectOrder:v1'

function loadPrefs(): Record<string, ProjState> {
  try {
    return JSON.parse(localStorage.getItem(PREFS_KEY) || '{}')
  } catch {
    return {}
  }
}
const projPrefs = ref<Record<string, ProjState>>(loadPrefs())

function loadProjectOrders(): Record<string, string[]> {
  try {
    const parsed = JSON.parse(localStorage.getItem(PROJECT_ORDER_KEY) || '{}') as Record<string, unknown>
    const orders: Record<string, string[]> = {}
    for (const [agentName, value] of Object.entries(parsed)) {
      if (Array.isArray(value) && value.every((item) => typeof item === 'string')) {
        orders[agentName] = value
      }
    }
    return orders
  } catch {
    return {}
  }
}
const projectOrders = ref<Record<string, string[]>>(loadProjectOrders())
const currentProjectOrder = computed(() => projectOrders.value[agent.value] ?? [])

function setProjectOrder(dirNames: string[]) {
  projectOrders.value = { ...projectOrders.value, [agent.value]: dirNames }
  localStorage.setItem(PROJECT_ORDER_KEY, JSON.stringify(projectOrders.value))
}

function prefKey(p: ProjectInfo): string {
  return `${agent.value}::${p.dirName}`
}
function projStateOf(p: ProjectInfo): ProjState | undefined {
  return projPrefs.value[prefKey(p)]
}
function setProjState(p: ProjectInfo, state: ProjState) {
  const key = prefKey(p)
  if (projPrefs.value[key] === state) {
    delete projPrefs.value[key]
  } else {
    projPrefs.value[key] = state
  }
  projPrefs.value = { ...projPrefs.value }
  localStorage.setItem(PREFS_KEY, JSON.stringify(projPrefs.value))
}

// 缓存包括项目置顶/沉底和手动排序偏好，字节数等于其 JSON 序列化后的 UTF-8 长度。
const cacheBytes = computed(() => {
  const json = JSON.stringify({ prefs: projPrefs.value, order: projectOrders.value })
  if (json === '{"prefs":{},"order":{}}') return 0
  return new TextEncoder().encode(json).length
})

// ---------- 项目右键菜单 ----------
interface CtxMenu {
  x: number
  y: number
  project: ProjectInfo
  isGitRepo: boolean
}
const ctxMenu = ref<CtxMenu | null>(null)
function openCtxMenu(e: MouseEvent, p: ProjectInfo) {
  e.preventDefault()
  // 菜单大约 176×220，靠近视口右/下边时收回来一点，避免被截掉
  const W = 176
  const H = 220
  const x = Math.min(e.clientX, window.innerWidth - W - 8)
  const y = Math.min(e.clientY, window.innerHeight - H - 8)
  ctxMenu.value = { x, y, project: p, isGitRepo: false }
  // 「创建 Worktree」仅对 git 仓库、且自身不是 worktree 的项目显示；且仅 Claude/Codex 开放
  // （opencode/agy 按 git 仓库归属会话，worktree 会话会塌回主仓库，故整体隐藏）。git 探测
  // 是异步的，先把菜单弹出来，探测回来后再点亮该项（响应式更新，避免右键卡一下）。
  if (p.exists && !p.worktreeName && (agent.value === 'claude' || agent.value === 'codex')) {
    const target = p.displayPath
    api.gitHasRepo(target)
      .then((has) => {
        if (ctxMenu.value?.project.displayPath === target) ctxMenu.value.isGitRepo = has
      })
      .catch(() => {})
  }
}
function closeCtxMenu() {
  ctxMenu.value = null
}
function ctxToggleState(state: ProjState) {
  if (!ctxMenu.value) return
  setProjState(ctxMenu.value.project, state)
  closeCtxMenu()
}
function ctxRefresh() {
  closeCtxMenu()
  refreshAll()
}
function ctxOpenProjectFolder() {
  const p = ctxMenu.value?.project
  closeCtxMenu()
  if (!p) return
  api.revealInFinder(p.displayPath).catch((e) => notify(`${e}`, true))
}
function ctxDeleteProject() {
  const p = ctxMenu.value?.project
  closeCtxMenu()
  if (!p) return
  deleteProject(p)
}
function ctxRemoveBookmark() {
  const p = ctxMenu.value?.project
  closeCtxMenu()
  if (!p) return
  removeBookmark(p)
}
function ctxCreateWorktree() {
  const p = ctxMenu.value?.project
  closeCtxMenu()
  if (!p) return
  openWorktreeModal(p)
}
function ctxDeleteWorktree() {
  const p = ctxMenu.value?.project
  closeCtxMenu()
  if (!p) return
  deleteWorktree(p)
}

// ---------- 创建 / 删除 worktree ----------
interface WorktreeModalState {
  show: boolean
  projectPath: string
  value: string
}
const worktreeModal = ref<WorktreeModalState>({ show: false, projectPath: '', value: '' })
const creatingWorktree = ref(false)

function openWorktreeModal(p: ProjectInfo) {
  worktreeModal.value = { show: true, projectPath: p.displayPath, value: '' }
}

async function confirmCreateWorktree() {
  const m = worktreeModal.value
  if (!m.show || creatingWorktree.value) return
  const name = m.value.trim()
  if (!name) return
  creatingWorktree.value = true
  try {
    const newPath = await api.createWorktree(m.projectPath, name)
    m.show = false
    await loadProjects()
    // 后端返回的是归一化正斜杠路径；injected 条目的 displayPath 同样归一化，直接命中。
    const added = projects.value.find((p) => p.displayPath === newPath)
    if (added) {
      selectProject(added.dirName)
      nextTick(() => {
        const el = document.querySelector<HTMLElement>('.proj-item.active')
        if (el) {
          el.classList.add('flash')
          el.addEventListener('animationend', () => el.classList.remove('flash'), { once: true })
        }
      })
    }
    notify(t('toast.worktreeCreated', { name }))
  } catch (e) {
    notify(t('toast.worktreeCreateFail', { e: String(e) }), true)
  } finally {
    creatingWorktree.value = false
  }
}

// worktree 是 Claude / Codex 共享的物理目录 —— 两个 agent 都可能在里面跑过会话，各自按自己的
// 布局存 transcript。删 worktree、统计会话数都要覆盖这两个 agent。
const WORKTREE_AGENTS: Agent[] = ['claude', 'codex']

const normPath = (s: string) => s.replace(/\\/g, '/').replace(/\/+$/, '')

/** 删除彻底：codex 把内部 + 归档会话都算上，别在 worktree 移除后留下孤儿 transcript。 */
function worktreeScanOptions(a: Agent): { includeCodexInternal: boolean; includeCodexArchived: boolean } | undefined {
  return a === 'codex' ? { includeCodexInternal: true, includeCodexArchived: true } : undefined
}

/** 在某 agent 的项目列表里找到这个 worktree 路径对应的真实项目（跳过合成占位 key）。 */
function findWorktreeProjectIn(projs: ProjectInfo[], worktreePath: string): ProjectInfo | undefined {
  const target = normPath(worktreePath)
  return projs.find(
    (x) =>
      !x.dirName.startsWith('worktree:') &&
      !x.dirName.startsWith('bookmark:') &&
      normPath(x.displayPath) === target,
  )
}

/** 两个 agent 在该 worktree 路径下的会话总数之和（供删除确认框如实告知）。 */
async function countWorktreeSessions(worktreePath: string): Promise<number> {
  let total = 0
  for (const a of WORKTREE_AGENTS) {
    try {
      const projs = await api.listProjects(a, worktreeScanOptions(a))
      const proj = findWorktreeProjectIn(projs, worktreePath)
      if (proj) total += proj.sessionCount
    } catch {}
  }
  return total
}

async function deleteWorktree(p: ProjectInfo) {
  // 共享 worktree → 统计两 agent 的会话总数再弹框，让「将永久删除 N 个会话」如实反映实际删除量。
  const n = await countWorktreeSessions(p.displayPath)
  ask({
    title: t('dialog.deleteWorktree.title'),
    message: t('dialog.deleteWorktree.body', {
      name: p.worktreeName ?? shortName(p.displayPath),
      n,
    }),
    // 一次性全部删除：工作树 + 分支（不可撤销）。会话仍进回收站（app 铁律）。
    okText: t('dialog.deleteWorktree.ok'),
    danger: true,
    onOk: () => performWorktreeDelete(p),
  })
}

/** 停掉 cwd 落在这个 worktree 目录下的所有 live GUI chat 子进程（含连带的 chat view tab）。
 *  worktree 删除前必须先做：Windows 上 chat 的 codex/claude 子进程把 worktree 目录占着，
 *  不停它 `git worktree remove` / rmdir 都会 os error 32（文件被占用）。closeChat 会 await
 *  子进程真正停止，故此处 await 完再删目录才安全。 */
async function stopWorktreeChats(worktreePath: string) {
  const target = normPath(worktreePath)
  // filter 出独立数组 —— closeChat 会 splice chatSessions.value，边删边迭代原数组会漏。
  const victims = chatSessions.value.filter((c) => normPath(c.cwd) === target)
  for (const c of victims) {
    const tab = viewTabs.value.find((t) => t.type === 'chat' && t.chatSession?.uiId === c.uiId)
    if (tab) removeViewTab(tab.uiId)
    try {
      await closeChat(c.uiId)
    } catch {}
  }
}

/** 物理删除某 agent 在这个 worktree 路径下的全部会话（不进回收站，不可恢复）+ 杀掉其 tab。
 *  合成占位 key 无会话则直接空转返回。 */
async function hardDeleteWorktreeSessionsFor(a: Agent, worktreePath: string) {
  const opts = worktreeScanOptions(a)
  let proj: ProjectInfo | undefined
  try {
    proj = findWorktreeProjectIn(await api.listProjects(a, opts), worktreePath)
  } catch {
    return
  }
  if (!proj) return
  closeTabsByProject(proj.dirName)
  const all: SessionMeta[] = []
  let offset = 0
  while (true) {
    const page = await api.listSessions(a, proj.dirName, offset, 200, opts)
    all.push(...page.sessions)
    offset += page.sessions.length
    if (all.length >= page.total || page.sessions.length === 0) break
  }
  for (const s of all) {
    try {
      await api.hardDeleteSession(a, s.path)
      removeViewEverywhere(a, s.id || s.path)
    } catch {}
  }
}

const deletingWorktree = ref(false)
async function performWorktreeDelete(p: ProjectInfo) {
  // 防重入：确认弹框淡出的一瞬间 onOk 可能被连点触发两次，第二次会对已删的 worktree
  // 再跑一遍 git remove → 报错。一把锁挡掉。
  if (deletingWorktree.value) return
  deletingWorktree.value = true
  const label = p.worktreeName ?? shortName(p.displayPath)
  try {
    // 先停掉占着 worktree 目录的进程，否则 Windows 删目录会 os error 32（文件被占用）：
    //   1) live GUI chat 的 codex/claude 子进程（cwd = worktree）—— await 到真正停止；
    //   2) 内嵌 TUI / shell 的 PTY（closeTabsByProject 里 ptyKill，异步，句柄释放由后端重试兜底）。
    await stopWorktreeChats(p.displayPath)
    closeTabsByProject(p.dirName)
    // worktree 是 Claude / Codex 共享目录 → 两个 agent 在该路径下的会话都物理删除，不可恢复，
    // 否则移除工作树后会残留另一 agent 的孤儿 transcript。最后再移除工作树 + 删除分支。
    for (const a of WORKTREE_AGENTS) {
      await hardDeleteWorktreeSessionsFor(a, p.displayPath)
    }
    // 清理各 agent 的项目元数据目录（如 Claude 的 ~/.claude/projects/<encoded>/），
    // 这些目录可能含有 CLI 配置等非会话文件，hard_delete_session 不会删它们。
    await api.cleanupWorktreeProjectDirs(p.displayPath)
    await api.removeWorktree(p.displayPath)
    if (activeDir.value === p.dirName) {
      activeDir.value = null
      sessions.value = []
      setActiveViewTab(null)
    }
    await loadProjects()
    notify(t('toast.worktreeDeleted', { name: label }))
  } catch (e) {
    notify(t('toast.worktreeDeleteFail', { e: String(e) }), true)
  } finally {
    deletingWorktree.value = false
  }
}

function deleteProject(p: ProjectInfo) {
  ask({
    title: t('dialog.deleteProject.title'),
    message: t('dialog.deleteProject.body', {
      name: shortName(p.displayPath),
      n: p.sessionCount,
    }),
    okText: t('dialog.deleteProject.ok'),
    danger: true,
    onOk: async () => {
      // 在该项目从侧边栏移除前抓取起点，触发飞向回收站的弧线动画
      const srcRect = projectSourceRect(p)
      try {
        // 先刷新项目列表：TUI 运行期间 CLI 可能已在 ~/.claude/projects/ 下
        // 创建了真实项目目录，但此前的 projects.value 还没有它。刷新后
        // counterpart 才能发现真实项目，确保其会话也被一并删除。
        await loadProjects()
        // 书签和真实项目（~/.claude/projects/ 下同 displayPath 的目录）可能同时存在，
        // 且会话只存在于真实项目目录里。两边都要扫、都要删才能彻底清除。
        const counterpart = projects.value.find(
          (rp) => rp.dirName !== p.dirName && rp.displayPath === p.displayPath,
        )
        const keysToScan = [p.dirName]
        if (counterpart) keysToScan.push(counterpart.dirName)

        // 先杀 PTY，再移文件——否则 CLI 进程检测到文件消失会重建空会话。
        closeTabsByProject(p.dirName)
        if (counterpart) closeTabsByProject(counterpart.dirName)

        const all: SessionMeta[] = []
        for (const key of keysToScan) {
          let offset = 0
          while (true) {
            const page = await api.listSessions(agent.value, key, offset, 200, sessionListOptions())
            all.push(...page.sessions)
            offset += page.sessions.length
            if (all.length >= page.total || page.sessions.length === 0) break
          }
        }
        for (const s of all) {
          try {
            await api.softDeleteSession(agent.value, s.path, p.displayPath)
            removeViewEverywhere(agent.value, s.id || s.path)
          } catch {}
        }
        // 始终尝试移除书签：书签可能已被 loadProjects 合并进真实项目，
        // counterpart 在当前列表里找不到。removeBookmark 是幂等的，不存在也不会报错。
        await api.removeBookmark(agent.value, p.displayPath)
        fly({
          from: srcRect,
          to: document.querySelector<HTMLElement>('.topbar-trash-btn'),
          variant: 'trash',
        })
        if (activeDir.value === p.dirName || activeDir.value === counterpart?.dirName) {
          activeDir.value = null
          sessions.value = []
          setActiveViewTab(null)
        }
        await loadProjects()
        // 批量删除后刷新回收站，保持顶栏红点准确
        api.listTrash().then((items) => { trash.value = items }).catch(() => {})
        notify(t('toast.projDeleted'))
      } catch (e) {
        notify(t('toast.deleteFail', { e: String(e) }), true)
      }
    },
  })
}

function batchDeleteProjects(dirs: string[]) {
  if (!dirs.length) return
  const totalSessions = dirs.reduce((sum, dir) => {
    const p = projects.value.find(pp => pp.dirName === dir)
    return sum + (p?.sessionCount ?? 0)
  }, 0)
  ask({
    title: t('dialog.batchDeleteProject.title'),
    message: t('dialog.batchDeleteProject.body', { n: dirs.length, sessions: totalSessions }),
    okText: t('dialog.batchDeleteProject.ok'),
    danger: true,
    onOk: async () => {
      try {
        await loadProjects()
        for (const dir of dirs) {
          const p = projects.value.find(pp => pp.dirName === dir)
          if (!p) continue
          const counterpart = projects.value.find(
            (rp) => rp.dirName !== p.dirName && rp.displayPath === p.displayPath,
          )
          closeTabsByProject(p.dirName)
          if (counterpart) closeTabsByProject(counterpart.dirName)

          const all: SessionMeta[] = []
          const keysToScan = [p.dirName]
          if (counterpart) keysToScan.push(counterpart.dirName)
          for (const key of keysToScan) {
            let offset = 0
            while (true) {
              const page = await api.listSessions(agent.value, key, offset, 200, sessionListOptions())
              all.push(...page.sessions)
              offset += page.sessions.length
              if (all.length >= page.total || page.sessions.length === 0) break
            }
          }
          for (const s of all) {
            try {
              for (const vt of [...viewTabs.value]) {
                if (vt.session?.path === s.path) removeViewTab(vt.uiId)
              }
              await api.softDeleteSession(agent.value, s.path, p.displayPath)
              removeViewEverywhere(agent.value, s.id || s.path)
            } catch {}
          }
          await api.removeBookmark(agent.value, p.displayPath)
        }
        if (activeDir.value && dirs.includes(activeDir.value)) {
          activeDir.value = null
          sessions.value = []
          setActiveViewTab(null)
        }
        sidebarRef.value?.exitSelect()
        await loadProjects()
        api.listTrash().then((items) => { trash.value = items }).catch(() => {})
        notify(t('toast.batchProjDeleted', { n: dirs.length }))
      } catch (e) {
        notify(t('toast.deleteFail', { e: String(e) }), true)
      }
    },
  })
}

// ---------- 确认弹窗 ----------
interface ConfirmState {
  show: boolean
  title: string
  message: string
  okText: string
  danger: boolean
  onOk: () => void
  altText?: string
  onAlt?: () => void
}
const confirm = ref<ConfirmState>({
  show: false,
  title: '',
  message: '',
  okText: '',
  danger: false,
  onOk: () => {},
})
function ask(opts: Partial<ConfirmState> & { onOk: () => void }) {
  confirm.value = {
    show: true,
    title: opts.title ?? t('common.confirm'),
    message: opts.message ?? '',
    okText: opts.okText ?? t('common.ok'),
    danger: opts.danger ?? false,
    onOk: opts.onOk,
    altText: opts.altText,
    onAlt: opts.onAlt,
  }
}
function runConfirm() {
  const fn = confirm.value.onOk
  confirm.value.show = false
  fn()
}

function runAlt() {
  const fn = confirm.value.onAlt
  confirm.value.show = false
  fn?.()
}

// ---------- 重命名会话 ----------
// 等价于 Claude Code 的 `/rename` —— 后端往原 JSONL 末尾追加官方 schema 的
// 元数据行（Claude 是 custom-title，Codex 是 event_msg.thread_name_updated），
// 不动用户对话内容，CLI 端再次读取这个会话时也会看到新名字。
interface RenameState {
  show: boolean
  agent: Agent
  path: string
  id: string
  value: string
  defaultTitle: string
  /** shell tab 重命名不走后端，直接改内存中的 tab title。 */
  shellTabUiId?: number
  /** saved（懒恢复）tab 重命名：不走后端，只改 savedTabs 里的标题。 */
  savedTab?: SavedTab
  /** 全新 GUI live chat（还没有可定位的源文件）：不走后端，只改内存中的 live 标题。 */
  liveChatUiId?: number
  /** view tab 重命名（无源文件时）：只改内存标题。 */
  viewTabUiId?: number
}
const renameModal = ref<RenameState>({
  show: false,
  agent: 'claude',
  path: '',
  id: '',
  value: '',
  defaultTitle: '',
})
const renaming = ref(false)
function openRename(s: SessionMeta) {
  renameModal.value = {
    show: true,
    agent: agent.value,
    path: s.path,
    id: s.id,
    value: s.title,
    defaultTitle: s.title,
  }
}

// live chat 头部的「重命名」：
//  - 续聊（openSession 存在）：claude --resume 续写的就是源会话那个文件，直接走后端
//    rename 持久化；confirmRename 成功后会顺带把 live 标题同步过来。
//  - 全新 GUI 会话（没有可定位的源文件）：只改内存里的 live 标题（即时反映，不落盘）。
function openRenameLiveChat() {
  const c = liveChat.value
  if (!c) return
  if (openSession.value?.path) {
    openRename(openSession.value)
    return
  }
  renameModal.value = {
    show: true,
    agent: c.agent,
    path: '',
    id: c.sessionId,
    value: c.title,
    defaultTitle: c.title,
    liveChatUiId: c.uiId,
  }
}

function openRenameState(a: Agent, path: string, id: string, title: string) {
  renameModal.value = {
    show: true,
    agent: a,
    path,
    id,
    value: title,
    defaultTitle: title,
  }
}

async function confirmRename() {
  const m = renameModal.value
  if (!m.show || renaming.value) return
  const name = m.value.trim()
  if (!name || name === m.defaultTitle) {
    m.show = false
    return
  }
  if (m.shellTabUiId != null) {
    setTabTitleByUiId(m.shellTabUiId, name)
    m.show = false
    notify(t('toast.renamed'))
    saveTabState()
    return
  }
  if (m.savedTab) {
    renameSavedTab(m.savedTab.sessionPath ? m.savedTab.sessionPath : m.savedTab, name)
    m.show = false
    notify(t('toast.renamed'))
    saveTabState()
    return
  }
  if (m.viewTabUiId != null) {
    const vt = viewTabs.value.find(t => t.uiId === m.viewTabUiId)
    if (vt) {
      vt.title = name
      if (vt.chatSession) {
        vt.chatSession.title = name
        if (vt.chatSession.chatId) api.agentChatSetTitle(vt.chatSession.chatId, name)
      }
    }
    m.show = false
    notify(t('toast.renamed'))
    saveTabState()
    return
  }
  if (m.liveChatUiId != null) {
    // 全新 GUI 会话：只改内存里的 live 标题（reactive proxy，原地改即可刷新头部）。
    if (liveChat.value?.uiId === m.liveChatUiId) {
      liveChat.value.title = name
      if (liveChat.value.chatId) api.agentChatSetTitle(liveChat.value.chatId, name)
    }
    // Views 历史里这条（按 session id 记录的）新建 chat 标题也同步。
    setViewTitle(m.agent, m.id, name)
    m.show = false
    notify(t('toast.renamed'))
    return
  }
  renaming.value = true
  try {
    await api.renameSession(m.agent, m.path, name)
    const patch = (s: SessionMeta) =>
      s.path === m.path ? { ...s, title: name } : s
    sessions.value = sessions.value.map(patch)
    // 同步所有指向该 path 的 view tab 标题
    for (const vt of viewTabs.value) {
      if (vt.session?.path === m.path) {
        vt.session = { ...vt.session, title: name }
        vt.title = name
      }
      if (vt.type === 'chat' && vt.sourceSession?.path === m.path) {
        vt.sourceSession = { ...vt.sourceSession, title: name }
      }
      if (vt.type === 'chat' && vt.chatSession && vt.chatSession.sessionId === m.id) {
        vt.chatSession.title = name
        vt.title = name
        if (vt.chatSession.chatId) api.agentChatSetTitle(vt.chatSession.chatId, name)
      }
    }
    // Views 历史里那条同源 view 的标题也跟着更新（按 session id，回退 path）。
    setViewTitle(m.agent, m.id || m.path, name)
    syncTabTitleBySessionPath(m.agent, m.path, name)
    m.show = false
    notify(t('toast.renamed'))
    saveTabState()
  } catch (e) {
    notify(t('toast.renameFail', { e: String(e) }), true)
  } finally {
    renaming.value = false
  }
}

// ---------- toast ----------
const toast = ref({ show: false, msg: '', error: false })
let toastTimer: number | undefined
function notify(msg: string, error = false) {
  toast.value = { show: true, msg, error }
  clearTimeout(toastTimer)
  toastTimer = window.setTimeout(() => (toast.value.show = false), 2600)
}

function loadWindowCloseAction(): WindowCloseAction | null {
  const value = localStorage.getItem(WINDOW_CLOSE_PREF_KEY)
  return value === 'tray' || value === 'exit' ? value : null
}

async function runWindowCloseAction(action: WindowCloseAction) {
  if (windowCloseRunning.value) return
  windowCloseRunning.value = true
  windowClosePrompt.value.show = false
  if (windowClosePrompt.value.remember) {
    localStorage.setItem(WINDOW_CLOSE_PREF_KEY, action)
  }
  // 退出前显式保存一次 tab 状态：exit 路径不会触发 visibilitychange/beforeunload，
  // 不能只靠 500ms 防抖兜底（最后一次变更可能还没落）。
  saveTabState()
  try {
    if (action === 'tray') await api.windowHideToTray()
    else await api.windowExitApp()
  } catch (e) {
    windowCloseRunning.value = false
    notify(t('windowClose.actionFailed', { e: String(e) }), true)
    return
  }
  windowCloseRunning.value = false
}

function chooseWindowCloseAction(action: WindowCloseAction) {
  runWindowCloseAction(action)
}

// Rust 侧退出拦截（ExitRequested → app://before-quit）给前端的最后保存机会：
// 托盘 Quit / ⌘Q 这类不经过 runWindowCloseAction 的退出路径全靠这里兜底。
async function installBeforeQuitSave() {
  beforeQuitUnlisten = await listen('app://before-quit', () => saveTabState())
}

async function installWindowClosePrompt() {
  if (!isWindows) return
  windowCloseUnlisten = await listen('window://close-requested', () => {
    if (windowCloseRunning.value || windowClosePrompt.value.show) return
    const savedAction = loadWindowCloseAction()
    if (savedAction) {
      runWindowCloseAction(savedAction)
      return
    }
    windowClosePrompt.value = { show: true, remember: false }
  })
}

// ---------- 数据加载 ----------
function keepExistingProjectOrder(next: ProjectInfo[]): ProjectInfo[] {
  const byDirName = new Map(next.map((project) => [project.dirName, project]))
  const retained = projects.value.flatMap((project) => {
    const replacement = byDirName.get(project.dirName)
    if (!replacement) return []
    byDirName.delete(project.dirName)
    return [replacement]
  })
  return [...byDirName.values(), ...retained]
}

function applyProjects(next: ProjectInfo[], preserveExistingOrder = false) {
  projects.value = preserveExistingOrder ? keepExistingProjectOrder(next) : next
  reconcileSyntheticKeys()
}

async function loadProjects(options: { preserveExistingOrder?: boolean } = {}) {
  try {
    applyProjects(
      await api.listProjects(agent.value, sessionListOptions()),
      options.preserveExistingOrder,
    )
  } catch (e) {
    notify(t('toast.loadProjectsFail', { e: String(e) }), true)
    projects.value = []
  }
}

/** 合成 key（`bookmark:<path>` / `worktree:<path>`）前缀判定。 */
function isSyntheticKey(key: string): boolean {
  return key.startsWith('bookmark:') || key.startsWith('worktree:')
}

/**
 * 合成条目 → 真实项目的 key 迁移。
 *
 * 书签 / 空 worktree 在侧栏用合成 key 占位；一旦其路径下真的跑出会话，后端 list_projects
 * 会按 display_path 把它并入该 agent 的真实项目（dirName 变成 agent 自己的路径 key），合成
 * 条目随之消失。此时仍挂在旧 key 上的分屏布局 / pane / TUI tab / view tab 全部要迁到新
 * dirName，否则：activeDir 查错目录落欢迎页；点 List / 切回来时 currentLayout 新建空布局，
 * 刚开的会话 & List tab「凭空消失」（见用户反馈的两个 bug）。
 *
 * 不止当前 activeDir —— 后台开着的 worktree tab（用户已切到别的项目）也要迁，否则切回去落空
 * pane。故扫描所有仍在用的合成 key，逐个找真实项目并整体迁移。路径按分隔符归一化再比对：
 * Codex 记反斜杠，合成 key 用正斜杠。
 */
function reconcileSyntheticKeys() {
  const norm = (s: string) => s.replace(/\\/g, '/').replace(/\/+$/, '')
  const liveKeys = new Set(projects.value.map((p) => p.dirName))
  const stale = new Set<string>()
  const consider = (key: string | null | undefined) => {
    if (key && isSyntheticKey(key) && !liveKeys.has(key)) stale.add(key)
  }
  consider(activeDir.value)
  for (const tab of viewTabs.value) consider(tab.projectKey)
  for (const tab of tuiTabs.value) consider(tab.projectKey)

  for (const oldKey of stale) {
    const path = oldKey.slice(oldKey.indexOf(':') + 1)
    const real = projects.value.find(
      (p) => !isSyntheticKey(p.dirName) && norm(p.displayPath) === norm(path),
    )
    if (!real) continue // 真实项目还没出现（会话尚未落盘）→ 保持合成占位，下次刷新再迁。
    migrateProjectKey(oldKey, real.dirName)
    if (activeDir.value === oldKey) activeDir.value = real.dirName
  }
}

/** 把某项目所有 key 化的状态（分屏布局 / pane / TUI tab / view tab / 记忆的导航映射）从
 *  oldKey 整体迁到 newKey。合成 key 并入真实项目时调用（见 reconcileSyntheticKeys）。 */
function migrateProjectKey(oldKey: string, newKey: string) {
  if (oldKey === newKey) return
  migratePaneProjectKey(agent.value, oldKey, newKey)
  migrateViewTabsProjectKey(oldKey, newKey)
  migrateTabsProjectKey(oldKey, newKey)
  // live chat 的 projectKey 也迁：修正后续 restart/clear 传的 key，并作为 ChatView 重测虚拟列表
  // 的信号（修复 worktree 首轮渲染空白）。
  migrateChatSessionsProjectKey(oldKey, newKey)
  // 记忆的导航映射按 viewKey 存 —— 一起搬，切走再切回来才能原样恢复活跃 tab。
  const oldVK = viewKey(agent.value, oldKey)
  const newVK = viewKey(agent.value, newKey)
  const av = activeViewByProject.get(oldVK)
  if (av) { activeViewByProject.delete(oldVK); activeViewByProject.set(newVK, av) }
  const at = activeTuiByProject.get(oldVK)
  if (at) { activeTuiByProject.delete(oldVK); activeTuiByProject.set(newVK, at) }
}

// 新会话落盘 → 侧栏会话计数过期，重载项目列表（见 projectsRefresh.ts）。
//
// 难点：会话文件何时落盘因 agent 而异。Claude GUI/CLI 起会话时 transcript 立刻在盘上；Codex
// app-server 要到首轮真正产出才 flush rollout —— 首个 send（此刻已发 markProjectsDirty）之后
// 好几秒。单发一次去抖重载会赶在 Codex 落盘前跑、读到旧值且再不重试 → 计数卡住不更新（用户
// 反馈：Codex 要点一下 List 才更）。故改成「一次信号 → 退避重试若干拍」：每拍重载一次，直到项目
// 指纹（项目数 + 各项目会话数之和）相对基线变化即收工，否则打满退避序列兜底。多个信号合并进同
// 一序列。loadProjects 内部还会把并入真实项目的合成 worktree/书签 key 迁移掉。
const PROJECTS_RELOAD_BACKOFF_MS = [700, 1800, 4000, 8000]
let projectsReloadTimer = 0
let projectsReloadStep = 0
let projectsBaselineSig = ''

function projectsCountSignature(): string {
  let sum = 0
  for (const p of projects.value) sum += p.sessionCount
  return `${projects.value.length}:${sum}`
}

function scheduleProjectsReload() {
  if (projectsReloadTimer) return
  const delay = PROJECTS_RELOAD_BACKOFF_MS[Math.min(projectsReloadStep, PROJECTS_RELOAD_BACKOFF_MS.length - 1)]
  projectsReloadTimer = window.setTimeout(async () => {
    projectsReloadTimer = 0
    projectsReloadStep++
    await loadProjects({ preserveExistingOrder: true })
    // 计数已变化（新会话被计入 / 空 worktree 合成条目并入真实项目）→ 收工，别再空转重载。
    if (projectsCountSignature() !== projectsBaselineSig) return
    if (projectsReloadStep < PROJECTS_RELOAD_BACKOFF_MS.length) scheduleProjectsReload()
  }, delay)
}

watch(projectsDirty, () => {
  // 新一轮信号 → 记基线、从头退避（覆盖刚出现 / 即将落盘的会话）。
  projectsBaselineSig = projectsCountSignature()
  projectsReloadStep = 0
  scheduleProjectsReload()
})

async function addBookmarkByPath(path: string) {
  // 先刷新项目列表，避免用 stale 的列表做重复判断
  await loadProjects()
  const existing = projects.value.find(p => p.displayPath === path)
  if (existing) {
    // 已有同路径项目 → 不重复添加，直接选中它
    selectProject(existing.dirName)
    notify(t('toast.bookmarkExists'))
    return
  }
  try {
    await api.addBookmark(agent.value, path)
    await loadProjects()
    notify(t('toast.bookmarkAdded'))
    const added = projects.value.find(p => p.displayPath === path)
    if (added) {
      selectProject(added.dirName)
      nextTick(() => {
        const el = document.querySelector<HTMLElement>(`.proj-item.active`)
        if (el) {
          el.classList.add('flash')
          el.addEventListener('animationend', () => el.classList.remove('flash'), { once: true })
        }
      })
    }
  } catch (e) {
    notify(`${e}`, true)
  }
}

async function addBookmark() {
  const { open } = await import('@tauri-apps/plugin-dialog')
  const selected = await open({ directory: true, multiple: false })
  if (!selected) return
  const path = typeof selected === 'string' ? selected : selected[0]
  if (!path) return
  await addBookmarkByPath(path)
}


async function removeBookmark(p: ProjectInfo) {
  try {
    await api.removeBookmark(agent.value, p.displayPath)
    await loadProjects()
    notify(t('toast.bookmarkRemoved'))
  } catch (e) {
    notify(`${e}`, true)
  }
}

// 用户在设置里关掉了当前所处的 agent → 自动切到第一个仍可见的 agent，
// 否则界面会停在一个已隐藏、且切换栏里再也点不到的 agent 上。
watch(visibleAgents, (list) => {
  if (!list.includes(agent.value)) switchAgent(list[0])
})

// 记住「当前 agent + 当前项目」里活跃的 view tab / TUI tab，供切项目、切 agent
// 之后原样恢复。activeDir 为空（欢迎页）时无可记，直接返回。
function rememberActiveNav() {
  if (!activeDir.value) return
  const k = viewKey(agent.value, activeDir.value)
  activeViewByProject.set(k, {
    viewUiId: activeViewTabId.value,
    wasTui: activeUiId.value !== null,
  })
  const curTab = activeUiId.value !== null
    ? tuiTabs.value.find((t) => t.uiId === activeUiId.value)
    : undefined
  if (curTab) {
    activeTuiByProject.set(k, {
      uiId: curTab.uiId,
      sessionPath: curTab.sessionPath,
      ...(curTab.isShell ? { isShell: true } : {}),
    })
  } else {
    activeTuiByProject.delete(k)
  }
  persistTuiMap()
}

function switchAgent(a: Agent) {
  if (agent.value === a) return
  // 离开当前 agent 前，记下它停在哪个项目 + 该项目活跃的 tab，切回来好原样恢复。
  rememberActiveNav()
  lastDirByAgent.set(agent.value, activeDir.value)
  agent.value = a
  activeDir.value = null
  sessions.value = []
  setActiveViewTab(null)
  showTrash.value = false
  showExportHistory.value = false
  showPricing.value = false
  // 任何主区视图切换 → 把 TUI 层收起，让用户看到刚切到的视图。TUI tab 不关，
  // 用户在 TerminalStrip 里随时能切回。
  setActiveTui(null)
  closeAllSideChats()
  closeAllCodexSideChats()
  // showStats 不重置 —— 统计是 agent-scoped，切 agent 后 StatsView 自己 refetch。
  // 乐观恢复目标 agent 上次停留的项目（含其活跃 tab）：立刻 selectProject 定位过去，
  // 不等 loadProjects 回来 —— codex 的项目扫描慢，先闪一下欢迎页很明显。selectProject
  // 拉会话走 listSessions，本就不依赖项目列表；loadProjects 并行跑，回来后仅在记的项目
  // 确实已不存在时才回落欢迎页。
  const lastDir = lastDirByAgent.get(a)
  if (lastDir) selectProject(lastDir)
  loadProjects().then(() => {
    if (agent.value !== a) return
    if (activeDir.value && !projects.value.some((p) => p.dirName === activeDir.value)) {
      activeDir.value = null
      sessions.value = []
      sessionTotal.value = 0
    }
  })
}

async function selectProject(dir: string, opts: { activateTerminal?: boolean } = {}) {
  const shouldActivateTerminal = opts.activateTerminal === true
  const rememberedTui = activeTuiByProject.get(viewKey(agent.value, dir))
  const sameProject = activeDir.value === dir && !showTrash.value && !showStats.value
  if (sameProject && shouldActivateTerminal && await maybeActivateProjectTui(dir, rememberedTui)) {
    return
  }
  // 记住当前项目活跃的 view tab 和 TUI tab
  if (activeDir.value && activeDir.value !== dir) {
    rememberActiveNav()
  }
  setActiveTui(null)
  closeAllSideChats()
  closeAllCodexSideChats()
  // 再次点击当前已选中的项目：
  //   - 有 view tab → 取消 view tab 激活，回到列表
  //   - 无 view tab → 收起项目
  if (sameProject) {
    if (activeViewTab.value) {
      setActiveViewTab(null)
      return
    }
    activeDir.value = null
    sessions.value = []
    sessionTotal.value = 0
    resetSessionsToolbar()
    return
  }
  showTrash.value = false
  showStats.value = false
  showExportHistory.value = false
  showPricing.value = false
  sessionStatsTarget.value = null
  activeDir.value = dir
  recordRecent(agent.value, dir)
  // 切项目后，恢复上次活跃的 view tab（如有）
  const remembered = activeViewByProject.get(viewKey(agent.value, dir))
  const projectTabs = visibleViewTabs(agent.value, dir)
  if (remembered?.wasTui) {
    // 上次在 TUI tab → 不激活任何 view tab，后面 rememberedTui 会恢复 TUI
    setActiveViewTab(null)
  } else if (remembered?.viewUiId != null) {
    const matched = projectTabs.find(t => t.uiId === remembered.viewUiId)
    setActiveViewTab(matched ? matched.uiId : (projectTabs.length > 0 ? projectTabs[projectTabs.length - 1].uiId : null))
  } else if (projectTabs.length > 0) {
    setActiveViewTab(projectTabs[projectTabs.length - 1].uiId)
  } else {
    setActiveViewTab(null)
  }
  sessions.value = []
  sessionTotal.value = 0
  savedListScroll = 0
  resetSessionsToolbar()
  loadingList.value = true
  try {
    const page = await api.listSessions(agent.value, dir, 0, PAGE_SIZE, sessionListOptions())
    sessions.value = page.sessions
    sessionTotal.value = page.total
  } catch (e) {
    notify(t('toast.loadSessionsFail', { e: String(e) }), true)
    sessions.value = []
  } finally {
    loadingList.value = false
  }
  // 上次停在 TUI（wasTui）→ 无条件恢复那一个终端/会话 tab 的激活态（不止 sidebar 显式点击，
  // 切 agent 走的 selectProject 也要），与 view tab 恢复对称，修复「终端/会话 tab 切换后掉回 List」。
  if (shouldActivateTerminal || remembered?.wasTui) await maybeActivateProjectTui(dir, rememberedTui)
}

async function loadMore() {
  if (loadingMore.value || loadingList.value || !activeDir.value) return
  if (sessions.value.length >= sessionTotal.value) return
  loadingMore.value = true
  try {
    const page = await api.listSessions(
      agent.value,
      activeDir.value,
      sessions.value.length,
      PAGE_SIZE,
      sessionListOptions(),
    )
    sessions.value = [...sessions.value, ...page.sessions]
    sessionTotal.value = page.total
  } catch (e) {
    notify(t('toast.loadMoreFail', { e: String(e) }), true)
  } finally {
    loadingMore.value = false
  }
}

function onListScroll(scrollTop: number) {
  savedListScroll = scrollTop
}

// 一次性把当前项目剩余的会话全部拉进来。分页窗口只覆盖已滚动到的部分，
// 而搜索 / 排序需要面向整个项目才正确，故工具栏一旦被激活就补齐全量。
async function loadAllSessions() {
  if (!activeDir.value || loadingList.value || loadingMore.value) return
  if (sessions.value.length >= sessionTotal.value) return
  loadingMore.value = true
  try {
    const page = await api.listSessions(
      agent.value,
      activeDir.value,
      0,
      sessionTotal.value,
      sessionListOptions(),
    )
    sessions.value = page.sessions
    sessionTotal.value = page.total
    syncTuiTabsFromCurrentSessions()
  } catch (e) {
    notify(t('toast.loadMoreFail', { e: String(e) }), true)
  } finally {
    loadingMore.value = false
  }
}

// 工具栏从默认态切到「有筛选」时补齐全量会话；清空筛选后已加载的全量列表保留即可。
watch(sessionsFilterActive, (active) => {
  if (active) loadAllSessions()
})

function syncTuiTabsFromCurrentSessions() {
  if (!activeDir.value) return
  reconcileNewTabs(activeDir.value, sessions.value, agent.value)
  syncTabTitlesFromSessions(agent.value, activeDir.value, sessions.value)
}

function hasCurrentProjectTuiTabs(): boolean {
  if (!activeDir.value || showTrash.value || showStats.value) return false
  return tuiTabs.value.some(
    (tab) =>
      tab.agent === agent.value &&
      tab.projectKey === activeDir.value &&
      isTabProcessAlive(tab),
  )
}

async function syncTuiTitlesNow() {
  if (!activeDir.value || syncingTuiTitles || !hasCurrentProjectTuiTabs()) return
  syncingTuiTitles = true
  try {
    const page = await api.listSessions(
      agent.value,
      activeDir.value,
      0,
      Math.max(PAGE_SIZE, sessions.value.length),
      sessionListOptions(),
    )
    sessions.value = page.sessions
    sessionTotal.value = page.total
    syncTuiTabsFromCurrentSessions()
    // 内嵌 TUI 运行期间会话数增长（跑出新会话）→ 请侧栏重载计数（authoritative list_projects，
    // 口径与徽标一致、不抖动）。仅在总数相对上次同步变化时触发，避免每 4s 空转重载。
    if (lastTuiSyncedTotal >= 0 && page.total !== lastTuiSyncedTotal) markProjectsDirty()
    lastTuiSyncedTotal = page.total
  } catch {
    // 后台标题同步不能打扰正在运行的 TUI；用户手动刷新时会看到错误 toast。
  } finally {
    syncingTuiTitles = false
  }
}

async function refreshSessions() {
  if (!activeDir.value || loadingList.value) return
  loadingList.value = true
  try {
    const page = await api.listSessions(
      agent.value,
      activeDir.value,
      0,
      Math.max(PAGE_SIZE, sessions.value.length),
      sessionListOptions(),
    )
    sessions.value = page.sessions
    sessionTotal.value = page.total
    syncTuiTabsFromCurrentSessions()
  } catch (e) {
    notify(t('toast.loadSessionsFail', { e: String(e) }), true)
  } finally {
    loadingList.value = false
  }
}

// 打开统计概览：和回收站 / 会话视图互斥；再点一次同一按钮收起。
// 数据加载自身在 StatsView 里完成，App 这一层只切顶层状态。
function openStats() {
  setActiveTui(null)
  if (showStats.value) {
    showStats.value = false
    sessionStatsTarget.value = null
    return
  }
  showStats.value = true
  // 全局统计模式：清掉单会话目标，避免上次留下来。
  sessionStatsTarget.value = null
  showTrash.value = false
  showExportHistory.value = false
  showPricing.value = false
  activeDir.value = null
  setActiveViewTab(null)
  sessions.value = []
  sessionTotal.value = 0
}

async function loadTrash() {
  setActiveTui(null)
  showTrash.value = true
  showStats.value = false
  showExportHistory.value = false
  showPricing.value = false
  sessionStatsTarget.value = null
  activeDir.value = null
  setActiveViewTab(null)
  resetTrashToolbar()
  loadingList.value = true
  try {
    trash.value = await api.listTrash()
  } catch (e) {
    notify(t('toast.loadTrashFail', { e: String(e) }), true)
    trash.value = []
  } finally {
    loadingList.value = false
  }
}

async function openChat(s: SessionMeta) {
  setActiveTui(null)
  openTrashItem.value = null
  // 已有同 path 的 tab（read 或 chat）→ 切过去；chat tab 就地转回 read
  const existing = findViewTab(t =>
    (t.type === 'session' && t.session?.path === s.path) ||
    (t.type === 'chat' && (t.sourceSession?.path === s.path || t.chatSession?.sessionId === s.id))
  )
  if (existing) {
    if (existing.type === 'chat') {
      existing.type = 'session'
      existing.session = s
    }
    existing.loadingMsgs = true
    api.readSession(agent.value, s.path).then(msgs => {
      existing.msgs = msgs
      existing.loadingMsgs = false
    }).catch(() => { existing.loadingMsgs = false })
    setActiveViewTab(existing.uiId)
    return
  }
  const tab = createViewTab({
    type: 'session',
    agent: agent.value,
    projectKey: activeDir.value ?? '',
    title: s.title,
    session: s,
    loadingMsgs: true,
  })
  await nextTick()
  try {
    tab.msgs = await api.readSession(agent.value, s.path)
    try {
      await api.watchSession(agent.value, s.path)
      const ageMs = Date.now() - (s.modified ?? 0)
      if (ageMs >= 0 && ageMs < LIVE_FRESH_MS) {
        tab.liveTailing = true
        tab.liveFadeTimer = window.setTimeout(() => {
          tab.liveTailing = false
        }, LIVE_STALE_MS)
      }
    } catch {}
  } catch (e) {
    notify(t('toast.readFail', { e: String(e) }), true)
    removeViewTab(tab.uiId)
    return
  } finally {
    tab.loadingMsgs = false
  }
  if (activeDir.value) {
    recordView({ agent: agent.value, dir: activeDir.value, session: s, mode: 'read' })
  }
}

// 导出历史视图入口（侧栏按钮）—— 和回收站 / 统计 / 价格互斥；再点一次同一按钮收起。
function openExportHistory() {
  setActiveTui(null)
  if (showExportHistory.value) {
    showExportHistory.value = false
    return
  }
  showExportHistory.value = true
  showTrash.value = false
  showStats.value = false
  showPricing.value = false
  sessionStatsTarget.value = null
  activeDir.value = null
  setActiveViewTab(null)
  sessions.value = []
  sessionTotal.value = 0
}

// 价格视图入口（顶栏 More 菜单）—— 和回收站 / 统计 / 历史互斥；再点一次收起。
function openPricing() {
  setActiveTui(null)
  if (showPricing.value) {
    showPricing.value = false
    return
  }
  showPricing.value = true
  showTrash.value = false
  showStats.value = false
  showExportHistory.value = false
  sessionStatsTarget.value = null
  activeDir.value = null
  setActiveViewTab(null)
  sessions.value = []
  sessionTotal.value = 0
}

// 点开导出历史里的一条 —— 用平时查看会话的同一套逻辑（read_session）打开**原始**
// transcript，和落盘的导出文件无关。沿用回收站的跨 agent 打开机制：用 importedAgent
// 记录这条记录的 agent，不切换整个侧栏。原始文件已被移动 / 删除时后端抛错 —— 仅提示，
// 不自动删历史（可能只是临时不可达，让用户在列表里手动移除）。showExportHistory 保持
// true，关闭会话详情时自动回到历史列表（与回收站一致）。
async function openHistorySession(rec: ExportRecord) {
  setActiveTui(null)
  openTrashItem.value = null
  const meta: SessionMeta = {
    id: rec.sessionId,
    fileName: shortName(rec.path),
    path: rec.path,
    title: rec.title,
    cwd: rec.cwd,
    modified: 0,
    size: 0,
    messageCount: 0,
    codexAppListScanned: 0,
    codexAppFirstPageSize: 0,
    codexAppFirstPagePosition: 0,
    codexInternal: false,
    codexArchived: false,
  }
  const tab = createViewTab({
    type: 'session',
    agent: rec.agent,
    projectKey: activeDir.value ?? '',
    title: rec.title,
    session: meta,
    importedAgent: rec.agent,
    loadingMsgs: true,
  })
  try {
    tab.msgs = await api.readSession(rec.agent, rec.path)
  } catch (e) {
    notify(t('toast.readFail', { e: String(e) }), true)
    removeViewTab(tab.uiId)
  } finally {
    tab.loadingMsgs = false
  }
}

// 会话统计入口：从 ChatTopbar 的统计按钮触发，跳到独立统计页面。
// 走和全局统计一样的 SSE 推送通道，主聊天页面保持轻量 —— 后端 scope 拼成
// `session:<agent>:<path>`，由 stats::stream::run_session_scope 单独处理。
function openSessionStats() {
  if (!openSession.value) return
  const sess = openSession.value
  sessionStatsTarget.value = {
    agent: chatAgent.value,
    path: sess.path,
    title: sess.title,
  }
  sessionStatsFrom.value = 'chat'
  showStats.value = true
  showTrash.value = false
  // 注意：不清空 openSession / activeDir —— 用户关闭统计页时回到原会话上下文。
}

// 从全局 StatsView 的 Top Sessions 列表跳进单会话统计。和上面的区别只在 "from"，
// 决定返回时回到全局统计而不是某个聊天。
function openSessionStatsFromGlobal(a: Agent, path: string, title?: string) {
  sessionStatsTarget.value = { agent: a, path, title }
  sessionStatsFrom.value = 'global'
  // showStats 保持 true —— 我们仍然在 StatsView 里，只是 props.session 变了，
  // StatsView 内部的 watch(props.session?.path) 会重启流。
}

function closeStats() {
  // 单会话模式下点「返回」：根据进入路径决定回到哪
  if (sessionStatsTarget.value) {
    if (sessionStatsFrom.value === 'global') {
      // 仍留在 StatsView，但切回全局视图
      sessionStatsTarget.value = null
      sessionStatsFrom.value = null
      return
    }
    // 'chat' / null：完整关闭，openSession 还在 → 自动回落到 ChatView
  }
  showStats.value = false
  sessionStatsTarget.value = null
  sessionStatsFrom.value = null
}

// 在回收站里打开一个已删除会话的只读详情。回收站 JSONL 仍是完整文件，
// 直接按 trashPath 解析即可；详情页通过 openTrashItem 进入「回收站模式」。
async function openTrashSession(item: TrashItem) {
  setActiveTui(null)
  openTrashItem.value = item
  const meta: SessionMeta = {
    id: '',
    fileName: item.trashFile,
    path: item.trashPath,
    title: item.title,
    modified: item.deletedAt,
    size: item.size,
    messageCount: 0,
    codexAppListRank: null,
    codexAppListScanned: 0,
    codexAppFirstPageSize: 50,
    codexAppFirstPagePosition: 0,
    codexInternal: false,
    codexArchived: false,
  }
  const tab = createViewTab({
    type: 'session',
    agent: item.agent,
    projectKey: activeDir.value ?? '__trash__',
    title: item.title,
    session: meta,
    trashAgent: item.agent,
    loadingMsgs: true,
  })
  try {
    tab.msgs = await api.readSession(item.agent, item.trashPath)
  } catch (e) {
    notify(t('toast.readFail', { e: String(e) }), true)
    removeViewTab(tab.uiId)
  } finally {
    tab.loadingMsgs = false
  }
}

// ---------- 删除 / 恢复 ----------
// 删除起点矩形：列表里取对应 .session-card，详情页取聊天顶栏的删除按钮。
function deleteSourceRect(s: SessionMeta): DOMRect | null {
  const cards = document.querySelectorAll<HTMLElement>('.session-card')
  for (const c of cards) {
    if (c.dataset.path === s.path) return c.getBoundingClientRect()
  }
  const chatDel = document.querySelector<HTMLElement>('.chat-head .icon-btn.danger')
  return chatDel ? chatDel.getBoundingClientRect() : null
}

// 删除项目起点矩形：侧边栏里该项目的行。
function projectSourceRect(p: ProjectInfo): DOMRect | null {
  for (const el of document.querySelectorAll<HTMLElement>('.proj-item')) {
    if (el.dataset.path === p.displayPath) return el.getBoundingClientRect()
  }
  return null
}

// 恢复起点矩形：回收站列表里对应的 .session-card（按 trashFile 匹配），
// 在回收站详情页里恢复时没有列表卡片，改用顶栏的恢复按钮作起点。
function restoreSourceRect(item: TrashItem): DOMRect | null {
  for (const c of document.querySelectorAll<HTMLElement>('.session-card')) {
    if (c.dataset.trash === item.trashFile) return c.getBoundingClientRect()
  }
  const headBtn = document.querySelector<HTMLElement>('.chat-head .chat-restore-btn')
  return headBtn ? headBtn.getBoundingClientRect() : null
}

// 恢复落点：侧边栏里该会话所属项目的行（trashFile 的 projectLabel == 项目 displayPath）；
// 项目此刻尚未出现在侧边栏时退回到整个项目列表容器。
function restoreTarget(item: TrashItem): HTMLElement | null {
  for (const el of document.querySelectorAll<HTMLElement>('.proj-item')) {
    if (el.dataset.path === item.projectLabel) return el
  }
  return document.querySelector<HTMLElement>('.proj-list')
}

function deleteSession(s: SessionMeta) {
  const fromChat = openSession.value?.path === s.path
  const deleteAgent = fromChat ? chatAgent.value : agent.value
  const deleteKey = s.id || s.path
  const afterDelete = async () => {
    closeTabBySessionPath(s.path)
    sessions.value = sessions.value.filter((x) => x.path !== s.path)
    sessionTotal.value = Math.max(0, sessionTotal.value - 1)
    // 关闭指向被删会话的所有 view tab
    for (const vt of [...viewTabs.value]) {
      if (vt.session?.path === s.path) removeViewTab(vt.uiId)
      if (vt.type === 'chat' && vt.sourceSession?.path === s.path) closeLiveChat(vt.uiId)
    }
    removeViewEverywhere(deleteAgent, deleteKey)
    if (sessions.value.length === 0 && activeProject.value) {
      const proj = activeProject.value
      closeTabsByProject(proj.dirName)
      if (proj.bookmarked || proj.dirName.startsWith('bookmark:')) {
        await api.removeBookmark(agent.value, proj.displayPath)
      }
      activeDir.value = null
    }
    await loadProjects()
  }
  ask({
    title: t('dialog.delete.title'),
    message: t('dialog.delete.body', { title: s.title }),
    okText: t('dialog.delete.ok'),
    altText: t('dialog.delete.permOk'),
    onAlt: async () => {
      try {
        const label = activeProject.value?.displayPath ?? s.cwd ?? ''
        closeTabBySessionPath(s.path)
        await api.softDeleteSession(deleteAgent, s.path, label)
        const trashItems = await api.listTrash()
        const match = trashItems.find(item => item.originalPath === s.path)
        if (match) await api.permanentDeleteTrash(match.trashFile)
        await afterDelete()
        notify(t('toast.permDeleted'))
      } catch (e) {
        notify(t('toast.deleteFail', { e: String(e) }), true)
      }
    },
    onOk: async () => {
      // 在移除该行之前抓取起点，触发飞向回收站的弧线动画
      const srcRect = deleteSourceRect(s)
      // 从聊天页删除时，会话可能来自「导出历史」（跨 agent，且 activeProject 为空）——
      // 此时用会话自身的 agent / cwd，而不是侧栏当前 agent / 项目，否则回收站条目
      // 的归属项目会变成空（显示「—」）甚至 agent 标错。
      const label = activeProject.value?.displayPath ?? s.cwd ?? ''
      try {
        closeTabBySessionPath(s.path)
        await api.softDeleteSession(deleteAgent, s.path, label)
        fly({
          from: srcRect,
          to: document.querySelector<HTMLElement>('.topbar-trash-btn'),
          variant: 'trash',
        })
        await afterDelete()
        api.listTrash().then((items) => { trash.value = items }).catch(() => {})
        notify(t('toast.moved'))
      } catch (e) {
        notify(t('toast.deleteFail', { e: String(e) }), true)
      }
    },
  })
}

function restore(item: TrashItem) {
  ask({
    title: t('dialog.restore.title'),
    message: t('dialog.restore.body', { title: item.title }),
    okText: t('dialog.restore.ok'),
    onOk: async () => {
      // 在该行被移除前抓取起点与落点，触发飞回侧边栏项目列表的弧线动画
      const srcRect = restoreSourceRect(item)
      try {
        await api.restoreSession(item.trashFile)
        trash.value = trash.value.filter((x) => x.trashFile !== item.trashFile)
        if (openTrashItem.value?.trashFile === item.trashFile) {
          setActiveViewTab(null)
        }
        await loadProjects()
        await nextTick()
        const target = restoreTarget(item)
        fly({ from: srcRect, to: target, variant: 'restore' })
        notify(t('toast.restored'))
      } catch (e) {
        notify(t('toast.restoreFail', { e: String(e) }), true)
      }
    },
  })
}

function permanentDelete(item: TrashItem) {
  ask({
    title: t('dialog.perm.title'),
    message: t('dialog.perm.body', { title: item.title }),
    okText: t('dialog.perm.ok'),
    danger: true,
    onOk: async () => {
      try {
        await api.permanentDeleteTrash(item.trashFile)
        trash.value = trash.value.filter((x) => x.trashFile !== item.trashFile)
        notify(t('toast.permDeleted'))
      } catch (e) {
        notify(t('toast.deleteFail', { e: String(e) }), true)
      }
    },
  })
}

// 批量恢复：恢复 trashToolbar 里勾选的会话。失败项跳过，只从 trash 移除成功项。
function batchRestore() {
  const keys = new Set(selectedTrash.value)
  const items = trash.value.filter((x) => keys.has(x.trashFile))
  if (!items.length) return
  ask({
    title: t('dialog.batchRestore.title'),
    message: t('dialog.batchRestore.body', { n: items.length }),
    okText: t('dialog.batchRestore.ok'),
    onOk: async () => {
      const srcRect = restoreSourceRect(items[0])
      const restored = new Set<string>()
      const errors: string[] = []
      for (const it of items) {
        try {
          await api.restoreSession(it.trashFile)
          restored.add(it.trashFile)
        } catch (e) {
          errors.push(`${it.title}: ${e}`)
        }
      }
      trash.value = trash.value.filter((x) => !restored.has(x.trashFile))
      exitSelectMode()
      await loadProjects()
      if (restored.size) {
        await nextTick()
        const target = restoreTarget(items[0])
        fly({ from: srcRect, to: target, variant: 'restore' })
      }
      if (errors.length) {
        notify(errors.join('; '), true)
      } else {
        notify(t('toast.batchRestored', { n: restored.size }))
      }
    },
  })
}

function batchPermanentDelete() {
  const keys = new Set(selectedTrash.value)
  const items = trash.value.filter((x) => keys.has(x.trashFile))
  if (!items.length) return
  ask({
    title: t('dialog.batchPerm.title'),
    message: t('dialog.batchPerm.body', { n: items.length }),
    okText: t('dialog.batchPerm.ok'),
    danger: true,
    onOk: async () => {
      let count = 0
      for (const it of items) {
        try {
          await api.permanentDeleteTrash(it.trashFile)
          count++
        } catch { /* skip */ }
      }
      trash.value = trash.value.filter((x) => !keys.has(x.trashFile))
      exitSelectMode()
      notify(t('toast.batchPermDeleted', { n: count }))
    },
  })
}

// 批量删除：把会话列表里勾选的会话一并 soft-delete 进回收站。失败项跳过，
// 不重置滚动；单条删除的弧线动画在此处一并跳过（一次性 N 个抛物线太喧闹）。
function batchDeleteSessions() {
  const keys = new Set(selectedSessions.value)
  const items = sessions.value.filter((s) => keys.has(s.path))
  if (!items.length) return
  ask({
    title: t('dialog.batchDelete.title'),
    message: t('dialog.batchDelete.body', { n: items.length }),
    okText: t('dialog.batchDelete.ok'),
    danger: true,
    onOk: async () => {
      const dir = activeProject.value?.displayPath ?? ''
      const srcRect = deleteSourceRect(items[0])
      for (const s of items) closeTabBySessionPath(s.path)
      const deleted = new Set<string>()
      for (const s of items) {
        try {
          await api.softDeleteSession(agent.value, s.path, dir)
          removeViewEverywhere(agent.value, s.id || s.path)
          deleted.add(s.path)
        } catch {
          /* 跳过失败项，继续删除其余 */
        }
      }
      if (deleted.size) {
        fly({
          from: srcRect,
          to: document.querySelector<HTMLElement>('.topbar-trash-btn'),
          variant: 'trash',
        })
      }
      sessions.value = sessions.value.filter((x) => !deleted.has(x.path))
      sessionTotal.value = Math.max(0, sessionTotal.value - deleted.size)
      for (const vt of [...viewTabs.value]) {
        if (vt.session && deleted.has(vt.session.path)) removeViewTab(vt.uiId)
      }
      if (sessions.value.length === 0 && activeProject.value) {
        const p = activeProject.value
        closeTabsByProject(p.dirName)
        if (p.bookmarked || p.dirName.startsWith('bookmark:')) {
          await api.removeBookmark(agent.value, p.displayPath)
        }
        activeDir.value = null
      }
      exitSessionSelectMode()
      await loadProjects()
      api.listTrash().then((items) => { trash.value = items }).catch(() => {})
      notify(t('toast.batchDeleted', { n: deleted.size }))
    },
  })
}

// 批量导出：让用户挑一个目标目录，把勾选的会话一次性写成 MD / HTML 文件。
// 失败项跳过，结尾给一个汇总 toast。逐个 readSession 是简单可控的做法
// （会话数量本就不会很大），可以接受。
async function batchExportSessions(kind: ExportKind) {
  const keys = new Set(selectedSessions.value)
  const items = sessions.value.filter((s) => keys.has(s.path))
  if (!items.length) return
  let parent: string | null = null
  try {
    parent = await pickExportDir()
  } catch (e) {
    notify(t('toast.batchExportFail', { e: String(e) }), true)
    return
  }
  if (!parent) return
  // 在用户选的目录里按约定再开一个子目录：`export-YYYYMMDD-HHMMSS-<kind>/`。
  // 这样多次批量导出不会互相覆盖，文件夹名一眼就能看出是什么时候、哪种格式的导出。
  // write_file 会自动 create_dir_all 父目录，不需要单独再发一次"建目录"命令。
  const dir = `${parent}/${batchExportFolderName(kind)}`
  let ok = 0
  let lastPath = ''
  for (const s of items) {
    try {
      const msgs = await api.readSession(agent.value, s.path)
      const fn =
        kind === 'md'
          ? exportMarkdownToDir
          : kind === 'json'
            ? exportJsonToDir
            : exportHtmlToDir
      lastPath = await fn(s, msgs, agent.value, dir)
      recordExport({ path: s.path, title: s.title, agent: agent.value, sessionId: s.id, cwd: s.cwd, exportedAt: Date.now() })
      ok++
    } catch {
      /* 跳过失败项，继续导出其余 */
    }
  }
  exitSessionSelectMode()
  if (ok > 0) {
    notify(t('toast.batchExported', { n: ok, dir }))
    if (lastPath) api.revealInFinder(lastPath).catch(() => {})
  } else {
    notify(t('toast.batchExportFail', { e: t('toast.batchExportNone') }), true)
  }
}

function clearTrash() {
  if (!trash.value.length) return
  ask({
    title: t('dialog.empty.title'),
    message: t('dialog.empty.body', { n: trash.value.length }),
    okText: t('dialog.empty.ok'),
    danger: true,
    onOk: async () => {
      try {
        await api.emptyTrash()
        trash.value = []
        exitSelectMode()
        notify(t('toast.trashEmptied'))
      } catch (e) {
        notify(t('toast.emptyFail', { e: String(e) }), true)
      }
    },
  })
}

async function reveal(path: string) {
  try {
    await api.revealInFinder(path)
  } catch (e) {
    notify(`${e}`, true)
  }
}

function exportFn(kind: ExportKind) {
  return kind === 'md' ? exportMarkdown : kind === 'json' ? exportJson : exportHtml
}

function getHiddenKeys(sessionPath: string): string[] {
  try {
    const raw = localStorage.getItem(`hidden:${sessionPath}`)
    return raw ? JSON.parse(raw) : []
  } catch { return [] }
}

async function exportSession(kind: ExportKind) {
  if (!openSession.value) return
  const s = openSession.value
  const a = chatAgent.value
  try {
    const hiddenKeys = kind === 'html' ? getHiddenKeys(s.path) : undefined
    const path = await exportFn(kind)(s, chatMsgs.value, a, hiddenKeys)
    // 用户在 Save As 对话框点了取消时返回 null —— 静默放弃
    if (!path) return
    recordExport({ path: s.path, title: s.title, agent: a, sessionId: s.id, cwd: s.cwd, exportedAt: Date.now() })
    notify(t('toast.exported', { path }))
    api.revealInFinder(path).catch(() => {})
  } catch (e) {
    notify(t('toast.exportFail', { e: String(e) }), true)
  }
}

// 列表里直接导出某个会话：不打开会话，临时把消息读出来即可。
async function exportFromList(s: SessionMeta, kind: ExportKind) {
  try {
    const msgs = await api.readSession(agent.value, s.path)
    const path = await exportFn(kind)(s, msgs, agent.value)
    if (!path) return
    recordExport({ path: s.path, title: s.title, agent: agent.value, sessionId: s.id, cwd: s.cwd, exportedAt: Date.now() })
    notify(t('toast.exported', { path }))
    api.revealInFinder(path).catch(() => {})
  } catch (e) {
    notify(t('toast.exportFail', { e: String(e) }), true)
  }
}

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text)
    notify(t('toast.copied'))
  } catch (e) {
    notify(t('toast.copyFail', { e: String(e) }), true)
  }
}

// （之前还有一个 `resume()` 走外部 Terminal.app 的版本；现在 ChatView / SessionsView
// 的 Resume 全部统一到窗口内 TUI tab，对应的 api.resumeSession + lib.rs::resume_session
// 后端命令仍保留，便于以后真要给"在外部 Terminal 打开"加按钮时直接复用。）

// ---------- TerminalStrip 的 List / View 切换 ----------
// List → 关闭当前会话 + 退出 TUI（落回 SessionsView）
async function onTuiListClick() {
  setActiveTui(null)
  setActiveViewTab(null)
  if (activeDir.value) {
    await loadProjects()
    await refreshSessions()
  }
}

function startTuiTitleSyncTimer() {
  window.clearInterval(tuiTitleSyncTimer)
  tuiTitleSyncTimer = window.setInterval(() => {
    syncTuiTitlesNow()
  }, TUI_TITLE_SYNC_INTERVAL_MS)
}
function onTuiViewTabClick(uiId: number) {
  setActiveTui(null)
  setActiveViewTab(uiId)
}

async function onTuiViewClose(tabUiId: number) {
  const id = tabUiId
  if (!id) return
  const tab = viewTabs.value.find(t => t.uiId === id)
  if (!tab) return
  if (tab.type === 'chat') closeLiveChat(id)
  else removeViewTab(id)
  if (activeUiId.value === null && !activeViewTab.value && activeDir.value && !showTrash.value && !showStats.value) {
    void Promise.all([loadProjects(), refreshSessions()])
  }
}

function onViewCloseOthers(vt: ViewTab) {
  const others = viewTabs.value.filter(t => t.type === vt.type && t.uiId !== vt.uiId && t.agent === vt.agent && t.projectKey === vt.projectKey)
  for (const t of others) {
    if (t.type === 'chat') closeLiveChat(t.uiId)
    else removeViewTab(t.uiId)
  }
}

function onViewCloseProject(type: 'session' | 'chat' | 'git') {
  const targets = viewTabs.value.filter(t => t.type === type && t.agent === agent.value && t.projectKey === (activeDir.value ?? ''))
  for (const t of [...targets]) {
    if (t.type === 'chat') closeLiveChat(t.uiId)
    else removeViewTab(t.uiId)
  }
}

function onViewRename(vt: ViewTab) {
  const session = vt.session ?? vt.sourceSession
  if (session?.path) {
    openRenameState(vt.agent, session.path, session.id ?? '', vt.title)
  } else {
    renameModal.value = {
      show: true,
      agent: vt.agent,
      path: '',
      id: '',
      value: vt.title,
      defaultTitle: vt.title,
      viewTabUiId: vt.uiId,
    }
  }
}

async function onCloseOthersAll(keepUiId: number, keepKind: 'tui' | 'view') {
  const pk = activeDir.value ?? ''
  // 关闭所有 view tabs except the kept one
  for (const t of [...viewTabs.value]) {
    if (keepKind === 'view' && t.uiId === keepUiId) continue
    if (t.type === 'chat') closeLiveChat(t.uiId)
    else removeViewTab(t.uiId)
  }
  // 关闭所有终端 tabs except the kept one
  const visible = tuiTabs.value.filter(t => t.agent === agent.value && t.projectKey === pk)
  for (const t of visible) {
    if (keepKind === 'tui' && t.uiId === keepUiId) continue
    closeTab(t.uiId)
  }
  // 关闭所有 saved tabs
  const visSaved = savedTabs.value.filter(s => s.agent === agent.value && s.projectKey === pk)
  for (const s of visSaved) removeSavedTab(s.sessionPath ? s.sessionPath : s)
  void Promise.all([loadProjects(), refreshSessions()])
}

async function onCloseAll() {
  // 关闭所有 view tabs (read + chat)
  for (const t of [...viewTabs.value]) {
    if (t.type === 'chat') closeLiveChat(t.uiId)
    else removeViewTab(t.uiId)
  }
  // 关闭所有终端 tabs
  const visible = tuiTabs.value.filter(t => t.agent === agent.value && t.projectKey === (activeDir.value ?? ''))
  for (const t of visible) closeTab(t.uiId)
  // 关闭所有 saved tabs
  const visSaved = savedTabs.value.filter(s => s.agent === agent.value && s.projectKey === (activeDir.value ?? ''))
  for (const s of visSaved) removeSavedTab(s.sessionPath ? s.sessionPath : s)
  void Promise.all([loadProjects(), refreshSessions()])
}

// 关闭当前活跃的 view/chat tab
async function closeActiveViewTab() {
  if (!activeViewTab.value) return
  await onTuiViewClose(activeViewTab.value.uiId)
}

// PTY tab 被手动关闭（× 按钮）后，若 TUI 层已空（无更多 tab），刷新数据，
// 确保 CLI 新建的会话出现在列表里。注意：不再清空 openSession —— View tab 由它自己的
// × 手动关闭，关掉终端 tab 不该让聊天详情消失（落回 View 即可）。
function onTuiTabClosed() {
  if (activeUiId.value !== null) return
  if (!activeDir.value || showTrash.value || showStats.value) return
  void Promise.all([loadProjects(), refreshSessions()])
}

function closeActiveTab() {
  const tab = currentActiveTab()
  if (tab) {
    closeTab(tab.uiId)
    onTuiTabClosed()
  } else if (activeViewTab.value) {
    onTuiViewClose(activeViewTab.value.uiId)
  }
}

function renameActiveTab() {
  const tab = currentActiveTab()
  if (tab) {
    openRenameFromTuiTab(tab)
  }
}

async function openRenameFromTuiTab(tab: TerminalTab) {
  if (tab.isShell) {
    renameModal.value = {
      show: true,
      agent: tab.agent,
      path: '',
      id: '',
      value: tab.title,
      defaultTitle: tab.title,
      shellTabUiId: tab.uiId,
    }
    return
  }
  if (!tab.sessionPath) {
    await syncTuiTitlesNow()
  }
  if (!tab.sessionPath) {
    renameModal.value = {
      show: true,
      agent: tab.agent,
      path: '',
      id: '',
      value: tab.title,
      defaultTitle: tab.title,
      shellTabUiId: tab.uiId,
    }
    return
  }
  openRenameState(tab.agent, tab.sessionPath, tab.sessionId, tab.title)
}

// saved（懒恢复）tab 重命名：placeholder 还没水合，没有 live tab / 后端会话可改，
// 只把弹窗指向这条 saved entry，确认后 renameSavedTab 改内存标题并持久化。
function openRenameFromSavedTab(saved: SavedTab) {
  renameModal.value = {
    show: true,
    agent: saved.agent,
    path: '',
    id: '',
    value: saved.title,
    defaultTitle: saved.title,
    savedTab: saved,
  }
}

// ---------- GUI chat（程序化聊天）tab 模式 ----------
// liveChat / openSession 现在是从 activeViewTab 派生的 computed（见上方定义），
// 不再是单一 ref。每个 chat 进程对应一个 ViewTab，多 chat 可并存。
const liveChatSourceSession = computed<SessionMeta | null>(() => {
  const tab = activeViewTab.value
  if (!tab || tab.type !== 'chat') return null
  return tab.sourceSession
})

/** 给 ChatView 的 session prop 造一个合成 SessionMeta（live 模式没有真正的列表条目）。 */
const liveChatMeta = computed<SessionMeta>(() => {
  const c = liveChat.value
  const source = liveChatSourceSession.value
  return {
    id: c?.sessionId ?? '',
    fileName: source?.fileName ?? '',
    path: source?.path ?? '',
    title: c?.title ?? t('list.action.newSessionGui'),
    cwd: source?.cwd ?? c?.cwd,
    created: c?.createdAt,
    modified: source?.modified ?? 0,
    size: source?.size ?? 0,
    messageCount: source?.messageCount ?? c?.msgs.length ?? 0,
    codexAppListRank: null,
    codexAppListScanned: 0,
    codexAppFirstPageSize: 0,
    codexAppFirstPagePosition: 0,
    codexInternal: false,
    codexArchived: false,
  }
})

// 标题清洗：对齐后端 util.rs::clean_title —— 去 <…> 标签、压空白、截断 100 字。
function cleanChatTitle(raw: string): string {
  const trimmed = raw.trim()
  if (trimmed.startsWith('Caveat:')) return ''
  let out = ''
  let depth = 0
  for (const ch of trimmed) {
    if (ch === '<') depth++
    else if (ch === '>' && depth > 0) depth--
    else if (depth === 0) out += ch
  }
  return out.split(/\s+/).filter(Boolean).join(' ').slice(0, 100)
}
// 新建 GUI 会话的标题派生：用第一条「真正的」用户消息文本（对齐会话列表的 first_user_title）。
function deriveFirstUserTitle(c: ChatSession): string {
  for (const m of c.msgs) {
    if (m.role === 'user' && !m.sidechain && !m.metaKind) {
      const txt = m.blocks
        .filter((b) => b.kind === 'text' && b.text)
        .map((b) => b.text as string)
        .join(' ')
      const clean = cleanChatTitle(txt)
      if (clean) return clean
    }
  }
  return ''
}

// chat tab 发出第一条消息后，把占位标题派生成消息内容标题。
// 遍历所有 chat tab 而非只看 active，确保后台 tab 也能派生标题。
watch(
  () => viewTabs.value.filter(t => t.type === 'chat').map(t => t.chatSession?.msgs.length ?? 0).join(','),
  () => {
    for (const tab of viewTabs.value) {
      if (tab.type !== 'chat' || !tab.chatSession) continue
      if (tab.chatSession.title !== t('list.action.newSessionGui')) continue
      const derived = deriveFirstUserTitle(tab.chatSession)
      if (derived) {
        tab.chatSession.title = derived
        tab.title = derived
        if (tab.chatSession.chatId) api.agentChatSetTitle(tab.chatSession.chatId, derived)
      }
    }
  },
)

// chat tab sessionId / 标题变化 → 登记 Views 历史
watch(
  () =>
    liveChat.value
      ? `${liveChat.value.sessionId} ${liveChat.value.title} ${liveChatSourceSession.value?.path ?? ''}`
      : '',
  () => {
    const c = liveChat.value
    if (!c || !c.sessionId || !activeDir.value) return
    recordView({
      agent: c.agent,
      dir: activeDir.value,
      session: liveChatMeta.value,
      mode: 'chat',
    })
  },
)

/** 关闭 chat tab（停子进程 + 移除 tab）。 */
function closeLiveChat(tabUiId?: number) {
  const id = tabUiId ?? activeViewTab.value?.uiId
  if (!id) return
  const tab = viewTabs.value.find(t => t.uiId === id)
  if (!tab || tab.type !== 'chat') return
  if (tab.chatSession) void closeChat(tab.chatSession.uiId)
  removeViewTab(id)
}

function switchLiveChatToRead() {
  const tab = activeViewTab.value
  if (!tab || tab.type !== 'chat') return
  const source = tab.sourceSession
  if (!source) return
  tab.type = 'session'
  tab.session = source
  tab.loadingMsgs = true
  api.readSession(agent.value, source.path).then(msgs => {
    tab.msgs = msgs
    tab.loadingMsgs = false
  }).catch(() => { tab.loadingMsgs = false })
}

/** 启动一个 live GUI chat，创建为独立的 chat ViewTab。 */
async function startLiveChat(opts: {
  cwd: string
  projectKey: string
  agent: Agent
  sessionId?: string
  title: string
  created?: string
  preloadMsgs?: Msg[]
  initialUsage?: UsageSummary
}) {
  if (!opts.cwd) {
    notify(t('toast.resumeNoCwd'), true)
    return
  }
  // tab 立刻出现：不提前清 activeUiId（否则 await 后端握手期间本 pane 无 active → 掉回 List），
  // 而是用 startChat 的 onReady 在 reactive session 一建好就 createViewTab —— activateViewTabInPane
  // 会原子地清 activeUiId + 指向新 chat。Codex 走 app-server，握手要几秒，这样既不闪 List、也不
  // 干等；session 是 reactive，spawning→running 自动反映到 ChatView。失败时 startChat 内部置
  // status='error'，tab 已在、显示错误态即可。
  try {
    await startChat({
      agent: opts.agent,
      projectKey: opts.projectKey,
      cwd: opts.cwd,
      sessionId: opts.sessionId,
      title: opts.title,
      created: opts.created,
      preloadMsgs: opts.preloadMsgs,
      initialUsage: opts.initialUsage,
      onReady: (session) => {
        createViewTab({
          type: 'chat',
          agent: opts.agent,
          projectKey: opts.projectKey,
          title: opts.title,
          chatSession: session,
          sourceSession: null,
        })
      },
    })
  } catch (e) {
    notify(`${e}`, true)
  }
}

/** 从会话详情开 / 续聊 live GUI chat（新开 chat tab，预载历史 + 上下文用量种子）。 */
async function resumeChatFromSession(s: SessionMeta) {
  // 已有同 sessionId 的 chat tab → 直接切过去
  const existingChat = findViewTab(t => t.type === 'chat' && t.chatSession?.sessionId === s.id)
  if (existingChat) {
    setActiveViewTab(existingChat.uiId)
    activeUiId.value = null
    return
  }
  // 已有同 sessionId 的 read tab（从 chat 切过来的，进程还在跑）→ 直接恢复
  const existingRead = findViewTab(t => t.type === 'session' && t.session?.id === s.id)
  if (existingRead?.chatSession) {
    existingRead.type = 'chat'
    setActiveViewTab(existingRead.uiId)
    activeUiId.value = null
    return
  }
  let preload: Msg[] = []
  if (existingRead) {
    preload = existingRead.msgs
  } else if (s.path) {
    try {
      preload = await api.readSession(chatAgent.value, s.path)
    } catch {
      preload = []
    }
  }
  let initialUsage: UsageSummary | undefined
  try {
    initialUsage = await api.sessionContextUsage(chatAgent.value, s.path)
  } catch {
    initialUsage = undefined
  }
  const cwd = s.cwd || activeProject.value?.displayPath || ''
  const projectKey = activeProject.value?.dirName ?? activeDir.value ?? ''
  activeUiId.value = null
  await startChat({
    agent: chatAgent.value,
    projectKey,
    cwd,
    sessionId: s.id,
    title: s.title,
    created: s.created,
    model: lastAssistantModel(preload),
    preloadMsgs: preload,
    initialUsage,
    onReady(chatSession) {
      if (existingRead) {
        existingRead.type = 'chat'
        existingRead.chatSession = chatSession
        existingRead.sourceSession = s
        setActiveViewTab(existingRead.uiId)
      } else {
        createViewTab({
          type: 'chat',
          agent: chatAgent.value,
          projectKey,
          title: s.title,
          chatSession: chatSession,
      sourceSession: s,
    })
  }
    },
  })
}

/** 列表行「chat」图标：把该会话作为 live GUI chat 打开。 */
async function chatFromList(s: SessionMeta) {
  await resumeChatFromSession(s)
}

function notifyArchivedBlock(cmd: string) {
  ask({
    title: t('toast.archivedBlock.title'),
    message: t('toast.archivedBlock.message', { cmd }),
    okText: t('toast.archivedBlock.copy'),
    danger: false,
    onOk: () => {
      void navigator.clipboard.writeText(cmd).catch(() => {})
      notify(t('toast.copied'))
      void newShellSession()
    },
  })
}

/** 入口 1(GUI)：在当前项目里新开一个空的 live GUI chat。 */
function newGuiSession() {
  if (_spawnLock) return
  _spawnLock = true
  startLiveChat({
    agent: agent.value,
    projectKey: activeProject.value?.dirName ?? activeDir.value ?? '',
    cwd: activeProject.value?.displayPath || '',
    title: t('list.action.newSessionGui'),
  })
  _spawnLock = false
}

/** `/fork`：把当前 live chat **克隆**成一个独立的新会话 —— 后端在磁盘上复制一份 transcript
 *  （全新 session id、新消息 uuid，零共享），标题 `<原标题> fork`，随即自动切到这个新会话
 *  续聊。原会话进程收掉（其 transcript 不动、仍可续聊）。仅 Claude 且当前会话已有 session id
 *  （有可克隆的落盘内容）时有效；否则提示无法 fork。 */
async function forkLiveChat() {
  const c = liveChat.value
  if (!c || c.agent !== 'claude' || !c.cwd) return
  if (!c.sessionId) {
    notify(t('toast.forkUnavailable'), true)
    return
  }
  const title = `${c.title} fork`
  const preload = [...c.msgs]
  let newId: string
  try {
    newId = await api.forkSession(c.agent, c.projectKey, c.sessionId, title)
  } catch (e) {
    notify(`${e}`, true)
    return
  }
  const projectKey = c.projectKey
  const cwd = c.cwd
  const created = c.createdAt
  const permissionMode = c.permissionMode
  const model = c.model
  const effort = c.effort
  const usage = c.usage
  // 关掉旧 chat tab
  closeLiveChat()
  activeUiId.value = null
  try {
    const session = await startChat({
      agent: 'claude',
      projectKey,
      cwd,
      sessionId: newId,
      title,
      created,
      preloadMsgs: preload,
      permissionMode,
      model,
      effort,
      initialUsage: usage,
    })
    createViewTab({
      type: 'chat',
      agent: 'claude',
      projectKey,
      title,
      chatSession: session,
      sourceSession: null,
    })
    notify(t('toast.forked', { title }))
    void loadProjects()
  } catch (e) {
    notify(`${e}`, true)
  }
}

/** 取消后编辑原提问：旧 tab 保留，新 tab 只继承该提问之前的上下文并回填原输入。 */
async function editCancelledPrompt(
  entry: ChatHistoryEntry,
  messageIndex: number,
  priorUserTurns: number,
) {
  const c = liveChat.value
  const sourceTab = activeViewTab.value
  if (!c || !sourceTab || sourceTab.type !== 'chat' || !c.cwd) return

  const title = `${c.title} fork`
  let sessionId: string | undefined
  try {
    if (priorUserTurns > 0) {
      if (!c.sessionId || c.chatId === null) throw new Error(t('toast.forkUnavailable'))
      sessionId = c.agent === 'codex'
        ? await api.agentChatForkBeforeLastTurn(c.chatId)
        : await api.forkSessionBeforeUserTurn(
            c.agent,
            c.projectKey,
            c.sessionId,
            title,
            priorUserTurns,
          )
    }

    await startChat({
      agent: c.agent,
      projectKey: c.projectKey,
      cwd: c.cwd,
      sessionId,
      title,
      created: c.createdAt,
      preloadMsgs: c.msgs.slice(0, messageIndex),
      initialDraft: entry,
      permissionMode: c.permissionMode,
      model: c.model,
      effort: c.effort,
      onReady: (session) => {
        createViewTab({
          type: 'chat',
          agent: c.agent,
          projectKey: c.projectKey,
          paneId: sourceTab.paneId,
          title,
          chatSession: session,
          sourceSession: null,
        })
      },
    })
    void loadProjects()
  } catch (e) {
    notify(`${e}`, true)
  }
}

async function archiveLiveChat() {
  const c = liveChat.value
  if (!c || c.agent !== 'codex' || !c.sessionId) {
    notify(t('toast.archiveUnavailable'), true)
    return
  }
  try {
    await api.codexArchiveSession(c.sessionId)
    notify(t('toast.archived'))
    closeLiveChat()
    await refreshSessions()
  } catch (e) {
    notify(`${e}`, true)
  }
}

/** btw 侧聊浮框（按钮 / ⌘J / 主聊里输入 `/btw …`）。优先 fork 正在进行的 Claude 主聊
 *  以继承其上下文；否则在当前活动项目目录里开一个全新 Claude 侧聊。两者都缺时静默无操作。 */
function toggleBtwSideChat() {
  const lc = liveChat.value
  if (!lc || lc.agent !== 'claude' || !lc.cwd) return
  if (sideChat.value) { closeSideChat(); return }
  void openSideChat({
    projectKey: lc.projectKey,
    cwd: lc.cwd,
    forkSessionId: lc.sessionId || undefined,
    model: lc.model,
    effort: lc.effort,
  })
}

/** Codex `/side` 浮层（按钮 / ⌘J / 主聊中输入 `/side …`）。 */
function toggleCodexSideChat() {
  const lc = liveChat.value
  if (!lc || lc.agent !== 'codex' || !lc.cwd) return
  if (codexSideChat.value) { closeCodexSideChat(); return }
  void openCodexSideChat({
    projectKey: lc.projectKey,
    cwd: lc.cwd,
    forkThreadId: lc.sessionId || undefined,
    model: lc.model,
    effort: lc.effort,
    permissionMode: lc.permissionMode,
  })
}





// ---------- live chat 顶栏会话级动作（统计 / 导出 / 删除）----------

function openLiveChatStats() {
  const s = liveChatSourceSession.value
  if (!s) return
  sessionStatsTarget.value = { agent: chatAgent.value, path: s.path, title: s.title }
  sessionStatsFrom.value = 'chat'
  showStats.value = true
}

/** live chat 导出：导的是**实时**消息（liveChat.msgs），比来源会话的 chatMsgs 更全。 */
async function exportLiveChat(kind: ExportKind) {
  const c = liveChat.value
  if (!c) return
  try {
    const path = await exportFn(kind)(liveChatMeta.value, c.msgs, c.agent)
    if (!path) return
    notify(t('toast.exported', { path }))
    api.revealInFinder(path).catch(() => {})
  } catch (e) {
    notify(t('toast.exportFail', { e: String(e) }), true)
  }
}

/** live chat 里删除：有来源会话就软删它（确认后 afterDelete 清 openSession →
 *  上面的导航 watch 触发 closeLiveChat 自动停掉子进程）；全新会话无文件，直接关。 */
async function deleteFromLiveChat() {
  if (liveChatSourceSession.value) {
    deleteSession(liveChatSourceSession.value)
    return
  }
  const c = liveChat.value
  if (!c?.sessionId) { closeLiveChat(); return }
  await refreshSessions()
  const found = sessions.value.find(s => s.id === c.sessionId)
  if (found) {
    const tab = activeViewTab.value
    if (tab?.type === 'chat') tab.sourceSession = found
    deleteSession(found)
    return
  }
  closeLiveChat()
}


/** Resume 一个会话 —— 根据设置决定走窗口内 TUI 还是外部终端。 */
async function resumeHere(s: SessionMeta) {
  if (s.cwd?.startsWith('ide://')) {
    notify(t('toast.resumeIdeSession'))
    return
  }
  const cwd = s.cwd || activeProject.value?.displayPath || ''
  if (!cwd) {
    notify(t('toast.resumeNoCwd'), true)
    return
  }
  try {
    if (useExternalTerminal.value) {
      await api.resumeSession(chatAgent.value, s.id, cwd, s.path, launchArgs.value[chatAgent.value as keyof typeof launchArgs.value] || '', terminalApp.value)
    } else {
      await openOrFocusTui({
        agent: chatAgent.value,
        projectKey: activeProject.value?.dirName ?? activeDir.value ?? '',
        sessionId: s.id,
        sessionPath: s.path,
        title: s.title,
        cwd,
      })
    }
  } catch (e) {
    notify(`${e}`, true)
  }
}

async function hydrateSavedTab(saved: SavedTab): Promise<boolean> {
  try {
    if (saved.isShell) {
      await openShellTab({
        agent: saved.agent,
        projectKey: saved.projectKey,
        title: saved.title,
        cwd: saved.cwd,
        createdAt: saved.createdAt,
      })
    } else {
      await openOrFocusTui({
        agent: saved.agent,
        projectKey: saved.projectKey,
        sessionId: saved.sessionId,
        sessionPath: saved.sessionPath,
        title: saved.title,
        cwd: saved.cwd,
        createdAt: saved.createdAt,
        ...(!saved.sessionId ? { knownSessionPaths: sessions.value.map((s) => s.path) } : {}),
        ...(saved.userRenamed ? { userRenamed: true } : {}),
      })
    }
    return true
  } catch (e) {
    notify(`${e}`, true)
    return false
  }
}

async function hydrateStartupTerminalTabs(
  nav: SavedNav | null,
  hydrateTarget: SavedTab | undefined,
  restoredActiveViewId: number | null,
) {
  const restoreFocusedPaneId = focusedPane.value?.id ?? null
  if (nav?.view === 'tui') {
    // 上次停在 TUI（终端/会话）tab → 恢复「那一个」的激活态（水合它、切到它）。这一步之前
    // 被漏掉了（hydrateTarget 参数没用），导致刷新后终端/会话 tab 掉回 List —— 而 chat/read/git
    // view tab 却正常恢复。autoRestore 开关只决定是否**连带**恢复其它非激活终端，这里只管激活那个。
    if (hydrateTarget) await hydrateSavedTabOnce(hydrateTarget)
    return
  }
  for (const pane of currentPanes.value) pane.activeUiId = null
  if (nav?.view === 'view' && restoredActiveViewId != null) {
    setActiveViewTab(restoredActiveViewId)
  } else if (restoreFocusedPaneId != null) {
    focusPane(restoreFocusedPaneId)
  }
}

/** 开一个全新会话 —— 根据设置决定走窗口内 TUI 还是外部终端。 */
let _spawnLock = false
async function newSession() {
  const cwd = activeProject.value?.displayPath || ''
  if (!cwd || _spawnLock) return
  _spawnLock = true
  try {
    if (useExternalTerminal.value) {
      await api.newSession(agent.value, cwd, launchArgs.value[agent.value as keyof typeof launchArgs.value] || '', terminalApp.value)
    } else {
      await openOrFocusTui({
        agent: agent.value,
        projectKey: activeProject.value?.dirName ?? activeDir.value ?? '',
        sessionId: '',
        sessionPath: '',
        title: t('chat.tui.newSessionTitle'),
        cwd,
        knownSessionPaths: sessions.value.map((s) => s.path),
      })
    }
  } catch (e) {
    notify(`${e}`, true)
  } finally {
    _spawnLock = false
  }
}

/** 开一个纯 shell tab —— 不跑任何 agent CLI，用于执行任意 shell 命令。 */
async function newShellSession() {
  const cwd = activeProject.value?.displayPath || ''
  if (!cwd || _spawnLock) return
  _spawnLock = true
  try {
    await openShellTab({
      agent: agent.value,
      projectKey: activeProject.value?.dirName ?? activeDir.value ?? '',
      title: t('list.action.newTerminal'),
      cwd,
    })
  } catch (e) {
    notify(`${e}`, true)
  } finally {
    _spawnLock = false
  }
}

// 双击 tab 条空白处 / ⌘N / ⌘T 的「默认新建」手势 —— 按设置分流到 session/terminal/chat。
function newDefaultAction() {
  if (quickOpenTarget.value === 'terminal') {
    newShellSession()
  } else if (quickOpenTarget.value === 'chat') {
    if (!chatSupported(agent.value)) {
      notify(t('toast.chatUnsupported'))
      return
    }
    newGuiSession()
  } else {
    newSession()
  }
}

function nextGitDiffNumber(projectKey: string): number {
  const used = new Set(
    viewTabs.value
      .filter(t => t.type === 'git' && t.projectKey === projectKey)
      .map(t => parseInt(t.title.replace('Git Diff ', ''), 10))
      .filter(n => !isNaN(n)),
  )
  let n = 1
  while (used.has(n)) n++
  return n
}

async function openGitChangesTab(cwd?: string) {
  const repositoryCwd = cwd || activeProject.value?.displayPath
  if (!repositoryCwd) return
  const has = await api.gitHasRepo(repositoryCwd).catch(() => false)
  if (!has) return
  createViewTab({
    type: 'git',
    agent: agent.value,
    projectKey: activeDir.value ?? '',
    title: `Git Diff ${nextGitDiffNumber(activeDir.value ?? '')}`,
    gitCwd: repositoryCwd,
    gitRef: 'working',
  })
}

// 顶栏右上角的仓库入口
const REPO_URL = 'https://github.com/jerrywu001/cc-sessions-viewer'
function openRepo() {
  api.openUrl(REPO_URL).catch((e) => notify(`${e}`, true))
}

function runEditCommand(command: 'undo' | 'redo' | 'cut' | 'copy' | 'paste' | 'selectAll') {
  document.execCommand(command)
}

const menuHandlers: MenuHandlers = {
  'open-global-search': () => openGlobalSearch(),
  'find-in-session': () => activeUiId.value !== null ? focusTuiSearchBox() : focusSearchBox(),
  'find-next': () => chatNavigate(1),
  'find-prev': () => chatNavigate(-1),
  'toggle-sidebar': toggleSidebar,
  'new-session': () => newDefaultAction(),
  'new-tab': () => newDefaultAction(),
  'close-tab': () => closeActiveTab(),
  'rename-tab': () => renameActiveTab(),
  'add-folder': () => addBookmark(),
  'open-settings': () => {
    showSettings.value = true
  },
  'export-session': () => {
    if (!openSession.value) {
      notify(t('toast.exportNoSession'))
      return
    }
    exportSession('md')
  },
  'open-trash': () => loadTrash(),
  'open-stats': openStats,
  'check-update': () => {
    settingsTab.value = 'updates'
    showSettings.value = true
  },
  'theme:light': () => setTheme('light'),
  'theme:dark': () => setTheme('dark'),
  'theme:system': () => setTheme('system'),
  'theme:codex': () => setTheme('codex'),
  'theme:dracula': () => setTheme('dracula'),
  'lang:en': () => setLang('en'),
  'lang:zh': () => setLang('zh'),
  'lang:zh-TW': () => setLang('zh-TW'),
  'lang:ja': () => setLang('ja'),
  'help-docs': () => api.openUrl(`${REPO_URL}#readme`).catch((e) => notify(`${e}`, true)),
  'help-repo': () => openRepo(),
  'help-issue': () => api.openUrl(`${REPO_URL}/issues`).catch((e) => notify(`${e}`, true)),
  'edit:undo': () => runEditCommand('undo'),
  'edit:redo': () => runEditCommand('redo'),
  'edit:cut': () => runEditCommand('cut'),
  'edit:copy': () => runEditCommand('copy'),
  'edit:paste': () => runEditCommand('paste'),
  'edit:select-all': () => runEditCommand('selectAll'),
}

const windowMenus = computed<WindowMenuGroup[]>(() => [
  {
    label: t('menu.file'),
    items: [
      { type: 'item', id: 'new-session', label: t('menu.file.newSession'), shortcut: 'Ctrl+N', disabled: !activeProject.value },
      { type: 'item', id: 'new-tab', label: t('menu.file.newTab'), shortcut: 'Ctrl+T', disabled: !activeProject.value },
      { type: 'item', id: 'close-tab', label: t('menu.file.closeTab'), shortcut: 'Ctrl+W', disabled: !activeUiId.value && !openSession.value },
      { type: 'item', id: 'rename-tab', label: t('menu.file.renameTab'), shortcut: 'Ctrl+R', disabled: !activeUiId.value },
      { type: 'item', id: 'add-folder', label: t('menu.file.addFolder'), shortcut: 'Ctrl+O' },
      { type: 'separator' },
      { type: 'item', id: 'export-session', label: t('menu.file.export'), shortcut: 'Ctrl+E', disabled: !openSession.value },
    ],
  },
  {
    label: t('menu.edit'),
    items: [
      { type: 'item', id: 'edit:undo', label: t('menu.edit.undo'), shortcut: 'Ctrl+Z' },
      { type: 'item', id: 'edit:redo', label: t('menu.edit.redo'), shortcut: 'Ctrl+Y' },
      { type: 'separator' },
      { type: 'item', id: 'edit:cut', label: t('menu.edit.cut'), shortcut: 'Ctrl+X' },
      { type: 'item', id: 'edit:copy', label: t('menu.edit.copy'), shortcut: 'Ctrl+C' },
      { type: 'item', id: 'edit:paste', label: t('menu.edit.paste'), shortcut: 'Ctrl+V' },
      { type: 'item', id: 'edit:select-all', label: t('menu.edit.selectAll'), shortcut: 'Ctrl+A' },
    ],
  },
  {
    label: t('menu.view'),
    items: [
      { type: 'item', id: 'toggle-sidebar', label: t('menu.view.toggleSidebar'), shortcut: 'Ctrl+B' },
      { type: 'item', id: 'open-stats', label: t('menu.view.stats'), shortcut: 'Ctrl+Shift+S' },
      { type: 'separator' },
      {
        type: 'submenu',
        label: t('menu.view.theme'),
        items: [
          { type: 'item', id: 'theme:light', label: t('menu.view.theme.light'), checked: theme.value === 'light' },
          { type: 'item', id: 'theme:dark', label: t('menu.view.theme.dark'), checked: theme.value === 'dark' },
          { type: 'item', id: 'theme:system', label: t('menu.view.theme.system'), checked: theme.value === 'system' },
          { type: 'item', id: 'theme:codex', label: t('menu.view.theme.codex'), checked: theme.value === 'codex' },
          { type: 'item', id: 'theme:dracula', label: t('menu.view.theme.dracula'), checked: theme.value === 'dracula' },
        ],
      },
      {
        type: 'submenu',
        label: t('menu.view.language'),
        items: [
          { type: 'item', id: 'lang:en', label: 'English', checked: lang.value === 'en' },
          { type: 'item', id: 'lang:zh', label: '简体中文', checked: lang.value === 'zh' },
          { type: 'item', id: 'lang:zh-TW', label: '繁體中文', checked: lang.value === 'zh-TW' },
          { type: 'item', id: 'lang:ja', label: '日本語', checked: lang.value === 'ja' },
        ],
      },
    ],
  },
  {
    label: t('menu.find'),
    items: [
      { type: 'item', id: 'find-in-session', label: t('menu.find.inSession'), shortcut: 'Ctrl+F' },
      { type: 'item', id: 'find-next', label: t('menu.find.next'), shortcut: 'Ctrl+G' },
      { type: 'item', id: 'find-prev', label: t('menu.find.prev'), shortcut: 'Ctrl+Shift+G' },
      { type: 'separator' },
      { type: 'item', id: 'open-global-search', label: t('menu.find.inAll'), shortcut: 'Ctrl+Shift+F' },
    ],
  },
  {
    label: t('menu.window'),
    items: [
      { type: 'item', id: 'window:minimize', label: t('menu.window.minimize') },
      { type: 'item', id: 'window:maximize', label: t('menu.window.maximize') },
      { type: 'separator' },
      { type: 'item', id: 'open-trash', label: t('menu.window.trash'), shortcut: 'Ctrl+Shift+T' },
      { type: 'item', id: 'window:fullscreen', label: t('menu.window.fullscreen') },
    ],
  },
  {
    label: t('menu.help'),
    items: [
      { type: 'item', id: 'help-docs', label: t('menu.help.docs') },
      { type: 'item', id: 'help-repo', label: t('menu.help.repo') },
      { type: 'item', id: 'help-issue', label: t('menu.help.issue') },
    ],
  },
])

function onClearCache() {
  ask({
    title: t('dialog.clearCache.title'),
    message: t('dialog.clearCache.body'),
    okText: t('dialog.clearCache.ok'),
    danger: true,
    onOk: () => {
      clearAppCache()
      projPrefs.value = {}
      projectOrders.value = {}
      api.detectTerminals().then(applyTerminalDefault).catch(() => {})
      notify(t('toast.cacheCleared'))
    },
  })
}

function onClearTabs() {
  ask({
    title: t('dialog.clearTabs.title'),
    message: t('dialog.clearTabs.body'),
    okText: t('dialog.clearTabs.ok'),
    danger: true,
    onOk: () => {
      clearAllTabs()
      // view tab（会话查看 / GUI chat / Git Diff）一并关闭：chat 先停子进程再摘 tab
      for (const vt of [...viewTabs.value]) {
        if (vt.type === 'chat') closeLiveChat(vt.uiId)
        else removeViewTab(vt.uiId)
      }
      clearSavedViewTabs()
      // 每项目「上次活跃终端」记忆同步清空，避免切项目时试图恢复已不存在的 tab
      activeTuiByProject.clear()
      notify(t('toast.tabsCleared'))
    },
  })
}

// ---------- 窗口聚焦 / 失焦：与 Codex 一致的弱化态 ----------
const windowFocused = ref(document.hasFocus())
async function onFocus() {
  windowFocused.value = true
  const activeTuiTab = currentActiveTab()
  if (activeTuiTab) markTabViewed(activeTuiTab.uiId)
  clearPendingLiveNotification()
  const activeTab = viewTabs.value.find(t => t.uiId === activeViewTabId.value)
  if (activeTab && activeTab.type === 'session' && activeTab.session?.path) {
    try {
      const oldLen = activeTab.msgs.length
      const newMsgs = await api.readSession(activeTab.agent, activeTab.session.path)
      activeTab.msgs = newMsgs
      await api.watchSession(activeTab.agent, activeTab.session.path)
      if (newMsgs.length > oldLen) {
        activeTab.liveTailing = true
        window.clearTimeout(activeTab.liveFadeTimer)
        activeTab.liveFadeTimer = window.setTimeout(() => {
          activeTab.liveTailing = false
        }, LIVE_STALE_MS)
      }
    } catch {}
  }
}
function onBlur() {
  windowFocused.value = false
}
function appVisible() {
  return windowFocused.value && document.visibilityState === 'visible'
}
function onVisibilityChange() {
  if (document.visibilityState === 'visible') clearPendingLiveNotification()
  if (document.visibilityState === 'hidden') saveTabState()
}

function saveTabState() {
  const cur = currentActiveTab()
  const onView = !cur && !!activeViewTab.value
  const view: SavedNav['view'] = cur
    ? 'tui'
    : onView
      ? 'view'
      : activeDir.value
        ? 'list'
        : 'welcome'
  // TUI tab 记忆
  if (activeDir.value) {
    const k = viewKey(agent.value, activeDir.value)
    if (cur) {
      activeTuiByProject.set(k, {
        sessionPath: cur.sessionPath,
        ...(cur.isShell ? { isShell: true } : {}),
      })
    } else {
      activeTuiByProject.delete(k)
    }
  }
  persistTuiMap()
  persistViewTabs()
  persistLayouts()
  const noPathIdx = cur && !cur.sessionPath
    ? tuiTabs.value.filter((t) => !t.sessionPath).indexOf(cur)
    : undefined
  persistTabState({
    agent: agent.value,
    activeDir: activeDir.value,
    activeSessionPath: cur?.sessionPath ?? null,
    view,
    ...(noPathIdx != null && noPathIdx >= 0 ? { activeSavedIndex: noPathIdx } : {}),
  })
}

// 主题变化时把原生窗口外观（标题栏 / 失焦红绿灯灰圈）钉到当前主题——CSS 管不到
// 原生按钮，浅色主题失焦时灰圈会糊在浅色顶栏上看不见。immediate 保证启动即同步。
watch(
  theme,
  (t) => {
    void api.setTitlebarTheme(nativeAppearance(t)).catch(() => {})
  },
  { immediate: true },
)

onMounted(() => {
  // 恢复上次退出时的侧栏导航状态
  const nav = loadSavedNav()
  if (nav) {
    agent.value = nav.agent
    activeDir.value = nav.activeDir
  }
  // 恢复每个项目各自的 TUI tab 记忆 —— 切到任意项目时恢复它上次活跃的终端 tab。
  for (const v of loadSavedActiveTui()) {
    activeTuiByProject.set(viewKey(v.agent, v.dir), { sessionPath: v.sessionPath, ...(v.isShell ? { isShell: true } : {}) })
  }

  loadProjects().then(async () => {
    // 退出时停在终端 tab → 先按该 tab 的项目为准定位它（nav.activeDir 可能因竞态不一致），
    // 但**不**马上水合 —— 先把 View tab 恢复成背景，再把终端 tab 顶到前面。
    let hydrateTarget: SavedTab | undefined
    if ((nav?.activeSessionPath || nav?.activeSavedIndex != null) && nav?.view === 'tui') {
      if (nav.activeSavedIndex != null) {
        const noPath = savedTabs.value.filter((s) => !s.sessionPath)
        hydrateTarget = noPath[nav.activeSavedIndex] ?? noPath[0]
      } else {
        hydrateTarget = savedTabs.value.find((s) => s.sessionPath === nav.activeSessionPath)
      }
      if (hydrateTarget) activeDir.value = hydrateTarget.projectKey
    }
    if (activeDir.value) await refreshSessions()
    // 恢复上次退出时的 view tabs（session read + chat 元数据）
    const savedVT = loadSavedViewTabs()
    const savedChatTabs = savedVT.tabs.filter(t => t.type === 'chat')
    let restoredActiveIdx: number | null = null
    const activeTabs: ViewTab[] = []
    suppressActivation(() => {
      for (let i = 0; i < savedVT.tabs.length; i++) {
        const sv = savedVT.tabs[i]
        if (sv.type === 'chat') continue
        if (sv.type === 'git') {
          if (!sv.gitCwd) continue
          const tab = createViewTab({
            type: 'git',
            agent: sv.agent,
            projectKey: sv.projectKey,
            paneId: sv.paneId,
            title: sv.title,
            createdAt: sv.createdAt,
            gitCwd: sv.gitCwd,
            gitRef: sv.gitRef || 'working',
            gitSelectedPath: sv.gitSelectedPath || null,
          })
          if (sv.isActive) activeTabs.push(tab)
          if (i === savedVT.activeIdx) restoredActiveIdx = tab.uiId
          continue
        }
        if (!sv.session) continue
        const tab = createViewTab({
          type: 'session',
          agent: sv.agent,
          projectKey: sv.projectKey,
          paneId: sv.paneId,
          title: sv.title,
          createdAt: sv.createdAt,
          session: sv.session,
          loadingMsgs: true,
          trashAgent: sv.trashAgent,
          importedAgent: sv.importedAgent,
        })
        if (sv.isActive) activeTabs.push(tab)
        if (i === savedVT.activeIdx) restoredActiveIdx = tab.uiId
        api.readSession(sv.agent, sv.session.path).then(msgs => {
          tab.msgs = msgs
          tab.loadingMsgs = false
        }).catch(() => {
          removeViewTab(tab.uiId)
        })
      }
    })
    for (const tab of activeTabs) {
      const pane = panes.get(tab.paneId)
      if (pane) pane.activeViewTabId = tab.uiId
    }
    // 页面刷新后重连后端仍存活的 chat 进程 → 每个重连的 chat 创建一个 chat tab
    const reconnected = await reconnectChats()
    for (const session of reconnected) {
      const saved = savedChatTabs.find(s => s.sessionId && s.sessionId === session.sessionId)
      const title = saved?.title || session.title
      session.title = title
      if (saved?.session?.path) {
        try {
          const diskMsgs = await api.readSession(session.agent, saved.session.path)
          if (diskMsgs.length > session.msgs.length) session.msgs = diskMsgs
          if (!session.lastModel) session.lastModel = lastAssistantModel(session.msgs)
        } catch {}
      }
      const tab = createViewTab({
        type: 'chat',
        agent: session.agent,
        projectKey: session.projectKey,
        paneId: saved?.paneId,
        title,
        createdAt: saved?.createdAt,
        chatSession: session,
        sourceSession: saved?.session ?? null,
      })
      const savedIdx = savedVT.tabs.indexOf(saved!)
      if (savedIdx >= 0 && savedIdx === savedVT.activeIdx) {
        restoredActiveIdx = tab.uiId
      }
    }
    // 没被 reconnect 恢复的 chat tab → 重新启动进程恢复为 chat
    const reconnectedIds = new Set(reconnected.map(s => s.sessionId))
    const deadChats: { sv: SavedViewTab; idx: number }[] = []
    for (let i = 0; i < savedVT.tabs.length; i++) {
      const sv = savedVT.tabs[i]
      if (sv.type !== 'chat' || !sv.session) continue
      if (sv.sessionId && reconnectedIds.has(sv.sessionId)) continue
      deadChats.push({ sv, idx: i })
    }
    for (const { sv, idx } of deadChats) {
      const s = sv.session!
      let preload: Msg[] = []
      try { preload = await api.readSession(sv.agent, s.path) } catch {}
      let initialUsage: UsageSummary | undefined
      try { initialUsage = await api.sessionContextUsage(sv.agent, s.path) } catch {}
      const cwd = s.cwd || ''
      try {
        const chatSession = await startChat({
          agent: sv.agent,
          projectKey: sv.projectKey,
          cwd,
          sessionId: s.id,
          title: sv.title,
          created: s.created,
          preloadMsgs: preload,
          initialUsage,
        })
        const tab = createViewTab({
          type: 'chat',
          agent: sv.agent,
          projectKey: sv.projectKey,
          paneId: sv.paneId,
          title: sv.title,
          createdAt: sv.createdAt,
          chatSession,
          sourceSession: s,
        })
        if (idx === savedVT.activeIdx) restoredActiveIdx = tab.uiId
      } catch {
        // 启动失败 → 降级为 read tab
        const tab = createViewTab({
          type: 'session',
          agent: sv.agent,
          projectKey: sv.projectKey,
          paneId: sv.paneId,
          title: sv.title,
          createdAt: sv.createdAt,
          session: s,
          loadingMsgs: false,
          msgs: preload,
        })
        if (idx === savedVT.activeIdx) restoredActiveIdx = tab.uiId
      }
    }
    // 所有 tab 恢复完毕后，再统一设置 active（createViewTab 会自动激活最后一个，需要修正）
    if (restoredActiveIdx != null) {
      setActiveViewTab(restoredActiveIdx)
    } else if (savedVT.tabs.length > 0 && nav?.view !== 'view') {
      setActiveViewTab(null)
    }
    // 启动阶段不水合终端 tab；终端恢复延后到用户点击左侧项目或 saved tab 时触发。
    await hydrateStartupTerminalTabs(nav, hydrateTarget, restoredActiveIdx)
    markViewTabsRestored()
  })
  // 启动时拉一次回收站，让顶栏红点从一开始就准确（不必先打开回收站视图）
  api.listTrash().then((items) => { trash.value = items }).catch(() => {})
  // 检测可用终端，首次启动时自动选默认（有 cmux 就默认 cmux）
  api.detectTerminals().then(applyTerminalDefault).catch(() => {})

  // 关窗 / 隐藏 / 退出时保存 tab 状态
  window.addEventListener('beforeunload', saveTabState)

  // 实时防抖存：状态变化时 500ms 后自动持久化，进程被 kill 也不丢状态。
  // 只 watch 影响恢复的信号（agent / 项目 / 激活的 tab / tab 数量 / 是否开着 View tab /
  // View 的 read⇄chat 子模式），不 deep watch tuiTabs 内部高频字段（lastOutputAt / turnState 等）。
  let saveTimer: number | null = null
  const debouncedSave = () => {
    if (saveTimer !== null) clearTimeout(saveTimer)
    saveTimer = window.setTimeout(saveTabState, 500)
  }
  const tabCount = computed(() => tuiTabs.value.length)
  const savedCount = computed(() => savedTabs.value.length)
  const viewTabCount = computed(() => viewTabs.value.length)
  watch([agent, activeDir, activeUiId, tabCount, savedCount, viewTabCount, activeViewTabId], debouncedSave)
  // 分屏树几何 / sizes / 聚焦格子变化也要存（split / close / resize / focus）。deep + 500ms
  // 防抖，resize 拖拽的高频 sizes 更新会被合并成一次落盘。
  watch(currentLayout, debouncedSave, { deep: true })
  // 后台检查 GitHub release —— 缓存 24h，失败完全静默；结果驱动侧边栏 Settings
  // 按钮上的"有新版本"小红点。
  runBackgroundCheck()
  startTuiTitleSyncTimer()
  window.addEventListener('focus', onFocus)
  window.addEventListener('blur', onBlur)
  window.addEventListener('resize', onWindowResize)
  document.addEventListener('visibilitychange', onVisibilityChange)
  // 右键菜单的全局关闭：任意点击 / 滚轮 / ESC
  document.addEventListener('mousedown', (e) => {
    if (!ctxMenu.value) return
    const target = e.target as HTMLElement | null
    if (target && target.closest('.ctx-menu')) return
    closeCtxMenu()
  })
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && ctxMenu.value) closeCtxMenu()
  })
  window.addEventListener('blur', closeCtxMenu)
  document.addEventListener('wheel', closeCtxMenu, { passive: true })

  // JS-side keyboard shortcuts — fallback for when native menu accelerators
  // don't fire (Windows WebView2 swallows some Ctrl combos, Linux varies).
  // Capture phase so child stopPropagation can't block us.
  const _isMac = /Mac/i.test(navigator.platform)
  window.addEventListener(
    'keydown',
    (e) => {
      const mod = _isMac ? e.metaKey : e.ctrlKey
      const otherMod = _isMac ? e.ctrlKey : e.metaKey

      // Cmd+Alt+方向键 → 分屏格子间移动聚焦（方向感知）。放在下面的 altKey 拒绝之前。
      if (mod && !otherMod && e.altKey && !e.shiftKey) {
        const dir = ({
          arrowleft: 'left', arrowright: 'right', arrowup: 'up', arrowdown: 'down',
        } as const)[e.key.toLowerCase()]
        if (dir) { e.preventDefault(); focusPaneDir(dir) }
        return
      }
      if (!mod || otherMod || e.altKey) return
      if (e.repeat) return

      const key = e.key.toLowerCase()
      if (key === 'w' && !e.shiftKey) {
        e.preventDefault(); closeActiveTab()
      } else if (key === 'w' && e.shiftKey) {
        e.preventDefault(); closeFocusedPane()
      } else if (key === 'd' && !e.shiftKey) {
        e.preventDefault(); splitFocusedPane('row')
      } else if (key === 'd' && e.shiftKey) {
        e.preventDefault(); splitFocusedPane('col')
      } else if (key === 't' && !e.shiftKey) {
        e.preventDefault(); newDefaultAction()
      } else if (key === 'r' && !e.shiftKey) {
        e.preventDefault(); renameActiveTab()
      } else if (key === 'f' && e.shiftKey) {
        e.preventDefault(); openGlobalSearch()
      } else if (key === 'f' && !e.shiftKey) {
        e.preventDefault()
        activeUiId.value !== null ? focusTuiSearchBox() : focusSearchBox()
      } else if (key === 'g' && !e.shiftKey) {
        e.preventDefault(); chatNavigate(1)
      } else if (key === 'g' && e.shiftKey) {
        e.preventDefault(); chatNavigate(-1)
      } else if (key === 'b' && e.shiftKey) {
        e.preventDefault(); openGitChangesTab()
      } else if (key === 'n' && !e.shiftKey) {
        e.preventDefault(); newDefaultAction()
      } else if (key === 'o' && !e.shiftKey) {
        e.preventDefault(); addBookmark()
      } else if (key === 'e' && !e.shiftKey) {
        e.preventDefault()
        if (openSession.value) exportSession('md')
      } else if (key === 'b' && !e.shiftKey) {
        e.preventDefault(); toggleSidebar()
      } else if (key === 'j' && !e.shiftKey) {
        if (liveChat.value?.agent === 'claude') {
          e.preventDefault(); toggleBtwSideChat()
        } else if (liveChat.value?.agent === 'codex') {
          e.preventDefault(); toggleCodexSideChat()
        }
      } else if (key === 's' && e.shiftKey) {
        e.preventDefault(); openStats()
      } else if (key === ',' && !e.shiftKey) {
        e.preventDefault(); settingsTab.value = undefined; showSettings.value = true
      } else if (key === 't' && e.shiftKey) {
        e.preventDefault(); loadTrash()
      } else if ((key === '/' || key === '?') && !e.shiftKey) {
        e.preventDefault()
        showSettings.value = true
        settingsTab.value = 'shortcuts'
      }
    },
    true,
  )

  // 原生菜单 → 前端动作路由。菜单项的 id 在 src-tauri/src/menu.rs 里定义。
  installMenuRouter(menuHandlers).then((fn) => {
    menuUnlisten = fn
  })

  // 启动时把当前 theme / lang 同步给菜单的 CheckMenuItem 勾选态。
  emitMenuSync('theme', theme.value)
  emitMenuSync('lang', lang.value)
})

// 主题 / 语言变化 → 同步菜单勾选态。
watch(theme, (v) => emitMenuSync('theme', v))
watch(lang, (v) => emitMenuSync('lang', v))

// (agent, activeDir) 切换后，如果当前 active 的 TUI tab 不在新范围里 → 自动让位回 view。
// 现存的导航函数（switchAgent / selectProject 等）已经显式 setActiveTui(null)，但有些
// 路径（直接改 activeDir / 关闭项目）走不到那里，这条 watch 兜底。tabs 本身不动 ——
// PTY 仍活着，切回原项目时 strip 会再次显示。
watch([agent, activeDir], () => {
  const cur = currentActiveTab()
  if (!cur) return
  if (cur.agent !== agent.value || cur.projectKey !== (activeDir.value ?? '')) {
    setActiveTui(null)
  }
})

watch([codexShowInternalSessions, codexShowArchivedSessions], () => {
  if (agent.value !== 'codex') return
  loadProjects()
  if (activeDir.value && !showTrash.value && !showStats.value) {
    refreshSessions()
  }
})

let menuUnlisten: UnlistenFn | null = null

// Live tail：监听 watch.rs emit 的 3 个事件。安装一次，整个应用生命周期共用。
//   session:append → 后端把新增的尾段 Msg 推过来；前端 push 进 chatMsgs，
//                    再调 ChatView.onLiveAppend(n) 让它做 smart-scroll。
//   session:reset  → 文件被截断 / 替换 → 整段重拉。
//   session:gone   → 文件不在了 → 关闭当前会话，toast 一下。
// path 兜底校验：用户在 emit 飞过来的极短窗口里切换了会话 / 关掉了详情页，
// 我们只接当前 openSession.path 一致的事件，避免把 A 会话的尾段塞到 B 里。
let liveUnlisteners: UnlistenFn[] = []

type TerminalTurnEvent = {
  agent: Agent
  path: string
  state: 'started' | 'completed' | 'blocked' | 'failed'
  source: 'hook'
}

type DesktopPetSessionTarget = Pick<DesktopTask, 'agent' | 'path'>

async function installLiveTailListeners() {
  const appendUnlisten = await listen<{ path: string; messages: Msg[] }>(
    'session:append',
    (e) => {
      const tab = viewTabs.value.find(t => t.type === 'session' && t.session?.path === e.payload.path)
      if (!tab) return
      const added = e.payload.messages
      if (!added.length) return
      markTabSessionActivity(tab.agent, e.payload.path)
      tab.msgs = tab.msgs.concat(added)
      tab.liveTailing = true
      window.clearTimeout(tab.liveFadeTimer)
      tab.liveFadeTimer = window.setTimeout(() => { tab.liveTailing = false }, LIVE_STALE_MS)
      enqueueLiveNotification({
        agent: tab.agent,
        sessionTitle: tab.session?.title || shortName(e.payload.path),
        sessionPath: e.payload.path,
        messages: added,
        appVisible: appVisible(),
      })
      if (tab.uiId === activeViewTabId.value) {
        nextTick(() => chatViewRef.value?.onLiveAppend?.(added.length))
      }
    },
  )
  const resetUnlisten = await listen<{ path: string }>('session:reset', async (e) => {
    const tab = viewTabs.value.find(t => t.type === 'session' && t.session?.path === e.payload.path)
    if (!tab) return
    try {
      markTabSessionActivity(tab.agent, e.payload.path)
      tab.msgs = await api.readSession(tab.agent, e.payload.path)
    } catch {}
  })
  const goneUnlisten = await listen<{ path: string }>('session:gone', (e) => {
    const tab = viewTabs.value.find(t => t.type === 'session' && t.session?.path === e.payload.path)
    if (!tab) return
    notify(t('toast.sessionGone'))
    removeViewTab(tab.uiId)
  })
  liveUnlisteners.push(appendUnlisten, resetUnlisten, goneUnlisten)
}

async function installTerminalTurnListeners() {
  const turnUnlisten = await listen<TerminalTurnEvent>('terminal-turn://state', (e) => {
    const { agent: eventAgent, path, state, source } = e.payload
    if (!path) return
    if (state === 'started') markTabTurnStarted(eventAgent, path, source)
    else if (state === 'completed') markTabTurnCompleted(eventAgent, path, source)
    else if (state === 'blocked') markTabTurnBlocked(eventAgent, path, source)
    else if (state === 'failed') markTabTurnFailed(eventAgent, path, source)
  })
  const desktopPetUnlisten = await listen<DesktopPetSessionTarget>(
    'desktop-pet://open-session',
    (event) => { void openDesktopPetSession(event.payload) },
  )
  liveUnlisteners.push(turnUnlisten, desktopPetUnlisten)
}

function prepareDesktopPetAgent(targetAgent: Agent) {
  if (agent.value === targetAgent) return
  rememberActiveNav()
  lastDirByAgent.set(agent.value, activeDir.value)
  agent.value = targetAgent
  activeDir.value = null
  projects.value = []
  sessions.value = []
  sessionTotal.value = 0
  setActiveViewTab(null)
  void loadProjects()
}

function closeDesktopPetNavigationOverlays() {
  setActiveTui(null)
  closeAllSideChats()
  closeAllCodexSideChats()
  openTrashItem.value = null
  showTrash.value = false
  showStats.value = false
  showExportHistory.value = false
  showPricing.value = false
  sessionStatsTarget.value = null
}

async function openDesktopPetSession(target: DesktopPetSessionTarget) {
  if (!['claude', 'codex', 'agy'].includes(target.agent) || !target.path) return

  const existingTui = tuiTabs.value.find(
    (tab) =>
      !tab.isShell
      && tab.agent === target.agent
      && sessionPathsEqual(tab.sessionPath, target.path),
  )
  const existingSavedTui = savedTabs.value.find(
    (tab) =>
      !tab.isShell
      && tab.agent === target.agent
      && sessionPathsEqual(tab.sessionPath, target.path),
  )
  const existingView = findViewTab(
    (tab) =>
      tab.type === 'session'
      && tab.agent === target.agent
      && sessionPathsEqual(tab.session?.path ?? '', target.path),
  )

  let resolved: DesktopPetResolvedSession | null = null
  let projectKey = existingTui?.projectKey
    || existingSavedTui?.projectKey
    || existingView?.projectKey
    || ''
  if (!projectKey || (!existingTui && !existingSavedTui && !existingView)) {
    try {
      resolved = await resolveDesktopPetSession(target)
    } catch (error) {
      notify(t('toast.readFail', { e: String(error) }), true)
      return
    }
    if (!resolved) {
      notify(t('desktopPet.activity.sessionMissing'), true)
      return
    }
    projectKey = resolved.projectKey
  }
  if (existingView && resolved) {
    existingView.projectKey = resolved.projectKey
    existingView.session = resolved.session
    existingView.title = resolved.session.title
  }

  prepareDesktopPetAgent(target.agent)
  closeDesktopPetNavigationOverlays()
  if (activeDir.value !== projectKey) await selectProject(projectKey)

  let opened = false
  if (existingTui) {
    setActiveTui(existingTui.uiId)
    opened = true
  } else if (existingSavedTui) {
    opened = await hydrateSavedTabOnce(existingSavedTui)
  } else if (existingView) {
    existingView.loadingMsgs = true
    setActiveViewTab(existingView.uiId)
    try {
      existingView.msgs = await api.readSession(target.agent, target.path)
      await api.watchSession(target.agent, target.path).catch(() => {})
      opened = true
    } catch (error) {
      notify(t('toast.readFail', { e: String(error) }), true)
    } finally {
      existingView.loadingMsgs = false
    }
  } else if (resolved) {
    const tab = createViewTab({
      type: 'session',
      agent: target.agent,
      projectKey: resolved.projectKey,
      title: resolved.session.title,
      session: resolved.session,
      loadingMsgs: true,
    })
    await nextTick()
    try {
      tab.msgs = await api.readSession(target.agent, target.path)
      await api.watchSession(target.agent, target.path).catch(() => {})
      opened = true
      recordView({
        agent: target.agent,
        dir: resolved.projectKey,
        session: resolved.session,
        mode: 'read',
      })
    } catch (error) {
      notify(t('toast.readFail', { e: String(error) }), true)
      removeViewTab(tab.uiId)
    } finally {
      tab.loadingMsgs = false
    }
  }

  if (opened) {
    await acknowledgeDesktopPetTask(target).catch((error) => {
      console.warn('[desktop-pet] failed to acknowledge activity:', error)
    })
  }
}

onMounted(() => {
  installWindowClosePrompt()
  installBeforeQuitSave()
  installLiveTailListeners()
  installTerminalTurnListeners()
  void refreshTurnHookStatus()
  void restoreDesktopPetWindow().catch((error) => {
    console.warn('[desktop-pet] failed to restore window:', error)
  })
})

onUnmounted(() => {
  windowCloseUnlisten?.()
  windowCloseUnlisten = null
  beforeQuitUnlisten?.()
  beforeQuitUnlisten = null
  menuUnlisten?.()
  menuUnlisten = null
  window.clearInterval(tuiTitleSyncTimer)
  tuiTitleSyncTimer = 0
  liveUnlisteners.forEach((u) => u())
  liveUnlisteners = []
  // 清理所有 view tab 的定时器
  for (const vt of viewTabs.value) {
    window.clearTimeout(vt.liveFadeTimer)
    if (vt.type === 'chat' && vt.chatSession) void closeChat(vt.chatSession.uiId)
  }
  document.body.classList.remove('is-sidebar-resizing')
  window.removeEventListener('resize', onWindowResize)
  window.removeEventListener('pointermove', onSidebarResizePointerMove)
  window.removeEventListener('pointerup', onSidebarResizePointerUp)
  window.removeEventListener('pointercancel', onSidebarResizePointerUp)
  clearPendingLiveNotification()
  api.unwatchSession().catch(() => {})
  window.removeEventListener('focus', onFocus)
  window.removeEventListener('blur', onBlur)
  document.removeEventListener('visibilitychange', onVisibilityChange)
})

// 全局搜索命中：跳到对应项目并打开会话；正文命中再滚到目标消息并触发闪烁动画。
// 如果命中所在项目不在已加载列表里（极少见 —— list_projects 通常涵盖全部），
// 先刷一次项目列表再跳。
async function onGlobalSearchOpen(hit: SearchHit) {
  setActiveTui(null)
  showStats.value = false
  showTrash.value = false
  showExportHistory.value = false
  showPricing.value = false
  sessionStatsTarget.value = null
  if (activeDir.value !== hit.projectKey) {
    if (!projects.value.some((p) => p.dirName === hit.projectKey)) {
      await loadProjects()
    }
    await selectProject(hit.projectKey)
  }
  await openChat(hit.session)
  if (hit.matchedField === 'text' && typeof hit.matchMsgIndex === 'number') {
    for (let i = 0; i < 10; i++) {
      await nextTick()
      if (chatViewRef.value) break
    }
    chatViewRef.value?.flashMessage(hit.matchMsgIndex, hit.matchMsgUuid ?? undefined)
  }
}

// 把 App 的全部处理函数下发给分屏格子（PaneContent）。见 paneActions.ts 的说明：
// PaneContent 只呈现，行为一律回调到这里；「交互即聚焦」保证无参 action 作用在被点的格子。
provide<PaneActions>(PaneActionsKey, {
  onTuiListClick,
  onTuiViewTabClick,
  onTuiViewClose,
  onViewRename,
  onViewCloseOthers,
  onViewCloseProject,
  onCloseOthersAll,
  onCloseAll,
  onTuiTabClosed,
  openRenameFromTuiTab,
  openRenameFromSavedTab,
  saveTabState,
  newSession,
  newDefaultAction,
  newGuiSession,
  newShellSession,
  hydrateSavedTab: (saved) => { void hydrateSavedTabOnce(saved) },
  closeLiveChat,
  openRenameLiveChat,
  forkLiveChat,
  editCancelledPrompt,
  archiveLiveChat,
  switchLiveChatToRead,
  openLiveChatStats,
  exportLiveChat,
  deleteFromLiveChat,
  closeActiveViewTab,
  openChat,
  deleteSession,
  resumeHere,
  resumeChatFromSession,
  openRename,
  copyText,
  exportSession,
  restore,
  openSessionStats,
  reveal,
  chatFromList,
  notifyArchivedBlock,
  exportFromList,
  refreshSessions,
  createWorktree: () => { if (activeProject.value) openWorktreeModal(activeProject.value) },
  exitPane,
  splitH: () => splitFocusedPane('row'),
  splitV: () => splitFocusedPane('col'),
  openGitChanges: openGitChangesTab,
  loadMore,
  onListScroll,
  batchDeleteSessions,
  batchExportSessions,
  selectProject,
  switchAgent,
  openRepo,
})
</script>

<template>
  <video
    v-if="backgroundVideoUrl"
    class="app-background-video"
    :src="backgroundVideoUrl"
    autoplay
    loop
    muted
    playsinline
    aria-hidden="true"
  />
  <div
    class="app"
    :style="appStyle"
    :class="[
      `agent-${agent}`,
      sidebarOpen ? 'sidebar-open' : 'sidebar-closed',
      { 'sidebar-resizing': sidebarResizing },
      { 'is-blurred': !windowFocused },
    ]"
  >
    <WindowsTitlebar
      v-if="isWindows"
      :menus="windowMenus"
      :handlers="menuHandlers"
    />
    <!-- 顶栏：normal flow，整条都是 macOS 拖动区。
         data-tauri-drag-region="deep" 让整个子树（除按钮等可点击元素外）
         都触发原生 startDragging；button/A/INPUT 等会自动 block 拖动，
         不需要手动 no-drag。同时保留 -webkit-app-region: drag 做 OS 层兜底。 -->
    <div class="app-topbar" :data-tauri-drag-region="isWindows ? undefined : 'deep'">
      <SidebarTopbar
        :show-trash="showTrash"
        :show-stats="showStats"
        :show-history="showExportHistory"
        :show-pricing="showPricing"
        :has-trash="trash.length > 0"
        @toggle-sidebar="toggleSidebar"
        @open-trash="loadTrash"
        @open-stats="openStats"
        @open-history="openExportHistory"
        @open-pricing="openPricing"
      />
      <!-- 顶栏右侧分发：每个页面把自己的工具栏组件挂这里。
           本身仍是 macOS 拖动区域，组件内部的可交互元素由 CSS 单独标 no-drag。 -->
      <div class="topbar-drag">
        <div class="topbar-context">
          <span class="topbar-agent-mark" aria-hidden="true">{{ activeAgentLabel.charAt(0) }}</span>
          <span class="topbar-context-text">
            <span class="topbar-context-title">{{ topbarContextTitle }}</span>
            <span v-if="topbarContextMeta" class="topbar-context-meta">
              / {{ topbarContextMeta }}
            </span>
          </span>
        </div>
        <!-- StatsView 自带顶部控制条，这里就让出空间（保持拖动区域）。
             showStats 优先级要高于 openSession，否则进入会话统计模式时
             还会渲染 ChatTopbar 的「会话统计」按钮，造成视觉重复。 -->
        <div v-if="showStats || (activeUiId === null && (activeViewTab?.type === 'git' || (liveChat && activeViewTab?.type === 'chat')))" />
        <TuiTopbar v-else-if="activeUiId !== null" />
        <ChatTopbar v-else-if="openSession && activeViewTab" />
        <TrashTopbar
          v-else-if="showTrash"
          :items="trash"
        />
        <SessionsTopbar
          v-else-if="activeProject"
          :sessions="visibleSessions"
        />
        <div v-else class="chat-topbar">
          <button
            type="button"
            class="ct-search topbar-global-search"
            v-tooltip="t('search.global.placeholder')"
            @click="openGlobalSearch"
          >
            <IconSearch class="ct-search-ic" />
            <span>{{ t('search.global.placeholder') }}</span>
          </button>
        </div>
      </div>
    </div>

    <div class="app-body">
    <!-- 侧栏 -->
    <Sidebar
      v-show="sidebarOpen"
      :agent="agent"
      :projects="projects"
      :active-dir="activeDir"
      :show-trash="showTrash"
      :proj-prefs="projPrefs"
      :project-order="currentProjectOrder"
      :refreshing="refreshing"
      @switch-agent="switchAgent"
      @select-project="(dir) => selectProject(dir, { activateTerminal: true })"
      @context-menu="openCtxMenu"
      @open-settings="(tab) => { settingsTab = tab; showSettings = true }"
      @refresh="refreshAll"
      @add-bookmark="addBookmark"
      @batch-delete="batchDeleteProjects"
      @reorder-projects="setProjectOrder"
      ref="sidebarRef"
    />
    <div
      v-show="sidebarOpen"
      class="sidebar-resizer"
      role="separator"
      aria-orientation="vertical"
      @pointerdown="onSidebarResizePointerDown"
    />

    <!-- 主区 -->
    <main class="main">
      <!-- 全局全区视图（统计 / 回收站 / 导出历史 / 计费）—— 接管整个主区，盖住分屏格子。
           它们是 app 级页面（由侧栏顶栏触发），不属于任何 pane。退出后分屏布局原样恢复。 -->
      <div
        v-if="showStats || showTrash || showExportHistory || showPricing"
        class="view-layer global-view-layer"
      >
        <StatsView
          v-if="showStats"
          :session="sessionStatsTarget"
          @close="closeStats"
          @open-project="(dir) => selectProject(dir)"
          @open-session="openSessionStatsFromGlobal"
        />
        <TrashView
          v-else-if="showTrash"
          :trash="trash"
          :loading="loadingList"
          @clear="clearTrash"
          @open="openTrashSession"
          @restore="restore"
          @permanent-delete="permanentDelete"
          @batch-restore="batchRestore"
          @batch-permanent-delete="batchPermanentDelete"
        />
        <ExportHistoryView
          v-else-if="showExportHistory"
          @open="openHistorySession"
        />
        <PricingView v-else-if="showPricing" />
      </div>

      <!-- 分屏格子：递归 PaneGrid 渲染整棵分屏树。每格 strip + 会话/列表/欢迎 + TUI 层由
           PaneContent 按各自 pane 解出。multi class 只在多格子时给聚焦格子加聚焦描边。 -->
      <div v-else class="pane-grid" :class="{ multi: paneCount > 1 }">
        <PaneGrid
          :node="currentLayout.tree"
          :active-project="activeProject"
          :agent="agent"
          :projects="projects"
          :sessions="visibleSessions"
          :session-total="sessionTotal"
          :loading-list="loadingList"
          :loading-more="loadingMore"
          :open-trash-item="openTrashItem"
          :has-git="projectHasGit"
        />
      </div>
    </main>
    </div>

    <!-- 确认弹窗 -->
    <ConfirmModal
      :show="confirm.show"
      :title="confirm.title"
      :message="confirm.message"
      :ok-text="confirm.okText"
      :danger="confirm.danger"
      :alt-text="confirm.altText"
      @confirm="runConfirm"
      @cancel="confirm.show = false"
      @alt="runAlt"
    />

    <Transition name="fade">
      <div
        v-if="isWindows && windowClosePrompt.show"
        class="overlay overlay-confirm"
        @click.self="windowClosePrompt.show = false"
      >
        <div class="modal window-close-modal" role="dialog" aria-modal="true">
          <h3>{{ t('windowClose.title') }}</h3>
          <p>{{ t('windowClose.body') }}</p>
          <label class="window-close-remember">
            <input v-model="windowClosePrompt.remember" type="checkbox">
            <span>{{ t('windowClose.remember') }}</span>
          </label>
          <div class="modal-actions">
            <button class="btn" @click="windowClosePrompt.show = false">
              {{ t('common.cancel') }}
            </button>
            <button class="btn danger" @click="chooseWindowCloseAction('exit')">
              {{ t('windowClose.exitApp') }}
            </button>
            <button class="btn primary" @click="chooseWindowCloseAction('tray')">
              {{ t('windowClose.minimizeToTray') }}
            </button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- 设置弹窗 -->
    <Transition name="fade">
      <SettingsModal
        v-if="showSettings"
        :cache-bytes="cacheBytes"
        :initial-tab="settingsTab"
        @close="showSettings = false; settingsTab = undefined"
        @clear-cache="onClearCache"
        @clear-tabs="onClearTabs"
      />
    </Transition>

    <!-- 重命名会话 -->
    <RenameModal
      v-model="renameModal.value"
      :show="renameModal.show"
      :default-title="renameModal.defaultTitle"
      @confirm="confirmRename"
      @cancel="renameModal.show = false"
    />

    <!-- 全局搜索（⌘⇧F / Ctrl⇧F） -->
    <GlobalSearchModal
      :show="globalSearchOpen"
      :agent="agent"
      @update:show="globalSearchOpen = $event"
      @open="onGlobalSearchOpen"
    />

    <!-- 项目右键菜单 -->
    <ProjectContextMenu
      v-if="ctxMenu"
      :x="ctxMenu.x"
      :y="ctxMenu.y"
      :project="ctxMenu.project"
      :proj-state="projStateOf(ctxMenu.project)"
      :is-git-repo="ctxMenu.isGitRepo"
      @toggle-state="ctxToggleState"
      @open-folder="ctxOpenProjectFolder"
      @refresh="ctxRefresh"
      @delete="ctxDeleteProject"
      @remove-bookmark="ctxRemoveBookmark"
      @create-worktree="ctxCreateWorktree"
      @delete-worktree="ctxDeleteWorktree"
    />

    <!-- 创建 worktree 命名弹框 -->
    <WorktreeModal
      v-model="worktreeModal.value"
      :show="worktreeModal.show"
      :project-path="worktreeModal.projectPath"
      @confirm="confirmCreateWorktree"
      @cancel="worktreeModal.show = false"
    />

    <!-- btw 侧聊浮框（右上角可拖动；Teleport 到 body，与主视图层无关） -->
    <ChatSidePanel
      v-if="sideChat && liveChat?.agent === 'claude'"
      :session="sideChat"
      :hidden="!liveChat || activeUiId !== null"
    />
    <!-- Codex `/side`：单独组件与 state，使用 ephemeral app-server fork。 -->
    <CodexSidePanel
      v-if="codexSideChat && liveChat?.agent === 'codex'"
      :session="codexSideChat"
      :hidden="!liveChat || activeUiId !== null"
    />

    <!-- toast -->
    <Transition name="fade">
      <div v-if="toast.show" class="toast" :class="{ error: toast.error }">
        {{ toast.msg }}
      </div>
    </Transition>
  </div>
</template>
