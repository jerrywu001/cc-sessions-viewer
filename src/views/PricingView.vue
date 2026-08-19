<script setup lang="ts">
// 「模型实时价格」视图 —— 跟 TrashView / ExportHistoryView 同级，从顶栏
// More 菜单里进。数据来自 src-tauri 启动期优先从 js-bridge 拉取、models.dev 兜底的内存表，by family
// 按已接入 agent family 分段展示。
//
// 主题：完全靠 style.css 里的 design tokens（--surface / --border / --text /
// --accent / --muted），dark/light 切换自动跟随，无 hardcoded 颜色。

import { computed, nextTick, onMounted, onUnmounted, ref, type Component } from 'vue'
import { t } from '../i18n'
import {
  forceRefresh,
  listPricing,
  pricingStatus,
  refreshStatus,
  type PricingEntry,
} from '../pricing'
import {
  IconRefresh,
  IconPriceTag,
  IconClaude,
  IconCodex,
  IconGrok,
  IconAgy,
  IconOpencode,
  IconSearch,
  IconClose,
  IconExternalLink,
} from '../components/icons'
import StatsLoadingIcon from '../components/StatsLoadingIcon.vue'
import { openUrl } from '../api'

// 首选价格数据源 —— 后端在其不可用时自动回退到 models.dev 原站。
const SOURCE_URL = 'https://www.js-bridge.com/api/models'
function openSource() {
  openUrl(SOURCE_URL).catch((e) => console.error('open pricing source failed:', e))
}

const entries = ref<PricingEntry[]>([])
const loading = ref(true)
const refreshing = ref(false)
const errorMsg = ref<string | null>(null)

async function load() {
  loading.value = true
  errorMsg.value = null
  await refreshStatus()
  const list = await listPricing()
  entries.value = list
  loading.value = false
  if (!list.length && pricingStatus.value.lastError) {
    errorMsg.value = pricingStatus.value.lastError
  }
  settleAfterLoad()
}

async function onRefresh() {
  refreshing.value = true
  errorMsg.value = null
  try {
    await forceRefresh()
    entries.value = await listPricing()
  } catch (e) {
    errorMsg.value = (e as Error)?.message || String(e)
  } finally {
    refreshing.value = false
  }
}

onMounted(load)

// 按 family 分桶 —— 后端已排好序（family, input 升序），这里仅做分组。
type Family = 'claude' | 'codex' | 'grok' | 'agy' | 'opencode'
const FAMILIES: { key: Family; icon: Component; label: string }[] = [
  { key: 'claude', icon: IconClaude, label: 'pricing.family.claude' },
  { key: 'codex', icon: IconCodex, label: 'pricing.family.codex' },
  { key: 'grok', icon: IconGrok, label: 'pricing.family.grok' },
  { key: 'agy', icon: IconAgy, label: 'pricing.family.agy' },
  { key: 'opencode', icon: IconOpencode, label: 'pricing.family.opencode' },
]
// 价格页是完整的参考价格表，不受设置中 agent 显隐影响。
const pricingFamilies = FAMILIES

// 搜索框：用户输入 draft，回车（@change / Enter keydown）才把 draft 同步到 query。
// 跨 family 全量搜，子串匹配 model name（大小写不敏感）。空 query = 显示全部。
const searchDraft = ref('')
const query = ref('')
function commitSearch() {
  query.value = searchDraft.value.trim().toLowerCase()
}
function clearSearch() {
  searchDraft.value = ''
  query.value = ''
}

const filtered = computed<PricingEntry[]>(() => {
  if (!query.value) return entries.value
  return entries.value.filter((e) => e.name.toLowerCase().includes(query.value))
})

const grouped = computed(() => {
  const map: Record<Family, PricingEntry[]> = {
    claude: [],
    codex: [],
    grok: [],
    agy: [],
    opencode: [],
  }
  for (const e of filtered.value) {
    if (e.family in map) map[e.family].push(e)
  }
  return map
})

// $/token → $/Mtok（用户熟悉的"每百万 tokens 多少美元"刻度），保留 2~3 位
// 小数；0 显示为占位符 — 表里很多模型 cache_write 没列价 = 不收费 / 未公开。
function fmtRate(perToken: number): string {
  if (!perToken || perToken <= 0) return t('pricing.unavailable')
  const perMtok = perToken * 1_000_000
  // 小于 $0.10/Mtok 用 3 位小数，其它 2 位足够
  return perMtok < 0.1 ? `$${perMtok.toFixed(3)}` : `$${perMtok.toFixed(2)}`
}

