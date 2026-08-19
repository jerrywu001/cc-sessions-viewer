<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue'
import { t } from '../i18n'
import { formatTime, shortName } from '../format'
import { agentLabel } from '../agentMeta'
import { history, removeExport, clearExportHistory, type ExportRecord } from '../exportHistory'
import { IconInbox, IconClose } from '../components/icons'

const emit = defineEmits<{
  (e: 'open', rec: ExportRecord): void
}>()

const records = computed(() => history.value)

// hover 跟随浮块：与会话 / 回收站列表一致的滑块交互。鼠标移到某张卡片上，把它的
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
  <section class="export-history-root">
    <div class="list-head list-head-row">
      <div class="grow">
        <h2>{{ t('history.title') }}</h2>
        <div class="path">{{ t('history.subtitle') }}</div>
      </div>
      <button
        class="btn danger"
        :disabled="!records.length"
        @click="clearExportHistory()"
      >
        {{ t('history.clearAll') }}
      </button>
    </div>

    <div v-if="!records.length" class="empty">
      <div class="big"><IconInbox /></div>
      <div>{{ t('history.empty') }}</div>
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
          v-for="rec in records"
          :key="rec.path"
          class="session-card"
          @click="emit('open', rec)"
        >
          <div class="session-main">
            <div class="session-title">
              <span class="agent-badge" :class="rec.agent">{{ agentLabel(rec.agent) }}</span>
              <span>{{ rec.title || t('chat.tui.untitled') }}</span>
            </div>
            <div class="session-meta">
              <span v-if="rec.cwd">{{ shortName(rec.cwd) }}</span>
              <span>{{ shortName(rec.path) }}</span>
              <span>{{ formatTime(rec.exportedAt) }}</span>
            </div>
          </div>
          <div class="session-actions" style="opacity: 1">
            <button
              class="icon-btn danger"
              v-tooltip="t('history.remove')"
              @click.stop="removeExport(rec.path)"
            >
              <IconClose />
            </button>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>
