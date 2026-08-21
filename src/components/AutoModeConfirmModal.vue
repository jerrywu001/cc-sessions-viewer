<script setup lang="ts">
// 切到 auto（自动）权限模式时的二次确认弹框 —— 样式严格对齐 Claude 桌面端截图：
// 标题 + 说明 + 工作区路径 + 「不再追问本工作区，详见 security guide」脚注（含外链）+
// Cancel / Enable auto mode。security guide 链接由 main.ts 的全局 <a> 拦截走 openUrl
// 在系统浏览器打开，这里只写普通 <a href>。
import { computed, onBeforeUnmount, watch } from 'vue'
import { t } from '../i18n'

const props = defineProps<{ show: boolean; cwd?: string }>()
const emit = defineEmits<{ (e: 'confirm'): void; (e: 'cancel'): void }>()

const SECURITY_URL = 'https://code.claude.com/docs/en/security'

// 脚注按 {link} 占位拆成「前 + 链接 + 后」，让各语言自行决定语序（链接位置随译文走）。
const footParts = computed(() => t('chat.autoMode.foot').split('{link}'))

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('cancel')
}
watch(
  () => props.show,
  (v) => {
    if (v) document.addEventListener('keydown', onKeydown)
    else document.removeEventListener('keydown', onKeydown)
  },
)
onBeforeUnmount(() => document.removeEventListener('keydown', onKeydown))
</script>

<template>
  <Teleport to="body">
    <Transition name="fade">
      <div v-if="show" class="app-overlay app-overlay-confirm" @click.self="emit('cancel')">
        <div class="modal am-modal" role="dialog" aria-modal="true">
          <h3>{{ t('chat.autoMode.title') }}</h3>
          <p class="am-body">{{ t('chat.autoMode.body') }}</p>
          <div v-if="cwd" class="am-path">{{ cwd }}</div>
          <p class="am-foot">
            {{ footParts[0]
            }}<a :href="SECURITY_URL" class="am-link">{{ t('chat.autoMode.foot.link') }}</a
            >{{ footParts[1] }}
          </p>
          <div class="modal-actions">
            <button class="btn" @click="emit('cancel')">{{ t('common.cancel') }}</button>
            <button class="btn primary" @click="emit('confirm')">
              {{ t('chat.autoMode.confirm') }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.am-modal {
  width: 440px;
}
.am-body {
  margin: 0 0 16px;
  color: var(--text);
  font-size: 13px;
  line-height: 1.55;
}
/* 工作区路径：等宽、可断行，视觉上和正文区分开（截图里是单独一行裸路径）。 */
.am-path {
  margin: 0 0 16px;
  font-family: var(--mono, ui-monospace, SFMono-Regular, Menlo, monospace);
  font-size: 12.5px;
  color: var(--text);
  word-break: break-all;
}
.am-foot {
  margin: 0 0 18px;
  color: var(--text-mute);
  font-size: 12px;
  line-height: 1.55;
}
.am-link {
  color: var(--text-mute);
  text-decoration: underline;
  text-underline-offset: 2px;
  cursor: pointer;
}
.am-link:hover {
  color: var(--text);
}
</style>
