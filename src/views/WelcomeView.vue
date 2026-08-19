<script setup lang="ts">
import { computed } from 'vue'
import type { Agent, ProjectInfo } from '../types'
import { shortName } from '../format'
import { t } from '../i18n'
import { clearRecents, getRecents, removeRecent } from '../recents'
import {
  IconHistory,
  IconChevronRight,
  IconClose,
  IconGithub,
  IconSearch,
  IconInfo,
  agentIcons,
} from '../components/icons'
import appIcon from '../assets/app-icon.png'
import { openGlobalSearch } from '../globalSearch'
import { visibleAgents } from '../settings'
import { agentLabel } from '../agentMeta'

const props = defineProps<{
  agent: Agent
  projects: ProjectInfo[]
}>()

const emit = defineEmits<{
  (e: 'select-project', dir: string): void
  (e: 'switch-agent', a: Agent): void
  (e: 'open-repo'): void
}>()

// 与侧栏 agent-switch 同规则：3 个及以上 agent 时分段控件放不下 icon+文字。
const iconsOnly = computed(() => visibleAgents.value.length > 2)

// 最近打开过的项目：拿 recents 里的 dirName 去当前 projects 取真身，
// 过滤掉已删除 / 换 agent 后不存在的（getRecents 读 recents.value，computed 自动随它刷新）。
const recentProjects = computed<ProjectInfo[]>(() => {
  const byDir = new Map(props.projects.map((p) => [p.dirName, p]))
  return getRecents(props.agent)
    .map((dir) => byDir.get(dir))
    .filter((p): p is ProjectInfo => !!p)
})

// 跨平台修饰符：mac 用 ⌘、其他用 Ctrl。给底部「⌘⇧F 全局搜索」提示用。
const isMac = /Mac/i.test(navigator.platform)
const modKey = isMac ? '⌘' : 'Ctrl'
</script>

<template>
  <div class="welcome">
    <!-- 滚动公告 -->
    <div class="welcome-announcement">
      <div class="announcement-icon">
        <IconInfo />
      </div>
      <div class="announcement-viewport">
        <div class="announcement-track">
          <div class="announcement-content">
            {{ t('welcome.announcement') }}
          </div>
          <div class="announcement-content">
            {{ t('welcome.announcement') }}
          </div>
        </div>
      </div>
    </div>
    <!-- 仓库入口：固定在主页面右上角 -->
    <button
      class="welcome-github"
      v-tooltip="t('topbar.github')"
      @click="emit('open-repo')"
    >
      <IconGithub />
    </button>
    <div class="welcome-inner">
      <div class="welcome-logo"><img :src="appIcon" alt="" /></div>
      <h1 class="welcome-title">Sessions Viewer</h1>

      <!-- 当前 agent 切换；≥3 个时收成纯图标（名字进 tooltip），与侧栏切换器同规则 -->
      <div
        v-if="visibleAgents.length > 1"
        class="welcome-agents"
        :class="{ 'icons-only': iconsOnly }"
      >
        <button
          v-for="a in visibleAgents"
          :key="a"
          class="welcome-agent"
          :class="{ active: a === agent }"
          v-tooltip="iconsOnly ? agentLabel(a) : ''"
          @click="emit('switch-agent', a)"
        >
          <component :is="agentIcons[a]" />
          <template v-if="!iconsOnly">{{ agentLabel(a) }}</template>
        </button>
      </div>

      <!-- 全局搜索快捷键提示 / 入口 —— 点了等同于按 ⌘⇧F；放在 tab 下方与最近列表上方 -->
      <button
        class="welcome-search-hint"
        v-tooltip="t('search.global.placeholder', { agent: agentLabel(agent) })"
        @click="openGlobalSearch"
      >
        <IconSearch class="welcome-search-ic" />
        <span class="welcome-search-label">{{ t('search.global.placeholder', { agent: agentLabel(agent) }) }}</span>
        <span class="welcome-search-kbd">
          <kbd class="gs-kbd">{{ modKey }}</kbd>
          <kbd class="gs-kbd">⇧</kbd>
          <kbd class="gs-kbd">F</kbd>
        </span>
      </button>

      <!-- 最近打开过的项目 —— 快捷跳转 -->
      <div v-if="recentProjects.length" class="welcome-recents">
        <div class="welcome-section">
          <IconHistory />
          <span>{{ t('welcome.recent') }}</span>
          <button
            class="welcome-section-clear"
            v-tooltip="t('welcome.clearRecent')"
            :aria-label="t('welcome.clearRecent')"
            @click="clearRecents(agent)"
          >
            {{ t('welcome.clearRecent') }}
          </button>
        </div>
        <div
          v-for="p in recentProjects"
          :key="p.dirName"
          class="welcome-recent"
          :class="{ missing: !p.exists }"
          role="button"
          tabindex="0"
          v-tooltip:right="p.exists ? p.displayPath : p.displayPath + t('proj.missing')"
          @click="emit('select-project', p.dirName)"
          @keydown.enter.prevent="emit('select-project', p.dirName)"
        >
          <span class="welcome-recent-name">{{ shortName(p.displayPath) }}</span>
          <span class="proj-count">{{ p.sessionCount }}</span>
          <!-- hover 时浮出来的小 × ；只删 recents 里的记录，不动磁盘上的 jsonl。 -->
          <button
            class="welcome-recent-remove"
            v-tooltip="t('welcome.removeRecent')"
            :aria-label="t('welcome.removeRecent')"
            @click.stop="removeRecent(agent, p.dirName)"
            @keydown.enter.stop
          >
            <IconClose />
          </button>
          <IconChevronRight class="welcome-recent-go" />
        </div>
      </div>

      <!-- 没有最近记录时回退到原提示 -->
      <p v-else class="welcome-hint">
        {{ t('main.pickProject', { agent: agentLabel(agent) }) }}
      </p>
    </div>
  </div>
</template>
