<script setup lang="ts">
// Codex `/side` 侧聊浮框。它沿用 btw 的紧凑浮层交互，但会话、样式状态与关闭语义均独立：
// app-server 以 ephemeral `thread/fork` 建立旁支，不会写入主会话或磁盘 history。
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { t } from '../i18n'
import { formatElapsedSeconds, renderText } from '../format'
import { now, sendPrompt, interruptChat, type ChatSession } from '../chatSessions'
import {
  closeCodexSideChat,
  isCodexSideMinimized,
  setCodexSideMinimized,
} from '../codexSideChat'
import { focusedPane } from '../panes'
import type { Block, Msg } from '../types'
import { IconChevronRight, IconClose, IconMinimize, IconSend, IconStop, IconZap } from './icons'

const props = defineProps<{ session: ChatSession; hidden?: boolean }>()

const panelEl = ref<HTMLElement | null>(null)

function containerRect(): DOMRect {
  const fp = focusedPane.value
  const pane = fp
    ? document.querySelector(`[data-pane-id="${fp.id}"] .main-body`)
    : document.querySelector('.pane-focused .main-body')
  return pane?.getBoundingClientRect() ?? new DOMRect(0, 0, window.innerWidth, window.innerHeight)
}

// ---------- 浮框宽度（右下角拖拽改宽；上限受容器约束） ----------
const W_KEY = 'codexSideChatWidth'
const MIN_W = 320
const MAX_W = 2000
function readWidth(): number {
  try {
    const raw = localStorage.getItem(W_KEY)
    if (raw) {
      const w = JSON.parse(raw)
      if (typeof w === 'number') return Math.max(MIN_W, w)
    }
  } catch {
    /* ignore */
  }
  return 400
}
const width = ref(readWidth())

// ---------- 浮框位置（锚右边缘：存「距右」+「距顶」，限制在容器内） ----------
const POS_KEY = 'codexSideChatDock'
function clampPos(right: number, top: number): { right: number; top: number } {
  const cr = containerRect()
  const vw = window.innerWidth
  const minRight = vw - cr.right + 8
  const maxRight = vw - cr.left - width.value - 8
  const minTop = cr.top + 8
  const panelH = panelEl.value?.offsetHeight ?? 300
  const maxTop = cr.bottom - panelH - 8
  return {
    right: Math.min(Math.max(minRight, right), Math.max(minRight, maxRight)),
    top: Math.min(Math.max(minTop, top), Math.max(minTop, maxTop)),
  }
}
function readPos(): { right: number; top: number } {
  try {
    const raw = localStorage.getItem(POS_KEY)
    if (raw) {
      const p = JSON.parse(raw)
      if (typeof p.right === 'number' && typeof p.top === 'number') return { right: p.right, top: p.top }
    }
  } catch {
    /* ignore */
  }
  return { right: 20, top: 64 }
}
const pos = ref(readPos())

function persistPos() {
  try {
    localStorage.setItem(POS_KEY, JSON.stringify(pos.value))
  } catch {
    /* ignore */
  }
}

// ---------- 浮框高度（右下角拖拽改高；null = 自适应内容，受 max-height 约束） ----------
const H_KEY = 'codexSideChatHeight'
const MIN_H = 240
function clampHeight(h: number): number {
  const max = Math.max(MIN_H, containerRect().bottom - pos.value.top - 8)
  return Math.min(Math.max(MIN_H, h), max)
}
function readHeight(): number | null {
  try {
    const raw = localStorage.getItem(H_KEY)
    if (raw) {
      const h = JSON.parse(raw)
      if (typeof h === 'number') return clampHeight(h)
    }
  } catch {
    /* ignore */
  }
  return null
}
const height = ref<number | null>(readHeight())

// 显式高度时撑满到指定高度并解除 max-height；否则保持自适应。
const panelStyle = computed<Record<string, string>>(() => {
  const s: Record<string, string> = {
    right: pos.value.right + 'px',
    top: pos.value.top + 'px',
    width: width.value + 'px',
  }
  if (height.value != null) {
    s.height = height.value + 'px'
    s.maxHeight = 'none'
  }
  return s
})

