<script setup lang="ts">
import { computed, inject, onMounted, onUnmounted, ref, watch } from 'vue'
import type { Agent, ProjectInfo, SessionMeta, UsageSummary } from '../types'
import { displaySessionId, formatSize, formatTime, formatTokens, highlightSegments, shortName } from '../format'
import { t } from '../i18n'
import {
  filterSessions,
  sessionSearch,
  sessionSort,
  sessionsFilterActive,
  sessionSelectMode,
  selectedSessions,
  toggleSessionSelected,
  exitSessionSelectMode,
} from '../sessionsToolbar'
import { searchSessions, cancelSearch, nextSearchRequestId, sessionUsage, sessionLastPrompt, gitHasRepo } from '../api'
import {
  IconTrash,
  IconPlay,
  IconFolder,
  IconInbox,
  IconPencil,
  IconCopy,
  IconDownload,
  IconMarkdown,
  IconHtml,
  IconJson,
  IconRefresh,
  IconPinUp,
  IconPinDown,
  IconCheck,
  IconSearch,
  IconPlus,
  IconSelect,
  IconClose,
  IconExitPane,
  IconChat,
  IconGitBranch,
} from '../components/icons'
import CreationSortIcon from '../components/CreationSortIcon.vue'
import NewMenu from '../components/NewMenu.vue'
import GitBranchControl from '../components/GitBranchControl.vue'
import { PaneActionsKey } from '../paneActions'
import { chatSupported } from '../chatComposerOptions'
import { agentSupports } from '../agentMeta'

const pa = inject(PaneActionsKey)!

const props = defineProps<{
  agent: Agent
  project: ProjectInfo
  sessions: SessionMeta[]
  sessionTotal: number
  loading: boolean
  loadingMore: boolean
  /** 多格分屏时才显示「退出分屏」按钮；单格无分屏可退。 */
  showExitPane?: boolean
}>()

const emit = defineEmits<{
  (e: 'open', s: SessionMeta): void
  (e: 'rename', s: SessionMeta): void
  (e: 'resume', s: SessionMeta): void
  (e: 'chat', s: SessionMeta): void
  (e: 'archivedBlock', cmd: string): void
  (e: 'reveal', path: string): void
  (e: 'delete', s: SessionMeta): void
  (e: 'copy', text: string): void
  (e: 'export', s: SessionMeta, kind: 'md' | 'html' | 'json'): void
  (e: 'refresh'): void
  (e: 'create-worktree'): void
  (e: 'new-session'): void
  (e: 'new-gui-session'): void
  (e: 'new-shell'): void
  /** 退出当前分屏格子（关闭并释放该格所有 tab）。 */
  (e: 'exit-pane'): void
  (e: 'load-more'): void
  (e: 'scroll', scrollTop: number): void
  /** 批量删除：原本由 SessionsTopbar 触发，现已挪到 list-head 顶栏里。 */
  (e: 'batch-delete'): void
  /** 批量导出：同上。 */
  (e: 'batch-export', kind: 'md' | 'html' | 'json'): void
}>()

const scrollEl = ref<HTMLElement>()

// git 探测：hasGit 控制「Git Changes」菜单项，对所有项目（含 worktree）生效；
// isGitRepo 仅限一级项目，控制「创建 Worktree」入口。
const hasGit = ref(false)
const isGitRepo = ref(false)
watch(
  () => props.project.displayPath,
  async (path) => {
    hasGit.value = false
    isGitRepo.value = false
    if (!path) return
    try {
      const ok = await gitHasRepo(path)
      hasGit.value = ok
      if (!props.project.worktreeName && !props.project.parentDirName) isGitRepo.value = ok
    } catch {
      hasGit.value = false
      isGitRepo.value = false
    }
  },
  { immediate: true },
)
// worktree 对按 cwd 归属会话的 Claude/Codex/Grok Build 开放；opencode/agy 会把 worktree
// 会话塌回主仓库，因此整体隐藏。
const agentSupportsWorktrees = computed(
  () => agentSupports(props.agent, 'worktree'),
)
const canCreateWorktree = computed(
  () =>
    agentSupportsWorktrees.value &&
    props.project.exists &&
    !props.project.worktreeName &&
    !props.project.parentDirName &&
    isGitRepo.value,
)

// ============================ 后端搜索（title + 用户消息正文） ============================
// 关键词搜索走后端：能命中 user-message 正文，而本地数组只有元数据。
// 防抖 + 可中断（cancelSearch + reqSeq 守卫）：保持与全局搜索一致的体感。
const SEARCH_DEBOUNCE_MS = 280
const SEARCH_MIN_LEN = 2
const searchHits = ref<SessionMeta[]>([])
const searching = ref(false)
let searchDebounceTimer = 0
let searchReqSeq = 0
let searchInFlight = false

