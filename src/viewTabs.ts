// View tabs — session 查看 / GUI chat 的 tab 化管理。
//
// 对标 terminals.ts（TUI tab），但不需要 xterm：
//   session tab → 只读查看会话，msgs 从磁盘读取
//   chat tab    → live GUI chat（子进程由 chatSessions 管理），msgs 从 ChatSession 实时推送
//
// 每个 tab 按 (agent, projectKey) 归属，切项目时隐藏但不杀；和终端 tab 行为一致。

import { ref, computed } from 'vue'
import type { Agent, SessionMeta, Msg } from './types'
import type { ChatSession } from './chatSessions'
import type { TabStatusKind } from './tabStatus'
import { activeViewTabId, panes, focusPane, ensureLayout } from './panes'

let nextViewTabId = 1

export interface ViewTab {
  uiId: number
  type: 'session' | 'chat' | 'git'
  agent: Agent
  projectKey: string
  /** 所属分屏格子 id（见 panes.ts）。 */
  paneId: number
  title: string
  createdAt: number
  // session tab
  session: SessionMeta | null
  msgs: Msg[]
  loadingMsgs: boolean
  // chat tab
  chatSession: ChatSession | null
  /** 最近一次已查看的 GUI chat turn；用于只在后台完成后显示一次 done 状态。 */
  lastViewedChatTurnStartedAt: number
  /** 最近一次已查看的 GUI chat 错误；同一个错误只在后台提醒一次。 */
  lastViewedChatErrorKey: string | null
  // 来源会话（chat tab 续聊时绑定的原始 transcript）
  sourceSession: SessionMeta | null
  // live tail 状态（session tab 的文件追踪）
  liveTailing: boolean
  liveFadeTimer: number
  // 回收站来源（session tab 从回收站打开时）
  trashAgent: Agent | null
  // 导出历史来源 agent（可能与侧栏 agent 不同）
  importedAgent: Agent | null
  // git tab：仓库工作目录 + 当前查看的 ref（"working" 或 commit hash）
  gitCwd: string | null
  gitRef: string | null
  gitSelectedPath: string | null
}

export const viewTabs = ref<ViewTab[]>([])
// activeViewTabId 现在是「聚焦 pane 的 activeViewTabId」投影（真身在 panes.ts）；从这里
// re-export 保持既有 import 路径。activeViewTab（解出真正的 ViewTab 对象）留在本模块，因为
// 它要查 viewTabs 数组。
export { activeViewTabId }

export const activeViewTab = computed<ViewTab | null>(() =>
  viewTabs.value.find(t => t.uiId === activeViewTabId.value) ?? null,
)

/** 在 view tab 自己的 pane 里激活它：露出 view 层、指向该 tab、聚焦该 pane。 */
function activateViewTabInPane(tab: ViewTab) {
  let pane = panes.get(tab.paneId)
  // 兜底：pane 不存在，或恢复后 paneId 撞到了别的项目的格子（id 每次启动重排）——
  // 一律回落到本项目聚焦格子，避免 tab 落进错误的 pane。
  if (!pane || pane.agent !== tab.agent || pane.projectKey !== tab.projectKey) {
    tab.paneId = ensureLayout(tab.agent, tab.projectKey).focusedPaneId
    pane = panes.get(tab.paneId)
  }
  if (pane) {
    pane.activeUiId = null
    pane.activeViewTabId = tab.uiId
    focusPane(pane.id)
  }
}

let _suppressActivate = false

export function suppressActivation(fn: () => void) {
  _suppressActivate = true
  try { fn() } finally { _suppressActivate = false }
}

