<script setup lang="ts">
// TUI tab 栏 —— main 顶部的横条。左边 List 固定，之后是 view tabs（会话查看 / chat），
// 再后面是当前 (agent, projectKey) 范围内的所有活跃 PTY tab。
// 隐藏的 PTY/view tab（别的项目 / 别的 agent）不在这里出现，但仍在后台活着。

import { computed, inject, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import type { Agent } from '../types'
import type { TerminalTab, SavedTab } from '../terminals'
import {
  tabs,
  setActive,
  closeTab,
  markTabViewed,
  savedTabs,
  removeSavedTab,
} from '../terminals'
import { statusKind } from '../tabStatus'
import { markViewTabViewed, viewTabStatusKind, type ViewTab } from '../viewTabs'
import type { Pane } from '../panes'
import { isFocused, paneCount, paneOf, primaryPaneId } from '../panes'
import { moveTabTo, dragState, resetDragState, sameRef, type DragKind } from '../tabDrag'
import {
  IconClose,
  IconChat,
  IconGitBranch,
  IconList,
  IconPlus,
  IconReader,
  IconSplitH,
  IconSplitV,
  IconTerminal,
  agentIcons,
} from './icons'
import { t } from '../i18n'
import { PaneActionsKey } from '../paneActions'
import { chatSupported } from '../chatComposerOptions'
import NewMenu from './NewMenu.vue'
import { fontScale } from '../settings'

const pa = inject(PaneActionsKey)!

const props = defineProps<{
  /** 本 strip 所属的分屏格子。tab 过滤 / active 判定 / 拖拽都以它为准。 */
  pane: Pane
  agent: Agent
  projectKey: string | null
  inProjectBrowse: boolean
  hasGit: boolean
  viewTabs: ViewTab[]
  activeViewTabId: number | null
}>()

const emit = defineEmits<{
  /** List —— 关闭当前会话 + 退出 TUI，回到项目会话列表 */
  listClick: []
  /** View tab 被点击 —— 激活指定 view tab */
  viewClick: [uiId: number]
  /** View tab × 被点击 —— 关闭指定 view tab */
  viewClose: [uiId: number]
  /** View tab 右键菜单操作 */
  viewRename: [vt: ViewTab]
  viewCloseOthers: [vt: ViewTab]
  viewCloseProject: [type: 'session' | 'chat' | 'git']
  /** 关闭除指定 tab 外的所有 tab（终端 + view） */
  closeOthersAll: [keepUiId: number, keepKind: 'tui' | 'view']
  /** 关闭当前项目所有 tab（终端 + view） */
  closeAll: []
  /** PTY tab 被手动关闭（点 ×）—— App 据此刷新数据 */
  tabClosed: []
  /** TUI tab 操作菜单 —— 复用会话重命名弹窗 */
  tabRename: [tab: TerminalTab]
  tabsReordered: []
  /** 入口 0 - 显式「新建会话(TUI)」（+ 菜单 / 右键菜单） */
  newSession: []
  /** 双击 tab 条空白处 / 默认新建手势 —— 由设置决定开 session/terminal/chat */
  newDefault: []
  /** 入口 1 - GUI：新开一个 live GUI chat */
  newGuiSession: []
  newShell: []
  /** 入口 2 - 打开当前项目的 Git Changes tab */
  gitChanges: []
  refresh: []
  hydrateSaved: [saved: SavedTab]
  /** saved tab 右键「重命名」—— 复用会话重命名弹窗（saved 分支只改内存标题） */
  savedRename: [saved: SavedTab]
}>()

const visibleTabs = computed(() =>
  tabs.value.filter(
    (t) =>
      t.agent === props.agent &&
      t.projectKey === (props.projectKey ?? '') &&
      t.paneId === props.pane.id,
  ),
)
// saved tab（重启后的懒 pill）按 paneId 精确归属到本格子，和活 tab 一样 —— 不再按聚焦态开关
// 显示（那会让聚焦/失焦时 pill 忽隐忽现）。上次退出记的 paneId 若在本次布局里已不存在（例如分屏树
// 尚未持久化恢复），兜底归到主 pane，保证孤儿 pill 不丢失、也不在多格子间重复。
const visibleSaved = computed(() => {
  const pk = props.projectKey ?? ''
  const primary = primaryPaneId(props.agent, props.projectKey)
  return savedTabs.value.filter((t) => {
    if (t.agent !== props.agent || t.projectKey !== pk) return false
    // 记的 paneId 必须仍存在**且属于本项目**才算数（id 每次启动重排，可能撞到别项目的格子）；
    // 否则回落到主 pane，保证孤儿 pill 不丢、也不在多格子间重复。
    const home = paneOf(t.paneId)
    const homeId = home && home.agent === props.agent && home.projectKey === pk ? home.id : primary
    return homeId === props.pane.id
  })
})

type UnifiedTab =
  | { kind: 'tui'; tab: TerminalTab; order: number; orderIndex: number }
  | { kind: 'saved'; saved: SavedTab; index: number; order: number; orderIndex: number }
  | { kind: 'view'; vt: ViewTab; order: number; orderIndex: number }
type OrderedTab =
  | { kind: 'tui'; tab: TerminalTab; order: number }
  | { kind: 'saved'; saved: SavedTab; index: number; order: number }
  | { kind: 'view'; vt: ViewTab; order: number }

const unifiedTabs = computed<UnifiedTab[]>(() => {
  const items: OrderedTab[] = []
  for (const tab of visibleTabs.value) {
    items.push({ kind: 'tui', tab, order: tab.createdAt })
  }
  for (let i = 0; i < visibleSaved.value.length; i++) {
    const saved = visibleSaved.value[i]
    items.push({ kind: 'saved', saved, index: i, order: saved.createdAt ?? 0 })
  }
  for (const vt of props.viewTabs) {
    items.push({ kind: 'view', vt, order: vt.createdAt })
  }
  items.sort((a, b) => a.order - b.order)
  return items.map((item, orderIndex) => ({ ...item, orderIndex })) as UnifiedTab[]
})

const isMac = /Mac/i.test(navigator.platform)
const modHintDown = ref(false)
const isShortcutPane = computed(() => isFocused(props.pane.id))
const modHintLabel = isMac ? '⌘' : 'Ctrl'
const tabShortcutPrefix = isMac ? `${modHintLabel}⇧` : `${modHintLabel}+Shift+`

function shortcutForIndex(index: number) {
  return index < 9 ? `${tabShortcutPrefix}${index + 1}` : ''
}

function digitFromEvent(e: KeyboardEvent): number | null {
  if (/^Digit[1-9]$/.test(e.code)) return Number(e.code.slice(5))
  if (/^Numpad[1-9]$/.test(e.code)) return Number(e.code.slice(6))
  const n = Number(e.key)
  return Number.isInteger(n) && n >= 1 && n <= 9 ? n : null
}

function isModShiftNumber(e: KeyboardEvent) {
  const mod = isMac ? e.metaKey : e.ctrlKey
  const otherMod = isMac ? e.ctrlKey : e.metaKey
  return mod && !otherMod && e.shiftKey && !e.altKey
}

function activateShortcutIndex(index: number) {
  if (props.inProjectBrowse && index === 0) {
    onListClick()
    return
  }
  const tabIndex = props.inProjectBrowse ? index - 1 : index
  const item = unifiedTabs.value[tabIndex]
  if (!item) return
  if (item.kind === 'tui') onTabClick(item.tab.uiId)
  else if (item.kind === 'saved') onSavedClick(item.saved)
  else onViewTabClick(item.vt.uiId)
}

// 修饰键本身，用来区分「只按了修饰键」和「按下了实义键」。
const isModifierKey = (k: string) => k === 'Meta' || k === 'Control' || k === 'Shift' || k === 'Alt'

// tab 编号提示只在「聚焦格 + 按住 ⌘/Ctrl（可含 ⇧，因快捷键是 ⌘⇧数字）且无实义键」时显示。
function isTabHintState(e: KeyboardEvent) {
  const mod = isMac ? e.metaKey : e.ctrlKey
  const otherMod = isMac ? e.ctrlKey : e.metaKey
  return isShortcutPane.value && mod && !otherMod && !e.altKey
}

// 提示「延时显示、立即隐藏」：只有按住修饰键满 MOD_HINT_DELAY 才亮，其间叠加实义键就取消——
// 敲组合键（⌘⇧5…）不会闪一下再灭。隐藏永远即时。阈值需大于「一前一后敲组合键」的自然间隔，
// 否则先按 ⌘ 再按第二键仍会闪；400ms 把顺手敲的组合键和特意按住查快捷键区分开。
const MOD_HINT_DELAY = 400
let modHintTimer: ReturnType<typeof setTimeout> | null = null
function clearModHintTimer() {
  if (modHintTimer !== null) { clearTimeout(modHintTimer); modHintTimer = null }
}
function showModHintSoon() {
  if (modHintDown.value) { clearModHintTimer(); return } // 已亮则维持，不重排
  clearModHintTimer()
  modHintTimer = setTimeout(() => { modHintTimer = null; modHintDown.value = true }, MOD_HINT_DELAY)
}
function hideModHint() {
  clearModHintTimer()
  modHintDown.value = false
}

function onShortcutKeydown(e: KeyboardEvent) {
  // 只按住修饰键时延时点亮；一旦叠加任何实义键（⌘C、⌘⇧5…）立即收起——那已是组合键在执行。
  if (isModifierKey(e.key) && isTabHintState(e)) showModHintSoon()
  else hideModHint()
  if (!isShortcutPane.value || !isModShiftNumber(e)) return
  if (e.repeat) return
  const n = digitFromEvent(e)
  if (!n) return
  e.preventDefault()
  activateShortcutIndex(n - 1)
}

function onShortcutKeyup(e: KeyboardEvent) {
  // 松开任意键后按剩余修饰键状态重算：仍满足则重新延时点亮，否则即时收起。
  if (isTabHintState(e)) showModHintSoon()
  else hideModHint()
}

function onShortcutBlur() {
  hideModHint()
}

watch(isShortcutPane, (v) => {
  if (!v) hideModHint()
})

// 一旦打开了会话（View tab 存在），整条 strip 就保持可见 —— 即使右侧 PTY tab 全部关闭，
// List / View 两个 meta tab 仍在，View 只能由它自己的 × 手动关闭，不再自动隐藏。
const visible = computed(
  () =>
    visibleTabs.value.length > 0 ||
    visibleSaved.value.length > 0 ||
    (props.inProjectBrowse && props.viewTabs.length > 0),
)

function onSavedClick(saved: SavedTab) {
  emit('hydrateSaved', saved)
}

function onSavedClose(saved: SavedTab, ev: Event) {
  ev.stopPropagation()
  removeSavedTab(saved.sessionPath ? saved.sessionPath : saved)
}
const listActive = computed(
  () => props.pane.activeUiId === null && props.activeViewTabId === null,
)
const listCtx = ref<{ x: number; y: number } | null>(null)
const tabCtx = ref<{ x: number; y: number; tab: TerminalTab } | null>(null)
const savedCtx = ref<{ x: number; y: number; saved: SavedTab } | null>(null)
const stripCtx = ref<{ x: number; y: number } | null>(null)
const viewTabCtx = ref<{ x: number; y: number; vt: ViewTab; typeLabel: string } | null>(null)
type DragPreview =
  | { kind: 'tui'; tab: TerminalTab; x: number; y: number; width: number; offsetX: number; offsetY: number }
  | { kind: 'view'; vt: ViewTab; x: number; y: number; width: number; offsetX: number; offsetY: number }
const dragPreview = ref<DragPreview | null>(null)
const nativeMenuSupported = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
let pendingDrag: { ref: { kind: DragKind; uiId: number }; startX: number; startY: number } | null = null
// body 挂着字号缩放 zoom（settings.ts）。拖拽预览 Teleport 到 body、position:fixed，其 left/top 会
// 被这个 zoom 整体缩放；而 clientX/getBoundingClientRect 都是**视觉像素**（已含 zoom）。所以写 style
// 时要 / zoom 抵消，否则预览会以 0.9× 速度朝左上角漂（和 tooltip.ts 同一坑）。拖拽期间 zoom 不变，
// 起手时读一次即可。
let dragZoom = 1
function currentZoom(): number {
  const z = parseFloat(getComputedStyle(document.body).zoom || '1')
  return Number.isFinite(z) && z > 0 ? z : 1
}
let suppressNextTabClick = false

// 有地方可拖才允许拖：本格子多于一个可见 tab（活 tab + saved 懒 pill 都算落点锚，可排序），
// 或存在多个 pane（可跨屏）。
const canDrag = computed(
  () =>
    paneCount.value > 1 ||
    visibleTabs.value.length + props.viewTabs.length + visibleSaved.value.length > 1,
)

function isDragSource(kind: DragKind, uiId: number): boolean {
  return sameRef(dragState.source, { kind, uiId })
}
// 落点线按「可见 tab 序号」锚定（不再按 tab ref），这样 saved 懒 pill 也能画线——它虽不能被
// 拿起，但可作为落点锚，否则活 tab 之间夹着 pill 时落点线会跳过它。
function dropSideAt(orderIndex: number): 'before' | 'after' | null {
  if (dragState.overPaneId !== props.pane.id) return null
  if (dragState.dropIndex !== orderIndex) return null
  return dragState.dropPosition
}

// ---- 横向滑动（无原生滚动条）: translateX + CSS transition ----
// 拿掉丑陋的横向滚动条，把 tab 条做成一个可滑动的遮罩区：所有 tab 放进 .term-strip-track，
// 用 transform: translateX(-scrollX) 平移；滚轮 / 拖空白处改 scrollX（跟手、关 transition），
// 点临近边缘的 tab / 新建 tab 则带 transition 平滑滑入。
const viewportRef = ref<HTMLElement>()
const trackRef = ref<HTMLElement>()
const scrollX = ref(0)
const maxScroll = ref(0)
// panning=true 时关掉 transition，让滚轮 / 拖拽 1:1 跟手；程序化滑动时为 false 走动画。
const panning = ref(false)
const canLeft = computed(() => scrollX.value > 0.5)
const canRight = computed(() => scrollX.value < maxScroll.value - 0.5)
const trackStyle = computed(() => ({ transform: `translateX(${-scrollX.value}px)` }))

function measure() {
  const vp = viewportRef.value
  const tr = trackRef.value
  maxScroll.value = vp && tr ? Math.max(0, tr.scrollWidth - vp.clientWidth) : 0
  if (scrollX.value > maxScroll.value) scrollX.value = maxScroll.value
}
function setScroll(x: number) {
  scrollX.value = Math.max(0, Math.min(x, maxScroll.value))
}

// 滚轮 / 触控板 → 横向平移（取代原生横向滚动）
let wheelIdleTimer = 0
function onWheel(ev: WheelEvent) {
  if (maxScroll.value <= 0) return
  const delta = Math.abs(ev.deltaX) > Math.abs(ev.deltaY) ? ev.deltaX : ev.deltaY
  if (!delta) return
  ev.preventDefault()
  panning.value = true
  setScroll(scrollX.value + delta)
  window.clearTimeout(wheelIdleTimer)
  wheelIdleTimer = window.setTimeout(() => (panning.value = false), 140)
}

// 拖拽空白处 → 平移（tab 本体的拖拽留给排序逻辑，不在此响应）
let pan: { startX: number; startScroll: number } | null = null
function onPanPointerDown(ev: PointerEvent) {
  if (ev.button !== 0 || maxScroll.value <= 0) return
  const target = ev.target as HTMLElement | null
  if (target?.closest('.term-tab, .term-tab-new')) return
  pan = { startX: ev.clientX, startScroll: scrollX.value }
  panning.value = true
  window.addEventListener('pointermove', onPanPointerMove)
  window.addEventListener('pointerup', onPanPointerUp)
  window.addEventListener('pointercancel', onPanPointerUp)
}
function onPanPointerMove(ev: PointerEvent) {
  if (!pan) return
  setScroll(pan.startScroll - (ev.clientX - pan.startX))
}
function onPanPointerUp() {
  pan = null
  panning.value = false
  window.removeEventListener('pointermove', onPanPointerMove)
  window.removeEventListener('pointerup', onPanPointerUp)
  window.removeEventListener('pointercancel', onPanPointerUp)
}

// 把某个 tab 完整滑入视野；点临近边缘（被遮挡）的 tab 时露出它被切掉的部分
function revealEl(el: HTMLElement | null | undefined) {
  measure()
  const vp = viewportRef.value
  if (!vp || !el || maxScroll.value <= 0) return
  const tabRect = el.getBoundingClientRect()
  const vpRect = vp.getBoundingClientRect()
  const margin = 16
  let dx = 0
  if (tabRect.left < vpRect.left + margin) dx = tabRect.left - (vpRect.left + margin)
  else if (tabRect.right > vpRect.right - margin) dx = tabRect.right - (vpRect.right - margin)
  if (dx === 0) return
  panning.value = false // 程序化滑动：保留 transition 动画
  setScroll(scrollX.value + dx)
}
function revealActiveTab() {
  nextTick(() => {
    const el = trackRef.value?.querySelector<HTMLElement>(
      `.term-tab[data-tab-ui-id="${props.pane.activeUiId}"]`,
    )
    revealEl(el)
  })
}

let stripRo: ResizeObserver | null = null
watch(
  viewportRef,
  (el) => {
    stripRo?.disconnect()
    stripRo = null
    if (!el || typeof ResizeObserver === 'undefined') return
    stripRo = new ResizeObserver(() => measure())
    stripRo.observe(el)
    nextTick(() => {
      if (trackRef.value && stripRo) stripRo.observe(trackRef.value)
      measure()
    })
  },
  { immediate: true },
)
watch([() => visibleTabs.value.length, () => visibleSaved.value.length, () => props.viewTabs.length], () => {
  nextTick(() => {
    measure()
    const id = props.activeViewTabId ?? props.pane.activeUiId
    if (id != null) {
      const el = trackRef.value?.querySelector<HTMLElement>(`.term-tab[data-tab-ui-id="${id}"]`)
      revealEl(el)
    }
  })
})
watch(() => props.pane.activeUiId, () => revealActiveTab())
watch(() => props.activeViewTabId, (id, previousId) => {
  if (previousId != null) {
    const previous = props.viewTabs.find((vt) => vt.uiId === previousId)
    if (previous) markViewTabViewed(previous)
  }
  if (id != null) {
    const current = props.viewTabs.find((vt) => vt.uiId === id)
    if (current) markViewTabViewed(current)
  }
  if (id == null) return
  nextTick(() => {
    const el = trackRef.value?.querySelector<HTMLElement>(`.term-tab[data-tab-ui-id="${id}"]`)
    revealEl(el)
  })
})
onUnmounted(() => {
  stripRo?.disconnect()
  window.clearTimeout(wheelIdleTimer)
  window.removeEventListener('pointermove', onPanPointerMove)
  window.removeEventListener('pointerup', onPanPointerUp)
  window.removeEventListener('pointercancel', onPanPointerUp)
})

// ---- 新建会话下拉菜单（+ 按钮） ----
const newMenuOpen = ref(false)
const newMenuEl = ref<HTMLElement>()
function toggleNewMenu(ev?: Event) {
  ev?.stopPropagation()
  newMenuOpen.value = !newMenuOpen.value
}
function pickNewAgent() {
  newMenuOpen.value = false
  emit('newSession')
}
function pickNewGui() {
  newMenuOpen.value = false
  emit('newGuiSession')
}
function pickNewShell() {
  newMenuOpen.value = false
  emit('newShell')
}
function pickGitChanges() {
  newMenuOpen.value = false
  emit('gitChanges')
}
function pickSplitH() {
  newMenuOpen.value = false
  pa.splitH()
}
function pickSplitV() {
  newMenuOpen.value = false
  pa.splitV()
}
function onNewMenuDocClick(e: MouseEvent) {
  if (!newMenuOpen.value) return
  if (newMenuEl.value?.contains(e.target as Node)) return
  newMenuOpen.value = false
}
onMounted(() => {
  document.addEventListener('click', onNewMenuDocClick)
})
onUnmounted(() => {
  document.removeEventListener('click', onNewMenuDocClick)
})

function shortTitle(title: string): string {
  if (!title) return t('chat.tui.untitled')
  if (title.length > 22) return title.slice(0, 20) + '…'
  return title
}

function isViewTabActive(vt: ViewTab): boolean {
  return props.pane.activeUiId === null && props.activeViewTabId === vt.uiId
}

function viewStatusKind(vt: ViewTab) {
  return viewTabStatusKind(vt, isViewTabActive(vt))
}

function onTabClick(uiId: number, ev?: Event) {
  if (suppressNextTabClick) {
    ev?.preventDefault()
    ev?.stopPropagation()
    suppressNextTabClick = false
    return
  }
  // 点临近边缘（被遮挡）的 tab 时，先把它完整滑入视野
  revealEl((ev?.currentTarget as HTMLElement) ?? null)
  markTabViewed(uiId)
  // 点已激活的 tab 不做切换 —— 避免和"× 关闭"的视觉位置混淆。要回 view 用左侧的 meta tab。
  if (props.pane.activeUiId === uiId) return
  setActive(uiId)
}

function onListClick() {
  emit('listClick')
}

async function onListContextMenu(ev: MouseEvent) {
  ev.preventDefault()
  ev.stopPropagation()
  closeTabCtx()
  if (await openNativeListContextMenu(ev)) return
  openFallbackListContextMenu(ev)
}

async function openNativeListContextMenu(ev: MouseEvent): Promise<boolean> {
  if (!nativeMenuSupported) return false
  try {
    const [{ Menu }, { LogicalPosition }] = await Promise.all([
      import('@tauri-apps/api/menu'),
      import('@tauri-apps/api/dpi'),
    ])
    const menu = await Menu.new({
      items: [
        {
          id: 'list-close-others',
          text: t('chat.tui.tabCloseOthersAll'),
          action: () => closeAllFromList(),
        },
        {
          id: 'list-close-all',
          text: t('chat.tui.tabCloseAll'),
          action: () => emit('closeAll'),
        },
      ],
    })
    const z = fontScale.value / 14
    await menu.popup(new LogicalPosition(ev.clientX * z, ev.clientY * z))
    return true
  } catch {
    return false
  }
}

function openFallbackListContextMenu(ev: MouseEvent) {
  const menuW = 220
  const menuH = 80
  listCtx.value = {
    x: Math.max(8, Math.min(ev.clientX, window.innerWidth - menuW - 8)),
    y: Math.max(8, Math.min(ev.clientY, window.innerHeight - menuH - 8)),
  }
}

function closeAllFromList() {
  for (const item of visibleTabs.value) closeTab(item.uiId)
  for (const s of [...visibleSaved.value]) removeSavedTab(s.sessionPath ? s.sessionPath : s)
  for (const vt of props.viewTabs) emit('viewClose', vt.uiId)
  emit('tabClosed')
}

function onViewTabClick(uiId: number, ev?: Event) {
  if (suppressNextTabClick) {
    ev?.preventDefault()
    ev?.stopPropagation()
    suppressNextTabClick = false
    return
  }
  emit('viewClick', uiId)
}
function onViewTabClose(uiId: number, ev: Event) {
  ev.stopPropagation()
  emit('viewClose', uiId)
}

async function onViewTabContextMenu(vt: ViewTab, ev: MouseEvent) {
  ev.preventDefault()
  ev.stopPropagation()
  if (await openNativeViewTabContextMenu(vt, ev)) return
  openFallbackViewTabContextMenu(vt, ev)
}

async function openNativeViewTabContextMenu(vt: ViewTab, ev: MouseEvent): Promise<boolean> {
  if (!nativeMenuSupported) return false
  try {
    const [{ Menu }, { LogicalPosition }] = await Promise.all([
      import('@tauri-apps/api/menu'),
      import('@tauri-apps/api/dpi'),
    ])
    const typeLabel = vt.type === 'chat' ? t('chat.tui.chatTab') : vt.type === 'git' ? t('chat.tui.diffTab') : t('chat.tui.viewTab')
    const menu = await Menu.new({
      items: [
        {
          id: 'vt-rename',
          text: t('chat.tui.tabRenameView'),
          action: () => emit('viewRename', vt),
        },
        { item: 'Separator' },
        {
          id: 'vt-close',
          text: t('chat.tui.tabClose'),
          action: () => emit('viewClose', vt.uiId),
        },
        {
          id: 'vt-close-others',
          text: t('chat.tui.tabCloseOthersView', { type: typeLabel }),
          action: () => emit('viewCloseOthers', vt),
        },
        {
          id: 'vt-close-project',
          text: t('chat.tui.tabCloseProjectView', { type: typeLabel }),
          action: () => emit('viewCloseProject', vt.type),
        },
        { item: 'Separator' },
        {
          id: 'vt-close-others-all',
          text: t('chat.tui.tabCloseOthersAll'),
          action: () => emit('closeOthersAll', vt.uiId, 'view'),
        },
        {
          id: 'vt-close-all',
          text: t('chat.tui.tabCloseAll'),
          action: () => emit('closeAll'),
        },
      ],
    })
    const z = fontScale.value / 14
    await menu.popup(new LogicalPosition(ev.clientX * z, ev.clientY * z))
    return true
  } catch {
    return false
  }
}

function openFallbackViewTabContextMenu(vt: ViewTab, ev: MouseEvent) {
  const typeLabel = vt.type === 'chat' ? t('chat.tui.chatTab') : vt.type === 'git' ? t('chat.tui.diffTab') : t('chat.tui.viewTab')
  viewTabCtx.value = {
    x: Math.max(8, Math.min(ev.clientX, window.innerWidth - 220 - 8)),
    y: Math.max(8, Math.min(ev.clientY, window.innerHeight - 200 - 8)),
    vt,
    typeLabel,
  }
}

function closeViewTabCtx() {
  viewTabCtx.value = null
}

function onClose(uiId: number, ev: Event) {
  ev.stopPropagation()
  closeTab(uiId)
  emit('tabClosed')
}

function renameTab(tab: TerminalTab, ev?: Event) {
  ev?.stopPropagation()
  closeTabCtx()
  emit('tabRename', tab)
}

function clearDragState() {
  pendingDrag = null
  dragPreview.value = null
  resetDragState()
  document.body.classList.remove('is-tab-reordering')
  window.removeEventListener('pointermove', onTabPointerMove)
  window.removeEventListener('pointerup', onTabPointerUp)
  window.removeEventListener('pointercancel', onTabPointerUp)
}

function beginDrag(ref: { kind: DragKind; uiId: number }, ev: PointerEvent) {
  if (ev.button !== 0 || !canDrag.value) return
  const target = ev.target as HTMLElement | null
  if (target?.closest('.term-tab-close')) return
  pendingDrag = { ref, startX: ev.clientX, startY: ev.clientY }
  window.addEventListener('pointermove', onTabPointerMove)
  window.addEventListener('pointerup', onTabPointerUp)
  window.addEventListener('pointercancel', onTabPointerUp)
}
function onTuiTabPointerDown(tab: TerminalTab, ev: PointerEvent) {
  beginDrag({ kind: 'tui', uiId: tab.uiId }, ev)
}
function onViewTabPointerDown(vt: ViewTab, ev: PointerEvent) {
  beginDrag({ kind: 'view', uiId: vt.uiId }, ev)
}

function onTabPointerMove(ev: PointerEvent) {
  if (!pendingDrag) return
  const dx = ev.clientX - pendingDrag.startX
  const dy = ev.clientY - pendingDrag.startY
  if (!dragState.active) {
    if (Math.hypot(dx, dy) < 5) return
    closeTabCtx()
    dragState.active = true
    dragState.source = pendingDrag.ref
    dragState.sourcePaneId = props.pane.id
    dragState.overPaneId = props.pane.id
    dragState.dropIndex = null
    dragState.dropReady = false
    suppressNextTabClick = true
    const { kind, uiId } = pendingDrag.ref
    const sourceEl = document.querySelector<HTMLElement>(
      `.term-tab[data-drag-kind="${kind}"][data-tab-ui-id="${uiId}"]`,
    )
    const rect = sourceEl?.getBoundingClientRect()
    if (rect) {
      dragZoom = currentZoom()
      // rect / clientX 都是视觉像素；预览在 zoom 后的 body 里，写 style 前一律 / zoom 抵消。
      const base = {
        x: rect.left / dragZoom,
        y: rect.top / dragZoom,
        width: rect.width / dragZoom,
        offsetX: pendingDrag.startX - rect.left,
        offsetY: pendingDrag.startY - rect.top,
      }
      if (kind === 'tui') {
        const tab = tabs.value.find((t) => t.uiId === uiId)
        if (tab) dragPreview.value = { kind: 'tui', tab, ...base }
      } else {
        const vt = props.viewTabs.find((v) => v.uiId === uiId)
        if (vt) dragPreview.value = { kind: 'view', vt, ...base }
      }
    }
    document.body.classList.add('is-tab-reordering')
  }
  ev.preventDefault()
  if (dragPreview.value) {
    dragPreview.value.x = (ev.clientX - dragPreview.value.offsetX) / dragZoom
    dragPreview.value.y = (ev.clientY - dragPreview.value.offsetY) / dragZoom
  }
  updateDropTargetFromPoint(ev.clientX, ev.clientY)
}

function updateDropTargetFromPoint(x: number, y: number) {
  const source = dragState.source
  if (!source) return
  const under = document.elementFromPoint(x, y)
  const paneEl = under?.closest<HTMLElement>('.pane[data-pane-id]')
  const pid = paneEl ? Number(paneEl.dataset.paneId) : NaN
  if (!paneEl || !Number.isFinite(pid)) {
    dragState.overPaneId = null
    dragState.dropIndex = null
    dragState.dropReady = false
    return
  }
  dragState.overPaneId = pid

  // 关键：几何用「整条可见 strip」的所有 tab（含 saved 懒 pill），不止可拖的活 tab。saved 虽不能被
  // 拿起，但占着位置、可作落点锚——否则活 tab 之间夹着 saved pill 时，落点线会跳过它蹦到后一个活
  // tab（就是「方向错了 / 跑后面」）。每个 tab 带 data-order（createdAt）+ data-order-index。
  const els = Array.from(paneEl.querySelectorAll<HTMLElement>('.term-tab[data-order-index]'))
  if (els.length === 0) {
    // 空 strip（如只有 List）：同 pane 无意义；跨屏进空 strip 才是有效移动（追加）。
    dragState.dropIndex = null
    dragState.dropBefore = null
    dragState.dropAfter = null
    dragState.dropReady = pid !== dragState.sourcePaneId
    return
  }
  const orderOf = (el: HTMLElement) => Number(el.dataset.order)
  const idxOf = (el: HTMLElement) => Number(el.dataset.orderIndex)
  const rects = els.map((el) => el.getBoundingClientRect())

  // 被拖预览的投影矩形（视觉像素）：左右边 pLeft/pRight、中心 pCenter。
  // pLeft = 光标 x − 抓取偏移；宽 = 预览宽 × 缩放（= 源 tab 视觉宽）。
  const pw = dragPreview.value ? dragPreview.value.width * dragZoom : rects[0].width
  const pLeft = dragPreview.value ? x - dragPreview.value.offsetX : x - pw / 2
  const pRight = pLeft + pw
  const pCenter = pLeft + pw / 2

  const srcVisIdx = els.findIndex(
    (el) => el.dataset.dragKind === source.kind && Number(el.dataset.tabUiId) === source.uiId,
  )

  // 插入槽位 insert ∈ [0, n]：插到第 insert 个可见 tab 之前。
  let insert: number
  if (srcVisIdx < 0) {
    // 跨屏：目标 strip 无源占位、无死区 —— 被拖中心越过多少个 tab 中心就插到第几个前。
    insert = 0
    for (const r of rects) {
      if (pCenter >= r.left + r.width / 2) insert++
      else break
    }
  } else {
    // 同 pane：源占着 srcVisIdx 的虚位（拖动中不实时重排）。前后自由排序、任意距离，落点线一路跟手。
    // cmux 式「重合过半才跨过」，用「前缘扫过中心」的位置式判定（不是连续吃邻居——那样拖过头会
    // 越过中间的 tab、前缘不再压着近邻，就断在半路不再前进，即「挤牙膏」）：
    //   · 向左：被拖 tab 左缘 pLeft 每扫过一个 tab 的中心 = 与它重合过半 → 插到它之前；
    //     插入点 = 中心 < pLeft 的 tab 个数（左侧尚未盖过半的 tab 都留在被拖 tab 左边）。
    //   · 向右：对称地用右缘 pRight，插到「中心 ≤ pRight 的最后一个 tab」之后。
    const srcCenter = rects[srcVisIdx].left + rects[srcVisIdx].width / 2
    insert = 0
    if (pCenter < srcCenter) {
      for (const r of rects) {
        if (r.left + r.width / 2 < pLeft) insert++
        else break
      }
    } else {
      for (const r of rects) {
        if (r.left + r.width / 2 <= pRight) insert++
        else break
      }
    }
    // 落回自身槽位（前后半格内，无邻居被盖过半）= 原地不动 → 不画线、松手不移动。
    if (insert === srcVisIdx || insert === srcVisIdx + 1) {
      dragState.dropIndex = null
      dragState.dropReady = false
      return
    }
  }

  // 落点线锚：insert<n → 画在 els[insert] 左缘；insert===n → 画在末尾元素右缘。
  if (insert >= els.length) {
    dragState.dropIndex = idxOf(els[els.length - 1])
    dragState.dropPosition = 'after'
  } else {
    dragState.dropIndex = idxOf(els[insert])
    dragState.dropPosition = 'before'
  }
  // 落点两侧 createdAt 边界（非原地时两侧邻居必不是 source，可直接读 DOM 上的 data-order）。
  dragState.dropBefore = insert > 0 ? orderOf(els[insert - 1]) : null
  dragState.dropAfter = insert < els.length ? orderOf(els[insert]) : null
  dragState.dropReady = true
}

function onTabPointerUp(ev: PointerEvent) {
  const source = dragState.source
  const targetPaneId = dragState.overPaneId
  let moved = false
  // dropReady=false 表示落回原槽位（原地）或无有效落点 → 不动。
  if (source && targetPaneId != null && dragState.dropReady) {
    moved = moveTabTo(source, targetPaneId, dragState.dropBefore, dragState.dropAfter)
  }
  const wasDragging = !!source
  clearDragState()
  if (moved) emit('tabsReordered')
  if (wasDragging) {
    ev.preventDefault()
    window.setTimeout(() => {
      suppressNextTabClick = false
    }, 0)
  }
}

function onStripDoubleClick(ev: MouseEvent) {
  const target = ev.target as HTMLElement | null
  if (target?.closest('.term-tab, .term-tab-ctx-menu')) return
  closeTabCtx()
  emit('newDefault')
}

async function onStripContextMenu(ev: MouseEvent) {
  const target = ev.target as HTMLElement | null
  if (target?.closest('.term-tab, .term-tab-ctx-menu, .term-strip-ctx-menu')) return
  ev.preventDefault()
  closeTabCtx()
  if (await openNativeStripContextMenu(ev)) return
  openFallbackStripContextMenu(ev)
}

async function onTabContextMenu(tab: TerminalTab, ev: MouseEvent) {
  ev.preventDefault()
  ev.stopPropagation()
  closeTabCtx()
  if (await openNativeTabContextMenu(tab, ev)) return
  openFallbackTabContextMenu(tab, ev)
}

// saved（懒恢复）tab 右键 —— 和 live tab 一致：先试原生 Tauri 菜单，失败再退 HTML 菜单。
// 在此之前 saved tab 没挂 @contextmenu，会落到 webview 原生菜单，和 live tab 不一致。
async function onSavedContextMenu(saved: SavedTab, ev: MouseEvent) {
  ev.preventDefault()
  ev.stopPropagation()
  closeTabCtx()
  if (await openNativeSavedContextMenu(saved, ev)) return
  openFallbackSavedContextMenu(saved, ev)
}

async function openNativeSavedContextMenu(saved: SavedTab, ev: MouseEvent): Promise<boolean> {
  if (!nativeMenuSupported) return false
  try {
    const [{ Menu }, { LogicalPosition }] = await Promise.all([
      import('@tauri-apps/api/menu'),
      import('@tauri-apps/api/dpi'),
    ])
    const menu = await Menu.new({
      items: [
        {
          id: 'saved-rename',
          text: t(saved.isShell ? 'chat.tui.tabRenameShell' : 'chat.tui.tabRename'),
          action: () => emit('savedRename', saved),
        },
        { item: 'Separator' },
        {
          id: 'saved-close',
          text: t('chat.tui.tabClose'),
          action: () => removeSaved(saved),
        },
        {
          id: 'saved-close-others',
          text: t('chat.tui.tabCloseOthers'),
          action: () => closeOthersFromSaved(saved),
        },
        {
          id: 'saved-close-project',
          text: t('chat.tui.tabCloseProject'),
          action: () => closeProjectAllTabs(),
        },
        { item: 'Separator' },
        {
          id: 'saved-close-all',
          text: t('chat.tui.tabCloseAll'),
          action: () => emit('closeAll'),
        },
      ],
    })
    const z = fontScale.value / 14
    await menu.popup(new LogicalPosition(ev.clientX * z, ev.clientY * z))
    return true
  } catch (err) {
    console.warn('Failed to open native saved tab context menu, falling back to HTML menu', err)
    return false
  }
}

function openFallbackSavedContextMenu(saved: SavedTab, ev: MouseEvent) {
  const menuW = 220
  const menuH = 318
  savedCtx.value = {
    x: Math.max(8, Math.min(ev.clientX, window.innerWidth - menuW - 8)),
    y: Math.max(8, Math.min(ev.clientY, window.innerHeight - menuH - 8)),
    saved,
  }
}

function removeSaved(saved: SavedTab) {
  removeSavedTab(saved.sessionPath ? saved.sessionPath : saved)
}

// 关闭「其它」：saved tab 视角下，其它 = 所有 live tab + 除自己外的 saved tab。
function closeOthersFromSaved(saved: SavedTab) {
  for (const item of visibleTabs.value) closeTab(item.uiId)
  for (const s of [...visibleSaved.value]) {
    if (s !== saved) removeSavedTab(s.sessionPath ? s.sessionPath : s)
  }
  emit('tabClosed')
}

// 关闭整个项目：live + saved 全清。
function closeProjectAllTabs() {
  for (const item of visibleTabs.value) closeTab(item.uiId)
  for (const s of [...visibleSaved.value]) removeSavedTab(s.sessionPath ? s.sessionPath : s)
  emit('tabClosed')
}

async function openNativeTabContextMenu(tab: TerminalTab, ev: MouseEvent): Promise<boolean> {
  if (!nativeMenuSupported) return false
  try {
    const [{ Menu }, { LogicalPosition }] = await Promise.all([
      import('@tauri-apps/api/menu'),
      import('@tauri-apps/api/dpi'),
    ])
    const menu = await Menu.new({
      items: [
        {
          id: 'tab-rename',
          text: t(tab.isShell ? 'chat.tui.tabRenameShell' : 'chat.tui.tabRename'),
          action: () => emit('tabRename', tab),
        },
        { item: 'Separator' },
        {
          id: 'tab-close',
          text: t('chat.tui.tabClose'),
          action: () => closeNativeCtxTab(tab),
        },
        {
          id: 'tab-close-others',
          text: t(tab.isShell ? 'chat.tui.tabCloseOthers' : 'chat.tui.tabCloseOthersSession'),
          action: () => closeOtherNativeCtxTabs(tab),
        },
        {
          id: 'tab-close-project',
          text: t(tab.isShell ? 'chat.tui.tabCloseProject' : 'chat.tui.tabCloseProjectSession'),
          action: () => closeProjectNativeCtxTabs(tab),
        },
        { item: 'Separator' },
        {
          id: 'tab-close-others-all',
          text: t('chat.tui.tabCloseOthersAll'),
          action: () => emit('closeOthersAll', tab.uiId, 'tui'),
        },
        {
          id: 'tab-close-all',
          text: t('chat.tui.tabCloseAll'),
          action: () => emit('closeAll'),
        },
      ],
    })
    const z = fontScale.value / 14
    await menu.popup(new LogicalPosition(ev.clientX * z, ev.clientY * z))
    return true
  } catch (err) {
    console.warn('Failed to open native tab context menu, falling back to HTML menu', err)
    return false
  }
}

async function openNativeStripContextMenu(ev: MouseEvent): Promise<boolean> {
  if (!nativeMenuSupported) return false
  try {
    const [{ Menu }, { LogicalPosition }] = await Promise.all([
      import('@tauri-apps/api/menu'),
      import('@tauri-apps/api/dpi'),
    ])
    const menu = await Menu.new({
      items: [
        {
          id: 'strip-new-agent',
          text: t('list.action.newSessionTui'),
          action: () => emit('newSession'),
        },
        ...(chatSupported(props.agent)
          ? [
              {
                id: 'strip-new-gui',
                text: t('list.action.newSessionGui'),
                action: () => emit('newGuiSession'),
              },
            ]
          : []),
        {
          id: 'strip-new-shell',
          text: t('list.action.newTerminal'),
          action: () => emit('newShell'),
        },
        ...(props.hasGit
          ? [
              { item: 'Separator' as const },
              {
                id: 'strip-git-changes',
                text: t('list.action.gitChanges'),
                action: () => emit('gitChanges'),
              },
            ]
          : []),
      ],
    })
    const z = fontScale.value / 14
    await menu.popup(new LogicalPosition(ev.clientX * z, ev.clientY * z))
    return true
  } catch (err) {
    console.warn('Failed to open native strip context menu, falling back to HTML menu', err)
    return false
  }
}

function openFallbackTabContextMenu(tab: TerminalTab, ev: MouseEvent) {
  const menuW = 220
  const menuH = 318
  tabCtx.value = {
    x: Math.max(8, Math.min(ev.clientX, window.innerWidth - menuW - 8)),
    y: Math.max(8, Math.min(ev.clientY, window.innerHeight - menuH - 8)),
    tab,
  }
}

function openFallbackStripContextMenu(ev: MouseEvent) {
  const menuW = 220
  const menuH = 80
  stripCtx.value = {
    x: Math.max(8, Math.min(ev.clientX, window.innerWidth - menuW - 8)),
    y: Math.max(8, Math.min(ev.clientY, window.innerHeight - menuH - 8)),
  }
}

function closeTabCtx() {
  listCtx.value = null
  tabCtx.value = null
  stripCtx.value = null
  savedCtx.value = null
  viewTabCtx.value = null
}

function newSessionFromStripCtx() {
  closeTabCtx()
  emit('newSession')
}
function newGuiFromStripCtx() {
  closeTabCtx()
  emit('newGuiSession')
}
function newShellFromStripCtx() {
  closeTabCtx()
  emit('newShell')
}
function gitChangesFromStripCtx() {
  closeTabCtx()
  emit('gitChanges')
}
function refreshFromStripCtx() {
  closeTabCtx()
  emit('refresh')
}
function splitHFromStripCtx() {
  closeTabCtx()
  pa.splitH()
}
function splitVFromStripCtx() {
  closeTabCtx()
  pa.splitV()
}

function renameCtxTab() {
  const tab = tabCtx.value?.tab
  closeTabCtx()
  if (tab) emit('tabRename', tab)
}

function closeCtxTab() {
  const tab = tabCtx.value?.tab
  closeTabCtx()
  if (!tab) return
  closeTab(tab.uiId)
  emit('tabClosed')
}

function closeOtherCtxTabs() {
  const tab = tabCtx.value?.tab
  closeTabCtx()
  if (!tab) return
  for (const item of visibleTabs.value) {
    if (item.uiId !== tab.uiId && item.isShell === tab.isShell) closeTab(item.uiId)
  }
  emit('tabClosed')
}

function closeProjectCtxTabs() {
  const tab = tabCtx.value?.tab
  closeTabCtx()
  if (tab) closeProjectNativeCtxTabs(tab)
}

// ---- saved tab 的 HTML fallback 菜单动作（读 savedCtx）----
function renameCtxSaved() {
  const saved = savedCtx.value?.saved
  closeTabCtx()
  if (saved) emit('savedRename', saved)
}
function closeCtxSaved() {
  const saved = savedCtx.value?.saved
  closeTabCtx()
  if (saved) removeSaved(saved)
}
function closeOthersCtxSaved() {
  const saved = savedCtx.value?.saved
  closeTabCtx()
  if (saved) closeOthersFromSaved(saved)
}
function closeProjectCtxSaved() {
  closeTabCtx()
  closeProjectAllTabs()
}

function closeNativeCtxTab(tab: TerminalTab) {
  closeTab(tab.uiId)
  emit('tabClosed')
}

function closeOtherNativeCtxTabs(tab: TerminalTab) {
  for (const item of visibleTabs.value) {
    if (item.uiId !== tab.uiId && item.isShell === tab.isShell) closeTab(item.uiId)
  }
  for (const s of [...visibleSaved.value]) {
    if (s.isShell === tab.isShell) removeSavedTab(s.sessionPath ? s.sessionPath : s)
  }
  emit('tabClosed')
}

function closeProjectNativeCtxTabs(tab: TerminalTab) {
  for (const item of visibleTabs.value) {
    if (item.isShell === tab.isShell) closeTab(item.uiId)
  }
  for (const s of [...visibleSaved.value]) {
    if (s.isShell === tab.isShell) removeSavedTab(s.sessionPath ? s.sessionPath : s)
  }
  emit('tabClosed')
}


function onDocMouseDown(e: MouseEvent) {
  if (!listCtx.value && !tabCtx.value && !stripCtx.value && !savedCtx.value && !viewTabCtx.value) return
  const target = e.target as HTMLElement | null
  if (target?.closest('.term-tab-ctx-menu, .term-strip-ctx-menu')) return
  closeTabCtx()
}

function onDocKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    closeTabCtx()
  }
}