function abortInFlightSearch() {
  if (!searchInFlight) return
  searchInFlight = false
  cancelSearch().catch(() => {})
}

async function runProjectSearch(query: string) {
  const trimmed = query.trim()
  if (trimmed.length < SEARCH_MIN_LEN) {
    searchHits.value = []
    searching.value = false
    return
  }
  const mySeq = ++searchReqSeq
  const reqId = nextSearchRequestId()
  searchInFlight = true
  try {
    const hits = await searchSessions(
      props.agent,
      trimmed,
      reqId,
      props.project.dirName,
      'keyword',
    )
    if (mySeq !== searchReqSeq) return
    // 后端返回 SearchHit[]；这里只关心 session 元数据，sort/withIdOnly 在 computed 里加。
    searchHits.value = hits.map((h) => h.session)
  } catch {
    if (mySeq !== searchReqSeq) return
    searchHits.value = []
  } finally {
    if (mySeq === searchReqSeq) {
      searchInFlight = false
      searching.value = false
    }
  }
}

// 关键词变化：每次新输入立刻打断在跑的搜索（fiber 风格），等防抖窗口稳定再发新请求。
watch(
  sessionSearch,
  (q) => {
    window.clearTimeout(searchDebounceTimer)
    abortInFlightSearch()
    if (q.trim().length < SEARCH_MIN_LEN) {
      searchHits.value = []
      searching.value = false
      return
    }
    searching.value = true
    searchDebounceTimer = window.setTimeout(() => {
      runProjectSearch(q)
    }, SEARCH_DEBOUNCE_MS)
  },
  { immediate: false },
)

// 切项目（dirName 变化）→ 清搜索结果；防抖里读的是更新后的 props
watch(
  () => props.project.dirName,
  () => {
    window.clearTimeout(searchDebounceTimer)
    abortInFlightSearch()
    searchHits.value = []
    searching.value = false
  },
)

// 工具栏（搜索 / 排序 / 仅带 ID）作用后的可见列表 —— 状态来自 sessionsToolbar 模块。
// 关键词非空时走后端搜索结果；空时走 props.sessions（分页 / 完整数组）。
// 两条路径都过 filterSessions，因为 sort + withIdOnly 是纯本地的策略。
// ============================ 单会话置顶 / 沉底（持久化,镜像项目那套 projPrefs）============================
// 重要会话置顶、不重要的沉底。key = `${agent}::${path}`（path 全局唯一）。stable sort 保组内 mtime 序。
// 注意：列表按 mtime 分页加载,这里只在**已加载**的会话内排序 —— 置顶一个很老、还没翻到的会话时,
// 要滚动加载到它才会浮顶（符合直觉：能点到的卡片一定已加载）。
type SessState = 'pinned' | 'sunk'
const SESS_PREFS_KEY = 'sessionPrefs:v1'
function loadSessPrefs(): Record<string, SessState> {
  try {
    return JSON.parse(localStorage.getItem(SESS_PREFS_KEY) || '{}')
  } catch {
    return {}
  }
}
const sessPrefs = ref<Record<string, SessState>>(loadSessPrefs())
function sessPrefKey(s: SessionMeta): string {
  return `${props.agent}::${s.path}`
}
function sessStateOf(s: SessionMeta): SessState | undefined {
  return sessPrefs.value[sessPrefKey(s)]
}
function setSessState(s: SessionMeta, state: SessState) {
  const key = sessPrefKey(s)
  if (sessPrefs.value[key] === state) {
    delete sessPrefs.value[key]
  } else {
    sessPrefs.value[key] = state
  }
  sessPrefs.value = { ...sessPrefs.value }
  localStorage.setItem(SESS_PREFS_KEY, JSON.stringify(sessPrefs.value))
}

const visibleSessions = computed(() => {
  const base =
    sessionSearch.value.trim().length >= SEARCH_MIN_LEN ? searchHits.value : props.sessions
  const filtered = filterSessions(base)
  // 置顶(0) → 普通(1) → 沉底(2)；Array.sort 稳定,组内保持原有（mtime / 搜索相关性）顺序。
  const rank = (s: SessionMeta) => {
    const st = sessStateOf(s)
    return st === 'pinned' ? 0 : st === 'sunk' ? 2 : 1
  }
  return [...filtered].sort((a, b) => rank(a) - rank(b))
})

