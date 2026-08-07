<script setup lang="ts">
// 一个分屏格子（pane）的完整内容：顶部 TerminalStrip + 主体（view 层 ⊕ TUI 层）。
//
// 这是把原来 App.vue 里「.main 的 strip + main-body」整块搬出来的呈现组件，改成按传入的
// `pane` 渲染，从而可以在 PaneGrid 里被实例化多次（每个叶子一个）。
//   · view 数据（openSession / liveChat / chatMsgs …）全部由**本 pane 的 active view tab**
//     派生 —— 不再读全局 activeViewTab 投影，这样多格子各显示各的。
//   · 行为（删除 / 导出 / resume / 新建 tab …）通过 inject 的 PaneActions 调用 App.vue。
//   · 根节点 pointerdown（capture）先聚焦 pane 并标记当前 TUI tab 已查看，于是那些读「聚焦
//     pane」的无参 action 天然作用在被点的格子上。
//
// 全局全区视图（stats / trash / history / pricing）不在这里 —— 它们在 App.vue 顶层接管整个
// 主区，不进分屏格子。

import { computed, inject, ref, watchEffect, onUnmounted } from 'vue'
import type { Agent, ProjectInfo, SessionMeta, TrashItem, Msg } from '../types'
import type { ChatSession } from '../chatSessions'
import { t } from '../i18n'
import { viewTabs, type ViewTab } from '../viewTabs'
import { type Pane, focusPane, isFocused, paneCount } from '../panes'
import { markTabViewed } from '../terminals'
import { dragState } from '../tabDrag'
import { registerPaneViews, unregisterPaneViews } from '../paneRegistry'
import { PaneActionsKey, type PaneActions } from '../paneActions'
import TerminalStrip from './TerminalStrip.vue'
import TerminalPaneSlot from './TerminalPaneSlot.vue'
import ChatView from '../views/ChatView.vue'
import SessionsView from '../views/SessionsView.vue'
import WelcomeView from '../views/WelcomeView.vue'
import GitChangesView from '../views/GitChangesView.vue'

const props = defineProps<{
  pane: Pane
  /** 当前侧栏选中的项目（所有格子共享同一 (agent, project)）。 */
  activeProject: ProjectInfo | undefined
  agent: Agent
  projects: ProjectInfo[]
  sessions: SessionMeta[]
  sessionTotal: number
  loadingList: boolean
  loadingMore: boolean
  /** 聚焦格子若打开的是回收站会话则非空（决定只读/恢复）。 */
  openTrashItem: TrashItem | null
  hasGit: boolean
}>()

const actions = inject(PaneActionsKey) as PaneActions

function focusCurrentPane() {
  focusPane(props.pane.id)
  if (props.pane.activeUiId !== null) markTabViewed(props.pane.activeUiId)
}

// 本 pane 的 ChatView / SessionsView 实例登记进注册表，App.vue 按聚焦 paneId 取用
// （flashMessage / onLiveAppend / 列表 scrollEl）。子实例挂载后 ref 变化会重登记。
const chatView = ref<InstanceType<typeof ChatView> | null>(null)
const sessionsView = ref<InstanceType<typeof SessionsView> | null>(null)
watchEffect(() => {
  registerPaneViews(props.pane.id, { chatView: chatView.value, sessionsView: sessionsView.value })
})
onUnmounted(() => unregisterPaneViews(props.pane.id))

// —— 本 pane 的 tab ——
const paneViewTab = computed<ViewTab | null>(
  () => viewTabs.value.find((tb) => tb.uiId === props.pane.activeViewTabId) ?? null,
)
const paneViewTabs = computed<ViewTab[]>(() =>
  viewTabs.value.filter(
    (tb) =>
      tb.agent === props.pane.agent &&
      tb.projectKey === props.pane.projectKey &&
      tb.paneId === props.pane.id,
  ),
)
// strip 只在选中了项目时显示 List/新建 等 in-project 元素；全局视图接管时本组件根本不渲染，
// 所以这里等价于 !!activeProject。
const inProjectBrowse = computed(() => !!props.activeProject)

// —— 从本 pane 的 view tab 派生的展示数据（逻辑同原 App.vue，仅数据源换成 paneViewTab）——
const openSession = computed<SessionMeta | null>(() => {
  const tab = paneViewTab.value
  if (!tab) return null
  if (tab.type === 'session') return tab.session
  if (tab.type === 'chat') return tab.sourceSession
  return null
})
const liveChat = computed<ChatSession | null>(() => {
  const tab = paneViewTab.value
  return tab?.type === 'chat' ? tab.chatSession : null
})
const chatMsgs = computed<Msg[]>(() => {
  const tab = paneViewTab.value
  if (!tab) return []
  if (tab.type === 'session') return tab.msgs
  if (tab.type === 'chat') return tab.chatSession?.msgs ?? []
  return []
})
const liveTailing = computed(() => paneViewTab.value?.liveTailing ?? false)
const chatAgent = computed<Agent>(
  () =>
    paneViewTab.value?.trashAgent ??
    paneViewTab.value?.importedAgent ??
    paneViewTab.value?.agent ??
    props.agent,
)
const chatCwd = computed<string>(() => {
  if (props.openTrashItem) return ''
  return openSession.value?.cwd || props.activeProject?.displayPath || ''
})
const liveChatSourceSession = computed<SessionMeta | null>(() => {
  const tab = paneViewTab.value
  if (!tab || tab.type !== 'chat') return null
  return tab.sourceSession
})
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
  } as SessionMeta
})
</script>

