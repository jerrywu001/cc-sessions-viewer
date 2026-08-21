<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { t } from '../i18n'
import { shortName } from '../format'

const props = defineProps<{
  show: boolean
  modelValue: string
  /** 父项目的展示路径，取末段作提示。 */
  projectPath: string
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', v: string): void
  (e: 'confirm'): void
  (e: 'cancel'): void
}>()

const inputEl = ref<HTMLInputElement>()

// 与后端 worktrees::valid_name 对齐：非空、不以 `-` 开头、仅 [A-Za-z0-9._-]、不含 ..。
const NAME_RE = /^[A-Za-z0-9._-]+$/
const trimmed = computed(() => props.modelValue.trim())
const invalidReason = computed(() => {
  const v = trimmed.value
  if (!v) return ''
  if (v.startsWith('-') || !NAME_RE.test(v) || v.includes('..') || v === '.' || v.length > 100) {
    return t('dialog.worktree.invalid')
  }
  return ''
})
const canConfirm = computed(() => !!trimmed.value && !invalidReason.value)

function onConfirm() {
  if (!canConfirm.value) return
  emit('confirm')
}

watch(
  () => props.show,
  (v) => {
    if (!v) return
    nextTick(() => {
      inputEl.value?.focus()
      inputEl.value?.select()
    })
  },
)
</script>

<template>
  <Transition name="fade">
    <div v-if="show" class="app-overlay" @click.self="emit('cancel')">
      <div class="modal rename-modal">
        <h3>{{ t('dialog.worktree.title') }}</h3>
        <p class="worktree-modal-sub">
          {{ t('dialog.worktree.sub', { name: shortName(projectPath) }) }}
        </p>
        <input
          ref="inputEl"
          :value="modelValue"
          type="text"
          class="rename-input"
          :placeholder="t('dialog.worktree.placeholder')"
          maxlength="100"
          @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
          @keydown.enter="onConfirm"
          @keydown.esc="emit('cancel')"
        />
        <p v-if="invalidReason" class="worktree-modal-err">{{ invalidReason }}</p>
        <div class="modal-actions">
          <button class="btn" @click="emit('cancel')">
            {{ t('common.cancel') }}
          </button>
          <button class="btn primary" :disabled="!canConfirm" @click="onConfirm">
            {{ t('dialog.worktree.ok') }}
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>
