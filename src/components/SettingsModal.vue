<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { convertFileSrc } from '@tauri-apps/api/core'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { t } from '../i18n'
import { agentLabel } from '../agentMeta'
import {
  codexShowArchivedSessions,
  codexShowInternalSessions,
  lang,
  setCodexShowArchivedSessions,
  setCodexShowInternalSessions,
  setLang,
  setTheme,
  setFontScale,
  applyFontScale,
  fontFamily,
  setFontFamily,
  applyFontFamily,
  setUseExternalTerminal,
  setAutoRestoreTerminalTabs,
  setTerminalApp,
  applyTerminalDefault,
  launchArgs,
  setLaunchArgs,
  theme,
  fontScale,
  useExternalTerminal,
  autoRestoreTerminalTabs,
  terminalApp,
  enabledAgents,
  visibleAgents,
  setAgentEnabled,
  ALL_AGENTS,
  quickOpenTarget,
  setQuickOpenTarget,
  useReclaude,
  setUseReclaude,
  showToolCalls,
  setShowToolCalls,
  exportShowMessageTime,
  setExportShowMessageTime,
  showChatRail,
  setShowChatRail,
  chatRailCount,
  setChatRailCount,
  chatSpacing,
  setChatSpacing,
  backgroundImagePath,
  backgroundImageOpacity,
  backgroundBorderOpacity,
  backgroundIsVideo,
  setBackgroundImagePath,
  setBackgroundImageOpacity,
  setBackgroundBorderOpacity,
  type Lang,
  type Theme,
  type TerminalApp,
  type QuickOpenTarget,
} from '../settings'
import {
  IconClose,
  IconRefresh,
  IconExternalLink,
  IconCheck,
  IconTrash,
  IconChevronDown,
  IconSettings,
  IconSliders,
  IconKeyboard,
  IconDownload,
  IconUpload,
  IconTerminal,
  IconWebhook,
  IconStar,
  IconFileImage,
  IconFolder,
  IconPalette,
  agentIcons,
  terminalIcons,
} from './icons'
import CliEnvironmentCheck from './CliEnvironmentCheck.vue'
import * as api from '../api'
import {
  checkAppUpdate,
  downloadAndInstallUpdate,
  latestVersion,
  openReleasePage,
  relaunchApp,
  updateDownloaded,
  updateDownloading,
  updateInstallError,
  updateProgress,
  updateAvailable,
  updaterUpdate,
} from '../updateCheck'
import {
  refreshTurnHookStatus,
  turnHookStatus,
  turnHookStatusError,
  turnHookStatusLoading,
} from '../turnHookStatus'
import {
  activeDesktopPet,
  desktopPetCatalog,
  desktopPetCatalogError,
  desktopPetCatalogLoading,
  desktopPetCharacter,
  desktopPetEnabled,
  desktopPetSize,
  DESKTOP_PET_MAX_SIZE,
  DESKTOP_PET_MIN_SIZE,
  deleteCustomDesktopPet,
  loadDesktopPetCatalog,
  notifyDesktopPetCharacter,
  notifyDesktopPetSize,
  setDesktopPetEnabled,
  updateDesktopPetWindow,
  type DesktopPetCharacter,
  type DesktopPetDefinition,
} from '../desktopPet'
import DesktopPetFallback from './DesktopPetFallback.vue'
import PetAtlasPlayer from './PetAtlasPlayer.vue'

type SettingsTab = 'general' | 'theme' | 'advanced' | 'hooks' | 'pet' | 'cli' | 'shortcuts' | 'updates'
const SETTINGS_ACTIVE_TAB_KEY = 'settingsActiveTab:v1'

// 左侧导航：图标 + 文案，激活项高亮（参考 Claude 客户端设置面板）。
const navItems = [
  { id: 'general', icon: IconSettings, key: 'settings.tab.general' },
  { id: 'theme', icon: IconPalette, key: 'settings.tab.theme' },
  { id: 'advanced', icon: IconSliders, key: 'settings.tab.advanced' },
  { id: 'hooks', icon: IconWebhook, key: 'settings.tab.hooks' },
  { id: 'pet', icon: IconStar, key: 'settings.tab.desktopPet' },
  { id: 'cli', icon: IconTerminal, key: 'settings.tab.cli' },
  { id: 'shortcuts', icon: IconKeyboard, key: 'settings.tab.shortcuts' },
  { id: 'updates', icon: IconDownload, key: 'settings.tab.updates' },
] as const

const isMac = /Mac/i.test(navigator.platform)
const mod = isMac ? '⌘' : 'Ctrl'
const shift = isMac ? '⇧' : 'Shift'
const opt = isMac ? '⌥' : 'Alt'
const sep = isMac ? '' : '+'
const k = (parts: string[]) => parts.join(sep)
// 分两组展示：全局（应用级，随处可用）/ 会话（作用于当前会话或其 tab）。
const shortcutGroups = [
  {
    title: 'settings.shortcut.groupGlobal',
    items: [
      { key: k([mod, shift, 'F']), label: 'settings.shortcut.globalSearch' },
      { key: k([mod, 'N']), label: 'settings.shortcut.newSession' },
      { key: k([mod, 'T']), label: 'settings.shortcut.newTab' },
      { key: k([mod, 'O']), label: 'settings.shortcut.addFolder' },
      { key: k([mod, 'B']), label: 'settings.shortcut.toggleSidebar' },
      { key: k([mod, shift, 'S']), label: 'settings.shortcut.stats' },
      { key: k([mod, shift, 'T']), label: 'settings.shortcut.trash' },
      { key: k([mod, ',']), label: 'settings.shortcut.settings' },
      { key: k([mod, '/']), label: 'settings.shortcut.shortcuts' },
      { key: 'Esc', label: 'settings.shortcut.escape' },
    ],
  },
  {
    title: 'settings.shortcut.groupSession',
    items: [
      { key: k([mod, 'F']), label: 'settings.shortcut.findInSession' },
      { key: k([mod, 'G']), label: 'settings.shortcut.findNext' },
      { key: k([mod, shift, 'G']), label: 'settings.shortcut.findPrev' },
      { key: k([mod, 'W']), label: 'settings.shortcut.closeTab' },
      { key: k([mod, 'R']), label: 'settings.shortcut.renameTab' },
      { key: k([mod, 'E']), label: 'settings.shortcut.exportSession' },
    ],
  },
  {
    title: 'settings.shortcut.groupChat',
    items: [
      { key: k([mod, 'U']), label: 'settings.shortcut.attachFiles' },
      { key: k([mod, 'J']), label: 'settings.shortcut.sideChat' },
      { key: 'Ctrl+S', label: 'settings.shortcut.stashInput' },
      { key: 'Ctrl+Del', label: 'settings.shortcut.deleteLine' },
      { key: 'Shift+Enter', label: 'settings.shortcut.newline' },
    ],
  },
  {
    title: 'settings.shortcut.groupPanes',
    items: [
      { key: k([mod, 'D']), label: 'settings.shortcut.splitRight' },
      { key: k([mod, shift, 'D']), label: 'settings.shortcut.splitDown' },
      { key: k([mod, shift, 'W']), label: 'settings.shortcut.closePane' },
      { key: `${k([mod, opt])} ←↑↓→`, label: 'settings.shortcut.focusPane' },
    ],
  },
]

const props = defineProps<{ initialTab?: SettingsTab }>()
const emit = defineEmits<{
  close: []
  resetSettings: []
  clearTabs: []
  notify: [message: string, error?: boolean]
}>()

function readLastSettingsTab(): SettingsTab {
  const value = localStorage.getItem(SETTINGS_ACTIVE_TAB_KEY)
  return navItems.some((item) => item.id === value) ? value as SettingsTab : 'general'
}