// ============================ 每卡 token 用量懒加载 ============================
// 1000+ 会话的列表里同步 fetch 每条 usage 会把 IPC 撑爆。IntersectionObserver
// 监听卡片进入视口，看到的才请求；已请求 / 已缓存的直接命中。
// 切项目（dirName 变）整张表清。
const usageMap = ref<Map<string, UsageSummary>>(new Map())
const usageInFlight = new Set<string>() // 防重复请求
let usageIO: IntersectionObserver | null = null
const usageCardsPending = new Set<HTMLElement>()

function fetchUsage(path: string) {
  if (usageMap.value.has(path) || usageInFlight.has(path)) return
  usageInFlight.add(path)
  sessionUsage(props.agent, path)
    .then((u) => {
      // 期间用户可能切了项目，校验下当前 props.project.dirName 不会有太大意义
      // —— 用户回到原项目时，cache 仍然命中，惨痛代价是闪一下 stale 值。
      // 这里直接写就行。
      const next = new Map(usageMap.value)
      next.set(path, u)
      usageMap.value = next
    })
    .catch(() => {})
    .finally(() => {
      usageInFlight.delete(path)
    })
}

const subtitleMap = ref<Map<string, { text: string; mtime: string }>>(new Map())
const subtitleInFlight = new Set<string>()

function fetchSubtitle(path: string) {
  if (subtitleInFlight.has(path)) return
  const session = props.sessions.find((s) => s.path === path)
  const mtime = String(session?.modified ?? '')
  const cached = subtitleMap.value.get(path)
  if (cached && cached.mtime === mtime) return
  subtitleInFlight.add(path)
  sessionLastPrompt(props.agent, path)
    .then((text) => {
      if (text) {
        const next = new Map(subtitleMap.value)
        next.set(path, { text, mtime })
        subtitleMap.value = next
      }
    })
    .catch(() => {})
    .finally(() => { subtitleInFlight.delete(path) })
}

// Vue ref callback：每张卡片 mounted 时把 element 注册到 observer；unmount 时取消。
function observeUsageCard(path: string, el: Element | null) {
  if (!el) return
  // 同一个 path 可能反复 mount / unmount（v-for key 重组），简单 observe 两次也无害。
  const card = el as HTMLElement
  card.dataset.usagePath = path
  if (usageIO) {
    usageIO.observe(card)
  } else {
    // Vue ref 回调先于 onMounted 执行；暂存这些节点，observer 建好后补注册。
    usageCardsPending.add(card)
  }
}

onMounted(() => {
  usageIO = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue
        const p = (e.target as HTMLElement).dataset.usagePath
        if (!p) continue
        fetchUsage(p)
        fetchSubtitle(p)
        usageIO?.unobserve(e.target)
      }
    },
    { rootMargin: '120px 0px' }, // 提前一屏开始预取
  )
  for (const card of usageCardsPending) {
    usageIO.observe(card)
  }
  usageCardsPending.clear()
})

onUnmounted(() => {
  usageIO?.disconnect()
  usageIO = null
  usageCardsPending.clear()
})

// 切项目 → 清缓存 + 在飞请求标记（前一项目的结果回来也不会写到新表里，因为表是新的）
watch(
  () => props.project.dirName,
  () => {
    usageMap.value = new Map()
    usageInFlight.clear()
  },
)

// ---------- list-head 的批量选择 UI（原本住在 SessionsTopbar，挪过来减少
// "topbar + list-head 两排 icon-only 按钮重叠" 的扫描负担）。
// selectedCount / allSelected / toggleSelectAll 和原 SessionsTopbar 完全一致 ——
// 状态都在 sessionsToolbar 模块里，topbar 和 view 任意一边写另一边都看得见。
const creationSortDirection = computed<'asc' | 'desc' | null>(() => {
  if (sessionSort.value === 'createdOldest') return 'asc'
  if (sessionSort.value === 'createdRecent') return 'desc'
  return null
})
const creationSortLabel = computed(() => {
  if (sessionSort.value === 'createdOldest') return t('list.tb.sortCreatedOldest')
  if (sessionSort.value === 'createdRecent') return t('list.tb.sortCreatedRecent')
  return t('list.tb.sortCreated')
})
function toggleCreationSort() {
  sessionSort.value =
    sessionSort.value === 'createdRecent' ? 'createdOldest' : 'createdRecent'
}
const headSelectedCount = computed(
  () => props.sessions.filter((s) => selectedSessions.value.has(s.path)).length,
)
const headAllSelected = computed(
  () =>
    visibleSessions.value.length > 0 &&
    visibleSessions.value.every((s) => selectedSessions.value.has(s.path)),
)
function headToggleSelectAll() {
  const next = new Set(selectedSessions.value)
  for (const s of visibleSessions.value) {
    if (headAllSelected.value) next.delete(s.path)
    else next.add(s.path)
  }
  selectedSessions.value = next
}