onMounted(() => {
  document.addEventListener('mousedown', onDocMouseDown)
  document.addEventListener('keydown', onDocKeydown)
  document.addEventListener('wheel', closeTabCtx, { passive: true })
  window.addEventListener('keydown', onShortcutKeydown, true)
  window.addEventListener('keyup', onShortcutKeyup, true)
  window.addEventListener('blur', onShortcutBlur)
  window.addEventListener('blur', closeTabCtx)
})

onUnmounted(() => {
  clearDragState()
  clearModHintTimer()
  document.removeEventListener('mousedown', onDocMouseDown)
  document.removeEventListener('keydown', onDocKeydown)
  document.removeEventListener('wheel', closeTabCtx)
  window.removeEventListener('keydown', onShortcutKeydown, true)
  window.removeEventListener('keyup', onShortcutKeyup, true)
  window.removeEventListener('blur', onShortcutBlur)
  window.removeEventListener('blur', closeTabCtx)
})
</script>

<template>
  <div
    v-if="visible"
    class="terminal-strip"
    data-tauri-drag-region="false"
    @dblclick="onStripDoubleClick"
    @contextmenu="onStripContextMenu"
  >
    <!-- 固定 meta tab：List / View 永远钉在左侧，不随终端 tab 一起滑动 -->
    <div v-if="inProjectBrowse" class="term-strip-meta">
      <!-- List —— 项目浏览模式下永久显示 -->
      <div
        class="term-tab view-tab"
        :class="{ active: listActive }"
        v-tooltip:bottom="t('chat.tui.listTabTooltip')"
        role="button"
        tabindex="0"
        @click="onListClick"
        @contextmenu="onListContextMenu"
        @keydown.enter.prevent="onListClick"
        @keydown.space.prevent="onListClick"
      >
        <IconList class="term-tab-agent" />
        <span class="term-tab-title">{{ t('chat.tui.listTab') }}</span>
        <span v-if="modHintDown && shortcutForIndex(0)" class="term-tab-shortcut">{{ shortcutForIndex(0) }}</span>
      </div>

      <div
        v-if="viewTabs.length > 0 || visibleTabs.length > 0 || visibleSaved.length > 0"
        class="term-tab-sep"
        aria-hidden="true"
      />
    </div>

    <!-- 滑动区（红框动效区域）：只放终端 / saved tab -->
    <div
      ref="viewportRef"
      class="term-strip-scroll"
      :class="{ 'can-left': canLeft, 'can-right': canRight }"
      @wheel="onWheel"
      @pointerdown="onPanPointerDown"
    >
      <div ref="trackRef" class="term-strip-track" :class="{ panning }" :style="trackStyle">

      <template v-for="ut in unifiedTabs" :key="ut.kind === 'tui' ? ut.tab.uiId : ut.kind === 'saved' ? 'saved:' + (ut.saved.sessionPath || `shell-${ut.index}`) : 'vt:' + ut.vt.uiId">
        <!-- TUI (live terminal/session) tab -->
        <div
          v-if="ut.kind === 'tui'"
          class="term-tab"
          :class="{
            active: pane.activeUiId === ut.tab.uiId,
            dragging: isDragSource('tui', ut.tab.uiId),
            'drop-before': dropSideAt(ut.orderIndex) === 'before',
            'drop-after': dropSideAt(ut.orderIndex) === 'after',
            'state-working': !ut.tab.isShell && statusKind(ut.tab) === 'working',
            'state-done': !ut.tab.isShell && statusKind(ut.tab) === 'done',
            'state-blocked': !ut.tab.isShell && statusKind(ut.tab) === 'blocked',
            'state-error': !ut.tab.isShell && statusKind(ut.tab) === 'error',
            'state-exited': !ut.tab.isShell && statusKind(ut.tab) === 'exited',
            'state-unknown': !ut.tab.isShell && statusKind(ut.tab) === 'unknown',
          }"
          v-tooltip:bottom="ut.tab.title"
          :data-tab-ui-id="ut.tab.uiId"
          data-drag-kind="tui"
          :data-order="ut.order"
          :data-order-index="ut.orderIndex"
          role="button"
          tabindex="0"
          @click="onTabClick(ut.tab.uiId, $event)"
          @dblclick.stop="renameTab(ut.tab, $event)"
          @contextmenu="onTabContextMenu(ut.tab, $event)"
          @pointerdown="onTuiTabPointerDown(ut.tab, $event)"
          @keydown.enter.prevent="onTabClick(ut.tab.uiId)"
          @keydown.space.prevent="onTabClick(ut.tab.uiId)"
        >
          <IconTerminal v-if="ut.tab.isShell" class="term-tab-agent" />
          <component v-else :is="agentIcons[ut.tab.agent]" class="term-tab-agent" :class="ut.tab.agent" />
          <span class="term-tab-title">{{ shortTitle(ut.tab.title) }}</span>
          <span v-if="modHintDown && shortcutForIndex(inProjectBrowse ? ut.orderIndex + 1 : ut.orderIndex)" class="term-tab-shortcut">{{ shortcutForIndex(inProjectBrowse ? ut.orderIndex + 1 : ut.orderIndex) }}</span>
          <span
            v-if="!ut.tab.isShell && statusKind(ut.tab) === 'working'"
            class="term-tab-status term-tab-status-working"
            aria-hidden="true"
          >
            <i />
            <i />
            <i />
          </span>
          <span
            v-else-if="!ut.tab.isShell && statusKind(ut.tab) !== 'none'"
            class="term-tab-status"
            :class="'term-tab-status-' + statusKind(ut.tab)"
            aria-hidden="true"
          />
          <span
            class="term-tab-close"
            v-tooltip:bottom="t('chat.tui.tabClose')"
            role="button"
            tabindex="0"
            @click="onClose(ut.tab.uiId, $event)"
            @keydown.enter.prevent="onClose(ut.tab.uiId, $event)"
          >
            <IconClose />
          </span>
        </div>

        <!-- Saved (lazy-restore) tab —— 不能被拿起，但带 data-order* 参与几何、可作落点锚 -->
        <div
          v-else-if="ut.kind === 'saved'"
          class="term-tab term-tab-saved"
          :class="{
            'drop-before': dropSideAt(ut.orderIndex) === 'before',
            'drop-after': dropSideAt(ut.orderIndex) === 'after',
          }"
          v-tooltip:bottom="ut.saved.title"
          :data-order="ut.order"
          :data-order-index="ut.orderIndex"
          role="button"
          tabindex="0"
          @click="onSavedClick(ut.saved)"
          @contextmenu="onSavedContextMenu(ut.saved, $event)"
        >
          <IconTerminal v-if="ut.saved.isShell" class="term-tab-agent" />
          <component v-else :is="agentIcons[ut.saved.agent]" class="term-tab-agent" :class="ut.saved.agent" />
          <span class="term-tab-title">{{ shortTitle(ut.saved.title) }}</span>
          <span v-if="modHintDown && shortcutForIndex(inProjectBrowse ? ut.orderIndex + 1 : ut.orderIndex)" class="term-tab-shortcut">{{ shortcutForIndex(inProjectBrowse ? ut.orderIndex + 1 : ut.orderIndex) }}</span>
          <span
            class="term-tab-close"
            v-tooltip:bottom="t('chat.tui.tabClose')"
            role="button"
            tabindex="0"
            @click="onSavedClose(ut.saved, $event)"
            @keydown.enter.prevent="onSavedClose(ut.saved, $event)"
          >
            <IconClose />
          </span>
        </div>

        <!-- View (read/chat) tab -->
        <div
          v-else
          class="term-tab view-tab view-tab-closable"
          :class="{
            active: isViewTabActive(ut.vt),
            dragging: isDragSource('view', ut.vt.uiId),
            'drop-before': dropSideAt(ut.orderIndex) === 'before',
            'drop-after': dropSideAt(ut.orderIndex) === 'after',
            'state-working': viewStatusKind(ut.vt) === 'working',
            'state-done': viewStatusKind(ut.vt) === 'done',
            'state-blocked': viewStatusKind(ut.vt) === 'blocked',
            'state-error': viewStatusKind(ut.vt) === 'error',
            'state-exited': viewStatusKind(ut.vt) === 'exited',
          }"
          v-tooltip:bottom="ut.vt.title || (ut.vt.type === 'chat' ? t('chat.tui.chatTab') : t('chat.tui.viewTab'))"
          :data-tab-ui-id="ut.vt.uiId"
          data-drag-kind="view"
          :data-order="ut.order"
          :data-order-index="ut.orderIndex"
          role="button"
          tabindex="0"
          @click="onViewTabClick(ut.vt.uiId, $event)"
          @contextmenu="onViewTabContextMenu(ut.vt, $event)"
          @pointerdown="onViewTabPointerDown(ut.vt, $event)"
          @keydown.enter.prevent="onViewTabClick(ut.vt.uiId)"
          @keydown.space.prevent="onViewTabClick(ut.vt.uiId)"
        >
          <component :is="ut.vt.type === 'chat' ? IconChat : ut.vt.type === 'git' ? IconGitBranch : IconReader" class="term-tab-agent" />
          <span class="term-tab-title">{{ ut.vt.title ? shortTitle(ut.vt.title) : (ut.vt.type === 'chat' ? t('chat.tui.chatTab') : t('chat.tui.viewTab')) }}</span>
          <span v-if="modHintDown && shortcutForIndex(inProjectBrowse ? ut.orderIndex + 1 : ut.orderIndex)" class="term-tab-shortcut">{{ shortcutForIndex(inProjectBrowse ? ut.orderIndex + 1 : ut.orderIndex) }}</span>
          <span
            v-if="viewStatusKind(ut.vt) === 'working'"
            class="term-tab-status term-tab-status-working"
            aria-hidden="true"
          >
            <i />
            <i />
            <i />
          </span>
          <span
            v-else-if="viewStatusKind(ut.vt) !== 'none'"
            class="term-tab-status"
            :class="'term-tab-status-' + viewStatusKind(ut.vt)"
            aria-hidden="true"
          />
          <span
            class="term-tab-close"
            v-tooltip:bottom="t('chat.tui.tabClose')"
            role="button"
            tabindex="0"
            @click="onViewTabClose(ut.vt.uiId, $event)"
            @keydown.enter.prevent="onViewTabClose(ut.vt.uiId, $event)"
          >
            <IconClose />
          </span>
        </div>
      </template>
      </div>
    </div>

    <div ref="newMenuEl" class="new-menu-wrap" style="flex-shrink:0">
      <div
        class="term-tab-new"
        :class="{ active: newMenuOpen }"
        v-tooltip:bottom="t('list.action.newSession')"
        role="button"
        tabindex="0"
        @click.stop="toggleNewMenu"
        @keydown.enter.prevent="toggleNewMenu"
      >
        <IconPlus />
      </div>
      <div v-if="newMenuOpen" class="new-menu" role="menu">
        <NewMenu :agent="agent" :has-git="hasGit" show-split @new-session="pickNewAgent" @new-gui="pickNewGui" @new-shell="pickNewShell" @git-changes="pickGitChanges" @split-h="pickSplitH" @split-v="pickSplitV" />
      </div>
    </div>

    <div
      class="term-tab-new"
      style="flex-shrink:0"
      v-tooltip:bottom="t('pane.splitH')"
      role="button"
      tabindex="0"
      @click="pa.splitH()"
    ><IconSplitH /></div>
    <div
      class="term-tab-new"
      style="flex-shrink:0"
      v-tooltip:bottom="t('pane.splitV')"
      role="button"
      tabindex="0"
      @click="pa.splitV()"
    ><IconSplitV /></div>

    <div
      v-if="stripCtx"
      class="new-menu new-menu-floating"
      role="menu"
      :style="{ left: stripCtx.x + 'px', top: stripCtx.y + 'px' }"
      @click.stop
      @contextmenu.prevent.stop
    >
      <NewMenu :agent="agent" :has-git="hasGit" show-refresh show-split @new-session="newSessionFromStripCtx" @new-gui="newGuiFromStripCtx" @new-shell="newShellFromStripCtx" @git-changes="gitChangesFromStripCtx" @refresh="refreshFromStripCtx" @split-h="splitHFromStripCtx" @split-v="splitVFromStripCtx" />
    </div>

    <div
      v-if="listCtx"
      class="ctx-menu term-tab-ctx-menu"
      :style="{ left: listCtx.x + 'px', top: listCtx.y + 'px' }"
      @click.stop
      @contextmenu.prevent.stop
    >
      <button type="button" class="ctx-item" @click="closeTabCtx(); closeAllFromList()">
        <span>{{ t('chat.tui.tabCloseOthersAll') }}</span>
      </button>
      <button type="button" class="ctx-item danger" @click="closeTabCtx(); emit('closeAll')">
        <span>{{ t('chat.tui.tabCloseAll') }}</span>
      </button>
    </div>

    <div
      v-if="tabCtx"
      class="ctx-menu term-tab-ctx-menu"
      :style="{ left: tabCtx.x + 'px', top: tabCtx.y + 'px' }"
      @click.stop
      @contextmenu.prevent.stop
    >
      <button type="button" class="ctx-item" data-menu-action="tab-rename" @click="renameCtxTab">
        <span>{{ t(tabCtx?.tab?.isShell ? 'chat.tui.tabRenameShell' : 'chat.tui.tabRename') }}</span>
      </button>
      <div class="ctx-sep" />
      <button type="button" class="ctx-item" data-menu-action="tab-close" @click="closeCtxTab">
        <span>{{ t('chat.tui.tabClose') }}</span>
      </button>
      <button
        type="button"
        class="ctx-item"
        data-menu-action="tab-close-others"
        @click="closeOtherCtxTabs"
      >
        <span>{{ t(tabCtx?.tab?.isShell ? 'chat.tui.tabCloseOthers' : 'chat.tui.tabCloseOthersSession') }}</span>
      </button>
      <button
        type="button"
        class="ctx-item danger"
        data-menu-action="tab-close-project"
        @click="closeProjectCtxTabs"
      >
        <span>{{ t(tabCtx?.tab?.isShell ? 'chat.tui.tabCloseProject' : 'chat.tui.tabCloseProjectSession') }}</span>
      </button>
      <div class="ctx-sep" />
      <button type="button" class="ctx-item" @click="closeTabCtx(); tabCtx && emit('closeOthersAll', tabCtx.tab.uiId, 'tui')">
        <span>{{ t('chat.tui.tabCloseOthersAll') }}</span>
      </button>
      <button type="button" class="ctx-item danger" @click="closeTabCtx(); emit('closeAll')">
        <span>{{ t('chat.tui.tabCloseAll') }}</span>
      </button>
    </div>

    <div
      v-if="savedCtx"
      class="ctx-menu term-tab-ctx-menu"
      :style="{ left: savedCtx.x + 'px', top: savedCtx.y + 'px' }"
      @click.stop
      @contextmenu.prevent.stop
    >
      <button type="button" class="ctx-item" data-menu-action="saved-rename" @click="renameCtxSaved">
        <span>{{ t(savedCtx?.saved?.isShell ? 'chat.tui.tabRenameShell' : 'chat.tui.tabRename') }}</span>
      </button>
      <div class="ctx-sep" />
      <button type="button" class="ctx-item" data-menu-action="saved-close" @click="closeCtxSaved">
        <span>{{ t('chat.tui.tabClose') }}</span>
      </button>
      <button
        type="button"
        class="ctx-item"
        data-menu-action="saved-close-others"
        @click="closeOthersCtxSaved"
      >
        <span>{{ t('chat.tui.tabCloseOthers') }}</span>
      </button>
      <button
        type="button"
        class="ctx-item danger"
        data-menu-action="saved-close-project"
        @click="closeProjectCtxSaved"
      >
        <span>{{ t('chat.tui.tabCloseProject') }}</span>
      </button>
      <div class="ctx-sep" />
      <button type="button" class="ctx-item danger" @click="closeTabCtx(); emit('closeAll')">
        <span>{{ t('chat.tui.tabCloseAll') }}</span>
      </button>
    </div>

    <!-- View tab 右键菜单 (fallback) -->
    <div
      v-if="viewTabCtx"
      class="ctx-menu term-tab-ctx-menu"
      :style="{ left: viewTabCtx.x + 'px', top: viewTabCtx.y + 'px' }"
      @click.stop
      @contextmenu.prevent.stop
    >
      <button type="button" class="ctx-item" @click="const vt = viewTabCtx!.vt; closeViewTabCtx(); emit('viewRename', vt)">
        <span>{{ t('chat.tui.tabRenameView') }}</span>
      </button>
      <div class="ctx-sep" />
      <button type="button" class="ctx-item" @click="const id = viewTabCtx!.vt.uiId; closeViewTabCtx(); emit('viewClose', id)">
        <span>{{ t('chat.tui.tabClose') }}</span>
      </button>
      <button type="button" class="ctx-item" @click="const vt = viewTabCtx!.vt; closeViewTabCtx(); emit('viewCloseOthers', vt)">
        <span>{{ t('chat.tui.tabCloseOthersView', { type: viewTabCtx!.typeLabel }) }}</span>
      </button>
      <button type="button" class="ctx-item danger" @click="const tp = viewTabCtx!.vt.type; closeViewTabCtx(); emit('viewCloseProject', tp)">
        <span>{{ t('chat.tui.tabCloseProjectView', { type: viewTabCtx!.typeLabel }) }}</span>
      </button>
      <div class="ctx-sep" />
      <button type="button" class="ctx-item" @click="const vt = viewTabCtx!.vt; closeViewTabCtx(); emit('closeOthersAll', vt.uiId, 'view')">
        <span>{{ t('chat.tui.tabCloseOthersAll') }}</span>
      </button>
      <button type="button" class="ctx-item danger" @click="closeViewTabCtx(); emit('closeAll')">
        <span>{{ t('chat.tui.tabCloseAll') }}</span>
      </button>
    </div>

    <Teleport to="body">
      <div
        v-if="dragPreview?.kind === 'tui'"
        class="term-tab term-tab-drag-preview"
        :class="{
          active: pane.activeUiId === dragPreview.tab.uiId,
          'state-working': !dragPreview.tab.isShell && statusKind(dragPreview.tab) === 'working',
          'state-done': !dragPreview.tab.isShell && statusKind(dragPreview.tab) === 'done',
          'state-blocked': !dragPreview.tab.isShell && statusKind(dragPreview.tab) === 'blocked',
          'state-error': !dragPreview.tab.isShell && statusKind(dragPreview.tab) === 'error',
          'state-exited': !dragPreview.tab.isShell && statusKind(dragPreview.tab) === 'exited',
          'state-unknown': !dragPreview.tab.isShell && statusKind(dragPreview.tab) === 'unknown',
        }"
        :style="{
          left: dragPreview.x + 'px',
          top: dragPreview.y + 'px',
          width: dragPreview.width + 'px',
        }"
      >
        <IconTerminal v-if="dragPreview.tab.isShell" class="term-tab-agent" />
        <component
          v-else
          :is="agentIcons[dragPreview.tab.agent]"
          class="term-tab-agent"
          :class="dragPreview.tab.agent"
        />
        <span class="term-tab-title">{{ shortTitle(dragPreview.tab.title) }}</span>
        <span
          v-if="!dragPreview.tab.isShell && statusKind(dragPreview.tab) === 'working'"
          class="term-tab-status term-tab-status-working"
          aria-hidden="true"
        >
          <i />
          <i />
          <i />
        </span>
        <span
          v-else-if="!dragPreview.tab.isShell && statusKind(dragPreview.tab) !== 'none'"
          class="term-tab-status"
          :class="'term-tab-status-' + statusKind(dragPreview.tab)"
          aria-hidden="true"
        />
      </div>
      <div
        v-else-if="dragPreview?.kind === 'view'"
        class="term-tab view-tab term-tab-drag-preview"
        :class="{
          active: isViewTabActive(dragPreview.vt),
          'state-working': viewStatusKind(dragPreview.vt) === 'working',
          'state-done': viewStatusKind(dragPreview.vt) === 'done',
          'state-blocked': viewStatusKind(dragPreview.vt) === 'blocked',
          'state-error': viewStatusKind(dragPreview.vt) === 'error',
          'state-exited': viewStatusKind(dragPreview.vt) === 'exited',
        }"
        :style="{
          left: dragPreview.x + 'px',
          top: dragPreview.y + 'px',
          width: dragPreview.width + 'px',
        }"
      >
        <component
          :is="dragPreview.vt.type === 'chat' ? IconChat : dragPreview.vt.type === 'git' ? IconGitBranch : IconReader"
          class="term-tab-agent"
        />
        <span class="term-tab-title">{{
          dragPreview.vt.title
            ? shortTitle(dragPreview.vt.title)
            : dragPreview.vt.type === 'chat'
              ? t('chat.tui.chatTab')
              : t('chat.tui.viewTab')
        }}</span>
        <span
          v-if="viewStatusKind(dragPreview.vt) === 'working'"
          class="term-tab-status term-tab-status-working"
          aria-hidden="true"
        >
          <i />
          <i />
          <i />
        </span>
        <span
          v-else-if="viewStatusKind(dragPreview.vt) !== 'none'"
          class="term-tab-status"
          :class="'term-tab-status-' + viewStatusKind(dragPreview.vt)"
          aria-hidden="true"
        />
      </div>
    </Teleport>
  </div>
</template>
