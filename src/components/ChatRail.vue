<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { t } from '../i18n'

/**
 * 会话左侧「快速用户消息导航」导轨（参考 Codex 官方客户端的 turn rail）。
 *
 * 纯展示组件：条目由父组件 ChatView 用它既有的「用户提问」口径（promptEntries）
 * 算好传进来，点击只 `emit('jump', idx, uuid)` —— 真正的滚动交给父组件的
 * `flashMessage`（内部 rowVirtualizer.scrollToIndex + rAF 反复校准，已扛住
 * Shiki/图片异步撑高）。父组件把「滚动位置 → 当前用户消息下标」的 spy 结果
 * 推进 `activeIndex`；本组件据此高亮，并把它滚回可视范围（如果它在列表外）。
 */

export interface RailEntry {
  idx: number
  seq: number
  uuid?: string
  text: string
  summary: string
}

const props = defineProps<{
  entries: RailEntry[]
  /** 当前滚动位置对应的用户提问下标（父组件 onScroll 里做 spy 后传入）。 */
  activeIndex: number | null
}>()

const emit = defineEmits<{
  /** 点击一条 → 父组件 `flashMessage(idx, uuid)`。 */
  jump: [index: number, uuid?: string]
}>()

/** 可视列表元素 —— active 高亮滚进视野用。 */
const listEl = ref<HTMLElement>()
const railEl = ref<HTMLElement>()
function scrollActiveIntoView() {
  const el = listEl.value
  if (!el || props.activeIndex == null) return
  const item = el.querySelector<HTMLElement>(`[data-rail-idx="${props.activeIndex}"]`)
  item?.scrollIntoView({ block: 'nearest' })
}
// activeIndex 变化 → 高亮自动跟随（watch 在渲染后跑，行已存在）+ 滚进视野。
watch(
  () => props.activeIndex,
  () => {
    if (props.activeIndex != null) scrollActiveIntoView()
  },
  { flush: 'post' },
)

const active = computed(() =>
  props.entries.find((e) => e.idx === props.activeIndex)?.idx ?? null,
)

const hoveredIndex = ref<number | null>(null)
const focusedIndex = ref<number | null>(null)
const previewY = ref(0)

const previewEntry = computed(() => {
  const index = hoveredIndex.value ?? focusedIndex.value
  return props.entries.find((entry) => entry.idx === index) ?? null
})

function setPreviewPosition(item: HTMLElement) {
  const rail = railEl.value
  if (!rail) return
  const itemRect = item.getBoundingClientRect()
  const railRect = rail.getBoundingClientRect()
  previewY.value = itemRect.top - railRect.top + itemRect.height / 2
}

function onPointerMove(event: PointerEvent) {
  const list = listEl.value
  if (!list) return
  const listRect = list.getBoundingClientRect()
  const items = [...list.querySelectorAll<HTMLElement>('.chat-rail-item')]
  let closest: HTMLElement | null = null
  let closestDistance = Number.POSITIVE_INFINITY
  for (const item of items) {
    const rect = item.getBoundingClientRect()
    // A short viewport makes the rail internally scrollable. Ignore rows that
    // have already left that viewport, otherwise an off-screen row can keep
    // the hover wave alive after the list scrolls.
    if (listRect.height > 0 && (rect.bottom <= listRect.top || rect.top >= listRect.bottom)) continue
    const distance = Math.abs(event.clientY - (rect.top + rect.height / 2))
    if (distance < closestDistance) {
      closest = item
      closestDistance = distance
    }
  }
  if (closest) {
    hoveredIndex.value = Number(closest.dataset.railIdx)
    setPreviewPosition(closest)
  }
}

function clearPointerHover() {
  hoveredIndex.value = null
}

function onItemFocus(index: number, event: FocusEvent) {
  focusedIndex.value = index
  setPreviewPosition(event.currentTarget as HTMLElement)
}

function clearItemFocus() {
  focusedIndex.value = null
}

/** Nearby ticks retain part of the hover width so movement through the rail reads as a small wave. */
function waveFor(index: number) {
  if (hoveredIndex.value == null) return 0
  const center = props.entries.findIndex((entry) => entry.idx === hoveredIndex.value)
  const current = props.entries.findIndex((entry) => entry.idx === index)
  if (center < 0 || current < 0) return 0
  return Math.max(0, 1 - Math.abs(center - current) / 3) ** 2
}

function dotWidthFor(index: number) {
  if (index === active.value) return '25px'
  return `${5 + waveFor(index) * 20}px`
}
</script>

<template>
  <aside
    ref="railEl"
    v-if="entries.length"
    class="chat-rail"
    :aria-label="t('chat.rail.label')"
    @pointermove="onPointerMove"
    @pointerleave="clearPointerHover"
  >
    <!-- 默认可见的窄导轨：一轮一个刻度。悬浮或键盘聚焦某刻度时，仅展开该轮的用户消息。 -->
    <div ref="listEl" class="chat-rail-list" @scroll="clearPointerHover">
      <button
        v-for="e in entries"
        :key="e.uuid ?? e.idx"
        type="button"
        class="chat-rail-item"
        :class="{ active: e.idx === active, hovered: e.idx === hoveredIndex }"
        :style="{ '--rail-dot-width': dotWidthFor(e.idx) }"
        :data-rail-idx="e.idx"
        :aria-label="`#${e.seq}: ${e.text}`"
        @focus="onItemFocus(e.idx, $event)"
        @blur="clearItemFocus"
        @click="emit('jump', e.idx, e.uuid)"
      >
        <span class="chat-rail-dot" aria-hidden="true" />
      </button>
    </div>
    <button
      v-if="previewEntry"
      type="button"
      class="chat-rail-preview"
      :style="{ '--rail-preview-y': `${previewY}px` }"
      @click="emit('jump', previewEntry.idx, previewEntry.uuid)"
    >
      <span class="chat-rail-preview-title">{{ previewEntry.text }}</span>
      <span v-if="previewEntry.summary" class="chat-rail-preview-summary">{{ previewEntry.summary }}</span>
    </button>
  </aside>
</template>
