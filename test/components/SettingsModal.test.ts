import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'

const { appVersionMock, backgroundMediaDirectoryMock, checkAppUpdateMock, deleteBackgroundMediaMock, emitToMock, exportBackgroundMediaMock, importBackgroundMediaMock, installTurnHooksMock, listBackgroundMediaMock, openDialogMock, openPathExternalMock, reclaudeInfoMock, tauriInvokeMock, turnHookStatusMock } = vi.hoisted(() => ({
  appVersionMock: vi.fn(),
  backgroundMediaDirectoryMock: vi.fn(),
  checkAppUpdateMock: vi.fn(),
  deleteBackgroundMediaMock: vi.fn(),
  emitToMock: vi.fn(),
  exportBackgroundMediaMock: vi.fn(),
  importBackgroundMediaMock: vi.fn(),
  installTurnHooksMock: vi.fn(),
  listBackgroundMediaMock: vi.fn(),
  openDialogMock: vi.fn(),
  openPathExternalMock: vi.fn(),
  reclaudeInfoMock: vi.fn(),
  tauriInvokeMock: vi.fn(),
  turnHookStatusMock: vi.fn(),
}))
vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => `asset://${path}`,
  invoke: tauriInvokeMock,
}))
vi.mock('@tauri-apps/api/event', () => ({ emitTo: emitToMock }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: openDialogMock }))
vi.mock('../../src/api', () => ({
  appVersion: appVersionMock,
  backgroundMediaDirectory: backgroundMediaDirectoryMock,
  deleteBackgroundMedia: deleteBackgroundMediaMock,
  exportBackgroundMedia: exportBackgroundMediaMock,
  importBackgroundMedia: importBackgroundMediaMock,
  installTurnHooks: installTurnHooksMock,
  listBackgroundMedia: listBackgroundMediaMock,
  openUrl: (url: string) => tauriInvokeMock('open_url', { url }),
  openPathExternal: openPathExternalMock,
  reclaudeInfo: reclaudeInfoMock,
  turnHookStatus: turnHookStatusMock,
}))
vi.mock('../../src/updateCheck', async (importOriginal) => {
  const orig: any = await importOriginal()
  return { ...orig, checkAppUpdate: checkAppUpdateMock }
})

import SettingsModal from '../../src/components/SettingsModal.vue'
import { vTooltip } from '../../src/tooltip'
import {
  lang,
  backgroundBorderOpacity,
  backgroundImageOpacity,
  backgroundImagePath,
  chatSpacing,
  chatRailCount,
  setBackgroundBorderOpacity,
  setBackgroundImageOpacity,
  setBackgroundImagePath,
  setChatSpacing,
  setChatRailCount,
  setLang,
  setShowToolCalls,
  setShowChatRail,
  setTheme,
  setUseReclaude,
  showToolCalls,
  showChatRail,
  theme,
  useReclaude,
} from '../../src/settings'
import {
  turnHookStatus,
  turnHookStatusError,
  turnHookStatusLoading,
} from '../../src/turnHookStatus'
import {
  desktopPetCatalog,
  desktopPetCatalogError,
  desktopPetCharacter,
  desktopPetEnabled,
  desktopPetSize,
  setDesktopPetCharacter,
  setDesktopPetEnabled,
  setDesktopPetSize,
} from '../../src/desktopPet'

const petCatalog = {
  pets: [
    {
      key: 'codex:codex',
      id: 'codex',
      displayName: 'Codex',
      description: 'The original Codex companion.',
      spriteVersionNumber: 2,
      spritesheetPath: 'C:/pets/codex.webp',
      source: 'codex',
    },
    {
      key: 'codex:bsod',
      id: 'bsod',
      displayName: 'BSOD',
      description: 'A tiny blue-screen gremlin.',
      spriteVersionNumber: 2,
      spritesheetPath: 'C:/pets/bsod.webp',
      source: 'codex',
    },
    {
      key: 'custom:pixel',
      id: 'pixel',
      displayName: 'Pixel',
      description: 'A custom companion.',
      spriteVersionNumber: 2,
      spritesheetPath: 'C:/Users/test/.codex/pets/pixel/spritesheet.webp',
      source: 'custom',
    },
  ],
  customDirectory: 'C:/Users/test/.codex/pets',
  codexInstalled: true,
}

