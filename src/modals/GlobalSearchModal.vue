<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import type { Agent, SearchField, SearchHit, SearchScope, SessionMeta } from '../types'
import { searchSessions, cancelSearch, nextSearchRequestId } from '../api'
import { t } from '../i18n'
import { shortName, highlightSegments } from '../format'
import { agentLabel } from '../agentMeta'
import {
  recentSearches,
  pushRecent,
  clearRecents,
  removeRecent,
} from '../globalSearch'
import {
  IconSearch,
  IconClose,
  IconCornerDownLeft,
  IconArrowUp,
  IconArrowDown,
  IconHistory,
} from '../components/icons'

const props = defineProps<{
  show: boolean
  agent: Agent
}>()

const emit = defineEmits<{
  (e: 'update:show', v: boolean): void
  (e: 'open', hit: SearchHit): void
}>()

const inputEl = ref<HTMLInputElement>()
const listEl = ref<HTMLElement>()
const query = ref('')
const hits = ref<SearchHit[]>([])
const searching = ref(false)
const selectedIdx = ref(0)
const scope = ref<SearchScope>('keyword')

const SCOPE_OPTIONS: { v: SearchScope; key: string }[] = [
  { v: 'keyword', key: 'search.global.scope.keyword' },
  { v: 'id', key: 'search.global.scope.id' },
]

const DEBOUNCE_MS = 350
const MIN_QUERY_LEN = 2
const RENDER_CAP = 80

let debounceTimer = 0
let reqSeq = 0
let inFlight = false
let composing = false

function setInputValue(val: string) {
  query.value = val
  if (inputEl.value) inputEl.value.value = val
}

watch(
  () => props.show,
  (v) => {
    if (v) {
      query.value = ''
      hits.value = []
      selectedIdx.value = 0
      searching.value = false
      composing = false
      window.clearTimeout(debounceTimer)
      nextTick(() => {
        if (inputEl.value) inputEl.value.value = ''
        inputEl.value?.focus()
      })
    }
  },
)

// 切换范围时重跑当前查询（与重新输入一致：打断在跑请求 + 防抖）。
watch(scope, () => {
  selectedIdx.value = 0
  scheduleSearch()
})

function scheduleSearch() {
  window.clearTimeout(debounceTimer)

  const trimmed = query.value.trim()
  if (trimmed.length < MIN_QUERY_LEN) {
    hits.value = []
    searching.value = false
    reqSeq++
    return
  }

  // Immediately: show spinner + invalidate any in-flight result
  searching.value = true
  reqSeq++

  debounceTimer = window.setTimeout(async () => {
    if (inFlight) {
      inFlight = false
      cancelSearch().catch(() => {})
    }

    const seq = ++reqSeq
    const reqId = nextSearchRequestId()
    inFlight = true
    try {
      const res = await searchSessions(props.agent, trimmed, reqId, undefined, scope.value)
      if (seq !== reqSeq) return
      hits.value = res
    } catch {
      if (seq !== reqSeq) return
      hits.value = []
    } finally {
      if (seq === reqSeq) {
        inFlight = false
        searching.value = false
      }
    }
  }, DEBOUNCE_MS)
}

function onInput(e: Event) {
  if (composing) return
  query.value = (e.target as HTMLInputElement).value
  selectedIdx.value = 0
  scheduleSearch()
}

function onCompositionStart() {
  composing = true
}

function onCompositionEnd(e: Event) {
  composing = false
  query.value = (e.target as HTMLInputElement).value
  selectedIdx.value = 0
  scheduleSearch()
}

const renderedHits = computed(() => hits.value.slice(0, RENDER_CAP))
const moreHidden = computed(() => Math.max(0, hits.value.length - RENDER_CAP))

type Group = { project: string; items: SearchHit[] }
const groups = computed<Group[]>(() => {
  const out: Group[] = []
  const map = new Map<string, Group>()
  for (const h of renderedHits.value) {
    let g = map.get(h.projectDisplay)
    if (!g) {
      g = { project: h.projectDisplay, items: [] }
      map.set(h.projectDisplay, g)
      out.push(g)
    }
    g.items.push(h)
  }
  return out
})