// 批量导出小弹层：与会话卡片自带的 export-menu 行为一致。
const headExportMenuOpen = ref(false)
const headExportMenuEl = ref<HTMLElement>()
function toggleHeadExportMenu(e: Event) {
  e.stopPropagation()
  headExportMenuOpen.value = !headExportMenuOpen.value
}
function pickHeadExport(kind: 'md' | 'html' | 'json', e: Event) {
  e.stopPropagation()
  headExportMenuOpen.value = false
  emit('batch-export', kind)
}
function onHeadDocClick(e: MouseEvent) {
  if (!headExportMenuOpen.value) return
  if (headExportMenuEl.value && headExportMenuEl.value.contains(e.target as Node)) return
  headExportMenuOpen.value = false
}
onMounted(() => document.addEventListener('click', onHeadDocClick))
onUnmounted(() => document.removeEventListener('click', onHeadDocClick))

// 每张卡片自己的导出菜单状态：只允许一个打开，按 session path 标识。
const openExportFor = ref<string | null>(null)
const exportMenuEls = ref<Record<string, HTMLElement | null>>({})
function setExportMenuEl(path: string, el: Element | null) {
  exportMenuEls.value[path] = el as HTMLElement | null
}
function toggleExport(path: string, e: Event) {
  e.stopPropagation()
  openExportFor.value = openExportFor.value === path ? null : path
}
function pickExport(s: SessionMeta, kind: 'md' | 'html' | 'json', e: Event) {
  e.stopPropagation()
  openExportFor.value = null
  emit('export', s, kind)
}
function onDocClick(e: MouseEvent) {
  const p = openExportFor.value
  if (!p) return
  const anchor = exportMenuEls.value[p]
  if (anchor && anchor.contains(e.target as Node)) return
  openExportFor.value = null
}
onMounted(() => document.addEventListener('click', onDocClick))
onUnmounted(() => document.removeEventListener('click', onDocClick))

// 工具栏搜索时把标题 / ID 里命中的关键词切成高亮片段（命中段加 .kw-hit）。
function titleSegs(title: string) {
  return highlightSegments(title, sessionSearch.value)
}
function idSegs(id: string) {
  return highlightSegments(displaySessionId(id), sessionSearch.value)
}

function codexRankLabel(s: SessionMeta): string {
  if (props.agent !== 'codex' || !s.codexAppListScanned) return ''
  const firstPageSize = s.codexAppFirstPageSize || 50
  const position = s.codexAppFirstPagePosition || 0
  const rank = s.codexAppListRank || '-'
  return `首屏 ${position}/${firstPageSize} · rank ${rank}`
}

function codexSpecialLabel(s: SessionMeta): string {
  if (props.agent !== 'codex') return ''
  if (s.codexArchived) return t('list.codex.archived')
  if (s.codexInternal) return t('list.codex.internal')
  return ''
}

// 批量模式下点整张卡片即勾选；否则按以往打开会话。
function onCardClick(s: SessionMeta) {
  if (sessionSelectMode.value) toggleSessionSelected(s.path)
  else emit('open', s)
}

onUnmounted(() => {
  clearTimeout(scrollIdle)
  window.clearTimeout(searchDebounceTimer)
  abortInFlightSearch()
})

// 滚动期间临时关掉 hover 滑块：滚动时 content 在静止光标下移动会狂发
// mouseover，再叠加滑块过渡，是滚动卡顿的一个来源。标记 scrolling 后
// mouseover 直接 return 并隐藏滑块；停止滚动 140ms 后恢复。
let scrolling = false
let scrollIdle = 0
function markScrolling() {
  if (!scrolling) {
    scrolling = true
    scrollEl.value?.classList.remove('has-spot')
    hoverPath.value = null
  }
  clearTimeout(scrollIdle)
  scrollIdle = window.setTimeout(() => {
    scrolling = false
  }, 140)
}

// 触底加载锁：emit('load-more') 后锁 300ms，避免一帧帧的 scroll 事件
// 在加载状态切换的间隙里重复触发。loadingMore / 全部加载完 也各有一道 guard。
let loadLockUntil = 0

// 滚动 → 一帧最多触发一次：
//   - emit('scroll', …) 用于父组件持久化滚动位置
//   - 接近底部 (<280px) 且没在加载、没全部加载完、且不在 300ms 锁内时 load-more
let scrollRaf = 0
function onScroll(e: Event) {
  markScrolling()
  if (scrollRaf) return
  const el = e.target as HTMLElement
  scrollRaf = requestAnimationFrame(() => {
    scrollRaf = 0
    emit('scroll', el.scrollTop)
    if (props.loadingMore) return
    if (props.sessions.length >= props.sessionTotal) return
    if (Date.now() < loadLockUntil) return
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 280) {
      loadLockUntil = Date.now() + 300
      emit('load-more')
    }
  })
}