const fullHookStatus = {
  enabled: true,
  claude: {
    installed: true,
    configPath: '/home/test/.claude/settings.json',
    events: ['UserPromptSubmit', 'Stop', 'StopFailure', 'Notification', 'PermissionRequest']
      .map((name) => ({ name, installed: true })),
    hooks: [
      {
        event: 'PreToolUse',
        category: null,
        matcher: 'Bash',
        hookType: 'command',
        detail: 'echo external-hook',
        managed: false,
      },
      {
        event: 'UserPromptSubmit',
        category: null,
        matcher: null,
        hookType: 'command',
        detail: 'node turn-signal-hook.cjs',
        managed: true,
      },
    ],
  },
  codex: {
    installed: true,
    configPath: '/home/test/.codex/hooks.json',
    events: ['UserPromptSubmit', 'Stop', 'PermissionRequest']
      .map((name) => ({ name, installed: true })),
    hooks: [{
      event: 'Stop',
      category: null,
      matcher: null,
      hookType: 'command',
      detail: 'node turn-signal-hook.cjs',
      managed: true,
    }],
  },
  agy: {
    installed: true,
    configPath: '/home/test/.gemini/config/hooks.json',
    events: ['PreInvocation', 'Stop'].map((name) => ({ name, installed: true })),
    hooks: [{
      event: 'PreInvocation',
      category: 'cc-sessions-viewer-turn-status',
      matcher: null,
      hookType: 'command',
      detail: 'node turn-signal-hook.cjs',
      managed: true,
    }],
  },
  grok: {
    installed: true,
    configPath: '/home/test/.grok/config.toml',
    events: ['UserPromptSubmit', 'Stop', 'StopFailure', 'StopCancelled', 'Notification:idle_prompt', 'Notification:permission_prompt']
      .map((name) => ({ name, installed: true })),
    hooks: [{
      event: 'Stop',
      category: null,
      matcher: null,
      hookType: 'command',
      detail: 'Managed status hook',
      managed: true,
    }],
  },
  kimicode: {
    installed: true,
    configPath: '/home/test/.kimi-code/config.toml',
    events: ['TurnStarted', 'Stop', 'StopFailure', 'PermissionRequest', 'Interrupt']
      .map((name) => ({ name, installed: true })),
    hooks: [{
      event: 'Stop',
      category: null,
      matcher: null,
      hookType: 'command',
      detail: 'Managed status hook',
      managed: true,
    }],
  },
}

beforeEach(() => {
  setLang('en')
  setTheme('system')
  appVersionMock.mockReset().mockResolvedValue('9.9.9')
  backgroundMediaDirectoryMock.mockReset().mockResolvedValue('/app-data/background-media')
  exportBackgroundMediaMock.mockReset().mockResolvedValue({ count: 0, directory: '/export/background-media' })
  checkAppUpdateMock.mockReset()
  installTurnHooksMock.mockReset().mockResolvedValue({})
  openPathExternalMock.mockReset().mockResolvedValue(undefined)
  reclaudeInfoMock.mockReset().mockResolvedValue({
    installed: false,
    daemonRunning: false,
    daemonPort: null,
  })
  tauriInvokeMock.mockReset().mockImplementation((command: string) => {
    if (command === 'desktop_pet_catalog') return Promise.resolve(petCatalog)
    return Promise.resolve(undefined)
  })
  emitToMock.mockReset().mockResolvedValue(undefined)
  turnHookStatusMock.mockReset().mockResolvedValue(fullHookStatus)
  turnHookStatus.value = null
  turnHookStatusLoading.value = false
  turnHookStatusError.value = ''
  setDesktopPetEnabled(false)
  setDesktopPetCharacter('codex:codex')
  setDesktopPetSize(112)
  setShowToolCalls(false)
  setChatSpacing(100)
  setChatRailCount(41)
  setShowChatRail(true)
  setUseReclaude(false)
  setBackgroundImagePath(null)
  setBackgroundImageOpacity(40)
  setBackgroundBorderOpacity(26)
  localStorage.removeItem('settingsActiveTab:v1')
  openDialogMock.mockReset()
  listBackgroundMediaMock.mockReset().mockResolvedValue([])
  importBackgroundMediaMock.mockReset()
  deleteBackgroundMediaMock.mockReset().mockResolvedValue(undefined)
  desktopPetCatalog.value = null
  desktopPetCatalogError.value = ''
})
afterEach(() => {
  setLang('en')
  setTheme('system')
  setShowToolCalls(false)
  setChatSpacing(100)
  setUseReclaude(false)
  setBackgroundImagePath(null)
  setBackgroundImageOpacity(40)
  setBackgroundBorderOpacity(26)
  localStorage.removeItem('settingsActiveTab:v1')
})