// 整框拖动（从标题栏起拽）
let dragRight0 = 0
let dragTop0 = 0
let dragX0 = 0
let dragY0 = 0
function onDragStart(e: PointerEvent) {
  if ((e.target as HTMLElement).closest('button')) return // 关闭按钮不触发拖动
  dragRight0 = pos.value.right
  dragTop0 = pos.value.top
  dragX0 = e.clientX
  dragY0 = e.clientY
  window.addEventListener('pointermove', onDragMove)
  window.addEventListener('pointerup', onDragEnd, { once: true })
}
function onDragMove(e: PointerEvent) {
  // 向右移光标 → right 变小（框右移）；向下 → top 变大。
  pos.value = clampPos(dragRight0 - (e.clientX - dragX0), dragTop0 + (e.clientY - dragY0))
}
function onDragEnd() {
  window.removeEventListener('pointermove', onDragMove)
  persistPos()
}

// 右下角拖拽（纯增量，左上角锚定不动）：宽 += dx、高 += dy，pos.right 抵消宽度增量。
// 全程只用光标位移，绝不碰 window.innerWidth —— 否则会和真实布局坐标差一个滚动条宽度，
// 表现为「一按下就缩一下、之后恒定偏移」。
let startX = 0
let startY = 0
let startW = 0
let startH = 0
let startRight = 0
let startTop = 0
function onResizeStart(e: PointerEvent) {
  e.stopPropagation()
  e.preventDefault()
  const rect = panelEl.value?.getBoundingClientRect()
  startX = e.clientX
  startY = e.clientY
  startW = width.value
  startH = height.value ?? rect?.height ?? MIN_H
  startRight = pos.value.right
  startTop = pos.value.top
  // 之前高度自适应的话，先用当前真实高度作起点，避免一拖就跳。
  if (height.value == null) height.value = startH
  window.addEventListener('pointermove', onResizeMove)
  window.addEventListener('pointerup', onResizeEnd, { once: true })
}
function onResizeMove(e: PointerEvent) {
  const dx = e.clientX - startX
  const dy = e.clientY - startY
  // 宽：左缘固定，右缘最多到「视口右 − 8」（即 pos.right ≥ 8）。
  const maxW = Math.min(MAX_W, startW + startRight - 8)
  const newW = Math.min(Math.max(MIN_W, startW + dx), maxW)
  const maxH = Math.max(MIN_H, containerRect().bottom - 8 - startTop)
  const newH = Math.min(Math.max(MIN_H, startH + dy), maxH)
  width.value = newW
  height.value = newH
  pos.value = { right: startRight - (newW - startW), top: startTop }
}
function onResizeEnd() {
  window.removeEventListener('pointermove', onResizeMove)
  try {
    localStorage.setItem(W_KEY, JSON.stringify(width.value))
    localStorage.setItem(H_KEY, JSON.stringify(height.value))
    localStorage.setItem(POS_KEY, JSON.stringify(pos.value))
  } catch {
    /* ignore */
  }
}

const fabStyle = computed(() => {
  void focusedPane.value
  const cr = containerRect()
  return {
    right: (window.innerWidth - cr.right + 20) + 'px',
    bottom: (window.innerHeight - cr.bottom + 20) + 'px',
  }
})

// ---------- 运行态 ----------
const running = computed(() => props.session.turnState === 'running')
const ready = computed(
  () => props.session.status === 'running' && (props.session.chatId !== null || props.session.needsResume),
)
const elapsedSec = computed(() =>
  running.value ? Math.max(0, Math.floor((now.value - props.session.turnStartedAt) / 1000)) : 0,
)
const elapsedLabel = computed(() => formatElapsedSeconds(elapsedSec.value))
const errored = computed(() => props.session.status === 'error' || props.session.status === 'exited')
const contextLabel = computed(() => t('chat.side.ephemeral'))

// ---------- 最小化：折叠成纯标题条（消息/输入隐藏，运行态仍在头部转圈） ----------
const minimized = ref(isCodexSideMinimized(props.session.uiId))
function toggleMin(e: Event) {
  e.stopPropagation() // 别触发标题栏拖动
  minimized.value = !minimized.value
  setCodexSideMinimized(props.session.uiId, minimized.value)
  if (!minimized.value) scrollToBottom()
}

// ---------- 消息呈现（精简版：只取 text / thinking / 工具行） ----------
function textHtml(blocks: Block[]): string {
  const txt = blocks
    .filter((b) => b.kind === 'text')
    .map((b) => b.text ?? '')
    .join('\n\n')
  return txt ? renderText(txt) : ''
}
function thinkingText(blocks: Block[]): string {
  return blocks
    .filter((b) => b.kind === 'thinking')
    .map((b) => b.text ?? '')
    .join('\n')
    .trim()
}
function toolNames(blocks: Block[]): string[] {
  return blocks.filter((b) => b.kind === 'tool_use').map((b) => b.toolName ?? 'tool')
}
function userText(m: Msg): string {
  return m.blocks
    .filter((b) => b.kind === 'text')
    .map((b) => b.text ?? '')
    .join('\n')
    .trim()
}
// 只展示有实质内容的消息（纯 tool_result 的 user 记录、空气泡都跳过）。
const rows = computed(() =>
  props.session.msgs.filter((m) => {
    if (m.metaKind) return false
    if (m.role === 'user') return !!userText(m)
    return (
      m.blocks.some((b) => b.kind === 'text' && (b.text ?? '').trim()) ||
      thinkingText(m.blocks) !== '' ||
      toolNames(m.blocks).length > 0
    )
  }),
)