const flatHits = computed(() => groups.value.flatMap((g) => g.items))

const hasQuery = computed(() => query.value.trim().length >= MIN_QUERY_LEN)
const showEmpty = computed(() => hasQuery.value && !searching.value && !hits.value.length)
const showResults = computed(() => hasQuery.value && hits.value.length > 0)
const showRecent = computed(() => !hasQuery.value && recentSearches.value.length > 0)
const showNoRecent = computed(() => !hasQuery.value && !recentSearches.value.length)

function stopSearch() {
  window.clearTimeout(debounceTimer)
  if (inFlight) {
    inFlight = false
    cancelSearch().catch(() => {})
  }
}

function close() {
  stopSearch()
  emit('update:show', false)
}

function chooseHit(hit: SearchHit) {
  pushRecent(query.value)
  close()
  emit('open', hit)
}

function onSelect() {
  if (searching.value && !hits.value.length) return
  const hit = flatHits.value[selectedIdx.value]
  if (hit) chooseHit(hit)
}

function moveSelection(delta: number) {
  const n = flatHits.value.length
  if (!n) return
  selectedIdx.value = (selectedIdx.value + delta + n) % n
  nextTick(() => {
    const el = listEl.value?.querySelector<HTMLElement>(`.gs-row[data-idx="${selectedIdx.value}"]`)
    el?.scrollIntoView?.({ block: 'nearest' })
  })
}

function onKeydown(e: KeyboardEvent) {
  if (!props.show) return
  switch (e.key) {
    case 'Escape':
      e.preventDefault()
      close()
      break
    case 'ArrowDown':
      e.preventDefault()
      moveSelection(1)
      break
    case 'ArrowUp':
      e.preventDefault()
      moveSelection(-1)
      break
    case 'Enter':
      e.preventDefault()
      onSelect()
      break
  }
}

function pickRecent(r: string) {
  setInputValue(r)
  selectedIdx.value = 0
  scheduleSearch()
  nextTick(() => inputEl.value?.focus())
}

function fieldLabel(f: SearchField): string {
  return t(`search.global.field.${f}`)
}

function indexOf(hit: SearchHit): number {
  return flatHits.value.indexOf(hit)
}

function segs(text: string) {
  return highlightSegments(text, query.value)
}

function sessionLabel(s: SessionMeta): string {
  return s.title || (s.id ? s.id.slice(0, 8) : '—')
}