const activeTab = ref<SettingsTab>(props.initialTab ?? readLastSettingsTab())
// 切换左侧导航时，右侧内容回到顶部（否则会沿用上一个 tab 的滚动位置）。
const bodyEl = ref<HTMLElement>()
watch(activeTab, () => {
  localStorage.setItem(SETTINGS_ACTIVE_TAB_KEY, activeTab.value)
  if (bodyEl.value) bodyEl.value.scrollTop = 0
})

const version = ref('—')
const updateMsg = ref('')
const checking = ref(false)
const installingTurnHooks = ref(false)
const turnHooksMsg = ref('')
const hookOpenError = ref('')
const turnHooksEnabled = computed(() => turnHookStatus.value?.enabled ?? false)
const turnHookAgents = computed(() => {
  const definitions = [
    {
      id: 'claude' as const,
      label: 'Claude Code',
      events: ['UserPromptSubmit', 'Stop', 'StopFailure', 'Notification', 'PermissionRequest'],
    },
    {
      id: 'codex' as const,
      label: 'Codex',
      events: ['UserPromptSubmit', 'Stop', 'PermissionRequest'],
    },
    {
      id: 'agy' as const,
      label: 'AGY · Antigravity CLI',
      events: ['PreInvocation', 'Stop'],
    },
    {
      id: 'grok' as const,
      label: 'Grok Build',
      events: ['UserPromptSubmit', 'Stop', 'StopFailure', 'StopCancelled', 'Notification:idle_prompt', 'Notification:permission_prompt'],
    },
    {
      id: 'kimicode' as const,
      label: 'Kimi Code',
      events: ['TurnStarted', 'Stop', 'StopFailure', 'PermissionRequest', 'Interrupt'],
    },
  ]
  return definitions.map((definition) => {
    const status = turnHookStatus.value?.[definition.id]
    return {
      ...definition,
      icon: agentIcons[definition.id],
      installed: status?.installed ?? false,
      configPath: status?.configPath ?? '',
      trackingEvents: status?.events
        ?? definition.events.map((name) => ({ name, installed: false })),
      hooks: status?.hooks ?? [],
    }
  })
})
const configuredHookFiles = computed(() =>
  turnHookAgents.value.filter((agent) => agent.configPath && agent.hooks.length > 0),
)
const desktopPetBusy = ref(false)
const desktopPetError = ref('')
const codexDesktopPets = computed(() =>
  desktopPetCatalog.value?.pets.filter((pet) => pet.source === 'codex') ?? [],
)
const customDesktopPets = computed(() =>
  desktopPetCatalog.value?.pets.filter((pet) => pet.source === 'custom') ?? [],
)
type DesktopPetCatalogTab = 'codex' | 'custom'
const desktopPetCatalogTab = ref<DesktopPetCatalogTab>(
  desktopPetCharacter.value.startsWith('custom:') ? 'custom' : 'codex',
)
const visibleDesktopPets = computed(() =>
  desktopPetCatalogTab.value === 'codex' ? codexDesktopPets.value : customDesktopPets.value,
)

const desktopPetUrl = (pet: DesktopPetDefinition) => convertFileSrc(pet.spritesheetPath)

async function refreshDesktopPets() {
  desktopPetError.value = ''
  try {
    await loadDesktopPetCatalog()
  } catch (error) {
    desktopPetError.value = t('settings.desktopPet.actionFail', { e: String(error) })
  }
}

async function toggleDesktopPet() {
  const enabled = !desktopPetEnabled.value
  desktopPetBusy.value = true
  desktopPetError.value = ''
  try {
    await updateDesktopPetWindow(enabled)
    setDesktopPetEnabled(enabled)
  } catch (error) {
    desktopPetError.value = t('settings.desktopPet.actionFail', { e: String(error) })
  } finally {
    desktopPetBusy.value = false
  }
}

function onDesktopPetSizeInput(event: Event) {
  void notifyDesktopPetSize(Number((event.target as HTMLInputElement).value))
}

async function chooseDesktopPet(character: DesktopPetCharacter) {
  desktopPetError.value = ''
  desktopPetCatalogTab.value = character.startsWith('custom:') ? 'custom' : 'codex'
  try {
    await notifyDesktopPetCharacter(character)
  } catch (error) {
    desktopPetError.value = t('settings.desktopPet.actionFail', { e: String(error) })
  }
}

async function deleteCustomPet(pet: DesktopPetDefinition) {
  if (!window.confirm(t('settings.desktopPet.deleteConfirm', { name: pet.displayName }))) return

  desktopPetError.value = ''
  const deletedActivePet = desktopPetCharacter.value === pet.key
  try {
    await deleteCustomDesktopPet(pet.id)
    const catalog = await loadDesktopPetCatalog()
    if (deletedActivePet) {
      const fallback = catalog.pets.find((candidate) => candidate.source === 'codex') ?? catalog.pets[0]
      if (fallback) {
        desktopPetCatalogTab.value = fallback.source === 'custom' ? 'custom' : 'codex'
        await notifyDesktopPetCharacter(fallback.key)
      }
    }
  } catch (error) {
    desktopPetError.value = t('settings.desktopPet.deleteFail', {
      name: pet.displayName,
      e: String(error),
    })
  }
}

function openMoreDesktopPets() {
  void api.openUrl('https://petdex.dev/').catch((error) => {
    desktopPetError.value = t('settings.desktopPet.actionFail', { e: String(error) })
  })
}

async function openDesktopPetDirectory() {
  desktopPetError.value = ''
  try {
    const catalog = desktopPetCatalog.value ?? await loadDesktopPetCatalog()
    await api.openPathExternal(catalog.customDirectory)
  } catch (error) {
    desktopPetError.value = t('settings.desktopPet.actionFail', { e: String(error) })
  }
}

watch(activeTab, (tab) => {
  if (tab === 'pet' && !desktopPetCatalog.value && !desktopPetCatalogLoading.value) {
    void refreshDesktopPets()
  }
}, { immediate: true })

async function openHookConfig(path: string) {
  hookOpenError.value = ''
  try {
    await api.openPathExternal(path)
  } catch (error) {
    hookOpenError.value = t('settings.hooks.openFail', { e: String(error) })
  }
}

const reclaudeInstalled = ref(false)
const reclaudeRunning = ref(false)

// custom dropdown state
const langMenuOpen = ref(false)
const themeMenuOpen = ref(false)
const terminalMenuOpen = ref(false)
const langWrapEl = ref<HTMLElement>()
const themeWrapEl = ref<HTMLElement>()
const terminalWrapEl = ref<HTMLElement>()

const isMacOS = /Mac/i.test(navigator.platform)
const availableTerminals = ref<string[]>([])
type TermOpt = { v: TerminalApp; key: string }
const terminalOptions = computed<TermOpt[]>(() => {
  const base: TermOpt[] = [{ v: 'terminal', key: 'settings.terminalApp.terminal' }]
  if (availableTerminals.value.includes('cmux'))
    base.push({ v: 'cmux', key: 'settings.terminalApp.cmux' })
  if (availableTerminals.value.includes('iterm2'))
    base.push({ v: 'iterm2', key: 'settings.terminalApp.iterm2' })
  if (availableTerminals.value.includes('ghostty'))
    base.push({ v: 'ghostty', key: 'settings.terminalApp.ghostty' })
  if (availableTerminals.value.includes('warp'))
    base.push({ v: 'warp', key: 'settings.terminalApp.warp' })
  return base
})
const currentTerminalLabel = computed(() => {
  const o = terminalOptions.value.find(o => o.v === terminalApp.value)
  return o ? t(o.key) : terminalApp.value
})