type Props = InstanceType<typeof SettingsModal>['$props']
const factory = (props: Partial<Props> = {}) =>
  mount(SettingsModal, {
    props: { ...props } as Props,
    global: { directives: { tooltip: vTooltip } },
    attachTo: document.body,
  })

describe('SettingsModal', () => {
  it('shows the restore-defaults action and emits resetSettings', async () => {
    const wrapper = factory()
    const resetBtn = wrapper.get('[data-reset-settings]')
    await resetBtn.trigger('click')
    expect(wrapper.emitted('resetSettings')).toHaveLength(1)
  })

  it('emits close only from the X button, not the overlay backdrop', async () => {
    const wrapper = factory({ initialTab: 'theme' })
    await wrapper.find('.app-overlay').trigger('click')
    expect(wrapper.emitted('close')).toBeUndefined()
    await wrapper.find('.modal-close').trigger('click')
    expect(wrapper.emitted('close')).toHaveLength(1)
  })

  it('restores the last tab selected in Settings', async () => {
    const wrapper = factory()
    await wrapper.findAll('.set-nav-item')[1].trigger('click')
    expect(localStorage.getItem('settingsActiveTab:v1')).toBe('theme')
    wrapper.unmount()

    const restored = factory()
    expect(restored.find('.set-nav-item.active').text()).toContain('Appearance')
  })

  it('switches language via the custom dropdown', async () => {
    const wrapper = factory()
    const dropdowns = wrapper.findAll('.set-dropdown-btn')
    await dropdowns[0].trigger('click')
    const items = wrapper.findAll('.set-dropdown-item')
    expect(items.length).toBeGreaterThanOrEqual(4)
    await items[1].trigger('click') // 简体中文
    expect(lang.value).toBe('zh')
  })

  it('switches theme via the custom dropdown', async () => {
    const wrapper = factory({ initialTab: 'theme' })
    const dropdowns = wrapper.findAll('.set-dropdown-btn')
    await dropdowns[0].trigger('click')
    const items = wrapper.findAll('.set-dropdown-item')
    // find the Dracula option (last one)
    await items[items.length - 1].trigger('click')
    expect(theme.value).toBe('dracula')
  })

  it('keeps ordinary tool calls hidden by default and persists the Chat toggle', async () => {
    expect(showToolCalls.value).toBe(false)
    const wrapper = factory()
    const row = wrapper
      .findAll('.set-row')
      .find((candidate) => candidate.find('.set-row-title').text() === 'Show tool calls')

    expect(row).toBeDefined()
    await row!.trigger('click')

    expect(showToolCalls.value).toBe(true)
    expect(localStorage.getItem('showToolCalls:v1')).toBe('1')
  })

  it('updates information spacing from the Chat slider', async () => {
    const wrapper = factory()
    const slider = wrapper.get('[data-chat-spacing-slider]')
    expect(slider.attributes('min')).toBe('30')
    expect(slider.attributes('max')).toBe('150')
    expect(slider.attributes('step')).toBe('2')
    await slider.setValue('30')

    expect(chatSpacing.value).toBe(30)
    expect(localStorage.getItem('chatSpacing:v1')).toBe('30')
    expect(document.documentElement.style.getPropertyValue('--chat-spacing-scale')).toBe('0.3')
  })

  it('persists the prompt navigation toggle and marker count', async () => {
    const wrapper = factory()
    const toggle = wrapper
      .findAll('.set-row')
      .find((candidate) => candidate.find('.set-row-title').text() === 'Show prompt navigation')
    expect(toggle).toBeDefined()
    await toggle!.trigger('click')
    expect(showChatRail.value).toBe(false)
    expect(localStorage.getItem('showChatRail:v1')).toBe('0')

    const slider = wrapper.get('[data-chat-rail-count-slider]')
    expect(slider.attributes('min')).toBe('21')
    expect(slider.attributes('max')).toBe('71')
    expect(slider.attributes('step')).toBe('1')
    expect(chatRailCount.value).toBe(41)
    await slider.setValue('42')
    expect(chatRailCount.value).toBe(42)
    expect(localStorage.getItem('chatRailCount:v1')).toBe('42')
  })

  it('selects, previews, adjusts, and removes a background image', async () => {
    openDialogMock.mockResolvedValue('/Users/test/Pictures/dream-skin.webp')
    importBackgroundMediaMock.mockResolvedValue({
      id: 'a0f7d0e7-97dd-4bbd-bceb-b840787163d1',
      name: 'dream-skin.webp',
      path: '/app-data/background-media/a0f7d0e7-97dd-4bbd-bceb-b840787163d1--dream-skin.webp',
    })
    const wrapper = factory({ initialTab: 'theme' })

    await wrapper.get('[data-background-image-choose]').trigger('click')
    await flushPromises()

    expect(openDialogMock).toHaveBeenCalledWith({
      multiple: false,
      filters: [{ name: 'Images and videos', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'avif', 'mp4'] }],
    })
    expect(importBackgroundMediaMock).toHaveBeenCalledWith('/Users/test/Pictures/dream-skin.webp')
    expect(backgroundImagePath.value).toBe('/app-data/background-media/a0f7d0e7-97dd-4bbd-bceb-b840787163d1--dream-skin.webp')
    expect(wrapper.get('.set-background-thumb').attributes('src')).toBe('asset:///app-data/background-media/a0f7d0e7-97dd-4bbd-bceb-b840787163d1--dream-skin.webp')
    expect(wrapper.get('.set-background-name').text()).toBe('dream-skin.webp')

    const slider = wrapper.get('[data-background-opacity-slider]')
    await slider.setValue('72')
    expect(backgroundImageOpacity.value).toBe(72)
    expect(localStorage.getItem('backgroundImageOpacity:v1')).toBe('72')

    const borderSlider = wrapper.get('[data-background-border-opacity-slider]')
    expect(borderSlider.attributes('min')).toBe('0')
    expect(borderSlider.attributes('max')).toBe('80')
    await borderSlider.setValue('42')
    expect(backgroundBorderOpacity.value).toBe(42)
    expect(localStorage.getItem('backgroundBorderOpacity:v1')).toBe('42')

    await wrapper.get('.set-background-remove').trigger('click')
    expect(backgroundImagePath.value).toBeNull()
    expect(wrapper.find('[data-background-opacity-slider]').exists()).toBe(false)
    expect(wrapper.find('[data-background-border-opacity-slider]').exists()).toBe(false)
  })

  it('shows a playable preview for an MP4 background', async () => {
    setBackgroundImagePath('/Users/test/Movies/dream-skin.mp4')
    const wrapper = factory({ initialTab: 'theme' })

    const preview = wrapper.get('video.set-background-thumb')
    expect(preview.attributes('src')).toBe('asset:///Users/test/Movies/dream-skin.mp4')
    expect((preview.element as HTMLVideoElement).loop).toBe(true)
    expect((preview.element as HTMLVideoElement).muted).toBe(true)
  })

  it('shows cached backgrounds and switches immediately when one is clicked', async () => {
    listBackgroundMediaMock.mockResolvedValue([
      { id: 'b', name: 'forest.jpg', path: '/app-data/background-media/b--forest.jpg' },
      { id: 'c', name: 'rain.mp4', path: '/app-data/background-media/c--rain.mp4' },
    ])
    const wrapper = factory({ initialTab: 'theme' })
    await flushPromises()

    const choices = wrapper.findAll('[data-background-media-select]')
    expect(choices).toHaveLength(2)
    expect(wrapper.findAll('video.set-background-thumb')).toHaveLength(0)
    expect(wrapper.findAll('.set-background-media-select video')).toHaveLength(1)
    expect(wrapper.findAll('.set-background-media-type')).toHaveLength(1)
    expect(wrapper.get('.set-background-media-type').text()).toBe('MP4')

    await choices[1].trigger('click')
    expect(backgroundImagePath.value).toBe('/app-data/background-media/c--rain.mp4')
  })

  it('opens the saved backgrounds folder', async () => {
    const wrapper = factory({ initialTab: 'theme' })

    await wrapper.get('[data-background-media-open-folder]').trigger('click')
    await flushPromises()

    expect(backgroundMediaDirectoryMock).toHaveBeenCalledOnce()
    expect(openPathExternalMock).toHaveBeenCalledWith('/app-data/background-media')
  })

  it('exports every saved background to a selected folder with their display names', async () => {
    listBackgroundMediaMock.mockResolvedValue([
      { id: 'b', name: 'forest.jpg', path: '/app-data/background-media/b--forest.jpg' },
      { id: 'c', name: 'rain.mp4', path: '/app-data/background-media/c--rain.mp4' },
    ])
    openDialogMock.mockResolvedValue('/Users/test/Desktop/background-export')
    exportBackgroundMediaMock.mockResolvedValue({
      count: 2,
      directory: '/Users/test/Desktop/background-export/background-media-20260819-170000',
    })
    const wrapper = factory({ initialTab: 'theme' })
    await flushPromises()

    await wrapper.get('[data-background-media-export]').trigger('click')
    await flushPromises()

    expect(openDialogMock).toHaveBeenCalledWith({ directory: true, multiple: false })
    expect(exportBackgroundMediaMock).toHaveBeenCalledWith('/Users/test/Desktop/background-export')
    expect(wrapper.text()).toContain('Exported 2 background media files')
    expect(wrapper.emitted('notify')).toEqual([['Exported 2 background media files']])
    expect(openPathExternalMock).toHaveBeenCalledWith('/Users/test/Desktop/background-export')
  })

  it('reuses an imported background returned from the cache without duplicating it in the library', async () => {
    const media = { id: 'b', name: 'forest.jpg', path: '/app-data/background-media/b--forest.jpg' }
    listBackgroundMediaMock.mockResolvedValue([media])
    openDialogMock.mockResolvedValue('/Users/test/Pictures/forest-copy.jpg')
    importBackgroundMediaMock.mockResolvedValue(media)
    const wrapper = factory({ initialTab: 'theme' })
    await flushPromises()

    await wrapper.get('[data-background-image-choose]').trigger('click')
    await flushPromises()

    expect(wrapper.findAll('[data-background-media-select]')).toHaveLength(1)
    expect(backgroundImagePath.value).toBe(media.path)
  })

  it('loads the app version on mount', async () => {
    // 版本与更新操作现在住在「Updates」tab 里
    const wrapper = factory({ initialTab: 'updates' })
    await flushPromises()
    expect(appVersionMock).toHaveBeenCalled()
    expect(wrapper.text()).toContain('v9.9.9')
  })

  it('hides ReClaude settings and clears stale routing when the wrapper is unavailable', async () => {
    setUseReclaude(true)
    const wrapper = factory()
    await flushPromises()

    expect(reclaudeInfoMock).toHaveBeenCalledOnce()
    expect(wrapper.text()).not.toContain('Route chat through ReClaude')
    expect(useReclaude.value).toBe(false)
  })

  it('shows hook config files without rendering individual hook details', async () => {
    turnHookStatus.value = fullHookStatus
    const wrapper = factory({ initialTab: 'hooks' })

    const files = wrapper.findAll('.set-hook-file')
    expect(files).toHaveLength(5)
    expect(wrapper.text()).toContain('5 files')
    expect(wrapper.text()).toContain('2 hooks')
    expect(wrapper.text()).not.toContain('echo external-hook')
    expect(wrapper.find('.set-desktop-pet-card').exists()).toBe(false)

    await files[0].trigger('click')
    expect(openPathExternalMock).toHaveBeenCalledWith('/home/test/.claude/settings.json')
    expect(wrapper.find('.set-hooks-enable').attributes('disabled')).toBeDefined()
    expect(wrapper.find('.set-hooks-enable').text()).toContain('Enabled')
  })

  it('refreshes hook status from disk without reinstalling hooks', async () => {
    turnHookStatus.value = fullHookStatus
    turnHookStatusMock.mockResolvedValueOnce({
      ...fullHookStatus,
      enabled: false,
      grok: {
        ...fullHookStatus.grok,
        installed: false,
        hooks: [],
        events: fullHookStatus.grok.events.map((event) => ({ ...event, installed: false })),
      },
    })
    const wrapper = factory({ initialTab: 'hooks' })

    await wrapper.get('.set-hook-list-refresh').trigger('click')
    await flushPromises()

    expect(turnHookStatusMock).toHaveBeenCalledOnce()
    expect(installTurnHooksMock).not.toHaveBeenCalled()
    expect(wrapper.find('.set-hooks-enable').text()).toContain('Enable session status tracking')
    expect(wrapper.findAll('.set-hook-file')).toHaveLength(4)
  })

  it('keeps the hook action enabled for a partial install and refreshes after repair', async () => {
    turnHookStatus.value = {
      ...fullHookStatus,
      enabled: false,
      codex: {
        ...fullHookStatus.codex,
        installed: false,
        events: fullHookStatus.codex.events.map((event, index) => ({
          ...event,
          installed: index !== 0,
        })),
      },
    }
    const wrapper = factory({ initialTab: 'hooks' })
    const action = wrapper.find('.set-hooks-enable')
    expect(action.attributes('disabled')).toBeUndefined()

    await action.trigger('click')
    await flushPromises()

    expect(installTurnHooksMock).toHaveBeenCalledOnce()
    expect(turnHookStatusMock).toHaveBeenCalledOnce()
    expect(wrapper.find('.set-hooks-enable').attributes('disabled')).toBeDefined()
  })

  it('keeps desktop pet enablement independent of tracking hooks', async () => {
    turnHookStatus.value = {
      ...fullHookStatus,
      enabled: false,
      codex: { ...fullHookStatus.codex, installed: false },
    }
    const wrapper = factory({ initialTab: 'pet' })

    expect(wrapper.get('.set-desktop-pet-toggle').attributes('disabled')).toBeUndefined()
    expect(wrapper.text()).not.toContain('Enable session status tracking in Hooks')
    await wrapper.get('.set-desktop-pet-toggle').trigger('click')
    await flushPromises()
    expect(tauriInvokeMock).toHaveBeenCalledWith('set_desktop_pet_enabled', { enabled: true })
  })

  it('opens the desktop pet and synchronizes character and size choices', async () => {
    turnHookStatus.value = fullHookStatus
    const wrapper = factory({ initialTab: 'pet' })

    await wrapper.get('.set-desktop-pet-toggle').trigger('click')
    await flushPromises()
    expect(tauriInvokeMock).toHaveBeenCalledWith('set_desktop_pet_enabled', { enabled: true })
    expect(desktopPetEnabled.value).toBe(true)

    await wrapper.findAll('.set-desktop-pet-character-select')[1].trigger('click')
    await flushPromises()
    expect(desktopPetCharacter.value).toBe('codex:bsod')
    expect(emitToMock).toHaveBeenCalledWith(
      'desktop-pet',
      'desktop-pet://preferences',
      { character: 'codex:bsod', size: 112 },
    )

    await wrapper.get('.set-desktop-pet-size input').setValue('176')
    await flushPromises()
    expect(desktopPetSize.value).toBe(176)
    expect(emitToMock).toHaveBeenLastCalledWith(
      'desktop-pet',
      'desktop-pet://preferences',
      { character: 'codex:bsod', size: 176 },
    )
  })

  it('refreshes the sprite catalog and opens the custom pet directory', async () => {
    turnHookStatus.value = fullHookStatus
    const wrapper = factory({ initialTab: 'pet' })
    await flushPromises()

    expect(wrapper.findAll('.set-desktop-pet-character')).toHaveLength(2)
    expect(wrapper.text()).toContain('Codex Desktop pets')
    expect(wrapper.text()).toContain('Custom pets')

    await wrapper.find('.set-desktop-pet-choice-head .set-desktop-pet-tool').trigger('click')
    await flushPromises()
    expect(tauriInvokeMock).toHaveBeenCalledWith('desktop_pet_catalog')

    await wrapper.get('[data-desktop-pet-tab="custom"]').trigger('click')
    expect(wrapper.findAll('.set-desktop-pet-character')).toHaveLength(1)

    await wrapper.findAll('.set-desktop-pet-panel-tools .set-desktop-pet-tool')[0].trigger('click')
    expect(tauriInvokeMock).toHaveBeenCalledWith('open_url', { url: 'https://petdex.dev/' })

    await wrapper.findAll('.set-desktop-pet-panel-tools .set-desktop-pet-tool')[1].trigger('click')
    await flushPromises()
    expect(openPathExternalMock).toHaveBeenCalledWith('C:/Users/test/.codex/pets')
  })

  it('deletes a confirmed custom pet and refreshes the catalog', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true)
    const wrapper = factory({ initialTab: 'pet' })
    await flushPromises()
    await wrapper.get('[data-desktop-pet-tab="custom"]').trigger('click')

    await wrapper.get('.set-desktop-pet-character-delete').trigger('click')
    await flushPromises()

    expect(confirm).toHaveBeenCalledWith(
      'Delete “Pixel”? Its folder and spritesheet will be permanently removed.',
    )
    expect(tauriInvokeMock).toHaveBeenCalledWith('delete_custom_desktop_pet', { petId: 'pixel' })
    expect(tauriInvokeMock).toHaveBeenCalledWith('desktop_pet_catalog')
  })

  it('shows a fallback pet preview when the sprite catalog is empty', async () => {
    tauriInvokeMock.mockImplementation((command: string) => {
      if (command === 'desktop_pet_catalog') {
        return Promise.resolve({
          pets: [],
          customDirectory: 'C:/Users/test/.codex/pets',
          codexInstalled: false,
        })
      }
      return Promise.resolve(undefined)
    })
    const wrapper = factory({ initialTab: 'pet' })
    await flushPromises()

    expect(wrapper.find('.set-desktop-pet-preview .desktop-pet-fallback').exists()).toBe(true)
    expect(wrapper.find('.set-desktop-pet-preview .pet-atlas-sprite').exists()).toBe(false)
    expect(wrapper.text()).toContain('No Codex Desktop pets were found')
  })

  it('reports when an update is available', async () => {
    checkAppUpdateMock.mockResolvedValue({ hasUpdate: true, latest: '2.0.0', current: '1.0.0' })
    const wrapper = factory({ initialTab: 'updates' })
    await flushPromises()

    const checkBtn = wrapper.find('.set-update-cta .btn')
    await checkBtn.trigger('click')
    await flushPromises()

    expect(checkAppUpdateMock).toHaveBeenCalled()
    expect(wrapper.text()).toContain('2.0.0')
  })

  it('reports when the app is up to date', async () => {
    checkAppUpdateMock.mockResolvedValue({ hasUpdate: false, latest: '1.0.0', current: '1.0.0' })
    const wrapper = factory({ initialTab: 'updates' })
    await flushPromises()

    const checkBtn = wrapper.find('.set-update-cta .btn')
    await checkBtn.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('latest version')
  })

  it('surfaces a failed update check', async () => {
    checkAppUpdateMock.mockRejectedValue(new Error('offline'))
    const wrapper = factory({ initialTab: 'updates' })
    await flushPromises()

    const checkBtn = wrapper.find('.set-update-cta .btn')
    await checkBtn.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Update check failed')
  })
})