function clearInput() {
  setInputValue('')
  hits.value = []
  searching.value = false
  stopSearch()
  nextTick(() => inputEl.value?.focus())
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onUnmounted(() => {
  window.removeEventListener('keydown', onKeydown)
  stopSearch()
})
</script>

<template>
  <Transition name="gs-fade">
    <div v-if="show" class="gs-backdrop" @click.self="close">
      <div class="gs-modal" role="dialog" aria-modal="true">
        <!-- Search input -->
        <div class="gs-header">
          <div class="gs-search-icon">
            <svg v-if="searching" class="gs-spinner" viewBox="0 0 24 24" fill="none">
              <circle cx="12" cy="12" r="10" stroke="currentColor" stroke-width="2.5" opacity="0.2" />
              <path d="M12 2a10 10 0 0 1 10 10" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" />
            </svg>
            <IconSearch v-else />
          </div>
          <input
            ref="inputEl"
            type="text"
            class="gs-input"
            :placeholder="
              scope === 'id'
                ? t('search.global.placeholderId', { agent: agentLabel(agent) })
                : t('search.global.placeholder', { agent: agentLabel(agent) })
            "
            spellcheck="false"
            autocomplete="off"
            @input="onInput"
            @compositionstart="onCompositionStart"
            @compositionend="onCompositionEnd"
          />
          <div class="gs-scope" role="group" :aria-label="t('search.global.scope.label')">
            <button
              v-for="o in SCOPE_OPTIONS"
              :key="o.v"
              type="button"
              class="gs-scope-btn"
              :class="{ active: scope === o.v }"
              :aria-pressed="scope === o.v"
              @click="scope = o.v"
            >
              {{ t(o.key) }}
            </button>
          </div>
          <button v-if="query" class="gs-clear-btn" @click="clearInput">
            <IconClose />
          </button>
        </div>

        <!-- Body -->
        <div ref="listEl" class="gs-body">
          <!-- No query: recent searches -->
          <template v-if="showNoRecent">
            <div class="gs-placeholder">
              <p class="gs-placeholder-text">{{ t('search.global.empty') }}</p>
              <p class="gs-placeholder-hint">{{ t('search.global.emptyHint', { agent: agentLabel(agent) }) }}</p>
            </div>
          </template>

          <template v-else-if="showRecent">
            <div class="gs-section-header">
              <span>{{ t('search.global.recent') }}</span>
              <button class="gs-section-action" @click="clearRecents">
                {{ t('search.global.clearRecent') }}
              </button>
            </div>
            <div
              v-for="r in recentSearches"
              :key="r"
              class="gs-recent-item"
              role="button"
              tabindex="0"
              @click="pickRecent(r)"
              @keydown.enter.prevent="pickRecent(r)"
            >
              <IconHistory class="gs-recent-icon" />
              <span class="gs-recent-label">{{ r }}</span>
              <button
                class="gs-recent-del"
                v-tooltip="t('search.global.removeRecent')"
                :aria-label="t('search.global.removeRecent')"
                @click.stop="removeRecent(r)"
              >
                <IconClose />
              </button>
            </div>
          </template>

          <!-- Has query: results / empty -->
          <template v-else-if="showEmpty">
            <div class="gs-placeholder">
              <svg class="gs-no-results-icon" viewBox="0 0 40 40" fill="none">
                <circle cx="20" cy="20" r="16" stroke="currentColor" stroke-width="2" />
                <path d="M12 28 28 12" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
              </svg>
              <p class="gs-placeholder-text">
                {{ t('search.global.noMatch') }}
                "<strong>{{ query.trim() }}</strong>"
              </p>
            </div>
          </template>

          <template v-else-if="showResults">
            <div v-for="g in groups" :key="g.project" class="gs-group">
              <div class="gs-group-label">{{ shortName(g.project) }}</div>
              <button
                v-for="h in g.items"
                :key="h.session.path"
                class="gs-row"
                :class="{ active: selectedIdx === indexOf(h) }"
                :data-idx="indexOf(h)"
                @click="chooseHit(h)"
                @mouseenter="selectedIdx = indexOf(h)"
              >
                <svg class="gs-row-icon" viewBox="0 0 20 20" fill="none">
                  <path v-if="h.matchedField === 'text'" d="M4 6h12M4 10h8M4 14h10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
                  <path v-else d="M4 3h8l4 4v10a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V3z" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
                <div class="gs-row-content">
                  <span class="gs-row-title">
                    <span v-for="(seg, i) in segs(sessionLabel(h.session))" :key="i" :class="{ 'gs-hl': seg.hit }">{{ seg.text }}</span>
                  </span>
                  <span v-if="h.matchedField === 'text' || h.matchedField === 'path' || h.matchedField === 'id'" class="gs-row-snippet">
                    <span v-for="(seg, i) in segs(h.snippet)" :key="i" :class="{ 'gs-hl': seg.hit }">{{ seg.text }}</span>
                  </span>
                </div>
                <span class="gs-row-badge">{{ fieldLabel(h.matchedField) }}</span>
                <svg v-if="selectedIdx === indexOf(h)" class="gs-row-enter" viewBox="0 0 20 20" fill="none">
                  <path d="M15 4v6a2 2 0 0 1-2 2H5m0 0 3-3m-3 3 3 3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
              </button>
            </div>
            <div v-if="moreHidden > 0" class="gs-more-hint">
              {{ t('search.global.moreHidden', { n: moreHidden }) }}
            </div>
          </template>

          <!-- Searching with no results yet: just the spinner in the header is enough -->
        </div>

        <!-- Footer keyboard hints -->
        <div class="gs-footer">
          <span class="gs-hint">
            <kbd class="gs-kbd"><IconCornerDownLeft /></kbd>
            {{ t('search.global.hint.select') }}
          </span>
          <span class="gs-hint">
            <kbd class="gs-kbd"><IconArrowDown /></kbd>
            <kbd class="gs-kbd"><IconArrowUp /></kbd>
            {{ t('search.global.hint.navigate') }}
          </span>
          <span class="gs-hint">
            <kbd class="gs-kbd gs-kbd-text">esc</kbd>
            {{ t('search.global.hint.close') }}
          </span>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
/* ---- Backdrop ---- */
.gs-backdrop {
  position: fixed;
  inset: 0;
  z-index: 80;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding-top: 12vh;
  background: rgba(0, 0, 0, 0.35);
}
:root.theme-dark .gs-backdrop {
  background: rgba(0, 0, 0, 0.55);
}

/* ---- Modal ---- */
.gs-modal {
  width: min(620px, calc(100vw - 32px));
  max-height: 70vh;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 12px;
  box-shadow: var(--shadow-lg);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

/* ---- Header (input) ---- */
.gs-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 16px;
  height: 56px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}

.gs-search-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  color: var(--text-mute);
  flex-shrink: 0;
}
.gs-search-icon :deep(svg) {
  width: 18px;
  height: 18px;
}

