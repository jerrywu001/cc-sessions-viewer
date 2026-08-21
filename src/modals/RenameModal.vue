<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import { t } from '../i18n'

const props = defineProps<{
  show: boolean
  modelValue: string
  defaultTitle: string
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', v: string): void
  (e: 'confirm'): void
  (e: 'cancel'): void
}>()

const inputEl = ref<HTMLInputElement>()

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
        <h3>{{ t('dialog.rename.title') }}</h3>
        <input
          ref="inputEl"
          :value="modelValue"
          type="text"
          class="rename-input"
          :placeholder="defaultTitle"
          maxlength="200"
          @input="
            emit('update:modelValue', ($event.target as HTMLInputElement).value)
          "
          @keydown.enter="emit('confirm')"
          @keydown.esc="emit('cancel')"
        />
        <div class="modal-actions">
          <button class="btn" @click="emit('cancel')">
            {{ t('common.cancel') }}
          </button>
          <button class="btn primary" @click="emit('confirm')">
            {{ t('common.ok') }}
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>
