<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { t } from '../../i18n'
import type { SessionMeta } from '../../types'
import {
  sessionSearch,
  sessionSort,
  sessionStarredOnly,
  toggleStarredOnly,
  type SessionSort,
} from '../../sessionsToolbar'
import { useDebouncedSearch } from '../../useDebouncedSearch'
import {
  IconSearch,
  IconClose,
  IconChevronDown,
  IconCheck,
  IconStar,
} from '../icons'

// `.ct-actions` 原本住在这里（仅 ID / 批量选择 / 批量导出 / 批量删除），现已挪到
// SessionsView 的 .list-head-actions 里，跟项目级动作 (新建/刷新/删除项目) 汇成
// 一排，避免顶栏 + body header 两层 icon-only 按钮在同一垂直线上视觉冲突。
defineProps<{ sessions: SessionMeta[] }>()

// 搜索框防抖：打字时 `draft` 立即跟着光标走，静止 220ms 后才同步到共享
// `sessionSearch`，避免每个按键都触发整张会话列表的 filter / 高亮重算。
// IME 组合中（中文 / 日文输入法）不会触发 —— 等 compositionend 才同步。
const {
  draft: searchDraft,
  commit: commitSearch,
  onInput: onSearchInput,
  onCompositionStart: onSearchCompStart,
  onCompositionEnd: onSearchCompEnd,
} = useDebouncedSearch(sessionSearch, 220)
const hasQuery = computed(() => searchDraft.value.length > 0)

function clearSearch() {
  commitSearch('')
}

// 排序下拉 —— 复用 .ct-scope-* 样式（同 ChatTopbar 的 scope / TrashTopbar 的项目筛选）。
const SORTS: { value: SessionSort; key: string }[] = [
  { value: 'recent', key: 'list.tb.sortRecent' },
  { value: 'oldest', key: 'list.tb.sortOldest' },
  { value: 'createdRecent', key: 'list.tb.sortCreatedRecent' },
  { value: 'createdOldest', key: 'list.tb.sortCreatedOldest' },
  { value: 'size', key: 'list.tb.sortSize' },
  { value: 'messages', key: 'list.tb.sortMessages' },
]
const sortMenuOpen = ref(false)
const sortMenuEl = ref<HTMLElement>()
const sortLabel = computed(() => {
  const found = SORTS.find((s) => s.value === sessionSort.value)
  return t(found?.key ?? 'list.tb.sortRecent')
})
function toggleSortMenu(e: Event) {
  e.stopPropagation()
  sortMenuOpen.value = !sortMenuOpen.value
}
function pickSort(s: SessionSort) {
  sessionSort.value = s
  sortMenuOpen.value = false
}
function onDocClick(e: MouseEvent) {
  if (!sortMenuOpen.value) return
  if (sortMenuEl.value && sortMenuEl.value.contains(e.target as Node)) return
  sortMenuOpen.value = false
}

// ⌘F / Ctrl+F：会话列表打开时拦截系统 Find，聚焦搜索框并全选。
// 只检测当前平台对应的修饰键，避免 macOS 上 Ctrl+F（光标右移）被误抢。
const searchInput = ref<HTMLInputElement>()
const isMac = /Mac/i.test(navigator.platform)
function onFindShortcut(e: KeyboardEvent) {
  if (e.key !== 'f' && e.key !== 'F') return
  const want = isMac ? e.metaKey : e.ctrlKey
  const other = isMac ? e.ctrlKey : e.metaKey
  if (!want || other || e.shiftKey || e.altKey) return
  e.preventDefault()
  searchInput.value?.focus()
  searchInput.value?.select()
}

onMounted(() => {
  document.addEventListener('click', onDocClick)
  window.addEventListener('keydown', onFindShortcut)
})
onUnmounted(() => {
  document.removeEventListener('click', onDocClick)
  window.removeEventListener('keydown', onFindShortcut)
})
</script>

<template>
  <div class="chat-topbar">
    <div class="ct-search" :class="{ active: hasQuery }">
      <div ref="sortMenuEl" class="ct-scope-wrap">
        <button
          type="button"
          class="ct-scope-btn"
          :class="{ active: sortMenuOpen }"
          v-tooltip:right="t('list.tb.sort')"
          @click="toggleSortMenu"
        >
          <span class="ct-scope-label">{{ sortLabel }}</span>
          <IconChevronDown class="ct-scope-chev" />
        </button>
        <div v-if="sortMenuOpen" class="ct-scope-menu" role="menu">
          <button
            v-for="s in SORTS"
            :key="s.value"
            type="button"
            class="ct-scope-item"
            :class="{ active: sessionSort === s.value }"
            role="menuitemradio"
            :aria-checked="sessionSort === s.value"
            @click="pickSort(s.value)"
          >
            <span class="ct-scope-check">
              <IconCheck v-if="sessionSort === s.value" />
            </span>
            <span>{{ t(s.key) }}</span>
          </button>
        </div>
      </div>
      <span class="ct-search-ic"><IconSearch /></span>
      <input
        ref="searchInput"
        :value="searchDraft"
        type="text"
        class="ct-search-input"
        :placeholder="t('list.tb.searchPlaceholder')"
        spellcheck="false"
        autocomplete="off"
        @input="onSearchInput"
        @compositionstart="onSearchCompStart"
        @compositionend="onSearchCompEnd"
      />
      <button
        v-if="hasQuery"
        class="ct-btn"
        v-tooltip="t('chat.tb.search.clear')"
        @click="clearSearch"
      >
        <IconClose />
      </button>
    </div>
    <button
      type="button"
      class="ct-btn session-star-filter"
      :class="{ active: sessionStarredOnly }"
      :aria-label="t('list.tb.starredOnly')"
      :aria-pressed="sessionStarredOnly"
      v-tooltip="t('list.tb.starredOnly')"
      @click="toggleStarredOnly"
    >
      <IconStar aria-hidden="true" />
      <span>{{ t('list.tb.starredOnly') }}</span>
    </button>
  </div>
</template>