.gs-spinner {
  width: 18px;
  height: 18px;
  animation: gs-spin 0.7s linear infinite;
}
@keyframes gs-spin {
  to { transform: rotate(360deg); }
}

.gs-input {
  flex: 1;
  background: transparent;
  border: none;
  outline: none;
  font-size: 16px;
  color: var(--text);
  height: 100%;
  min-width: 0;
}
.gs-input::placeholder {
  color: var(--text-mute);
}

.gs-clear-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: 6px;
  color: var(--text-mute);
  flex-shrink: 0;
  transition: background 0.12s, color 0.12s;
}
.gs-clear-btn:hover {
  background: var(--surface-hover);
  color: var(--text);
}
.gs-clear-btn :deep(svg) {
  width: 14px;
  height: 14px;
}

/* ---- Scope segmented control ---- */
.gs-scope {
  display: inline-flex;
  border: 1px solid var(--border);
  border-radius: 6px;
  overflow: hidden;
  flex-shrink: 0;
}
.gs-scope-btn {
  appearance: none;
  background: transparent;
  border: 0;
  border-right: 1px solid var(--border);
  padding: 3px 10px;
  color: var(--text-dim);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
  transition: background 0.12s, color 0.12s;
}
.gs-scope-btn:last-child {
  border-right: 0;
}
.gs-scope-btn:hover {
  background: var(--surface-hover);
}
.gs-scope-btn.active {
  background: var(--surface-hover);
  color: var(--text);
  font-weight: 600;
}

/* ---- Body ---- */
.gs-body {
  flex: 1 1 auto;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 8px 0;
  min-height: 80px;
}

/* ---- Placeholder (empty state) ---- */
.gs-placeholder {
  padding: 36px 20px;
  text-align: center;
}
.gs-no-results-icon {
  width: 40px;
  height: 40px;
  color: var(--text-mute);
  margin-bottom: 12px;
  opacity: 0.6;
}
.gs-placeholder-text {
  font-size: 13px;
  font-weight: 400;
  color: var(--text-dim);
}
.gs-placeholder-text strong {
  font-weight: 600;
  color: var(--text);
}
.gs-placeholder-hint {
  margin-top: 6px;
  font-size: 12px;
  color: var(--text-mute);
}

/* ---- Section header (recent) ---- */
.gs-section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 16px 4px;
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-mute);
}
.gs-section-action {
  font-size: 11px;
  font-weight: 400;
  color: var(--text-mute);
  padding: 2px 6px;
  border-radius: 4px;
  text-transform: none;
  letter-spacing: normal;
  transition: background 0.12s, color 0.12s;
}
.gs-section-action:hover {
  background: var(--surface-hover);
  color: var(--text-dim);
}