function pickLang(v: Lang) {
  setLang(v)
  langMenuOpen.value = false
}
function pickTheme(v: Theme) {
  setTheme(v)
  themeMenuOpen.value = false
}
function pickTerminal(v: TerminalApp) {
  setTerminalApp(v)
  terminalMenuOpen.value = false
}
function onDocClick(e: MouseEvent) {
  if (langMenuOpen.value && langWrapEl.value && !langWrapEl.value.contains(e.target as Node))
    langMenuOpen.value = false
  if (themeMenuOpen.value && themeWrapEl.value && !themeWrapEl.value.contains(e.target as Node))
    themeMenuOpen.value = false
  if (terminalMenuOpen.value && terminalWrapEl.value && !terminalWrapEl.value.contains(e.target as Node))
    terminalMenuOpen.value = false
}
onMounted(() => document.addEventListener('click', onDocClick, true))
onUnmounted(() => {
  document.removeEventListener('click', onDocClick, true)
  applyFontScale()
  applyFontFamily()
})

onMounted(async () => {
  try {
    version.value = await api.appVersion()
  } catch {
    /* ignore */
  }
  if (isMacOS) {
    try {
      const detected = await api.detectTerminals()
      availableTerminals.value = detected
      applyTerminalDefault(detected)
    } catch {
      /* ignore */
    }
  }
  if (updateAvailable.value && latestVersion.value) {
    updateMsg.value = t('settings.updateAvailable', {
      v: latestVersion.value,
      cur: version.value,
    })
  }
  try {
    const info = await api.reclaudeInfo()
    reclaudeInstalled.value = info.installed
    reclaudeRunning.value = info.daemonRunning
    if (!info.installed && useReclaude.value) setUseReclaude(false)
  } catch {
    /* ignore */
  }
})

const langOptions: { v: Lang; key: string }[] = [
  { v: 'en', key: 'settings.lang.en' },
  { v: 'zh', key: 'settings.lang.zh' },
  { v: 'zh-TW', key: 'settings.lang.zhTw' },
  { v: 'ja', key: 'settings.lang.ja' },
]
type ThemeOpt = { v: Theme; key: string }
const themeOptions: ThemeOpt[] = [
  { v: 'light', key: 'settings.theme.light' },
  { v: 'dark', key: 'settings.theme.dark' },
  { v: 'system', key: 'settings.theme.system' },
  { v: 'codex', key: 'settings.theme.codex' },
  { v: 'dracula', key: 'settings.theme.dracula' },
]

function onFontSlider(e: Event) {
  setFontScale(Number((e.target as HTMLInputElement).value))
}

function onChatSpacingSlider(e: Event) {
  setChatSpacing(Number((e.target as HTMLInputElement).value))
}

function onChatRailCountSlider(e: Event) {
  setChatRailCount(Number((e.target as HTMLInputElement).value))
}

function onBackgroundImageOpacitySlider(e: Event) {
  setBackgroundImageOpacity(Number((e.target as HTMLInputElement).value))
}

function onBackgroundBorderOpacitySlider(e: Event) {
  setBackgroundBorderOpacity(Number((e.target as HTMLInputElement).value))
}

const backgroundImageError = ref('')
const backgroundMediaSuccess = ref('')
const backgroundMedia = ref<api.BackgroundMedia[]>([])
const backgroundMediaLoading = ref(false)
const backgroundMediaExporting = ref(false)
const backgroundImageUrl = computed(() =>
  backgroundImagePath.value ? convertFileSrc(backgroundImagePath.value) : '',
)
const backgroundImageName = computed(() =>
  (backgroundImagePath.value?.split(/[\\/]/).pop() || '')
    .replace(/^[\da-f]{8}(?:-[\da-f]{4}){3}-[\da-f]{12}--/i, ''),
)
const isSelectedBackgroundMedia = (media: api.BackgroundMedia) => media.path === backgroundImagePath.value
const isBackgroundVideo = (media: api.BackgroundMedia) => media.path.toLowerCase().endsWith('.mp4')

async function refreshBackgroundMedia() {
  if (backgroundMediaLoading.value) return
  backgroundMediaLoading.value = true
  try {
    backgroundMedia.value = await api.listBackgroundMedia()
  } catch (error) {
    backgroundImageError.value = t('settings.backgroundImage.loadFail', { e: String(error) })
  } finally {
    backgroundMediaLoading.value = false
  }
}

async function chooseBackgroundImage() {
  backgroundImageError.value = ''
  backgroundMediaSuccess.value = ''
  try {
    const selected = await openDialog({
      multiple: false,
      filters: [{ name: t('settings.backgroundImage.filter'), extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'avif', 'mp4'] }],
    })
    const path = typeof selected === 'string' ? selected : selected?.[0]
    if (!path) return
    const media = await api.importBackgroundMedia(path)
    setBackgroundImagePath(media.path)
    backgroundMedia.value = [media, ...backgroundMedia.value.filter((item) => item.id !== media.id)]
  } catch (error) {
    backgroundImageError.value = t('settings.backgroundImage.chooseFail', { e: String(error) })
  }
}

async function openBackgroundMediaDirectory() {
  backgroundImageError.value = ''
  backgroundMediaSuccess.value = ''
  try {
    await api.openPathExternal(await api.backgroundMediaDirectory())
  } catch (error) {
    backgroundImageError.value = t('settings.backgroundImage.openDirectoryFail', { e: String(error) })
  }
}

async function exportBackgroundMedia() {
  if (backgroundMediaExporting.value || !backgroundMedia.value.length) return
  backgroundImageError.value = ''
  backgroundMediaSuccess.value = ''
  try {
    const selected = await openDialog({ directory: true, multiple: false })
    const destination = typeof selected === 'string' ? selected : selected?.[0]
    if (!destination) return
    backgroundMediaExporting.value = true
    const result = await api.exportBackgroundMedia(destination)
    const message = t('settings.backgroundImage.exportSuccess', { n: result.count })
    backgroundMediaSuccess.value = message
    emit('notify', message)
    api.openPathExternal(destination).catch(() => {})
  } catch (error) {
    backgroundImageError.value = t('settings.backgroundImage.exportFail', { e: String(error) })
  } finally {
    backgroundMediaExporting.value = false
  }
}

function removeBackgroundImage() {
  backgroundImageError.value = ''
  backgroundMediaSuccess.value = ''
  setBackgroundImagePath(null)
}

function selectBackgroundMedia(media: api.BackgroundMedia) {
  backgroundImageError.value = ''
  backgroundMediaSuccess.value = ''
  setBackgroundImagePath(media.path)
}

async function deleteBackgroundMedia(media: api.BackgroundMedia) {
  backgroundImageError.value = ''
  backgroundMediaSuccess.value = ''
  try {
    await api.deleteBackgroundMedia(media.id)
    backgroundMedia.value = backgroundMedia.value.filter((item) => item.id !== media.id)
    if (isSelectedBackgroundMedia(media)) setBackgroundImagePath(null)
  } catch (error) {
    backgroundImageError.value = t('settings.backgroundImage.deleteFail', { e: String(error) })
  }
}

watch(activeTab, (tab) => {
  if (tab === 'theme') void refreshBackgroundMedia()
}, { immediate: true })

function onFontFamilyInput(e: Event) {
  setFontFamily((e.target as HTMLInputElement).value.trim())
}

type QuickOpenOpt = { v: QuickOpenTarget; key: string }
const quickOpenOptions: QuickOpenOpt[] = [
  { v: 'session', key: 'settings.quickOpen.session' },
  { v: 'terminal', key: 'settings.quickOpen.terminal' },
  { v: 'chat', key: 'settings.quickOpen.chat' },
]

const currentLangLabel = computed(() => {
  const o = langOptions.find(o => o.v === lang.value)
  return o ? t(o.key) : lang.value
})
const currentThemeLabel = computed(() => {
  const o = themeOptions.find(o => o.v === theme.value)
  return o ? t(o.key) : theme.value
})