// ---------- 自动滚到底 ----------
const listEl = ref<HTMLElement | null>(null)
function scrollToBottom() {
  nextTick(() => {
    const el = listEl.value
    if (el) el.scrollTop = el.scrollHeight
  })
}
watch(
  () => [props.session.msgs.length, props.session.live?.text, running.value],
  scrollToBottom,
)

// ---------- 紧凑输入 ----------
const draft = ref('')
const taEl = ref<HTMLTextAreaElement | null>(null)
const canSend = computed(() => draft.value.trim().length > 0 && ready.value)
function autosize() {
  const el = taEl.value
  if (!el) return
  el.style.height = 'auto'
  el.style.height = Math.min(el.scrollHeight, 120) + 'px'
}
async function submit() {
  if (running.value || !canSend.value) return
  const body = draft.value
  draft.value = ''
  nextTick(autosize)
  await sendPrompt(props.session, body)
}
function onPrimary() {
  if (running.value) void interruptChat(props.session)
  else void submit()
}
function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
    e.preventDefault()
    void submit()
  }
}

onMounted(() => nextTick(() => taEl.value?.focus()))

defineExpose({ focusInput: () => taEl.value?.focus() })
</script>

<template>
  <Teleport to="body">
    <div v-show="!minimized && !hidden" ref="panelEl" class="codex-side-chat" :style="panelStyle">
      <!-- 右下角：拖拽同时改宽 + 改高 -->
      <div class="sc-resize" @pointerdown="onResizeStart" />

      <!-- 头部：拖动手柄 + 标题 + 上下文标记 + 最小化 + 关闭 -->
      <div class="sc-head" @pointerdown="onDragStart">
        <span class="sc-brand"><IconZap /></span>
        <span class="sc-title">side</span>
        <span class="sc-ctx">{{ contextLabel }}</span>
        <button
          class="sc-x"
          v-tooltip="t('chat.side.minimize')"
          @pointerdown.stop
          @click="toggleMin"
        >
          <IconMinimize />
        </button>
        <button class="sc-x" v-tooltip="t('common.close')" @pointerdown.stop @click="closeCodexSideChat">
          <IconClose />
        </button>
      </div>

      <!-- 消息区 -->
      <div ref="listEl" class="sc-body">
        <div v-if="!rows.length && !running && !errored" class="sc-empty">
          <span class="sc-empty-ic"><IconZap /></span>
          <p>{{ t('chat.side.empty') }}</p>
        </div>

        <div v-for="(m, i) in rows" :key="m.uuid ?? i" class="sc-msg" :class="m.role">
          <!-- 用户气泡 -->
          <div v-if="m.role === 'user'" class="sc-bubble user">{{ userText(m) }}</div>
          <!-- 助手：思考（默认折叠） + 工具行 + 正文 -->
          <template v-else>
            <details v-if="thinkingText(m.blocks)" class="sc-think">
              <summary class="sc-think-head">
                <span class="sc-think-chev"><IconChevronRight /></span>
                <span>{{ t('tool.thinking') }}</span>
              </summary>
              <div class="sc-think-body">{{ thinkingText(m.blocks) }}</div>
            </details>
            <div v-if="toolNames(m.blocks).length" class="sc-tools">
              <span v-for="(nm, ti) in toolNames(m.blocks)" :key="ti" class="sc-tool">{{ nm }}</span>
            </div>
            <div v-if="textHtml(m.blocks)" class="sc-bubble bot" v-html="textHtml(m.blocks)" />
          </template>
        </div>

        <!-- 流式进行中的文本 -->
        <div v-if="session.live && session.live.text" class="sc-msg assistant">
          <div class="sc-bubble bot" :class="{ thinking: session.live.kind === 'thinking' }">
            {{ session.live.text }}
          </div>
        </div>
        <div v-else-if="running" class="sc-running">
          <span class="sc-spinner" />{{ t('chat.side.thinking') }} · {{ elapsedLabel }}
        </div>
        <div v-if="errored" class="sc-error">{{ session.errorMessage || t('chat.side.ended') }}</div>
      </div>

      <!-- 紧凑输入 -->
      <div class="sc-foot">
        <textarea
          ref="taEl"
          v-model="draft"
          class="sc-input"
          rows="1"
          :placeholder="t('chat.side.placeholder')"
          :disabled="errored"
          @input="autosize"
          @keydown="onKeydown"
        />
        <button
          class="sc-send"
          :class="{ stop: running }"
          :disabled="!running && !canSend"
          v-tooltip="running ? t('chat.composer.stop') : t('chat.composer.send')"
          @click="onPrimary"
        >
          <IconStop v-if="running" />
          <IconSend v-else />
        </button>
      </div>
    </div>

    <!-- 最小化：缩到右下角的悬浮球；运行中显示转圈 + 计时，点一下还原 -->
    <button
      v-if="minimized && !hidden"
      class="sc-fab"
      :class="{ running }"
      :style="fabStyle"
      v-tooltip="t('chat.side.restore')"
      @click="toggleMin"
    >
      <span class="sc-spinner" v-if="running" />
      <span class="sc-fab-ic" v-else><IconZap /></span>
      <span class="sc-fab-label">side</span>
      <span v-if="running" class="sc-fab-sec">{{ elapsedLabel }}</span>
    </button>
  </Teleport>
