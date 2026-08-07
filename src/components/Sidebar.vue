<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import type { Agent, ProjectInfo } from '../types'
import { shortName } from '../format'
import { t } from '../i18n'
import { IconDownload, IconRefresh, IconSettings, IconClose, IconCheck, IconTrash, IconSelect, IconGitBranch, agentIcons } from './icons'
import { latestVersion, updateAvailable } from '../updateCheck'
import { visibleAgents } from '../settings'

type ProjState = 'pinned' | 'sunk'

const props = defineProps<{
  agent: Agent
  projects: ProjectInfo[]
  activeDir: string | null
  showTrash: boolean
  projPrefs: Record<string, ProjState>
  projectOrder?: string[]
  refreshing?: boolean
}>()

const emit = defineEmits<{
  (e: 'switch-agent', a: Agent): void
  (e: 'select-project', dir: string): void
  (e: 'context-menu', evt: MouseEvent, p: ProjectInfo): void
  (e: 'open-settings', tab?: 'general' | 'updates'): void
  (e: 'refresh'): void
  (e: 'add-bookmark'): void
  (e: 'batch-delete', dirs: string[]): void
  (e: 'reorder-projects', dirNames: string[]): void
}>()

const agentLabel = (a: Agent) =>
  a === 'codex' ? 'Codex' : a === 'agy' ? 'agy' : a === 'opencode' ? 'opencode' : 'Claude'
const agentName = computed(() => agentLabel(props.agent))
// 3 个及以上 agent 时分段控件放不下 icon+文字 —— 收成纯图标，名字挪进 tooltip。
const switcherIconsOnly = computed(() => visibleAgents.value.length > 2)

function prefKey(p: ProjectInfo): string {
  return `${props.agent}::${p.dirName}`
}
function projStateOf(p: ProjectInfo): ProjState | undefined {
  return props.projPrefs[prefKey(p)]
}

function projectGroupRank(p: ProjectInfo) {
  return projStateOf(p) === 'pinned' ? 0 : projStateOf(p) === 'sunk' ? 2 : 1
}

function orderKey(p: ProjectInfo) {
  return p.parentDirName ?? p.dirName
}

const sortedProjects = computed(() => {
  const inputIndex = new Map(props.projects.map((project, index) => [project.dirName, index]))
  const orderIndex = new Map((props.projectOrder ?? []).map((dirName, index) => [dirName, index]))
  const indexOf = (project: ProjectInfo) => orderIndex.get(orderKey(project)) ?? Number.MAX_SAFE_INTEGER
  return [...props.projects].sort((a, b) => {
    const rankDiff = projectGroupRank(a) - projectGroupRank(b)
    if (rankDiff) return rankDiff
    const orderDiff = indexOf(a) - indexOf(b)
    if (orderDiff) return orderDiff
    if (projectGroupRank(a) === 1) {
      const bookmarkDiff = Number(!(a.bookmarked && !a.sessionCount)) - Number(!(b.bookmarked && !b.sessionCount))
      if (bookmarkDiff) return bookmarkDiff
    }
    return (inputIndex.get(a.dirName) ?? 0) - (inputIndex.get(b.dirName) ?? 0)
  })
})

type SidebarEntry =
  | { kind: 'solo'; project: ProjectInfo }
  | { kind: 'parent'; project: ProjectInfo; children: ProjectInfo[] }
  | { kind: 'child'; project: ProjectInfo; parentDirName: string }

const collapsedWorktrees = ref(new Set<string>())

function toggleWorktreeCollapse(parentDir: string) {
  const next = new Set(collapsedWorktrees.value)
  if (next.has(parentDir)) next.delete(parentDir)
  else next.add(parentDir)
  collapsedWorktrees.value = next
}