/* ---- Recent items ---- */
.gs-recent-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 16px;
  font-size: 13px;
  color: var(--text-dim);
  cursor: pointer;
  transition: background 0.1s;
}
.gs-recent-item:hover {
  background: var(--surface-hover);
  color: var(--text);
}
.gs-recent-icon {
  width: 14px;
  height: 14px;
  color: var(--text-mute);
  flex-shrink: 0;
}
.gs-recent-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.gs-recent-del {
  width: 20px;
  height: 20px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 4px;
  color: var(--text-mute);
  opacity: 0;
  flex-shrink: 0;
  transition: opacity 0.1s, background 0.1s, color 0.1s;
}
.gs-recent-del :deep(svg) {
  width: 12px;
  height: 12px;
}
.gs-recent-item:hover .gs-recent-del {
  opacity: 1;
}
.gs-recent-del:hover {
  background: var(--surface-active);
  color: var(--text);
}

/* ---- Results ---- */
.gs-group + .gs-group {
  margin-top: 4px;
}
.gs-group-label {
  padding: 10px 16px 4px;
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-mute);
}
.gs-row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 8px 16px;
  text-align: left;
  cursor: pointer;
  border-radius: 0;
  transition: none;
}
.gs-row.active {
  background: var(--surface-hover);
}
.gs-row:not(.active):hover {
  background: var(--surface-hover);
}

.gs-row-icon {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  color: var(--text-mute);
}
.gs-row.active .gs-row-icon {
  color: var(--text-dim);
}

.gs-row-content {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.gs-row-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.gs-row.active .gs-row-title {
  color: var(--text);
}

.gs-row-snippet {
  font-size: 12px;
  color: var(--text-mute);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.gs-row.active .gs-row-snippet {
  color: var(--text-mute);
}

/* Highlight */
.gs-hl {
  font-weight: 600;
  color: var(--text);
}
.gs-row.active .gs-hl {
  color: var(--text);
}

.gs-row-badge {
  flex-shrink: 0;
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-mute);
  background: var(--surface-2);
  padding: 2px 6px;
  border-radius: 4px;
}
.gs-row.active .gs-row-badge {
  background: var(--surface-active, var(--surface-2));
  color: var(--text-mute);
}

.gs-row-enter {
  width: 16px;
  height: 16px;
  flex-shrink: 0;
  color: var(--text-mute);
}

.gs-more-hint {
  padding: 8px 16px;
  text-align: center;
  font-size: 11px;
  color: var(--text-mute);
  border-top: 1px dashed var(--border);
  margin-top: 6px;
}

/* ---- Footer ---- */
.gs-footer {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 8px 16px;
  border-top: 1px solid var(--border);
  background: var(--surface-2);
  flex-shrink: 0;
}
.gs-hint {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11.5px;
  color: var(--text-mute);
}
.gs-kbd {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 18px;
  height: 18px;
  padding: 0 4px;
  border: 1px solid var(--border);
  border-bottom-width: 2px;
  border-radius: 4px;
  background: var(--surface);
  color: var(--text-dim);
}
.gs-kbd :deep(svg) {
  width: 11px;
  height: 11px;
}
.gs-kbd-text {
  font-family: ui-monospace, SFMono-Regular, monospace;
  font-size: 10px;
  letter-spacing: 0.02em;
}

/* ---- Transition ---- */
.gs-fade-enter-active,
.gs-fade-leave-active {
  transition: opacity 0.15s ease;
}
.gs-fade-enter-active .gs-modal {
  transition: opacity 0.15s ease, transform 0.15s ease;
}
.gs-fade-leave-active .gs-modal {
  transition: opacity 0.1s ease, transform 0.1s ease;
}
.gs-fade-enter-from,
.gs-fade-leave-to {
  opacity: 0;
}
.gs-fade-enter-from .gs-modal {
  opacity: 0;
  transform: scale(0.98) translateY(-8px);
}
.gs-fade-leave-to .gs-modal {
  opacity: 0;
  transform: scale(0.98) translateY(-4px);
}
</style>
