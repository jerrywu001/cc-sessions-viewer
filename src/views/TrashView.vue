<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue'
import type { TrashItem } from '../types'
import { formatSize, formatTime, highlightSegments, shortName } from '../format'
import { t } from '../i18n'
import { agentLabel } from '../agentMeta'
import {
  IconTrashOpen,
  IconDeleteLine,
  IconTrash,
  IconRestore,
  IconInbox,
  IconCheck,
  IconSort,
  IconSelect,
  IconClose,
} from '../components/icons'
import {
  filterTrash,
  selectMode,
  selectedTrash,
  toggleTrashSelected,
  trashSearch,
  trashSort,
  exitSelectMode,
} from '../trashToolbar'

const props = defineProps<{
  trash: TrashItem[]
  loading: boolean
}>()

const emit = defineEmits<{
  (e: 'clear'): void
  (e: 'open', item: TrashItem): void
  (e: 'restore', item: TrashItem): void
  (e: 'permanent-delete', item: TrashItem): void
  /** 批量恢复：原本由 TrashTopbar 触发，现已挪到 list-head 顶栏里。 */
  (e: 'batch-restore'): void
  (e: 'batch-permanent-delete'): void
}>()

// 搜索 / 项目筛选 / 时间排序后的可见列表 —— 工具栏状态来自 trashToolbar 模块。
const visibleTrash = computed(() => filterTrash(props.trash))

// ---------- list-head 的批量选择 UI（原本住在 TrashTopbar，挪过来减少
// "topbar + list-head 两排 icon-only 按钮重叠" 的扫描负担）。
function toggleTrashSort() {
  trashSort.value = trashSort.value === 'recent' ? 'oldest' : 'recent'
}
const headSelectedCount = computed(
  () => props.trash.filter((it) => selectedTrash.value.has(it.trashFile)).length,
)
const headAllSelected = computed(
  () =>
    visibleTrash.value.length > 0 &&
    visibleTrash.value.every((it) => selectedTrash.value.has(it.trashFile)),
)
function headToggleSelectAll() {
  const next = new Set(selectedTrash.value)
  for (const it of visibleTrash.value) {
    if (headAllSelected.value) next.delete(it.trashFile)
    else next.add(it.trashFile)
  }
  selectedTrash.value = next
}

// 批量模式下点整张卡片即勾选；否则打开该会话的只读详情。
function onCardClick(item: TrashItem) {
  if (selectMode.value) toggleTrashSelected(item.trashFile)
  else emit('open', item)
}

// 搜索时把标题 / 项目名里命中的关键词切成高亮片段（命中段加 .kw-hit）。
// filterTrash 用 title + projectLabel 匹配，故两处都做高亮。
function titleSegs(title: string) {
  return highlightSegments(title, trashSearch.value)
}
function projSegs(projectLabel: string) {
  return highlightSegments(shortName(projectLabel), trashSearch.value)
}
// hover 跟随浮块：与会话列表一致的滑块交互。鼠标移到某张卡片上，把它的
// offsetTop / offsetHeight 写进 --spot-y / --spot-h 驱动 .list-spotlight；
// 滚动期间临时隐藏，停止 140ms 后恢复，避免内容在静止光标下移动时抖动。
const scrollEl = ref<HTMLElement>()
const spotlightEl = ref<HTMLElement>()
let scrolling = false
let scrollIdle = 0
function markScrolling() {
  if (!scrolling) {
    scrolling = true
    scrollEl.value?.classList.remove('has-spot')
  }
  clearTimeout(scrollIdle)
  scrollIdle = window.setTimeout(() => {
    scrolling = false
  }, 140)
}
function onListMouseOver(e: MouseEvent) {
  if (scrolling) return
  const sa = scrollEl.value
  const sp = spotlightEl.value
  if (!sa || !sp) return
  const card = (e.target as HTMLElement | null)?.closest<HTMLElement>('.session-card')
  if (!card || !sa.contains(card)) return
  // 从隐藏态重新出现时先 no-slide 直接跳到目标行再淡入，避免整屏滑动的突兀感。
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
}
onUnmounted(() => clearTimeout(scrollIdle))
</script>