async function doCheck() {
  if (checking.value) return
  checking.value = true
  updateMsg.value = t('settings.checking')
  try {
    const r = await checkAppUpdate()
    updateMsg.value = r.hasUpdate
      ? t('settings.updateAvailable', { v: r.latest, cur: r.current })
      : t('settings.upToDate', { v: r.current })
  } catch (e) {
    updateMsg.value = t('settings.updateFail', { e: String(e) })
  } finally {
    checking.value = false
  }
}

async function installUpdate() {
  if (updateDownloading.value) return
  updateMsg.value = t('settings.updateDownloading')
  try {
    await downloadAndInstallUpdate()
    updateMsg.value = t('settings.updateReady')
  } catch (e) {
    updateInstallError.value = String(e)
    updateMsg.value = ''
  }
}

async function installTurnHooks() {
  if (installingTurnHooks.value || turnHookStatusLoading.value || turnHooksEnabled.value) return
  installingTurnHooks.value = true
  turnHooksMsg.value = t('settings.turnStatus.installing')
  try {
    await api.installTurnHooks()
    await refreshTurnHookStatus()
    turnHooksMsg.value = turnHookStatusError.value
      ? t('settings.turnStatus.installFail', { e: turnHookStatusError.value })
      : t('settings.turnStatus.installed')
  } catch (e) {
    turnHooksMsg.value = t('settings.turnStatus.installFail', { e: String(e) })
  } finally {
    installingTurnHooks.value = false
  }
}

async function refreshTurnHooks() {
  if (installingTurnHooks.value || turnHookStatusLoading.value) return
  turnHooksMsg.value = ''
  await refreshTurnHookStatus()
}
</script>