// 上下文窗口 (tokens) → "200K" / "1M" / "1.05M"。
// 上游有时不给（0），显示占位符；≥1M 用 M (保 2 位最多)，≥1K 用 K（整数）。
function fmtContext(tokens: number): string {
  if (!tokens || tokens <= 0) return t('pricing.unavailable')
  if (tokens >= 1_000_000) {
    const m = tokens / 1_000_000
    // 1.048576 → 1.05M；1.0 → 1M
    const s = m >= 10 ? m.toFixed(0) : m.toFixed(2).replace(/\.?0+$/, '')
    return `${s}M`
  }
  if (tokens >= 1_000) {
    return `${Math.round(tokens / 1_000)}K`
  }
  return String(tokens)
}

// 锚点快速跳转 ——
// 用户场景：模型表 200+ 行，找一家厂商的价格要滚很久。顶部按 family 提供锚点
// chip，点击 smooth-scroll 到对应 section 顶端，且滚动
// 时根据视窗里第一个可见的 section 高亮当前 chip。
const scrollEl = ref<HTMLElement>()
const toolbarEl = ref<HTMLElement>()
const sectionEls = ref<Record<Family, HTMLElement | null>>({
  claude: null,
  codex: null,
  grok: null,
  agy: null,
  opencode: null,
})
const activeFamily = ref<Family>('claude')

function setSectionRef(key: Family, el: Element | null | undefined) {
  sectionEls.value[key] = (el as HTMLElement | null) ?? null
}
/** sticky toolbar 的实际占位高度（含 margin-bottom 间距）—— 跳转和滚动判定
 *  都要减它，否则 section 头会被工具栏盖住。 */
function toolbarOffset(): number {
  const tb = toolbarEl.value
  if (!tb) return 0
  // offsetHeight 含 border 不含 margin；toolbar 下方的 margin-bottom 也算视觉间隔。
  const style = getComputedStyle(tb)
  const mb = parseFloat(style.marginBottom) || 0
  return tb.offsetHeight + mb
}
function jumpTo(key: Family) {
  const el = sectionEls.value[key]
  const scroller = scrollEl.value
  if (!el || !scroller) return
  // section.offsetTop 是相对滚动容器的位置（容器有 position:relative）。
  // 减 toolbar 占位 + 一点呼吸，让 family-head 落在工具栏下方而不是被盖住。
  const top = Math.max(0, el.offsetTop - toolbarOffset() - 4)
  scroller.scrollTo({ top, behavior: 'smooth' })
  activeFamily.value = key
}

// 滚动时根据视窗里第一个 top ≥ 0 的 section 反推当前 chip。
// 不用 IntersectionObserver —— 视窗顶部空 chrome 高度可变，offsetTop 比 root margin
// 准。节流到 rAF 一次。
let scrollRaf = 0
function onScroll() {
  if (scrollRaf) return
  scrollRaf = requestAnimationFrame(() => {
    scrollRaf = 0
    const scroller = scrollEl.value
    if (!scroller) return
    // 判定视窗：把"工具栏下沿"作为基准线，刚被它挡住的 section 才算"当前可见"。
    const y = scroller.scrollTop + toolbarOffset() + 8
    const fams = pricingFamilies
    if (!fams.length) return
    let best: Family = fams[0].key
    for (const f of fams) {
      const el = sectionEls.value[f.key]
      if (!el) continue
      if (el.offsetTop <= y) best = f.key
    }
    activeFamily.value = best
  })
}
onUnmounted(() => {
  if (scrollRaf) cancelAnimationFrame(scrollRaf)
})
// 数据到位后等一帧拿到 section ref 再算一次初始 active。
async function settleAfterLoad() {
  await nextTick()
  onScroll()
}
</script>