<template>
  <div class="list-head list-head-row">
    <div class="grow">
      <h2>{{ t('trash.title') }}</h2>
      <div class="path">{{ t('trash.subtitle') }}</div>
    </div>
    <div class="list-head-actions">
      <template v-if="selectMode">
        <span class="ct-search-count">{{
          t('trash.tb.selectedCount', { n: headSelectedCount })
        }}</span>
        <!-- 选择控制：select-all + 取消选择，跟下面的"对选中项的动作"用细竖线分开。 -->
        <button
          class="icon-btn"
          :class="{ active: headAllSelected }"
          v-tooltip="headAllSelected ? t('trash.tb.selectNone') : t('trash.tb.selectAll')"
          @click="headToggleSelectAll"
        >
          <IconCheck />
        </button>
        <button
          class="icon-btn"
          v-tooltip="t('trash.tb.selectCancel')"
          @click="exitSelectMode"
        >
          <IconClose />
        </button>
        <span class="list-head-divider" aria-hidden="true" />
        <button
          class="icon-btn"
          :disabled="headSelectedCount === 0"
          v-tooltip="t('trash.tb.restoreSelected')"
          @click="emit('batch-restore')"
        >
          <IconRestore />
        </button>
        <button
          class="icon-btn danger"
          :disabled="headSelectedCount === 0"
          v-tooltip="t('trash.tb.deleteSelected')"
          @click="emit('batch-permanent-delete')"
        >
          <IconDeleteLine />
        </button>
      </template>
      <template v-else>
        <!-- 排序 / 进入批量模式 —— 原本住在 TrashTopbar 的 .ct-actions 里；
             与下方 Empty Trash 隔了一层 topbar，两行 icon-only 控件视觉冲突。
             挪到这里后顶栏只剩 项目筛选 + 搜索 一条横线。 -->
        <button
          v-if="trash.length > 1"
          class="icon-btn"
          v-tooltip="
            trashSort === 'recent'
              ? t('trash.tb.sortRecent')
              : t('trash.tb.sortOldest')
          "
          @click="toggleTrashSort"
        >
          <IconSort />
        </button>
        <button
          v-if="trash.length > 1"
          class="icon-btn"
          v-tooltip="t('trash.tb.select')"
          @click="selectMode = true"
        >
          <IconSelect />
        </button>
        <button class="btn danger" :disabled="!trash.length" @click="emit('clear')">
          {{ t('trash.clearAll') }}
        </button>
      </template>
    </div>
  </div>
  <div v-if="loading" class="loading">{{ t('common.loading') }}</div>
  <div v-else-if="!trash.length" class="empty">
    <div class="big"><IconTrashOpen /></div>
    <div>{{ t('trash.empty') }}</div>
  </div>
  <div v-else-if="!visibleTrash.length" class="empty">
    <div class="big"><IconInbox /></div>
    <div>{{ t('trash.noMatch') }}</div>
  </div>
  <div
    v-else
    ref="scrollEl"
    class="scroll-area"
    @scroll.passive="markScrolling"
    @mouseover.passive="onListMouseOver"
    @mouseleave.passive="onListMouseLeave"
  >
    <div class="vlist">
      <div ref="spotlightEl" class="list-spotlight" aria-hidden="true" />
      <div
        v-for="item in visibleTrash"
        :key="item.trashFile"
        class="session-card"
        :data-trash="item.trashFile"
        :class="{
          'list-selectable': selectMode,
          'list-selected': selectMode && selectedTrash.has(item.trashFile),
        }"
        @click="onCardClick(item)"
      >
        <span
          v-if="selectMode"
          class="list-check"
          :class="{ on: selectedTrash.has(item.trashFile) }"
          aria-hidden="true"
        >
          <IconCheck v-if="selectedTrash.has(item.trashFile)" />
        </span>
        <div class="session-main">
          <div class="session-title">
            <span class="agent-badge" :class="item.agent">{{ agentLabel(item.agent) }}</span>
            <span><span
              v-for="(seg, i) in titleSegs(item.title)"
              :key="i"
              :class="{ 'kw-hit': seg.hit }"
            >{{ seg.text }}</span></span>
          </div>
          <div class="session-meta">
            <span v-if="!shortName(item.projectLabel)">—</span>
            <span v-else><span
              v-for="(seg, i) in projSegs(item.projectLabel)"
              :key="i"
              :class="{ 'kw-hit': seg.hit }"
            >{{ seg.text }}</span></span>
            <span>{{ formatSize(item.size) }}</span>
            <span>{{
              t('trash.deletedAt', { time: formatTime(item.deletedAt) })
            }}</span>
          </div>
        </div>
        <div v-if="!selectMode" class="session-actions" style="opacity: 1">
          <button
            class="icon-btn"
            v-tooltip="t('trash.restore')"
            @click.stop="emit('restore', item)"
          >
            <IconRestore />
          </button>
          <button
            class="icon-btn danger"
            v-tooltip="t('trash.permDelete')"
            @click.stop="emit('permanent-delete', item)"
          >
            <IconTrash />
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