// hover spotlight：.vlist 里放一块绝对定位的高亮浮块，鼠标 mouseover 命中
// 任一 .session-card 就把它的 offsetTop / offsetHeight 写到 --spot-y / --spot-h
// （驱动浮块的 top / height）。
// 注意：.scroll-area 在 v-else 分支里，onMounted 时可能还没渲染（loading 态），
// 所以走模板 @mouseover / @mouseleave 绑定。
const spotlightEl = ref<HTMLElement>()
// 当前 hover 行的 session.path —— 驱动 .is-hover（操作按钮 / 重命名 / 复制图标显隐）。
// 用 JS 而非 CSS :hover，让操作按钮与滑块同源：滚动中两者一起隐藏。
const hoverPath = ref<string | null>(null)

function onListMouseOver(e: MouseEvent) {
  if (scrolling) return // 滚动中不触发滑块
  // 导出菜单展开时钉住 hover：菜单浮层悬在下一张卡片之上，鼠标移进去会
  // 把 hoverPath 翻给下方卡片，连带抽走菜单所在行的 .is-hover。
  if (openExportFor.value) return
  const sa = scrollEl.value
  const sp = spotlightEl.value
  if (!sa || !sp) return
  const card = (e.target as HTMLElement | null)?.closest<HTMLElement>('.session-card')
  if (!card || !sa.contains(card)) return
  hoverPath.value = card.dataset.path ?? null
  // 滑块刚从隐藏态重新出现时，先 no-slide 直接跳到目标位置再淡入，
  // 避免"从上一个位置滑过整屏"的突兀感；同行内移动则保持平滑滑动。
  const reappearing = !sa.classList.contains('has-spot')
  if (reappearing) sp.classList.add('no-slide')
  sp.style.setProperty('--spot-y', `${card.offsetTop}px`)
  sp.style.setProperty('--spot-h', `${card.offsetHeight}px`)
  sa.classList.add('has-spot')
  if (reappearing) {
    requestAnimationFrame(() =>
      requestAnimationFrame(() => sp.classList.remove('no-slide')),
    )
  }
}

function onListMouseLeave() {
  scrollEl.value?.classList.remove('has-spot')
  hoverPath.value = null
}

// ---- 新建会话下拉菜单 ----
const newMenuOpen = ref(false)
const newMenuEl = ref<HTMLElement>()
// 标题空白处右键 → 同一组「新建」菜单，跟随光标定位（fixed 视口坐标）。
const ctxMenuPos = ref<{ x: number; y: number } | null>(null)
const ctxMenuEl = ref<HTMLElement>()
function toggleNewMenu(e: Event) {
  e.stopPropagation()
  newMenuOpen.value = !newMenuOpen.value
}
function openContextMenu(e: MouseEvent) {
  // 目录已不存在时没有可执行的新建动作（与「+」按钮同样 v-if="project.exists"），不弹。
  if (!props.project.exists) return
  ctxMenuPos.value = { x: e.clientX, y: e.clientY }
}
function pickNewAgent() {
  newMenuOpen.value = false
  ctxMenuPos.value = null
  emit('new-session')
}
function pickNewGui() {
  newMenuOpen.value = false
  ctxMenuPos.value = null
  emit('new-gui-session')
}
function pickNewShell() {
  newMenuOpen.value = false
  ctxMenuPos.value = null
  emit('new-shell')
}
function pickRefresh() {
  newMenuOpen.value = false
  ctxMenuPos.value = null
  emit('refresh')
}
function pickSplitH() {
  ctxMenuPos.value = null
  pa.splitH()
}
function pickSplitV() {
  ctxMenuPos.value = null
  pa.splitV()
}
function pickGitChanges() {
  newMenuOpen.value = false
  ctxMenuPos.value = null
  pa.openGitChanges()
}
function onNewMenuDocClick(e: MouseEvent) {
  if (newMenuOpen.value && !newMenuEl.value?.contains(e.target as Node)) {
    newMenuOpen.value = false
  }
  if (ctxMenuPos.value && !ctxMenuEl.value?.contains(e.target as Node)) {
    ctxMenuPos.value = null
  }
}
onMounted(() => document.addEventListener('click', onNewMenuDocClick))
onUnmounted(() => document.removeEventListener('click', onNewMenuDocClick))

defineExpose({ scrollEl })
</script>