export function createViewTab(partial: Partial<ViewTab> & Pick<ViewTab, 'type' | 'agent' | 'projectKey'>): ViewTab {
  const uiId = nextViewTabId++
  const tab: ViewTab = {
    uiId,
    title: '',
    session: null,
    msgs: [],
    loadingMsgs: false,
    chatSession: null,
    lastViewedChatTurnStartedAt: 0,
    lastViewedChatErrorKey: null,
    sourceSession: null,
    liveTailing: false,
    liveFadeTimer: 0,
    trashAgent: null,
    importedAgent: null,
    gitCwd: null,
    gitRef: null,
    gitSelectedPath: null,
    ...partial,
    // paneId 放在 spread 之后并带兜底：恢复旧数据（无 paneId）或 partial 显式传了 undefined 时，
    // 仍回落到本项目聚焦格子，而不是被 undefined 覆盖。
    paneId: partial.paneId ?? ensureLayout(partial.agent, partial.projectKey).focusedPaneId,
    createdAt: partial.createdAt ?? Date.now(),
  }
  viewTabs.value.push(tab)
  // Return the reactive proxy, not the plain object, so callers' mutations trigger reactivity
  const proxy = viewTabs.value[viewTabs.value.length - 1]
  if (!_suppressActivate) activateViewTabInPane(proxy)
  return proxy
}

export function findViewTab(predicate: (t: ViewTab) => boolean): ViewTab | undefined {
  return viewTabs.value.find(predicate)
}

/** GUI chat tab 的状态语义与 TUI tab 对齐，但直接使用 chat 会话的权威生命周期。 */
export function viewTabStatusKind(tab: ViewTab, isActive: boolean): TabStatusKind {
  if (tab.type !== 'chat' || !tab.chatSession) return 'none'
  const session = tab.chatSession
  const errorKey = chatErrorKey(session)
  if (errorKey) {
    return !isActive && errorKey !== tab.lastViewedChatErrorKey ? 'error' : 'none'
  }
  if (session.status === 'exited') return 'exited'
  if (session.pendingPermissions.length || session.pendingQuestions.length) return 'blocked'
  if (session.turnState === 'running') return 'working'
  if (
    !isActive &&
    session.lastTurnOutcome === 'completed' &&
    session.turnStartedAt > tab.lastViewedChatTurnStartedAt
  ) return 'done'
  return 'none'
}

function chatErrorKey(session: ChatSession): string | null {
  if (session.status !== 'error' && session.lastTurnOutcome !== 'failed') return null
  return [
    session.chatId ?? '',
    session.turnStartedAt,
    session.lastTurnOutcome ?? '',
    session.errorMessage ?? '',
  ].join('\0')
}

/** 确认已显示的完成/错误状态；运行中切走仍应在后台完成时亮 done。 */
export function markViewTabViewed(tab: ViewTab) {
  const session = tab.chatSession
  if (session) {
    const errorKey = chatErrorKey(session)
    if (errorKey) tab.lastViewedChatErrorKey = errorKey
  }
  if (
    tab.type === 'chat' &&
    session?.turnState === 'idle' &&
    session.lastTurnOutcome === 'completed'
  ) {
    tab.lastViewedChatTurnStartedAt = Math.max(
      tab.lastViewedChatTurnStartedAt,
      session.turnStartedAt,
    )
  }
}

export function removeViewTab(uiId: number) {
  const idx = viewTabs.value.findIndex(t => t.uiId === uiId)
  if (idx < 0) return
  const tab = viewTabs.value[idx]
  window.clearTimeout(tab.liveFadeTimer)
  viewTabs.value.splice(idx, 1)
  // 主动丢弃重引用：session 的 msgs 可能是几 MB 的 transcript，chatSession 指向已停的
  // 会话对象。清空后立刻可回收，避免关 tab 后内存不降。
  tab.msgs = []
  tab.chatSession = null
  const pane = panes.get(tab.paneId)
  if (pane && pane.activeViewTabId === uiId) {
    // 关闭当前 tab 后，激活同 pane 的上一个 view tab（如有），否则露出主页
    const sameCtx = viewTabs.value.filter(
      t => t.agent === tab.agent && t.projectKey === tab.projectKey && t.paneId === tab.paneId,
    )
    pane.activeViewTabId = sameCtx.length > 0 ? sameCtx[sameCtx.length - 1].uiId : null
  }
}

export function setActiveViewTab(uiId: number | null) {
  if (uiId === null) {
    activeViewTabId.value = null
    return
  }
  const tab = viewTabs.value.find(t => t.uiId === uiId)
  if (!tab) return
  activateViewTabInPane(tab)
}

export function visibleViewTabs(agent: Agent, projectKey: string | null): ViewTab[] {
  return viewTabs.value.filter(
    t => t.agent === agent && t.projectKey === (projectKey ?? ''),
  )
}