<template>
  <div
    class="pane"
    :class="{
      'pane-focused': isFocused(pane.id),
      'pane-drop-target':
        dragState.active && dragState.overPaneId === pane.id && dragState.sourcePaneId !== pane.id,
    }"
    :data-pane-id="pane.id"
    @pointerdown.capture="focusCurrentPane"
  >
    <TerminalStrip
      :pane="pane"
      :agent="pane.agent"
      :project-key="pane.projectKey"
      :in-project-browse="inProjectBrowse"
      :has-git="hasGit"
      :view-tabs="paneViewTabs"
      :active-view-tab-id="pane.activeViewTabId"
      @list-click="actions.onTuiListClick"
      @view-click="actions.onTuiViewTabClick"
      @view-close="actions.onTuiViewClose"
      @view-rename="actions.onViewRename"
      @view-close-others="actions.onViewCloseOthers"
      @view-close-project="actions.onViewCloseProject"
      @close-others-all="actions.onCloseOthersAll"
      @close-all="actions.onCloseAll"
      @tab-closed="actions.onTuiTabClosed"
      @tab-rename="actions.openRenameFromTuiTab"
      @saved-rename="actions.openRenameFromSavedTab"
      @tabs-reordered="actions.saveTabState"
      @new-session="actions.newSession"
      @new-default="actions.newDefaultAction"
      @new-gui-session="actions.newGuiSession"
      @new-shell="actions.newShellSession"
      @git-changes="actions.openGitChanges"
      @refresh="actions.refreshSessions"
      @hydrate-saved="actions.hydrateSavedTab"
    />

    <div class="main-body">
      <!-- view 层：本 pane 无 active TUI tab 时显示 -->
      <div class="view-layer" v-show="pane.activeUiId === null">
        <!-- live GUI chat tab -->
        <ChatView
          v-if="paneViewTab?.type === 'chat' && liveChat"
          :agent="liveChat.agent"
          :session="liveChatMeta"
          :messages="liveChat.msgs"
          :live-session="liveChat"
          :cwd="liveChat.cwd"
          :has-read-view="!!liveChatSourceSession"
          @back="actions.closeLiveChat()"
          @rename="actions.openRenameLiveChat"
          @fork="actions.forkLiveChat"
          @edit-cancelled="actions.editCancelledPrompt"
          @archive="actions.archiveLiveChat"
          @switch-to-read="actions.switchLiveChatToRead"
          @open-session-stats="actions.openLiveChatStats"
          @reveal="actions.reveal(liveChatSourceSession?.path || liveChat.cwd || '')"
          @export-md="actions.exportLiveChat('md')"
          @export-html="actions.exportLiveChat('html')"
          @export-json="actions.exportLiveChat('json')"
          @delete="actions.deleteFromLiveChat"
        />

        <!-- session tab（只读查看） -->
        <template v-else-if="paneViewTab?.type === 'session' && openSession">
          <div v-if="paneViewTab.loadingMsgs" class="loading">{{ t('common.loading') }}</div>
          <ChatView
            v-else
            ref="chatView"
            :agent="chatAgent"
            :session="openSession"
            :messages="chatMsgs"
            :trashed="!!openTrashItem"
            :live="liveTailing"
            :cwd="chatCwd"
            @back="actions.closeActiveViewTab"
            @refresh="actions.openChat(openSession)"
            @delete="actions.deleteSession(openSession)"
            @resume-here="actions.resumeHere(openSession)"
            @switch-to-chat="actions.resumeChatFromSession(openSession)"
            @rename="actions.openRename(openSession)"
            @reveal="actions.reveal(openSession.path)"
            @copy-id="actions.copyText(openSession.id)"
            @export-md="actions.exportSession('md')"
            @export-html="actions.exportSession('html')"
            @export-json="actions.exportSession('json')"
            @restore="openTrashItem && actions.restore(openTrashItem)"
            @open-session-stats="actions.openSessionStats"
          />
        </template>

        <GitChangesView
          v-else-if="paneViewTab?.type === 'git' && paneViewTab.gitCwd"
          :key="paneViewTab.uiId"
          :cwd="paneViewTab.gitCwd"
          :git-ref="paneViewTab.gitRef || 'working'"
          :selected-path="paneViewTab.gitSelectedPath"
          @ref-change="(r: string) => { if (paneViewTab) paneViewTab.gitRef = r }"
          @path-change="(p: string | null) => { if (paneViewTab) paneViewTab.gitSelectedPath = p }"
        />

        <SessionsView
          v-else-if="activeProject"
          ref="sessionsView"
          :agent="agent"
          :project="activeProject"
          :sessions="sessions"
          :session-total="sessionTotal"
          :loading="loadingList"
          :loading-more="loadingMore"
          :show-exit-pane="paneCount > 1"
          @open="actions.openChat"
          @rename="actions.openRename"
          @resume="actions.resumeHere"
          @chat="actions.chatFromList"
          @archived-block="actions.notifyArchivedBlock"
          @reveal="actions.reveal"
          @delete="actions.deleteSession"
          @copy="actions.copyText"
          @export="actions.exportFromList"
          @refresh="actions.refreshSessions"
          @create-worktree="actions.createWorktree"
          @new-session="actions.newSession"
          @new-shell="actions.newShellSession"
          @exit-pane="actions.exitPane(pane.id)"
          @load-more="actions.loadMore"
          @scroll="actions.onListScroll"
          @batch-delete="actions.batchDeleteSessions"
          @batch-export="actions.batchExportSessions"
          @new-gui-session="actions.newGuiSession"
        />

        <WelcomeView
          v-else
          :agent="agent"
          :projects="projects"
          @select-project="actions.selectProject"
          @switch-agent="actions.switchAgent"
          @open-repo="actions.openRepo"
        />
      </div>

      <!-- TUI 层 -->
      <TerminalPaneSlot
        v-show="pane.activeUiId !== null"
        :pane="pane"
        class="tui-layer"
      />
    </div>
  </div>
</template>
