// 分屏格子（PaneContent）→ App.vue 的动作总线。
//
// PaneContent 是「呈现层」：它只负责按 pane 的 active tab 渲染 strip / 会话 / 列表 / 欢迎页，
// 所有真正的行为（开会话、删除、导出、resume、新建 tab…）都留在 App.vue —— 那些函数闭包
// 了 App 的一大堆 ref/状态，没法搬进子组件。为了不给 PaneContent 挂几十个 emit / prop，
// App.vue 用 provide() 把这一整包处理函数下发，PaneContent inject 后直接调用。
//
// 语义靠「交互即聚焦」保证正确：PaneContent 根节点 pointerdown 时先 focusPane，于是所有
// 读 activeViewTab（= 聚焦 pane 的投影）的无参 handler（exportSession / closeActiveViewTab …）
// 天然作用在被点的那个格子上，无需把 tab 逐个透传。

import type { InjectionKey } from 'vue'
import type { Agent, SessionMeta, TrashItem } from './types'
import type { ExportKind } from './export'
import type { ViewTab } from './viewTabs'
import type { TerminalTab, SavedTab } from './terminals'
import type { ChatHistoryEntry } from './chatInputHistory'

export interface PaneActions {
  // —— TerminalStrip ——
  onTuiListClick: () => void
  onTuiViewTabClick: (uiId: number) => void
  onTuiViewClose: (uiId: number) => void
  onViewRename: (vt: ViewTab) => void
  onViewCloseOthers: (vt: ViewTab) => void
  onViewCloseProject: (type: 'session' | 'chat' | 'git') => void
  onCloseOthersAll: (keepUiId: number, keepKind: 'tui' | 'view') => void
  onCloseAll: () => void
  onTuiTabClosed: () => void
  openRenameFromTuiTab: (tab: TerminalTab) => void
  openRenameFromSavedTab: (saved: SavedTab) => void
  saveTabState: () => void
  newSession: () => void
  newDefaultAction: () => void
  newGuiSession: () => void
  newShellSession: () => void
  hydrateSavedTab: (saved: SavedTab) => void
  // —— ChatView（GUI chat tab）——
  closeLiveChat: (tabUiId?: number) => void
  openRenameLiveChat: () => void
  forkLiveChat: () => void
  editCancelledPrompt: (
    entry: ChatHistoryEntry,
    messageIndex: number,
    priorUserTurns: number,
  ) => void
  archiveLiveChat: () => void
  switchLiveChatToRead: () => void
  openLiveChatStats: () => void
  exportLiveChat: (kind: ExportKind) => void
  deleteFromLiveChat: () => void
  // —— ChatView（只读会话 tab）——
  closeActiveViewTab: () => void
  openChat: (s: SessionMeta) => void
  deleteSession: (s: SessionMeta) => void
  resumeHere: (s: SessionMeta) => void
  resumeChatFromSession: (s: SessionMeta) => void
  openRename: (s: SessionMeta) => void
  copyText: (text: string) => void
  exportSession: (kind: ExportKind) => void
  restore: (item: TrashItem) => void
  openSessionStats: () => void
  reveal: (path: string) => void
  // —— SessionsView（项目主页 / 会话列表）——
  chatFromList: (s: SessionMeta) => void
  notifyArchivedBlock: (cmd: string) => void
  exportFromList: (s: SessionMeta, kind: ExportKind) => void
  refreshSessions: () => void
  /** 顶栏「创建 Worktree」：对当前一级 git 项目开命名弹框，逻辑同侧栏右键。 */
  createWorktree: () => void
  /** 退出指定分屏格子：关闭并释放该格所有 tab（二次确认后调用）。 */
  exitPane: (paneId: number) => void
  splitH: () => void
  splitV: () => void
  /** 打开仓库的工作区变更；传 cwd 时优先查看会话所属仓库。 */
  openGitChanges: (cwd?: string) => void
  loadMore: () => void
  onListScroll: (scrollTop: number) => void
  batchDeleteSessions: () => void
  batchExportSessions: (kind: ExportKind) => void
  // —— WelcomeView ——
  selectProject: (dir: string) => void
  switchAgent: (a: Agent) => void
  openRepo: () => void
}

export const PaneActionsKey: InjectionKey<PaneActions> = Symbol('paneActions')