</template>

<style scoped>
.codex-side-chat {
  position: fixed;
  z-index: 1200;
  display: flex;
  flex-direction: column;
  max-height: 76vh;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 14px;
  box-shadow:
    0 1px 2px rgba(0, 0, 0, 0.06),
    0 18px 48px -12px rgba(0, 0, 0, 0.32);
  overflow: hidden;
}
.sc-resize {
  position: absolute;
  right: 0;
  bottom: 0;
  width: 18px;
  height: 18px;
  cursor: nwse-resize;
  z-index: 3;
  touch-action: none;
}
/* 右下角 "⌟" 形抓手：默认淡，hover 高亮 */
.sc-resize::before {
  content: '';
  position: absolute;
  right: 4px;
  bottom: 4px;
  width: 7px;
  height: 7px;
  border-right: 2px solid var(--border);
  border-bottom: 2px solid var(--border);
  border-bottom-right-radius: 3px;
  opacity: 0.65;
  transition: border-color 0.15s ease, opacity 0.15s ease;
}
.sc-resize:hover::before {
  opacity: 1;
  border-color: var(--accent);
}

/* ---------- 头部 ---------- */
.sc-head {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 9px 10px 9px 13px;
  border-bottom: 1px solid var(--border);
  cursor: grab;
  user-select: none;
  background: var(--surface-hover);
}
.sc-head:active {
  cursor: grabbing;
}
.sc-brand {
  display: inline-flex;
  color: var(--brand-codex);
}
.sc-brand :deep(svg) {
  width: 15px;
  height: 15px;
}
.sc-title {
  font-weight: 650;
  font-size: 13px;
  letter-spacing: 0.01em;
  color: var(--text);
}
.sc-ctx {
  flex: 1;
  min-width: 0;
  font-size: 11px;
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sc-x {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease;
}
.sc-x:hover {
  background: var(--surface-active);
  color: var(--text);
}
.sc-x :deep(svg) {
  width: 15px;
  height: 15px;
}

/* ---------- 最小化：右下角悬浮球 ---------- */
.sc-fab {
  position: fixed;
  z-index: 1200;
  display: inline-flex;
  align-items: center;
  gap: 7px;
  height: 40px;
  padding: 0 15px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: var(--surface);
  color: var(--text);
  cursor: pointer;
  box-shadow:
    0 1px 2px rgba(0, 0, 0, 0.06),
    0 12px 32px -10px rgba(0, 0, 0, 0.34);
  transition: transform 0.12s ease, box-shadow 0.12s ease, background 0.12s ease;
}
.sc-fab:hover {
  background: var(--surface-hover);
  transform: translateY(-1px);
  box-shadow:
    0 2px 4px rgba(0, 0, 0, 0.08),
    0 16px 40px -10px rgba(0, 0, 0, 0.4);
}
.sc-fab.running {
  border-color: color-mix(in srgb, var(--brand-codex) 45%, var(--border));
}
.sc-fab-ic {
  display: inline-flex;
  color: var(--brand-codex);
}
.sc-fab-ic :deep(svg) {
  width: 16px;
  height: 16px;
}
.sc-fab-label {
  font-weight: 650;
  font-size: 13px;
  letter-spacing: 0.01em;
}
.sc-fab-sec {
  font-size: 11px;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
}

/* ---------- 消息区 ---------- */
.sc-body {
  flex: 1;
  min-height: 96px;
  overflow-y: auto;
  padding: 12px 13px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.sc-empty {
  margin: auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 9px;
  padding: 14px 24px;
  text-align: center;
}
.sc-empty-ic {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  border-radius: 50%;
  background: var(--surface-hover);
  color: var(--brand-codex);
}
.sc-empty-ic :deep(svg) {
  width: 17px;
  height: 17px;
}
.sc-empty p {
  margin: 0;
  max-width: 240px;
  font-size: 12.5px;
  line-height: 1.5;
  color: var(--text-dim);
}
.sc-msg {
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.sc-msg.user {
  align-items: flex-end;
}
.sc-bubble {
  max-width: 90%;
  padding: 8px 11px;
  border-radius: 12px;
  font-size: 13px;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
}
.sc-bubble.user {
  background: var(--accent);
  color: var(--surface);
  border-bottom-right-radius: 4px;
}
.sc-bubble.bot {
  background: var(--surface-hover);
  color: var(--text);
  border-bottom-left-radius: 4px;
  white-space: normal;
}
.sc-bubble.bot.thinking {
  background: transparent;
  color: var(--text-dim);
  font-style: italic;
  padding: 0 2px;
}
.sc-bubble.bot :deep(p) {
  margin: 0 0 6px;
}
.sc-bubble.bot :deep(p:last-child) {
  margin-bottom: 0;
}
.sc-bubble.bot :deep(pre) {
  margin: 6px 0;
  padding: 9px 10px;
  border-radius: 8px;
  background: var(--code-bg);
  overflow-x: auto;
  font-size: 12px;
}
.sc-bubble.bot :deep(code) {
  font-size: 12px;
}

/* ---------- 思考折叠 ---------- */
.sc-think {
  font-size: 11.5px;
  color: var(--text-dim);
}
.sc-think-head {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  cursor: pointer;
  list-style: none;
  user-select: none;
  font-style: italic;
  opacity: 0.85;
}
.sc-think-head:hover {
  opacity: 1;
}
.sc-think-head::-webkit-details-marker {
  display: none;
}
.sc-think-chev {
  display: inline-flex;
  transition: transform 0.15s ease;
}
.sc-think[open] .sc-think-chev {
  transform: rotate(90deg);
}
.sc-think-chev :deep(svg) {
  width: 11px;
  height: 11px;
}
.sc-think-body {
  margin-top: 5px;
  font-style: italic;
  white-space: pre-wrap;
  word-break: break-word;
  border-left: 2px solid var(--border);
  padding-left: 9px;
}

/* ---------- 工具行 / 运行态 ---------- */
.sc-tools {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.sc-tool {
  font-size: 10.5px;
  color: var(--text-dim);
  background: var(--surface-hover);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 1px 7px;
}
.sc-running {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 12px;
  color: var(--text-dim);
}
.sc-spinner {
  width: 11px;
  height: 11px;
  border: 2px solid var(--border);
  border-top-color: var(--brand-codex);
  border-radius: 50%;
  animation: sc-spin 0.7s linear infinite;
}
@keyframes sc-spin {
  to {
    transform: rotate(360deg);
  }
}
.sc-error {
  font-size: 12px;
  color: var(--danger);
}

/* ---------- 输入 ---------- */
.sc-foot {
  display: flex;
  align-items: flex-end;
  gap: 7px;
  padding: 9px 10px;
  border-top: 1px solid var(--border);
  background: var(--surface-hover);
}
.sc-input {
  flex: 1;
  resize: none;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--surface);
  color: var(--text);
  font: inherit;
  font-size: 13px;
  line-height: 1.45;
  padding: 8px 11px;
  max-height: 120px;
  outline: none;
  transition: border-color 0.12s ease, box-shadow 0.12s ease;
}
.sc-input:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 18%, transparent);
}
.sc-send {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  flex: none;
  border: none;
  border-radius: 10px;
  background: var(--accent);
  color: var(--surface);
  cursor: pointer;
  transition: opacity 0.12s ease, background 0.12s ease;
}
.sc-send:hover:not(:disabled) {
  opacity: 0.88;
}
.sc-send:disabled {
  opacity: 0.35;
  cursor: default;
}
.sc-send.stop {
  background: var(--danger);
  color: #fff;
}
.sc-send :deep(svg) {
  width: 15px;
  height: 15px;
}
</style>