const groupedEntries = computed<SidebarEntry[]>(() => {
  const list = sortedProjects.value
  const childrenMap = new Map<string, ProjectInfo[]>()
  const parentDirs = new Set<string>()
  for (const p of list) {
    if (p.parentDirName) {
      parentDirs.add(p.parentDirName)
      const arr = childrenMap.get(p.parentDirName) || []
      arr.push(p)
      childrenMap.set(p.parentDirName, arr)
    }
  }
  const entries: SidebarEntry[] = []
  for (const p of list) {
    if (p.parentDirName) continue
    const children = childrenMap.get(p.dirName)
    if (children?.length) {
      entries.push({ kind: 'parent', project: p, children })
      if (!collapsedWorktrees.value.has(p.dirName)) {
        for (const c of children) {
          entries.push({ kind: 'child', project: c, parentDirName: p.dirName })
        }
      }
    } else {
      entries.push({ kind: 'solo', project: p })
    }
  }
  // orphan worktrees whose parent isn't in the list
  for (const p of list) {
    if (p.parentDirName && !list.some(x => x.dirName === p.parentDirName)) {
      entries.push({ kind: 'solo', project: p })
    }
  }
  return entries
})

function pinColor(p: ProjectInfo): string {
  let h = 0
  const s = p.dirName
  for (let i = 0; i < s.length; i++) h = ((h << 5) - h + s.charCodeAt(i)) | 0
  const hue = ((h % 360) + 360) % 360
  return `hsl(${hue} 72% 52%)`
}

const selecting = ref(false)
const selectedDirs = ref(new Set<string>())
watch(() => props.agent, () => exitSelect())

function exitSelect() {
  selecting.value = false
  selectedDirs.value = new Set()
}

function toggleSelect(dir: string) {
  const next = new Set(selectedDirs.value)
  if (next.has(dir)) next.delete(dir)
  else next.add(dir)
  selectedDirs.value = next
  if (next.size === 0) selecting.value = false
}

const allSelected = computed(() =>
  props.projects.length > 0 && selectedDirs.value.size === props.projects.length,
)

const isMac = /Mac/i.test(navigator.platform)
const modHintDown = ref(false)
const modHintLabel = isMac ? '⌘' : 'Ctrl'

function shortcutForIndex(index: number) {
  return index < 9 ? `${modHintLabel}${isMac ? '' : '+'}${index + 1}` : ''
}

function digitFromEvent(e: KeyboardEvent): number | null {
  if (/^Digit[1-9]$/.test(e.code)) return Number(e.code.slice(5))
  if (/^Numpad[1-9]$/.test(e.code)) return Number(e.code.slice(6))
  const n = Number(e.key)
  return Number.isInteger(n) && n >= 1 && n <= 9 ? n : null
}

function isPlainModNumber(e: KeyboardEvent) {
  const mod = isMac ? e.metaKey : e.ctrlKey
  const otherMod = isMac ? e.ctrlKey : e.metaKey
  return mod && !otherMod && !e.shiftKey && !e.altKey
}

// 修饰键本身（⌘/Ctrl/⇧/⌥），用来区分「只按了修饰键」和「按下了实义键」。
const isModifierKey = (k: string) => k === 'Meta' || k === 'Control' || k === 'Shift' || k === 'Alt'

// 提示「延时显示、立即隐藏」：只有单独按住 ⌘/Ctrl 满 MOD_HINT_DELAY 才亮，其间一旦叠加实义键就取消——
// 这样敲组合键（⌘C、⌘⇧F…）不会闪一下再灭。隐藏永远即时，保证敲组合键时提示不抢戏。
// 阈值要大于「一前一后敲组合键」的自然间隔（几十~两三百 ms），否则先按 ⌘ 再按第二键仍会闪；
// 400ms 把「顺手敲的组合键」和「特意按住 ⌘ 查快捷键」区分开——真想查的人按住半秒即可。
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
  // 当前按下的是修饰键本身、且此刻恰好只按住 ⌘/Ctrl（无 ⇧/⌥/另一修饰键）→ 延时点亮；
  // 一旦叠加任何实义键（⌘C、⌘5、⌘⇧F…）立即收起——那已是组合键。
  if (isModifierKey(e.key) && isPlainModNumber(e)) showModHintSoon()
  else hideModHint()
  if (!isPlainModNumber(e)) return
  if (e.repeat) return
  const n = digitFromEvent(e)
  if (!n) return
  const entry = groupedEntries.value[n - 1]
  if (!entry) return
  e.preventDefault()
  emit('select-project', entry.project.dirName)
}