<template>
  <div class="list-head list-head-row">
    <div class="grow" @contextmenu.prevent="openContextMenu">
      <h2>{{ shortName(project.displayPath) }}</h2>
      <div class="path">
        {{ project.displayPath }}<span
          v-if="!project.exists"
          class="dir-missing-tag"
        >{{ t('list.dirMissing') }}</span>
      </div>
    </div>
    <!-- 标题 / 列表区右键菜单：复用「新建」三项 + 刷新，position:fixed 跟随光标。 -->
    <div
      v-if="ctxMenuPos"
      ref="ctxMenuEl"
      class="new-menu new-menu-floating"
      role="menu"
      :style="{ left: ctxMenuPos.x + 'px', top: ctxMenuPos.y + 'px' }"
      @click.stop
    >
      <NewMenu
        :agent="agent"
        :has-git="hasGit"
        show-refresh
        show-split
        @new-session="pickNewAgent"
        @new-gui="pickNewGui"
        @new-shell="pickNewShell"
        @git-changes="pickGitChanges"
        @refresh="pickRefresh"
        @split-h="pickSplitH"
        @split-v="pickSplitV"
      />
    </div>
    <div class="list-head-actions">
      <template v-if="sessionSelectMode">
        <span class="ct-search-count">{{
          t('list.tb.selectedCount', { n: headSelectedCount })
        }}</span>
        <!-- 选择控制：select-all + 取消选择。两个按钮挨着，跟下面的"对选中项的动作"
             用 .list-head-divider 一条细竖线分开。 -->
        <button
          class="icon-btn"
          :class="{ active: headAllSelected }"
          v-tooltip="headAllSelected ? t('list.tb.selectNone') : t('list.tb.selectAll')"
          @click="headToggleSelectAll"
        >
          <IconCheck />
        </button>
        <button
          class="icon-btn"
          v-tooltip="t('list.tb.selectCancel')"
          @click="exitSessionSelectMode"
        >
          <IconClose />
        </button>
        <span class="list-head-divider" aria-hidden="true" />
        <!-- 对选中项的动作：批量导出 + 批量删除。 -->
        <div ref="headExportMenuEl" class="export-menu-wrap">
          <button
            class="icon-btn"
            :class="{ active: headExportMenuOpen }"
            :disabled="headSelectedCount === 0"
            v-tooltip="t('list.tb.exportSelected')"
            @click="toggleHeadExportMenu"
          >
            <IconDownload />
          </button>
          <div
            v-if="headExportMenuOpen"
            class="export-menu"
            role="menu"
            @click.stop
          >
            <button
              class="export-menu-item"
              role="menuitem"
              @click.stop="pickHeadExport('md', $event)"
            >
              <IconMarkdown />
              <span>{{ t('chat.tb.export.md') }}</span>
            </button>
            <button
              class="export-menu-item"
              role="menuitem"
              @click.stop="pickHeadExport('html', $event)"
            >
              <IconHtml />
              <span>{{ t('chat.tb.export.html') }}</span>
            </button>
            <button
              class="export-menu-item"
              role="menuitem"
              @click.stop="pickHeadExport('json', $event)"
            >
              <IconJson />
              <span>{{ t('chat.tb.export.json') }}</span>
            </button>
          </div>
        </div>
        <button
          class="icon-btn danger"
          :disabled="headSelectedCount === 0"
          v-tooltip="t('list.tb.deleteSelected')"
          @click="emit('batch-delete')"
        >
          <IconTrash />
        </button>
      </template>
      <template v-else>
        <GitBranchControl
          v-if="project.exists && hasGit"
          class="list-head-branch"
          :cwd="project.displayPath"
        />
        <span
          v-if="project.exists && hasGit"
          class="list-head-branch-divider"
          aria-hidden="true"
        />
        <!-- 进入批量模式 —— 原本住在 SessionsTopbar 的 .ct-actions 里，
             跟下方 new/refresh/delete 隔一行 topbar 视觉冲突，挪到这里集中显示。 -->
        <button
          v-if="sessions.length > 1"
          class="icon-btn"
          v-tooltip="t('list.tb.select')"
          @click="sessionSelectMode = true"
        >
          <IconSelect />
        </button>
        <div v-if="project.exists" ref="newMenuEl" class="new-menu-wrap">
          <button
            class="icon-btn"
            :class="{ active: newMenuOpen }"
            v-tooltip="t('list.action.newSession')"
            @click="toggleNewMenu"
          >
            <IconPlus />
          </button>
          <div v-if="newMenuOpen" class="new-menu" role="menu">
            <NewMenu :agent="agent" :has-git="hasGit" show-split @new-session="pickNewAgent" @new-gui="pickNewGui" @new-shell="pickNewShell" @git-changes="pickGitChanges" @split-h="pickSplitH" @split-v="pickSplitV" />
          </div>
        </div>
        <button
          v-if="canCreateWorktree"
          class="icon-btn"
          v-tooltip="t('proj.createWorktree')"
          @click="emit('create-worktree')"
        >
          <IconGitBranch />
        </button>
        <button
          v-if="sessions.length > 1"
          class="icon-btn creation-sort-button"
          :aria-label="creationSortLabel"
          @click="toggleCreationSort"
        >
          <CreationSortIcon :direction="creationSortDirection" />
        </button>
        <button
          v-if="project.exists"
          class="icon-btn"
          :disabled="loading"
          v-tooltip="t('list.action.refresh')"
          @click="emit('refresh')"
        >
          <IconRefresh />
        </button>
        <button
          v-if="showExitPane"
          class="icon-btn"
          v-tooltip="t('pane.exit')"
          @click="emit('exit-pane')"
        >
          <IconExitPane />
        </button>
      </template>
    </div>
  </div>
  <div v-if="loading" class="loading">{{ t('common.loading') }}</div>
  <div v-else-if="!sessions.length" class="empty" @contextmenu.prevent="openContextMenu">
    <div class="big"><IconInbox /></div>
    <div>{{ t('list.empty') }}</div>
  </div>
  <div v-else-if="!visibleSessions.length" class="empty" @contextmenu.prevent="openContextMenu">
    <div class="big"><IconSearch /></div>
    <div>{{ t('list.noMatch') }}</div>
  </div>
  <div
    v-else
    ref="scrollEl"
    class="scroll-area"
    @contextmenu.prevent="openContextMenu"
    @scroll="onScroll"
    @mouseover.passive="onListMouseOver"
    @mouseleave.passive="onListMouseLeave"
  >
    <div class="vlist">
      <div ref="spotlightEl" class="list-spotlight" aria-hidden="true" />
      <div
        v-for="s in visibleSessions"
        :key="s.path"
        class="session-card"
        :class="{
          'is-hover': s.path === hoverPath,
          'menu-open': openExportFor === s.path,
          'list-selectable': sessionSelectMode,
          'list-selected': sessionSelectMode && selectedSessions.has(s.path),
          'sess-pinned': sessStateOf(s) === 'pinned',
          'sess-sunk': sessStateOf(s) === 'sunk',
        }"
        :data-path="s.path"
        @click="onCardClick(s)"
      >
      <span
        v-if="sessionSelectMode"
        class="list-check"
        :class="{ on: selectedSessions.has(s.path) }"
        aria-hidden="true"
      >
        <IconCheck v-if="selectedSessions.has(s.path)" />
      </span>
      <div class="session-main">
        <div class="session-title">
          <span class="session-title-text"><span
            v-for="(seg, i) in titleSegs(s.title)"
            :key="i"
            :class="{ 'kw-hit': seg.hit }"
          >{{ seg.text }}</span></span>
          <button
            v-if="!sessionSelectMode"
            class="title-rename-ic"
            v-tooltip="t('list.action.rename')"
            @click.stop="emit('rename', s)"
          >
            <IconPencil />
          </button>
        </div>
        <div v-if="subtitleMap.get(s.path)?.text && subtitleMap.get(s.path)!.text !== s.title" class="session-subtitle">{{ subtitleMap.get(s.path)!.text }}</div>
        <div class="session-meta">
          <span>{{ t('list.messages', { n: s.messageCount }) }}</span>
          <span>{{ formatSize(s.size) }}</span>
          <span>{{ t('session.updated', { time: formatTime(s.modified) }) }}</span>
          <!-- Token 角标：IntersectionObserver 看到这条 chip 才发请求 (`observeUsageCard`)。
               cache 命中时显示 total；空数据显示 "—"；loading 显示空占位。 -->
          <span
            class="session-tok"
            :ref="(el) => observeUsageCard(s.path, el as Element | null)"
            v-tooltip="t('chat.tb.usage.tooltip')"
          >
            <template v-if="usageMap.has(s.path)">
              <template v-if="usageMap.get(s.path)!.total > 0">
                {{ formatTokens(usageMap.get(s.path)!.total) }} {{ t('chat.tb.usage.label') }}
              </template>
              <template v-else>—</template>
            </template>
          </span>
          <span v-if="s.id" class="session-id" v-tooltip="s.id">
            <span class="session-id-label">{{ t('session.id') }}</span>
            <span class="session-id-text"><span
              v-for="(seg, i) in idSegs(s.id)"
              :key="i"
              :class="{ 'kw-hit': seg.hit }"
            >{{ seg.text }}</span></span>
            <button
              v-if="!sessionSelectMode"
              class="session-id-copy"
              v-tooltip="t('list.action.copyId')"
              @click.stop="emit('copy', s.id)"
            >
              <IconCopy />
            </button>
          </span>
          <span v-if="codexSpecialLabel(s)" class="codex-special-tag">
            {{ codexSpecialLabel(s) }}
          </span>
          <span v-else-if="codexRankLabel(s)">{{ codexRankLabel(s) }}</span>
        </div>
      </div>
      <div v-if="!sessionSelectMode" class="session-actions">
        <button
          v-if="chatSupported(agent) && project.exists"
          class="icon-btn"
          v-tooltip="t('list.action.openChat')"
          @click.stop="s.codexArchived ? emit('archivedBlock', `codex unarchive ${s.id}`) : emit('chat', s)"
        >
          <IconChat />
        </button>
        <button
          v-if="project.exists"
          class="icon-btn"
          v-tooltip="s.cwd?.startsWith('ide://') ? t('list.action.resumeIde') : t('list.action.resume')"
          @click.stop="s.codexArchived ? emit('archivedBlock', `codex unarchive ${s.id}`) : emit('resume', s)"
        >
          <IconPlay />
        </button>
        <button
          v-if="agent !== 'opencode'"
          class="icon-btn"
          v-tooltip="t('list.action.reveal')"
          @click.stop="emit('reveal', s.path)"
        >
          <IconFolder />
        </button>
        <div
          :ref="(el) => setExportMenuEl(s.path, el as Element | null)"
          class="export-menu-wrap"
        >
          <button
            class="icon-btn"
            :class="{ active: openExportFor === s.path }"
            v-tooltip:top="t('chat.tb.export.md') + ' / ' + t('chat.tb.export.html')"
            @click.stop="toggleExport(s.path, $event)"
          >
            <IconDownload />
          </button>
          <!-- @click.stop on the menu container itself: clicks landing on the
               menu's padding/gap (not an item) would otherwise bubble to the
               .session-card and navigate into the session. -->
          <div
            v-if="openExportFor === s.path"
            class="export-menu"
            role="menu"
            @click.stop
          >
            <button
              class="export-menu-item"
              role="menuitem"
              @click.stop="pickExport(s, 'md', $event)"
            >
              <IconMarkdown />
              <span>{{ t('chat.tb.export.md') }}</span>
            </button>
            <button
              class="export-menu-item"
              role="menuitem"
              @click.stop="pickExport(s, 'html', $event)"
            >
              <IconHtml />
              <span>{{ t('chat.tb.export.html') }}</span>
            </button>
            <button
              class="export-menu-item"
              role="menuitem"
              @click.stop="pickExport(s, 'json', $event)"
            >
              <IconJson />
              <span>{{ t('chat.tb.export.json') }}</span>
            </button>
          </div>
        </div>
        <span class="action-sep" aria-hidden="true" />
        <button
          class="icon-btn sess-pin-btn"
          :class="{ active: sessStateOf(s) === 'pinned' }"
          v-tooltip="sessStateOf(s) === 'pinned' ? t('list.action.unpin') : t('list.action.pin')"
          @click.stop="setSessState(s, 'pinned')"
        >
          <IconPinUp />
        </button>
        <button
          class="icon-btn sess-pin-btn"
          :class="{ active: sessStateOf(s) === 'sunk' }"
          v-tooltip="sessStateOf(s) === 'sunk' ? t('list.action.unsink') : t('list.action.sink')"
          @click.stop="setSessState(s, 'sunk')"
        >
          <IconPinDown />
        </button>
        <button
          class="icon-btn danger"
          v-tooltip="t('list.action.trash')"
          @click.stop="emit('delete', s)"
        >
          <IconTrash />
        </button>
      </div>
      </div>
    </div>
    <div class="list-footer">
      <span
        v-if="loadingMore"
        class="footer-loading"
        role="status"
        aria-live="polite"
      >
        <span class="chip-spinner" aria-hidden="true" />
        {{ t('list.footer.loading') }}
      </span>
      <span v-else-if="sessionsFilterActive">
        {{ t('list.footer.matched', { n: visibleSessions.length }) }}
      </span>
      <span v-else-if="sessions.length < sessionTotal">
        {{
          t('list.footer.partial', {
            shown: sessions.length,
            total: sessionTotal,
          })
        }}
      </span>
      <span v-else>
        {{ t('list.footer.all', { total: sessionTotal }) }}
      </span>
    </div>
  </div>
</template>