<template>
  <div class="pricing-root">
  <div class="list-head list-head-row">
    <div class="grow">
      <h2 class="pricing-title">
        {{ t('pricing.title') }}
        <button
          type="button"
          class="icon-btn"
          v-tooltip="t('pricing.openSource')"
          @click="openSource"
        >
          <IconExternalLink />
        </button>
      </h2>
      <div class="path">{{ t('pricing.subtitle') }}</div>
    </div>
    <button
      class="btn"
      :disabled="refreshing || loading"
      @click="onRefresh"
    >
      <IconRefresh />
      <span>{{ t('pricing.refresh') }}</span>
    </button>
  </div>

  <!-- 初次加载 / 刷新 —— 复用 Stats 同款 4-柱动画占位。纯 CSS keyframes
       在 compositor 跑，即便后端拉数据时 JS 主线程短暂卡顿，柱子也会持续
       脉动，不会像之前的 transparent overlay 那样看上去"卡死"。 -->
  <div v-if="loading || refreshing" class="stats-empty">
    <div class="big"><StatsLoadingIcon /></div>
    <div class="stats-loading-dots">{{ t('pricing.loading').replace(/[.…]+$/, '') }}</div>
  </div>

  <div v-else-if="errorMsg" class="empty">
    <div class="big"><IconPriceTag /></div>
    <div>{{ t('pricing.error', { err: errorMsg }) }}</div>
    <button class="btn" :disabled="refreshing" @click="onRefresh">
      {{ t('pricing.retry') }}
    </button>
  </div>

  <div v-else-if="!entries.length" class="empty">
    <div class="big"><IconPriceTag /></div>
    <div>{{ t('pricing.empty') }}</div>
  </div>

  <div
    v-else
    ref="scrollEl"
    class="scroll-area pricing-scroll"
    @scroll.passive="onScroll"
  >
    <div ref="toolbarEl" class="pricing-toolbar">
      <div class="pricing-search" :class="{ active: !!query }">
        <span class="pricing-search-ic"><IconSearch /></span>
        <input
          v-model="searchDraft"
          type="text"
          class="pricing-search-input"
          :placeholder="t('pricing.searchPlaceholder')"
          spellcheck="false"
          autocomplete="off"
          @keydown.enter.prevent="commitSearch"
          @change="commitSearch"
        />
        <button
          v-if="searchDraft || query"
          type="button"
          class="pricing-search-clear"
          v-tooltip="t('chat.tb.search.clear')"
          @click="clearSearch"
        >
          <IconClose />
        </button>
      </div>
      <nav class="pricing-anchors" role="tablist" :aria-label="t('pricing.title')">
        <button
          v-for="fam in pricingFamilies"
          :key="fam.key"
          type="button"
          class="pricing-anchor"
          :class="['agent-' + fam.key, { active: activeFamily === fam.key }]"
          role="tab"
          :aria-selected="activeFamily === fam.key"
          v-tooltip="t(fam.label)"
          @click="jumpTo(fam.key)"
        >
          <component :is="fam.icon" class="pricing-anchor-ic" />
          <span class="pricing-anchor-count">{{ grouped[fam.key].length }}</span>
        </button>
      </nav>
    </div>

    <div class="pricing-meta">
      <span>{{ t('pricing.lastUpdated', { n: entries.length }) }}</span>
      <span class="dot">·</span>
      <span>{{ t('pricing.unit') }}</span>
      <template v-if="query">
        <span class="dot">·</span>
        <span class="pricing-meta-filter">{{ t('pricing.filtered', { n: filtered.length }) }}</span>
      </template>
    </div>

    <section
      v-for="fam in pricingFamilies"
      :key="fam.key"
      :ref="(el) => setSectionRef(fam.key, el as Element | null)"
      class="pricing-family"
    >
      <header class="pricing-family-head">
        <span class="pricing-family-name">{{ t(fam.label) }}</span>
        <span class="pricing-family-count">{{ grouped[fam.key].length }}</span>
      </header>

      <div v-if="grouped[fam.key].length === 0" class="pricing-family-empty">
        {{ t('pricing.empty') }}
      </div>

      <div v-else class="pricing-table" role="table">
        <div class="pricing-row pricing-row-head" role="row">
          <span class="pricing-cell pricing-cell-model" role="columnheader">{{ t('pricing.column.model') }}</span>
          <span class="pricing-cell pricing-cell-num" role="columnheader">{{ t('pricing.column.context') }}</span>
          <span class="pricing-cell pricing-cell-num" role="columnheader">{{ t('pricing.column.input') }}</span>
          <span class="pricing-cell pricing-cell-num" role="columnheader">{{ t('pricing.column.output') }}</span>
          <span class="pricing-cell pricing-cell-num" role="columnheader">{{ t('pricing.column.cacheRead') }}</span>
          <span class="pricing-cell pricing-cell-num" role="columnheader">{{ t('pricing.column.cacheWrite') }}</span>
        </div>
        <div
          v-for="row in grouped[fam.key]"
          :key="row.name"
          class="pricing-row"
          role="row"
        >
          <span class="pricing-cell pricing-cell-model" role="cell">{{ row.name }}</span>
          <span class="pricing-cell pricing-cell-num" role="cell">{{ fmtContext(row.context) }}</span>
          <span class="pricing-cell pricing-cell-num" role="cell">{{ fmtRate(row.input) }}</span>
          <span class="pricing-cell pricing-cell-num" role="cell">{{ fmtRate(row.output) }}</span>
          <span class="pricing-cell pricing-cell-num" role="cell">{{ fmtRate(row.cacheRead) }}</span>
          <span class="pricing-cell pricing-cell-num" role="cell">{{ fmtRate(row.cacheWrite) }}</span>
        </div>
      </div>
    </section>
  </div>
  </div>
</template>