function onShortcutKeyup(e: KeyboardEvent) {
  // 松开任意键后按剩余修饰键状态重算：仍单独按住 ⌘/Ctrl 则重新延时点亮，否则即时收起。
  if (isPlainModNumber(e)) showModHintSoon()
  else hideModHint()
}

function onShortcutBlur() {
  hideModHint()
}

onMounted(() => {
  window.addEventListener('keydown', onShortcutKeydown, true)
  window.addEventListener('keyup', onShortcutKeyup, true)
  window.addEventListener('blur', onShortcutBlur)
})

onUnmounted(() => {
  clearModHintTimer()
  clearProjectDragState()
  window.removeEventListener('keydown', onShortcutKeydown, true)
  window.removeEventListener('keyup', onShortcutKeyup, true)
  window.removeEventListener('blur', onShortcutBlur)
})

function toggleSelectAll() {
  if (allSelected.value) {
    selectedDirs.value = new Set()
  } else {
    selectedDirs.value = new Set(props.projects.map(p => p.dirName))
  }
}

function onProjClick(p: ProjectInfo) {
  if (suppressProjectClick) return
  if (selecting.value) {
    toggleSelect(p.dirName)
    return
  }
  emit('select-project', p.dirName)
}

function onProjContextMenu(e: MouseEvent, p: ProjectInfo) {
  if (selecting.value) return
  emit('context-menu', e, p)
}

function doBatchDelete() {
  const dirs = [...selectedDirs.value]
  if (!dirs.length) return
  emit('batch-delete', dirs)
}

const draggingProject = ref<string | null>(null)
const dropTarget = ref<{ dirName: string; after: boolean } | null>(null)
const hoveredProject = ref<string | null>(null)
type ProjectDragPreview = {
  entry: SidebarEntry
  x: number
  y: number
  width: number
  offsetX: number
  offsetY: number
}
const projectDragPreview = ref<ProjectDragPreview | null>(null)
let projectDragZoom = 1
let pendingProjectDrag: {
  source: string
  sourceEl: HTMLElement
  entry: SidebarEntry
  startX: number
  startY: number
  active: boolean
} | null = null
let suppressProjectClick = false

function currentProjectDragZoom() {
  const zoom = parseFloat(getComputedStyle(document.body).zoom || '1')
  return Number.isFinite(zoom) && zoom > 0 ? zoom : 1
}

function canDragProject(entry: SidebarEntry) {
  return !selecting.value && entry.kind !== 'child'
}

function clearProjectDragState() {
  pendingProjectDrag = null
  draggingProject.value = null
  dropTarget.value = null
  projectDragPreview.value = null
  document.body.classList.remove('is-project-reordering')
  window.removeEventListener('pointermove', onProjectPointerMove)
  window.removeEventListener('pointerup', onProjectPointerUp)
  window.removeEventListener('pointercancel', onProjectPointerCancel)
}

function onProjectPointerDown(event: PointerEvent, entry: SidebarEntry) {
  if (event.button !== 0 || !canDragProject(entry)) return
  const sourceEl = (event.currentTarget as HTMLElement | null)?.closest<HTMLElement>('.proj-item')
  if (!sourceEl) return
  event.preventDefault()
  pendingProjectDrag = {
    source: entry.project.dirName,
    sourceEl,
    entry,
    startX: event.clientX,
    startY: event.clientY,
    active: false,
  }
  window.addEventListener('pointermove', onProjectPointerMove)
  window.addEventListener('pointerup', onProjectPointerUp)
  window.addEventListener('pointercancel', onProjectPointerCancel)
}

function updateProjectDropTargetFromPoint(x: number, y: number) {
  const source = pendingProjectDrag?.source
  const row = document.elementFromPoint(x, y)?.closest<HTMLElement>('.proj-item[data-project-dir]')
  const targetDir = row?.dataset.projectDir
  const dragged = source ? props.projects.find((project) => project.dirName === source) : undefined
  const target = targetDir ? props.projects.find((project) => project.dirName === targetDir) : undefined
  if (!row || !source || !targetDir || !target || source === targetDir || row.dataset.projectDraggable !== 'true'
    || !dragged || projectGroupRank(dragged) !== projectGroupRank(target)) {
    dropTarget.value = null
    return
  }
  const bounds = row.getBoundingClientRect()
  dropTarget.value = {
    dirName: targetDir,
    after: y >= bounds.top + bounds.height / 2,
  }
}