<template>
  <div class="app-overlay">
    <div class="modal settings-modal">
      <!-- 左侧导航：分组标题 + 图标项，激活项高亮（参考 Claude 客户端设置面板） -->
      <nav class="set-nav">
        <div class="set-nav-group-label">{{ t('settings.title') }}</div>
        <button
          v-for="n in navItems"
          :key="n.id"
          class="set-nav-item"
          :class="{ active: activeTab === n.id }"
          @click="activeTab = n.id"
        >
          <component :is="n.icon" class="set-nav-icon" />
          <span>{{ t(n.key) }}</span>
          <span v-if="n.id === 'updates' && updateAvailable" class="set-nav-dot" aria-hidden="true" />
        </button>
        <!-- 左栏底部：当前 app 版本号（margin-top:auto 顶到底） -->
        <div class="set-nav-version">v{{ version }}</div>
      </nav>

      <button
        class="modal-close"
        v-tooltip="t('common.close')"
        @click="emit('close')"
      >
        <IconClose />
      </button>

      <div ref="bodyEl" class="set-body">
        <template v-if="activeTab === 'general'">
          <!-- 通用：语言及应用级偏好。外观设置独立放到 Appearance 页面。 -->
          <div class="set-group">
            <div class="set-row">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.section.lang') }}</div>
              </div>
              <div ref="langWrapEl" class="set-dropdown-wrap set-row-control">
                <button
                  class="set-dropdown-btn"
                  :class="{ active: langMenuOpen }"
                  @click.stop="langMenuOpen = !langMenuOpen; themeMenuOpen = false"
                >
                  <span>{{ currentLangLabel }}</span>
                  <IconChevronDown class="set-dropdown-chev" />
                </button>
                <div v-if="langMenuOpen" class="set-dropdown-menu" role="menu">
                  <button
                    v-for="o in langOptions"
                    :key="o.v"
                    class="set-dropdown-item"
                    :class="{ active: lang === o.v }"
                    role="menuitem"
                    @click.stop="pickLang(o.v)"
                  >
                    <span class="set-dropdown-check"><IconCheck v-if="lang === o.v" /></span>
                    <span>{{ t(o.key) }}</span>
                  </button>
                </div>
              </div>
            </div>

          </div>

          <!-- Agents 显隐 —— 分组标题 + desc 直接显示，下面是每个 agent 的开关 -->
          <div class="set-group">
            <div class="set-group-head">
              <div class="set-group-title">{{ t('settings.section.agents') }}</div>
              <p class="set-group-desc">{{ t('settings.agentsVisibilityDesc') }}</p>
            </div>
            <label
              v-for="a in ALL_AGENTS"
              :key="a"
              class="set-row set-row-clickable"
              :class="{ disabled: enabledAgents[a] && visibleAgents.length === 1 }"
              @click.prevent="setAgentEnabled(a, !enabledAgents[a])"
            >
              <div class="set-row-text">
                <div class="set-row-title set-row-title-icon">
                  <component :is="agentIcons[a]" class="set-agent-toggle-icon" />
                  {{ agentLabel(a) }}
                </div>
              </div>
              <span class="set-toggle-track set-row-control" :class="{ on: enabledAgents[a] }">
                <span class="set-toggle-thumb" />
              </span>
            </label>
          </div>

          <!-- Chat 展示：过程性工具调用默认不占会话空间；文件改动和需要用户确认的卡片始终显示。 -->
          <div class="set-group">
            <div class="set-group-head">
              <div class="set-group-title">{{ t('settings.section.chat') }}</div>
            </div>
            <label class="set-row set-row-clickable" @click.prevent="setShowToolCalls(!showToolCalls)">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.showToolCalls') }}</div>
                <p class="set-row-desc">{{ t('settings.showToolCallsDesc') }}</p>
              </div>
              <span class="set-toggle-track set-row-control" :class="{ on: showToolCalls }">
                <span class="set-toggle-thumb" />
              </span>
            </label>
            <label class="set-row set-row-clickable" @click.prevent="setExportShowMessageTime(!exportShowMessageTime)">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.exportShowMessageTime') }}</div>
                <p class="set-row-desc">{{ t('settings.exportShowMessageTimeDesc') }}</p>
              </div>
              <span class="set-toggle-track set-row-control" :class="{ on: exportShowMessageTime }">
                <span class="set-toggle-thumb" />
              </span>
            </label>
            <label class="set-row set-row-clickable" @click.prevent="setShowChatRail(!showChatRail)">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.showChatRail') }}</div>
                <p class="set-row-desc">{{ t('settings.showChatRailDesc') }}</p>
              </div>
              <span class="set-toggle-track set-row-control" :class="{ on: showChatRail }">
                <span class="set-toggle-thumb" />
              </span>
            </label>
            <div class="set-row set-row-nosep">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.chatRailCount') }}</div>
                <p class="set-row-desc">{{ t('settings.chatRailCountDesc') }}</p>
              </div>
              <div class="set-font-slider set-row-control">
                <span class="set-slider-endpoint">21</span>
                <input
                  data-chat-rail-count-slider
                  type="range" min="21" max="71" step="1"
                  :value="chatRailCount"
                  :aria-label="t('settings.chatRailCount')"
                  @input="onChatRailCountSlider"
                  class="set-slider"
                >
                <span class="set-font-value">{{ chatRailCount }}</span>
              </div>
            </div>
            <div class="set-row">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.chatSpacing') }}</div>
                <p class="set-row-desc">{{ t('settings.chatSpacingDesc') }}</p>
              </div>
              <div class="set-font-slider set-row-control">
                <span class="set-slider-endpoint">{{ t('settings.chatSpacingCompact') }}</span>
                <input
                  data-chat-spacing-slider
                  type="range" min="30" max="150" step="2"
                  :value="chatSpacing"
                  :aria-label="t('settings.chatSpacing')"
                  @input="onChatSpacingSlider"
                  class="set-slider"
                >
                <span class="set-font-value">{{ chatSpacing }}%</span>
              </div>
            </div>
          </div>

          <!-- 数据 -->
          <div class="set-group">
            <div class="set-row">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.section.tabs') }}</div>
                <p class="set-row-desc">{{ t('settings.clearTabsDesc') }}</p>
              </div>
              <button class="btn danger set-row-control" @click="emit('clearTabs')">
                {{ t('settings.clearTabs') }}
              </button>
            </div>
          </div>

          <div class="set-group">
            <div class="set-row">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.resetDefaults') }}</div>
                <p class="set-row-desc">{{ t('settings.resetDefaultsDesc') }}</p>
              </div>
              <button
                class="btn danger set-row-control"
                data-reset-settings
                @click="emit('resetSettings')"
              >
                {{ t('settings.resetDefaults') }}
              </button>
            </div>
          </div>
        </template>

        <template v-else-if="activeTab === 'theme'">
          <div class="set-group set-appearance-theme">
            <div class="set-row">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.section.theme') }}</div>
              </div>
              <div ref="themeWrapEl" class="set-dropdown-wrap set-row-control">
                <button
                  class="set-dropdown-btn"
                  :class="{ active: themeMenuOpen }"
                  @click.stop="themeMenuOpen = !themeMenuOpen; langMenuOpen = false"
                >
                  <span class="theme-swatch theme-swatch-sm" :class="`theme-swatch-${theme}`">Aa</span>
                  <span>{{ currentThemeLabel }}</span>
                  <IconChevronDown class="set-dropdown-chev" />
                </button>
                <div v-if="themeMenuOpen" class="set-dropdown-menu" role="menu">
                  <button
                    v-for="o in themeOptions"
                    :key="o.v"
                    class="set-dropdown-item"
                    :class="{ active: theme === o.v }"
                    role="menuitem"
                    @click.stop="pickTheme(o.v)"
                  >
                    <span class="set-dropdown-check"><IconCheck v-if="theme === o.v" /></span>
                    <span class="theme-swatch theme-swatch-sm" :class="`theme-swatch-${o.v}`">Aa</span>
                    <span>{{ t(o.key) }}</span>
                  </button>
                </div>
              </div>
            </div>
          </div>

          <div class="set-group set-appearance-font">
            <div class="set-row">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.section.fontSize') }}</div>
              </div>
              <div class="set-font-slider set-row-control">
                <span class="set-font-label set-font-label-sm">A</span>
                <input
                  type="range" min="12" max="18" step="1"
                  :value="fontScale"
                  @input="onFontSlider"
                  class="set-slider"
                >
                <span class="set-font-label set-font-label-lg">A</span>
                <span class="set-font-value">{{ fontScale }}px</span>
              </div>
            </div>
            <div class="set-font-preview" :style="{ fontSize: fontScale + 'px' }">
              {{ t('settings.fontPreview') }}
            </div>

            <div class="set-row">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.section.fontFamily') }}</div>
              </div>
              <div class="set-row-control">
                <input
                  type="text"
                  class="set-input"
                  :value="fontFamily"
                  :placeholder="t('settings.fontFamilyPlaceholder')"
                  @input="onFontFamilyInput"
                >
              </div>
            </div>
            <div class="set-font-preview" :style="{ fontSize: fontScale + 'px', fontFamily: fontFamily || undefined }">
              {{ t('settings.fontPreview') }}
            </div>
          </div>

          <div class="set-group set-appearance-background">
            <div class="set-row set-background-row">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.section.backgroundImage') }}</div>
                <p class="set-row-desc">{{ t('settings.backgroundImageDesc') }}</p>
              </div>
              <div v-if="backgroundImagePath" class="set-background-selected set-row-control">
                <video
                  v-if="backgroundIsVideo"
                  class="set-background-thumb"
                  :src="backgroundImageUrl"
                  autoplay
                  loop
                  muted
                  playsinline
                  preload="metadata"
                />
                <img v-else class="set-background-thumb" :src="backgroundImageUrl" alt="">
                <span class="set-background-name" :title="backgroundImagePath">{{ backgroundImageName }}</span>
                <button
                  class="btn set-background-change"
                  type="button"
                  data-background-image-choose
                  @click="chooseBackgroundImage"
                >
                  {{ t('settings.backgroundImage.change') }}
                </button>
                <button
                  class="set-background-remove"
                  type="button"
                  :aria-label="t('settings.backgroundImage.remove')"
                  v-tooltip="t('settings.backgroundImage.remove')"
                  @click="removeBackgroundImage"
                >
                  <IconTrash />
                </button>
              </div>
              <button
                v-else
                class="btn set-row-control"
                type="button"
                data-background-image-choose
                @click="chooseBackgroundImage"
              >
                <IconFileImage />
                {{ t('settings.backgroundImage.choose') }}
              </button>
            </div>
            <div v-if="backgroundImagePath" class="set-row set-background-opacity-row">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.backgroundImage.opacity') }}</div>
              </div>
              <div class="set-font-slider set-row-control">
                <input
                  data-background-opacity-slider
                  type="range" min="0" max="100" step="1"
                  :value="backgroundImageOpacity"
                  :aria-label="t('settings.backgroundImage.opacity')"
                  @input="onBackgroundImageOpacitySlider"
                  class="set-slider"
                >
                <span class="set-font-value">{{ backgroundImageOpacity }}%</span>
              </div>
            </div>
            <div v-if="backgroundImagePath" class="set-row set-background-opacity-row">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.backgroundImage.borderOpacity') }}</div>
              </div>
              <div class="set-font-slider set-row-control">
                <input
                  data-background-border-opacity-slider
                  type="range" min="0" max="80" step="1"
                  :value="backgroundBorderOpacity"
                  :aria-label="t('settings.backgroundImage.borderOpacity')"
                  @input="onBackgroundBorderOpacitySlider"
                  class="set-slider"
                >
                <span class="set-font-value">{{ backgroundBorderOpacity }}%</span>
              </div>
            </div>
            <div class="set-background-library">
              <div class="set-background-library-head">
                <span class="set-background-library-title">{{ t('settings.backgroundImage.library') }}</span>
                <button
                  class="set-background-library-export"
                  type="button"
                  data-background-media-export
                  :disabled="backgroundMediaExporting || !backgroundMedia.length"
                  :aria-label="t('settings.backgroundImage.export')"
                  v-tooltip="t('settings.backgroundImage.export')"
                  @click="exportBackgroundMedia"
                >
                  <IconUpload />
                  <span>{{ t('settings.backgroundImage.export') }}</span>
                </button>
                <button
                  class="set-background-library-open"
                  type="button"
                  data-background-media-open-folder
                  :aria-label="t('settings.backgroundImage.openDirectory')"
                  v-tooltip="t('settings.backgroundImage.openDirectory')"
                  @click="openBackgroundMediaDirectory"
                >
                  <IconFolder />
                  {{ t('settings.backgroundImage.openDirectory') }}
                </button>
                <button
                  class="set-background-library-refresh"
                  type="button"
                  :class="{ spinning: backgroundMediaLoading }"
                  :disabled="backgroundMediaLoading"
                  :aria-label="t('settings.desktopPet.refresh')"
                  v-tooltip="t('settings.desktopPet.refresh')"
                  @click="refreshBackgroundMedia"
                >
                  <IconRefresh />
                </button>
              </div>
              <div v-if="backgroundMediaLoading && backgroundMedia.length === 0" class="set-background-library-empty">
                {{ t('common.loading') }}
              </div>
              <div v-else-if="backgroundMedia.length" class="set-background-media-grid">
                <div
                  v-for="media in backgroundMedia"
                  :key="media.id"
                  class="set-background-media-item"
                  :class="{ active: isSelectedBackgroundMedia(media) }"
                >
                  <button
                    class="set-background-media-select"
                    type="button"
                    data-background-media-select
                    :aria-pressed="isSelectedBackgroundMedia(media)"
                    :title="media.name"
                    @click="selectBackgroundMedia(media)"
                  >
                    <video
                      v-if="isBackgroundVideo(media)"
                      :src="convertFileSrc(media.path)"
                      autoplay
                      loop
                      muted
                      playsinline
                      preload="metadata"
                    />
                    <img v-else :src="convertFileSrc(media.path)" alt="">
                    <span v-if="isBackgroundVideo(media)" class="set-background-media-type">MP4</span>
                    <span v-if="isSelectedBackgroundMedia(media)" class="set-background-media-check"><IconCheck /></span>
                    <span class="set-background-media-name">{{ media.name }}</span>
                  </button>
                  <button
                    class="set-background-media-delete"
                    type="button"
                    data-background-media-delete
                    :aria-label="t('settings.backgroundImage.delete')"
                    v-tooltip="t('settings.backgroundImage.delete')"
                    @click.stop="deleteBackgroundMedia(media)"
                  >
                    <IconTrash />
                  </button>
                </div>
              </div>
              <div v-else class="set-background-library-empty">{{ t('settings.backgroundImage.empty') }}</div>
            </div>
            <p v-if="backgroundImageError" class="set-background-error">{{ backgroundImageError }}</p>
            <p v-if="backgroundMediaSuccess" class="set-background-success">{{ backgroundMediaSuccess }}</p>
          </div>
        </template>

        <template v-else-if="activeTab === 'advanced'">
          <!-- 终端 -->
          <div class="set-group">
            <div class="set-group-head">
              <div class="set-group-title">{{ t('settings.section.terminal') }}</div>
            </div>
            <label class="set-row set-row-clickable" @click.prevent="setAutoRestoreTerminalTabs(!autoRestoreTerminalTabs)">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.autoRestoreTerminalTabs') }}</div>
                <p class="set-row-desc">{{ t('settings.autoRestoreTerminalTabsDesc') }}</p>
              </div>
              <span class="set-toggle-track set-row-control" :class="{ on: autoRestoreTerminalTabs }">
                <span class="set-toggle-thumb" />
              </span>
            </label>

            <label class="set-row set-row-clickable" @click.prevent="setUseExternalTerminal(!useExternalTerminal)">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.useExternalTerminal') }}</div>
                <p class="set-row-desc">{{ t('settings.terminalDesc') }}</p>
              </div>
              <span class="set-toggle-track set-row-control" :class="{ on: useExternalTerminal }">
                <span class="set-toggle-thumb" />
              </span>
            </label>

            <div v-if="useExternalTerminal && isMacOS && terminalOptions.length > 1" class="set-row set-row-nosep">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.terminalApp.label') }}</div>
              </div>
              <div ref="terminalWrapEl" class="set-dropdown-wrap set-row-control">
                <button
                  class="set-dropdown-btn"
                  :class="{ active: terminalMenuOpen }"
                  @click.stop="terminalMenuOpen = !terminalMenuOpen; langMenuOpen = false; themeMenuOpen = false"
                >
                  <component :is="terminalIcons[terminalApp]" class="set-terminal-icon" />
                  <span>{{ currentTerminalLabel }}</span>
                  <IconChevronDown class="set-dropdown-chev" />
                </button>
                <div v-if="terminalMenuOpen" class="set-dropdown-menu" role="menu">
                  <button
                    v-for="o in terminalOptions"
                    :key="o.v"
                    class="set-dropdown-item"
                    :class="{ active: terminalApp === o.v }"
                    role="menuitem"
                    @click.stop="pickTerminal(o.v)"
                  >
                    <span class="set-dropdown-check"><IconCheck v-if="terminalApp === o.v" /></span>
                    <component :is="terminalIcons[o.v]" class="set-terminal-icon" />
                    <span>{{ t(o.key) }}</span>
                  </button>
                </div>
              </div>
            </div>
          </div>

          <!-- 双击 / 新建快捷键默认打开什么 -->
          <div class="set-group">
            <div class="set-row">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.section.quickOpen') }}</div>
                <p class="set-row-desc">{{ t('settings.quickOpenDesc') }}</p>
              </div>
              <div class="set-segment set-row-control">
                <button
                  v-for="o in quickOpenOptions"
                  :key="o.v"
                  class="set-segment-btn"
                  :class="{ active: quickOpenTarget === o.v }"
                  @click="setQuickOpenTarget(o.v)"
                >
                  {{ t(o.key) }}
                </button>
              </div>
            </div>
          </div>

          <!-- 启动参数 -->
          <div class="set-group">
            <div class="set-group-head">
              <div class="set-group-title">{{ t('settings.launchArgs') }}</div>
              <p class="set-group-desc">{{ t('settings.launchArgsDesc') }}</p>
            </div>
            <div class="set-launch-args">
              <div class="set-launch-args-row" v-for="a in (['claude', 'codex', 'grok', 'kimicode', 'agy', 'opencode'] as const)" :key="a">
                <component :is="agentIcons[a]" class="set-launch-args-icon" />
                <input
                  class="set-launch-args-input"
                  :value="launchArgs[a]"
                  @input="setLaunchArgs(a, ($event.target as HTMLInputElement).value)"
                  :placeholder="{ claude: '--dangerously-skip-permissions', codex: '--yolo', grok: '--yolo', kimicode: '', agy: '--dangerously-skip-permissions', opencode: '--auto' }[a]"
                  spellcheck="false"
                />
                <button
                  v-if="!launchArgs[a]"
                  class="set-launch-args-fill"
                  v-tooltip="t('settings.launchArgsFill')"
                  @click="setLaunchArgs(a, { claude: '--dangerously-skip-permissions', codex: '--yolo', grok: '--yolo', kimicode: '', agy: '--dangerously-skip-permissions', opencode: '--auto' }[a])"
                >↵</button>
              </div>
            </div>
          </div>

          <!-- Codex -->
          <div class="set-group">
            <div class="set-group-head">
              <div class="set-group-title">Codex</div>
              <p class="set-group-desc">{{ t('settings.codexVisibilityDesc') }}</p>
            </div>
            <label class="set-row set-row-clickable" @click.prevent="setCodexShowInternalSessions(!codexShowInternalSessions)">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.codex.showInternal') }}</div>
              </div>
              <span class="set-toggle-track set-row-control" :class="{ on: codexShowInternalSessions }">
                <span class="set-toggle-thumb" />
              </span>
            </label>
            <label class="set-row set-row-clickable" @click.prevent="setCodexShowArchivedSessions(!codexShowArchivedSessions)">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.codex.showArchived') }}</div>
              </div>
              <span class="set-toggle-track set-row-control" :class="{ on: codexShowArchivedSessions }">
                <span class="set-toggle-thumb" />
              </span>
            </label>
          </div>

          <!-- ReClaude -->
          <div v-if="reclaudeInstalled" class="set-group">
            <div class="set-group-head">
              <div class="set-group-title">{{ t('settings.section.reclaude') }}</div>
              <p class="set-group-desc">{{ t('settings.reclaude.desc') }}</p>
            </div>
            <label class="set-row set-row-clickable" @click.prevent="setUseReclaude(!useReclaude)">
              <div class="set-row-text">
                <div class="set-row-title">{{ t('settings.reclaude.toggle') }}</div>
                <p v-if="useReclaude && !reclaudeRunning" class="set-row-desc" style="color:var(--danger)">{{ t('settings.reclaude.notRunning') }}</p>
              </div>
              <span class="set-toggle-track set-row-control" :class="{ on: useReclaude }">
                <span class="set-toggle-thumb" />
              </span>
            </label>
          </div>

        </template>

        <template v-else-if="activeTab === 'hooks'">
          <div class="set-hooks-head">
            <h2 class="set-hooks-title">{{ t('settings.hooks.title') }}</h2>
            <p class="set-hooks-desc">{{ t('settings.hooks.desc') }}</p>
          </div>

          <section class="set-hook-tracking-card">
            <div class="set-hook-tracking-head">
              <span class="set-hook-tracking-icon"><IconWebhook /></span>
              <div class="set-hook-tracking-info">
                <div class="set-hook-tracking-title">{{ t('settings.turnStatus.categoryTitle') }}</div>
                <p class="set-hook-tracking-desc">{{ t('settings.turnStatus.desc') }}</p>
              </div>
              <span class="set-hooks-overall" :class="{ enabled: turnHooksEnabled }">
                <span class="set-hooks-overall-dot" />
                {{ turnHookStatusLoading
                  ? t('settings.turnStatus.checking')
                  : turnHooksEnabled
                    ? t('settings.turnStatus.enabled')
                    : t('settings.turnStatus.notEnabled') }}
              </span>
            </div>

            <div class="set-hook-tracking-agents">
              <div
                v-for="hookAgent in turnHookAgents"
                :key="hookAgent.id"
                class="set-hook-tracking-agent"
                :class="{ enabled: hookAgent.installed }"
              >
                <component :is="hookAgent.icon" />
                <span>{{ hookAgent.label }}</span>
                <span class="set-hook-tracking-count">
                  {{ hookAgent.trackingEvents.filter(event => event.installed).length }}/{{ hookAgent.trackingEvents.length }}
                </span>
              </div>
            </div>

            <div class="set-hooks-action">
              <div class="set-hooks-action-text">
                <div class="set-hooks-action-title">
                  {{ turnHooksEnabled
                    ? t('settings.turnStatus.readyTitle')
                    : t('settings.turnStatus.actionTitle') }}
                </div>
                <p class="set-hooks-action-desc" :class="{ error: turnHookStatusError }">
                  {{ turnHookStatusError
                    ? t('settings.turnStatus.detectFail', { e: turnHookStatusError })
                    : turnHooksEnabled
                      ? t('settings.turnStatus.readyDesc')
                      : t('settings.turnStatus.actionDesc') }}
                </p>
                <p v-if="turnHooksMsg" class="set-hooks-action-desc" :class="{ error: turnHookStatusError }">
                  {{ turnHooksMsg }}
                </p>
              </div>
              <button
                class="btn primary set-hooks-enable"
                :class="{ enabled: turnHooksEnabled }"
                :disabled="installingTurnHooks || turnHookStatusLoading || turnHooksEnabled"
                @click="installTurnHooks"
              >
                <IconCheck v-if="turnHooksEnabled" />
                {{ installingTurnHooks
                  ? t('settings.turnStatus.installing')
                  : turnHookStatusLoading
                    ? t('settings.turnStatus.checking')
                    : turnHooksEnabled
                      ? t('settings.turnStatus.enabled')
                      : t('settings.turnStatus.install') }}
              </button>
            </div>
          </section>

          <div class="set-hook-list-head">
            <div class="set-hook-list-heading">
              <div>
                <div class="set-hook-list-title">{{ t('settings.hooks.configuredTitle') }}</div>
                <p class="set-hook-list-desc">{{ t('settings.hooks.configuredDesc') }}</p>
              </div>
              <div class="set-hook-list-actions">
                <span class="set-hook-file-count">
                  {{ t('settings.hooks.filesCount', { n: configuredHookFiles.length }) }}
                </span>
                <button
                  type="button"
                  class="set-hook-list-refresh"
                  :class="{ spinning: turnHookStatusLoading }"
                  :disabled="turnHookStatusLoading || installingTurnHooks"
                  :aria-label="t('settings.hooks.refresh')"
                  v-tooltip="t('settings.hooks.refresh')"
                  @click="refreshTurnHooks"
                >
                  <IconRefresh />
                </button>
              </div>
            </div>
            <p v-if="hookOpenError" class="set-hook-list-error">{{ hookOpenError }}</p>
          </div>

          <div class="set-hook-files">
            <button
              v-for="hookAgent in configuredHookFiles"
              :key="hookAgent.id"
              type="button"
              class="set-hook-file"
              :title="hookAgent.configPath"
              @click="openHookConfig(hookAgent.configPath)"
            >
              <span class="set-hook-agent-icon"><component :is="hookAgent.icon" /></span>
              <div class="set-hook-agent-info">
                <div class="set-hook-agent-name">{{ hookAgent.label }}</div>
                <div class="set-hook-config-path">
                  {{ hookAgent.configPath }}
                </div>
              </div>
              <span class="set-hook-file-hooks">{{ t('settings.hooks.count', { n: hookAgent.hooks.length }) }}</span>
              <span class="set-hook-file-open">
                {{ t('settings.hooks.open') }}
                <IconExternalLink />
              </span>
            </button>
            <div v-if="!configuredHookFiles.length" class="set-hook-files-empty">
              {{ turnHookStatusLoading ? t('settings.turnStatus.checking') : t('settings.hooks.empty') }}
            </div>
          </div>
        </template>

        <template v-else-if="activeTab === 'pet'">
          <div class="set-hooks-head">
            <h2 class="set-hooks-title">{{ t('settings.desktopPet.title') }}</h2>
            <p class="set-hooks-desc">{{ t('settings.desktopPet.desc') }}</p>
          </div>

          <section class="set-desktop-pet-card standalone" :class="{ enabled: desktopPetEnabled }">
            <div class="set-desktop-pet-head">
              <span class="set-desktop-pet-preview">
                <PetAtlasPlayer
                  v-if="activeDesktopPet"
                  :src="desktopPetUrl(activeDesktopPet)"
                  state="idle"
                  :sprite-version-number="activeDesktopPet.spriteVersionNumber"
                  :label="activeDesktopPet.displayName"
                  paused
                />
                <DesktopPetFallback
                  v-else
                  state="idle"
                  label="Codex pet"
                  paused
                />
              </span>
              <div class="set-desktop-pet-info">
                <div class="set-desktop-pet-title">{{ t('settings.desktopPet.switchTitle') }}</div>
                <p class="set-desktop-pet-desc">{{ t('settings.desktopPet.switchDesc') }}</p>
              </div>
              <button
                type="button"
                class="set-desktop-pet-toggle"
                :class="{ on: desktopPetEnabled }"
                :disabled="desktopPetBusy"
                :aria-label="t('settings.desktopPet.enable')"
                :aria-pressed="desktopPetEnabled"
                @click="toggleDesktopPet"
              >
                <span />
              </button>
            </div>

            <div class="set-desktop-pet-size">
              <div>
                <strong>{{ t('settings.desktopPet.size') }}</strong>
                <span>{{ desktopPetSize }} px</span>
              </div>
              <input
                type="range"
                :min="DESKTOP_PET_MIN_SIZE"
                :max="DESKTOP_PET_MAX_SIZE"
                :value="desktopPetSize"
                :aria-label="t('settings.desktopPet.size')"
                @input="onDesktopPetSizeInput"
              />
            </div>

            <div class="set-desktop-pet-choice-head">
              <div class="set-desktop-pet-choice-label">{{ t('settings.desktopPet.appearance') }}</div>
              <button
                type="button"
                class="set-desktop-pet-tool"
                :disabled="desktopPetCatalogLoading"
                @click="refreshDesktopPets"
              >
                <IconRefresh />
                {{ t('settings.desktopPet.refresh') }}
              </button>
            </div>

            <div v-if="desktopPetCatalogLoading" class="set-desktop-pet-empty">
              {{ t('settings.desktopPet.loadingPets') }}
            </div>
            <div v-else class="set-desktop-pet-picker">
              <div class="set-desktop-pet-tabs" role="tablist" :aria-label="t('settings.desktopPet.appearance')">
                <button
                  id="desktop-pet-codex-tab"
                  type="button"
                  role="tab"
                  data-desktop-pet-tab="codex"
                  :class="{ active: desktopPetCatalogTab === 'codex' }"
                  :aria-selected="desktopPetCatalogTab === 'codex'"
                  aria-controls="desktop-pet-catalog-panel"
                  @click="desktopPetCatalogTab = 'codex'"
                >
                  <span>{{ t('settings.desktopPet.codexPets') }}</span>
                  <small>{{ codexDesktopPets.length }}</small>
                </button>
                <button
                  id="desktop-pet-custom-tab"
                  type="button"
                  role="tab"
                  data-desktop-pet-tab="custom"
                  :class="{ active: desktopPetCatalogTab === 'custom' }"
                  :aria-selected="desktopPetCatalogTab === 'custom'"
                  aria-controls="desktop-pet-catalog-panel"
                  @click="desktopPetCatalogTab = 'custom'"
                >
                  <span>{{ t('settings.desktopPet.customPets') }}</span>
                  <small>{{ customDesktopPets.length }}</small>
                </button>
              </div>

              <div
                id="desktop-pet-catalog-panel"
                class="set-desktop-pet-tabpanel"
                role="tabpanel"
                :aria-labelledby="`desktop-pet-${desktopPetCatalogTab}-tab`"
              >
                <div v-if="desktopPetCatalogTab === 'custom'" class="set-desktop-pet-panel-tools">
                  <button type="button" class="set-desktop-pet-tool" @click="openMoreDesktopPets">
                    <IconExternalLink />
                    {{ t('settings.desktopPet.downloadMore') }}
                  </button>
                  <button type="button" class="set-desktop-pet-tool" @click="openDesktopPetDirectory">
                    <IconExternalLink />
                    {{ t('settings.desktopPet.openDirectory') }}
                  </button>
                </div>

                <div v-if="visibleDesktopPets.length" class="set-desktop-pet-list">
                  <div
                    v-for="pet in visibleDesktopPets"
                    :key="pet.key"
                    class="set-desktop-pet-character"
                    :class="{ active: desktopPetCharacter === pet.key }"
                  >
                    <button
                      type="button"
                      class="set-desktop-pet-character-select"
                      :aria-label="pet.displayName"
                      :aria-pressed="desktopPetCharacter === pet.key"
                      @click="chooseDesktopPet(pet.key)"
                    >
                      <PetAtlasPlayer
                        class="set-desktop-pet-character-art"
                        :src="desktopPetUrl(pet)"
                        state="idle"
                        :sprite-version-number="pet.spriteVersionNumber"
                        :label="pet.displayName"
                        paused
                      />
                      <span
                        class="set-desktop-pet-character-copy"
                        v-tooltip:bottom="pet.description ?? ''"
                      >
                        <strong>{{ pet.displayName }}</strong>
                        <small v-if="pet.description">{{ pet.description }}</small>
                      </span>
                      <IconCheck
                        v-if="desktopPetCharacter === pet.key"
                        class="set-desktop-pet-character-check"
                      />
                    </button>
                    <button
                      v-if="desktopPetCatalogTab === 'custom'"
                      type="button"
                      class="set-desktop-pet-character-delete"
                      :aria-label="t('settings.desktopPet.delete', { name: pet.displayName })"
                      :title="t('settings.desktopPet.delete', { name: pet.displayName })"
                      @click="deleteCustomPet(pet)"
                    >
                      <IconTrash />
                    </button>
                  </div>
                </div>

                <p v-else class="set-desktop-pet-empty">
                  {{ desktopPetCatalogTab === 'codex'
                    ? t('settings.desktopPet.codexMissing')
                    : t('settings.desktopPet.customEmpty') }}
                </p>
                <code
                  v-if="desktopPetCatalogTab === 'custom' && desktopPetCatalog?.customDirectory"
                  class="set-desktop-pet-path"
                >
                  {{ desktopPetCatalog.customDirectory }}
                </code>
              </div>
            </div>
            <p v-if="desktopPetError || desktopPetCatalogError" class="set-desktop-pet-error">
              {{ desktopPetError || desktopPetCatalogError }}
            </p>
          </section>
        </template>

        <template v-else-if="activeTab === 'cli'">
          <CliEnvironmentCheck />
        </template>

        <template v-else-if="activeTab === 'updates'">
          <div class="set-group">
            <!-- 版本/更新状态卡片：标题 + 副标题 + 单个主操作按钮，不再堆一排按钮 -->
            <div class="set-update-card" :class="{ available: updateAvailable }">
              <span class="set-update-icon">
                <component :is="updateAvailable ? IconDownload : IconCheck" />
              </span>
              <div class="set-update-info">
                <div class="set-update-title">
                  {{ updateAvailable
                    ? t('settings.update.newVersion', { v: latestVersion ?? '' })
                    : t('settings.update.upToDateShort') }}
                </div>
                <div class="set-update-sub">
                  {{ updateAvailable
                    ? t('settings.update.fromTo', { cur: version, next: latestVersion ?? '' })
                    : t('settings.update.current', { v: version }) }}
                </div>
              </div>
              <div class="set-update-cta">
                <button
                  v-if="updateDownloaded"
                  class="btn primary"
                  @click="relaunchApp()"
                >
                  <IconCheck />
                  {{ t('settings.relaunch') }}
                </button>
                <button
                  v-else-if="updaterUpdate"
                  class="btn primary"
                  :disabled="updateDownloading"
                  @click="installUpdate"
                >
                  <IconRefresh v-if="updateDownloading" />
                  {{ updateDownloading ? t('settings.updateDownloading') : t('settings.installUpdate') }}
                </button>
                <button
                  v-else
                  class="btn"
                  :disabled="checking"
                  @click="doCheck"
                >
                  <IconRefresh v-if="!checking" />
                  {{ checking ? t('settings.checking') : t('settings.checkUpdate') }}
                </button>
              </div>
            </div>

            <!-- 下载进度条 -->
            <div v-if="updateDownloading && updateProgress !== null" class="set-update-progress">
              <span class="set-update-progress-track">
                <span class="set-update-progress-fill" :style="{ width: updateProgress + '%' }" />
              </span>
              <span class="set-update-progress-pct">{{ updateProgress }}%</span>
            </div>

            <!-- 下载/安装失败（不受 updateAvailable 门控） -->
            <p v-if="updateInstallError && !updateDownloading" class="set-update-status set-update-error">
              {{ t('settings.updateInstallFail', { e: updateInstallError }) }}
            </p>

            <!-- 检查结果（无新版本时显示，如"已是最新"或检查失败原因） -->
            <p v-if="updateMsg && !updateAvailable && !updateDownloading" class="set-update-status">
              {{ updateMsg }}
            </p>

            <!-- 次要操作：查看更新日志 / 手动下载 -->
            <button v-if="updateAvailable" class="set-update-notes" @click="openReleasePage()">
              <IconExternalLink />
              {{ t('settings.viewRelease', { v: latestVersion ?? '' }) }}
            </button>
          </div>
        </template>

        <template v-else>
          <div class="set-shortcuts">
            <div class="set-shortcut-group" v-for="g in shortcutGroups" :key="g.title">
              <div class="set-shortcut-group-title">{{ t(g.title) }}</div>
              <div class="set-shortcut-row" v-for="s in g.items" :key="s.key">
                <span class="set-shortcut-label">{{ t(s.label) }}</span>
                <kbd class="set-shortcut-key">{{ s.key }}</kbd>
              </div>
            </div>
          </div>
        </template>
      </div>
    </div>
  </div>
</template>
