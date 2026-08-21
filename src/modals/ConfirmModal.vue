<script setup lang="ts">
import { t } from '../i18n'

defineProps<{
  show: boolean
  title: string
  message: string
  okText: string
  danger: boolean
  altText?: string
}>()

const emit = defineEmits<{
  (e: 'confirm'): void
  (e: 'cancel'): void
  (e: 'alt'): void
}>()
</script>

<template>
  <Transition name="fade">
    <div v-if="show" class="app-overlay app-overlay-confirm" @click.self="emit('cancel')">
      <div class="modal">
        <h3>{{ title }}</h3>
        <p>{{ message }}</p>
        <div class="modal-actions">
          <button class="btn" @click="emit('cancel')">
            {{ t('common.cancel') }}
          </button>
          <button
            v-if="altText"
            class="btn danger"
            @click="emit('alt')"
          >
            {{ altText }}
          </button>
          <button
            class="btn"
            :class="danger ? 'danger' : 'primary'"
            @click="emit('confirm')"
          >
            {{ okText }}
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>