function moveProject(source: string, target: { dirName: string; after: boolean }) {
  if (source === target.dirName) return false

  const roots = groupedEntries.value
    .filter((item) => item.kind !== 'child')
    .map((item) => item.project)
  const sourceIndex = roots.findIndex((project) => project.dirName === source)
  const targetIndex = roots.findIndex((project) => project.dirName === target.dirName)
  if (sourceIndex < 0 || targetIndex < 0) return false

  const [dragged] = roots.splice(sourceIndex, 1)
  const adjustedTargetIndex = roots.findIndex((project) => project.dirName === target.dirName)
  roots.splice(adjustedTargetIndex + (target.after ? 1 : 0), 0, dragged)
  const order = roots.map((project) => project.dirName)
  const previous = groupedEntries.value
    .filter((item) => item.kind !== 'child')
    .map((item) => item.project.dirName)
  if (order.every((dirName, index) => dirName === previous[index])) return false
  emit('reorder-projects', order)
  return true
}

function onProjectPointerMove(event: PointerEvent) {
  const drag = pendingProjectDrag
  if (!drag) return
  if (!drag.active) {
    if (Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < 4) return
    drag.active = true
    draggingProject.value = drag.source
    const bounds = drag.sourceEl.getBoundingClientRect()
    projectDragZoom = currentProjectDragZoom()
    projectDragPreview.value = {
      entry: drag.entry,
      x: bounds.left / projectDragZoom,
      y: bounds.top / projectDragZoom,
      width: bounds.width / projectDragZoom,
      offsetX: drag.startX - bounds.left,
      offsetY: drag.startY - bounds.top,
    }
    document.body.classList.add('is-project-reordering')
  }
  event.preventDefault()
  if (projectDragPreview.value) {
    projectDragPreview.value.x = (event.clientX - projectDragPreview.value.offsetX) / projectDragZoom
    projectDragPreview.value.y = (event.clientY - projectDragPreview.value.offsetY) / projectDragZoom
  }
  updateProjectDropTargetFromPoint(event.clientX, event.clientY)
}

function onProjectPointerUp(event: PointerEvent) {
  const drag = pendingProjectDrag
  const wasDragging = drag?.active
  if (drag?.active && dropTarget.value) moveProject(drag.source, dropTarget.value)
  clearProjectDragState()
  if (wasDragging) {
    event.preventDefault()
    suppressProjectClick = true
    window.setTimeout(() => { suppressProjectClick = false }, 0)
  }
}

function onProjectPointerCancel() {
  clearProjectDragState()
}

defineExpose({ exitSelect })
</script>