export function closeViewTabsByProject(projectKey: string) {
  const toRemove = viewTabs.value.filter(t => t.projectKey === projectKey)
  for (const t of toRemove) removeViewTab(t.uiId)
}

/** 合成 key 被并入真实项目时，把挂在旧 key 上的 view tab（会话/聊天/git）迁到新 key，
 *  否则 visibleViewTabs(新 key) 查不到它们 → tab 条从标签栏消失。 */
export function migrateViewTabsProjectKey(oldKey: string, newKey: string) {
  if (oldKey === newKey) return
  for (const tab of viewTabs.value) {
    if (tab.projectKey === oldKey) tab.projectKey = newKey
  }
}

const SAVED_VIEW_TABS_KEY = 'savedViewTabs:v1'

export interface SavedViewTab {
  type: 'session' | 'chat' | 'git'
  agent: Agent
  projectKey: string
  /** 上次退出时所属分屏格子 id；恢复时回落到本项目聚焦格子（见 activateViewTabInPane）。 */
  paneId: number
  title: string
  createdAt: number
  session: SessionMeta | null
  sessionId: string | null
  trashAgent: Agent | null
  importedAgent: Agent | null
  gitCwd?: string | null
  gitRef?: string | null
  gitSelectedPath?: string | null
  isActive?: boolean
}

let _viewTabsRestoreComplete = false
export function markViewTabsRestored() { _viewTabsRestoreComplete = true }

export function persistViewTabs() {
  if (!_viewTabsRestoreComplete) return
  const activePaneTabIds = new Set<number>()
  for (const pane of panes.values()) {
    if (pane.activeViewTabId != null) activePaneTabIds.add(pane.activeViewTabId)
  }
  const items: SavedViewTab[] = viewTabs.value
    .filter(t => (t.type === 'session' && t.session) || t.type === 'chat' || t.type === 'git')
    .map(t => ({
      type: t.type,
      agent: t.agent,
      projectKey: t.projectKey,
      paneId: t.paneId,
      title: t.title,
      createdAt: t.createdAt,
      session: t.session ?? t.sourceSession,
      sessionId: t.chatSession?.sessionId ?? t.session?.id ?? null,
      trashAgent: t.trashAgent,
      importedAgent: t.importedAgent,
      gitCwd: t.gitCwd,
      gitRef: t.gitRef,
      gitSelectedPath: t.gitSelectedPath,
      isActive: activePaneTabIds.has(t.uiId),
    }))
  try {
    localStorage.setItem(SAVED_VIEW_TABS_KEY, JSON.stringify({
      tabs: items,
      activeIdx: activeViewTabId.value != null
        ? viewTabs.value.findIndex(t => t.uiId === activeViewTabId.value)
        : null,
    }))
  } catch {}
}

export function clearSavedViewTabs() {
  try { localStorage.removeItem(SAVED_VIEW_TABS_KEY) } catch {}
}

export function loadSavedViewTabs(): { tabs: SavedViewTab[]; activeIdx: number | null } {
  try {
    const raw = localStorage.getItem(SAVED_VIEW_TABS_KEY)
    if (!raw) return { tabs: [], activeIdx: null }
    const data = JSON.parse(raw)
    if (!data || !Array.isArray(data.tabs)) return { tabs: [], activeIdx: null }
    const valid = data.tabs.filter((t: any) => t && t.agent && (
      (t.type === 'session' && t.session) || t.type === 'chat' || (t.type === 'git' && t.gitCwd)
    )) as SavedViewTab[]
    const seen = new Set<string>()
    const deduped: SavedViewTab[] = []
    for (let i = valid.length - 1; i >= 0; i--) {
      const t = valid[i]
      const key = `${t.type}:${t.agent}:${t.type === 'git' ? `${t.gitCwd}:${t.title}` : (t.sessionId ?? t.session?.path ?? '')}`
      if (seen.has(key)) continue
      seen.add(key)
      deduped.unshift(t)
    }
    for (let i = 0; i < deduped.length; i++) {
      if (!deduped[i].createdAt) deduped[i].createdAt = i + 1
    }
    return {
      tabs: deduped,
      activeIdx: typeof data.activeIdx === 'number' ? data.activeIdx : null,
    }
  } catch {
    return { tabs: [], activeIdx: null }
  }
}
