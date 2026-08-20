<script setup lang="ts">
import { computed } from 'vue'
import type { Block } from '../types'
import { t } from '../i18n'
import CollapsibleBox from './CollapsibleBox.vue'
import { IconChevronRight, IconInfo } from './icons'
import { highlightJsonInPlace, looksLikeJson } from '../jsonHighlight'
import { highlightDiff, looksLikeDiff } from '../diffHighlight'
import { renderCodexFileChangeHtml } from '../codexApplyPatch'

const props = withDefaults(defineProps<{ block: Block; inUser?: boolean; persistOpen?: boolean; cwd?: string }>(), {
  persistOpen: undefined,
})
const emit = defineEmits<{ toggle: [open: boolean] }>()

// 结果文本的渲染优先级：
//   1. structured diff（block.diff，有 hunks）→ DiffBlock（保留交互）
//   2. 文本形态的 unified diff（Bash 跑 git diff / 工具吐 patch）→ 行级染色
//   3. JSON（含 Read .json 文件的 cat-n 行号格式）→ token 上色
//   4. 其它 → 原样 <pre>
// 判断顺序很重要：JSON 文件的 diff 既像 diff 又像 JSON，应该按 diff 渲染。
const diffHtml = computed(() => {
  const txt = props.block.text ?? ''
  if (!looksLikeDiff(txt)) return null
  return highlightDiff(txt)
})
const isTextDiff = computed(() => diffHtml.value !== null)
const jsonHtml = computed(() => {
  const txt = props.block.text ?? ''
  if (!looksLikeJson(txt)) return null
  return highlightJsonInPlace(txt)
})

function baseName(p?: string): string {
  if (!p) return ''
  const parts = p.split('/').filter(Boolean)
  return parts.length ? parts[parts.length - 1] : p
}

/** 从 `git diff` 的文件头中拿一个可读的目标路径。文本 diff 可能包含多文件，这里只取
 * 第一项作为卡片标题，完整内容和统计仍保留在展开区。 */
function firstTextDiffPath(text: string): string | undefined {
  const gitHeader = /^diff --git a\/(.+?) b\/(.+)$/m.exec(text)
  if (gitHeader?.[2]) return gitHeader[2]
  const newFileHeader = /^\+\+\+ (?:b\/)?(.+)$/m.exec(text)
  return newFileHeader?.[1] && newFileHeader[1] !== '/dev/null' ? newFileHeader[1] : undefined
}

const textDiffPath = computed(() => firstTextDiffPath(props.block.text ?? ''))

const label = computed(() => {
  if (props.block.diff || props.block.filePath || isTextDiff.value) {
    return t('tool.resultDiff', { file: baseName(props.block.filePath ?? textDiffPath.value) })
  }
  return props.block.isError ? t('tool.resultError') : t('tool.result')
})

const diffStat = computed(() => {
  let add = 0
  let del = 0
  if (props.block.diff) {
    for (const h of props.block.diff)
      for (const l of h.lines) {
        if (l.kind === 'add') add++
        else if (l.kind === 'del') del++
      }
  } else if (isTextDiff.value) {
    for (const line of (props.block.text ?? '').split('\n')) {
      if (line.startsWith('+') && !line.startsWith('+++')) add++
      else if (line.startsWith('-') && !line.startsWith('---')) del++
    }
  }
  return add || del ? `+${add} −${del}` : ''
})

const hasRenderableText = computed(() => {
  if (props.block.diff || props.block.filePath) return true
  return !!(props.block.text ?? '').trim()
})

const shouldAutoOpen = computed(() => !!props.block.diff || !!props.block.filePath || isTextDiff.value)
const fileChangeHtml = computed(() => {
  if (!props.block.filePath) return null
  return renderCodexFileChangeHtml(
    props.block.diff,
    props.block.filePath,
    props.block.fileChangeType,
    props.cwd,
  )
})
</script>

<template>
  <div
    v-if="fileChangeHtml"
    class="tool-result-file-change"
    v-html="fileChangeHtml"
  />
  <details
    v-else-if="hasRenderableText"
    :class="{
      'block-card': !block.isError,
      'thinking-block': block.isError,
      'tool-result-error': block.isError,
      'in-user': inUser,
      'auto-open': shouldAutoOpen,
      'text-diff-result': isTextDiff,
    }"
    :open="persistOpen ?? shouldAutoOpen"
    @toggle="emit('toggle', ($event.target as HTMLDetailsElement).open)"
  >
    <summary :class="block.isError ? 'thinking-summary' : 'block-summary'">
      <template v-if="block.isError">
        <IconInfo class="thinking-icon" aria-hidden="true" />
        <span class="thinking-label">{{ label }}</span>
        <span class="thinking-chev"><IconChevronRight /></span>
      </template>
      <template v-else>
        <span class="chev"><IconChevronRight /></span>
        <span class="label">{{ label }}</span>
        <span v-if="diffStat" class="diff-stat">{{ diffStat }}</span>
      </template>
    </summary>
    <div :class="block.isError ? 'thinking-content tool-result-error-content' : 'block-body'">
      <CollapsibleBox :max-height="400">
        <pre v-if="diffHtml" class="lang-diff" v-html="diffHtml" />
        <pre v-else-if="jsonHtml" class="lang-json" v-html="jsonHtml" />
        <pre v-else>{{ block.text }}</pre>
      </CollapsibleBox>
    </div>
  </details>
</template>