<template>
  <aside
    class="sidebar"
  >
    <div class="sidebar-top">
      <div
        v-if="visibleAgents.length > 1"
        class="agent-switch"
        :class="{ 'icons-only': switcherIconsOnly }"
      >
        <button
          v-for="a in visibleAgents"
          :key="a"
          :class="{ active: agent === a }"
          v-tooltip="switcherIconsOnly ? agentLabel(a) : ''"
          @click="emit('switch-agent', a)"
        >
          <component :is="agentIcons[a]" />
          <span v-if="!switcherIconsOnly">{{ agentLabel(a) }}</span>
        </button>
      </div>
      <div class="sidebar-sub">
        <template v-if="selecting">
          <span class="sidebar-sub-label">{{ t('sidebar.selectedCount', { n: selectedDirs.size }) }}</span>
          <button
            type="button"
            class="sidebar-sub-btn"
            v-tooltip="allSelected ? t('list.tb.selectNone') : t('list.tb.selectAll')"
            @click="toggleSelectAll"
          >
            <IconCheck />
          </button>
          <button
            type="button"
            class="sidebar-sub-btn"
            v-tooltip="t('list.tb.selectCancel')"
            @click="exitSelect"
          >
            <IconClose />
          </button>
          <span class="sidebar-sub-divider" />
          <button
            type="button"
            class="sidebar-sub-btn danger"
            :disabled="!selectedDirs.size"
            v-tooltip="t('sidebar.batchDelete')"
            @click="doBatchDelete"
          >
            <IconTrash />
          </button>
        </template>
        <template v-else>
          <span class="sidebar-sub-label">
            {{ agentName }} ·
            {{ t('sidebar.projectsCount', { count: projects.length }) }}
          </span>
          <button
            type="button"
            class="sidebar-sub-btn"
            v-tooltip="t('sidebar.addFolder')"
            @click="emit('add-bookmark')"
          >
            <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor"><path d="M8 2a.75.75 0 0 1 .75.75v4.5h4.5a.75.75 0 0 1 0 1.5h-4.5v4.5a.75.75 0 0 1-1.5 0v-4.5h-4.5a.75.75 0 0 1 0-1.5h4.5v-4.5A.75.75 0 0 1 8 2Z"/></svg>
          </button>
          <button
            v-if="projects.length > 1"
            type="button"
            class="sidebar-sub-btn"
            v-tooltip="t('list.tb.select')"
            @click="selecting = true"
          >
            <IconSelect />
          </button>
          <button
            type="button"
            class="sidebar-sub-btn"
            :class="{ spinning: refreshing }"
            v-tooltip="t('sidebar.refresh')"
            :disabled="refreshing"
            @click="emit('refresh')"
          >
            <IconRefresh />
          </button>
        </template>
      </div>
    </div>

    <div class="proj-list">
      <template v-for="(entry, index) in groupedEntries" :key="entry.project.dirName">
        <div
          class="proj-item"
          :class="{
            active: activeDir === entry.project.dirName && !showTrash && !selecting,
            missing: !entry.project.exists,
            pinned: projStateOf(entry.project) === 'pinned',
            sunk: projStateOf(entry.project) === 'sunk',
            selected: selecting && selectedDirs.has(entry.project.dirName),
            'wt-child': entry.kind === 'child',
            dragging: draggingProject === entry.project.dirName,
            'drop-before': dropTarget?.dirName === entry.project.dirName && !dropTarget.after,
            'drop-after': dropTarget?.dirName === entry.project.dirName && dropTarget.after,
          }"
          :data-path="entry.project.displayPath"
          :data-project-dir="entry.project.dirName"
          :data-project-draggable="canDragProject(entry)"
          v-tooltip:right="entry.project.exists ? entry.project.displayPath : entry.project.displayPath + t('proj.missing')"
          @click="onProjClick(entry.project)"
          @contextmenu="onProjContextMenu($event, entry.project)"
          @pointerenter="hoveredProject = entry.project.dirName"
          @pointerleave="hoveredProject = null"
        >
          <span
            v-if="canDragProject(entry) && (hoveredProject === entry.project.dirName || draggingProject === entry.project.dirName)"
            class="proj-drag-handle"
            aria-hidden="true"
            @pointerdown="onProjectPointerDown($event, entry)"
          >
            <svg viewBox="0 0 24 24">
              <path d="M8 5a1 1 0 1 0 2 0a1 1 0 1 0-2 0m0 7a1 1 0 1 0 2 0a1 1 0 1 0-2 0m0 7a1 1 0 1 0 2 0a1 1 0 1 0-2 0m6-14a1 1 0 1 0 2 0a1 1 0 1 0-2 0m0 7a1 1 0 1 0 2 0a1 1 0 1 0-2 0m0 7a1 1 0 1 0 2 0a1 1 0 1 0-2 0" />
            </svg>
          </span>
          <span v-if="selecting" class="proj-check" :class="{ checked: selectedDirs.has(entry.project.dirName) }">
            <IconCheck v-if="selectedDirs.has(entry.project.dirName)" />
          </span>
          <span
            v-if="!selecting && projStateOf(entry.project) === 'pinned'"
            class="pin-dot"
            :style="{ background: pinColor(entry.project) }"
            :aria-label="t('proj.pin')"
          />
          <span
            v-if="entry.kind === 'parent' && !selecting"
            class="wt-toggle"
            @click.stop="toggleWorktreeCollapse(entry.project.dirName)"
          >
            <svg
              viewBox="0 0 16 16" width="12" height="12" fill="currentColor"
              :class="{ collapsed: collapsedWorktrees.has(entry.project.dirName) }"
            >
              <path d="M5.5 3.5L10.5 8 5.5 12.5" stroke="currentColor" stroke-width="1.5" fill="none" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
          </span>
          <IconGitBranch v-if="entry.kind === 'child'" class="wt-icon" />
          <span class="proj-name">{{
            entry.kind === 'child' ? entry.project.worktreeName ?? shortName(entry.project.displayPath) : shortName(entry.project.displayPath)
          }}</span>
          <span v-if="modHintDown && shortcutForIndex(index)" class="proj-shortcut">{{ shortcutForIndex(index) }}</span>
          <span v-else class="proj-count">{{ entry.project.sessionCount }}</span>
        </div>
      </template>
      <div v-if="!projects.length" class="sidebar-sub" style="padding: 12px">
        {{ t('sidebar.noSessions', { agent: agentName }) }}
      </div>
    </div>

    <div class="sidebar-footer">
      <button
        class="trash-tab"
        :class="{ 'has-update': updateAvailable }"
        v-tooltip="updateAvailable
          ? t('sidebar.updateAvailable', { v: latestVersion ?? '' })
          : t('sidebar.settings')"
        @click="emit('open-settings')"
      >
        <IconSettings /> {{ t('sidebar.settings') }}
        <!-- 有新版本时，行尾多挂一个"更新"入口按钮：点它直接跳到设置里的「更新」tab
             （不再直接跳 GitHub）。@click.stop 防止冒泡到外层 button 打开通用设置。 -->
        <span
          v-if="updateAvailable"
          class="sidebar-release-btn"
          role="button"
          tabindex="0"
          v-tooltip="t('sidebar.updateAvailable', { v: latestVersion ?? '' })"
          :aria-label="t('sidebar.updateAvailable', { v: latestVersion ?? '' })"
          @click.stop="emit('open-settings', 'updates')"
          @keydown.enter.stop.prevent="emit('open-settings', 'updates')"
          @keydown.space.stop.prevent="emit('open-settings', 'updates')"
        >
          <IconDownload />
        </span>
        <span v-if="updateAvailable" class="update-dot" aria-hidden="true" />
      </button>
    </div>

    <Teleport to="body">
      <div
        v-if="projectDragPreview"
        class="proj-item proj-item-drag-preview"
        :class="{
          active: activeDir === projectDragPreview.entry.project.dirName && !showTrash,
          missing: !projectDragPreview.entry.project.exists,
          pinned: projStateOf(projectDragPreview.entry.project) === 'pinned',
          sunk: projStateOf(projectDragPreview.entry.project) === 'sunk',
        }"
        :style="{
          left: projectDragPreview.x + 'px',
          top: projectDragPreview.y + 'px',
          width: projectDragPreview.width + 'px',
        }"
      >
        <span class="proj-drag-handle" aria-hidden="true">
          <svg viewBox="0 0 24 24">
            <path d="M8 5a1 1 0 1 0 2 0a1 1 0 1 0-2 0m0 7a1 1 0 1 0 2 0a1 1 0 1 0-2 0m0 7a1 1 0 1 0 2 0a1 1 0 1 0-2 0m6-14a1 1 0 1 0 2 0a1 1 0 1 0-2 0m0 7a1 1 0 1 0 2 0a1 1 0 1 0-2 0m0 7a1 1 0 1 0 2 0a1 1 0 1 0-2 0" />
          </svg>
        </span>
        <span
          v-if="projStateOf(projectDragPreview.entry.project) === 'pinned'"
          class="pin-dot"
          :style="{ background: pinColor(projectDragPreview.entry.project) }"
        />
        <span v-if="projectDragPreview.entry.kind === 'parent'" class="wt-toggle">
          <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor">
            <path d="M5.5 3.5L10.5 8 5.5 12.5" stroke="currentColor" stroke-width="1.5" fill="none" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
        </span>
        <span class="proj-name">{{ shortName(projectDragPreview.entry.project.displayPath) }}</span>
        <span class="proj-count">{{ projectDragPreview.entry.project.sessionCount }}</span>
      </div>
    </Teleport>
  </aside>
</template>
