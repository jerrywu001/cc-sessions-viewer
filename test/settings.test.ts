import { afterEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => `asset://${path}`,
}))

import {
  applyTheme,
  backgroundBorderOpacity,
  backgroundImageOpacity,
  backgroundImagePath,
  backgroundIsVideo,
  chatSpacing,
  chatRailCount,
  clearAppCache,
  resetSettings,
  lang,
  setBackgroundBorderOpacity,
  setBackgroundImageOpacity,
  setBackgroundImagePath,
  setChatSpacing,
  setChatRailCount,
  setLang,
  setTheme,
  theme,
} from '../src/settings'

const DARK = 'theme-dark'

// Replace window.matchMedia so `theme: 'system'` resolves deterministically.
function stubMatchMedia(matches: boolean) {
  vi.stubGlobal(
    'matchMedia',
    vi.fn().mockImplementation((query: string) => ({
      matches,
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  )
}

afterEach(() => {
  vi.unstubAllGlobals()
  document.documentElement.classList.remove(DARK)
  setBackgroundImagePath(null)
  setBackgroundImageOpacity(40)
  setBackgroundBorderOpacity(26)
  setChatSpacing(100)
  setChatRailCount(51)
  setLang('en')
  setTheme('system')
})

describe('setLang', () => {
  it('updates the ref and persists to localStorage', () => {
    setLang('ja')
    expect(lang.value).toBe('ja')
    expect(localStorage.getItem('lang')).toBe('ja')
  })
})

describe('setTheme', () => {
  it('updates the ref and persists to localStorage', () => {
    setTheme('dark')
    expect(theme.value).toBe('dark')
    expect(localStorage.getItem('theme')).toBe('dark')
  })
})

describe('chat spacing', () => {
  async function freshChatSpacing(stored?: string) {
    localStorage.clear()
    if (stored !== undefined) localStorage.setItem('chatSpacing:v1', stored)
    vi.resetModules()
    return import('../src/settings')
  }

  it('defaults to the current 100% spacing when no preference is stored', async () => {
    const mod = await freshChatSpacing()
    expect(mod.chatSpacing.value).toBe(100)
    expect(document.documentElement.style.getPropertyValue('--chat-spacing-scale')).toBe('1')
  })

  it('restores only supported persisted steps', async () => {
    const restored = await freshChatSpacing('150')
    expect(restored.chatSpacing.value).toBe(150)
    expect(document.documentElement.style.getPropertyValue('--chat-spacing-scale')).toBe('1.5')

    const fallback = await freshChatSpacing('151')
    expect(fallback.chatSpacing.value).toBe(100)
  })

  it('persists and applies a new spacing immediately', () => {
    setChatSpacing(70)
    expect(chatSpacing.value).toBe(70)
    expect(localStorage.getItem('chatSpacing:v1')).toBe('70')
    expect(document.documentElement.style.getPropertyValue('--chat-spacing-scale')).toBe('0.7')
  })
})

describe('chat rail', () => {
  async function freshChatRailCount(stored?: string) {
    localStorage.clear()
    if (stored !== undefined) localStorage.setItem('chatRailCount:v1', stored)
    vi.resetModules()
    return import('../src/settings')
  }

  it('defaults to 41 markers and restores only values in the supported range', async () => {
    const defaults = await freshChatRailCount()
    expect(defaults.chatRailCount.value).toBe(41)

    const restored = await freshChatRailCount('71')
    expect(restored.chatRailCount.value).toBe(71)

    const fallback = await freshChatRailCount('72')
    expect(fallback.chatRailCount.value).toBe(41)
  })

  it('persists marker count changes', () => {
    setChatRailCount(42)
    expect(chatRailCount.value).toBe(42)
    expect(localStorage.getItem('chatRailCount:v1')).toBe('42')
  })

  it('restores application defaults and clears cached project preferences', () => {
    setTheme('dark')
    setChatRailCount(71)
    setChatSpacing(150)
    setBackgroundImagePath('/tmp/wallpaper.webp')
    localStorage.setItem('projPrefs:v1', '{"project":{"state":"pinned"}}')

    resetSettings()

    expect(theme.value).toBe('system')
    expect(chatRailCount.value).toBe(41)
    expect(chatSpacing.value).toBe(100)
    expect(backgroundImagePath.value).toBeNull()
    expect(localStorage.getItem('projPrefs:v1')).toBeNull()
  })
})

describe('custom background', () => {
  it('uses the intended opacity defaults when no wallpaper preferences are stored', async () => {
    localStorage.removeItem('backgroundImageOpacity:v1')
    localStorage.removeItem('backgroundBorderOpacity:v1')
    vi.resetModules()
    const settings = await import('../src/settings')

    expect(settings.backgroundImageOpacity.value).toBe(40)
    expect(settings.backgroundBorderOpacity.value).toBe(26)
  })

  it('persists the selected image and applies it to the app shell', () => {
    setBackgroundImagePath('/Users/test/Pictures/aurora.webp')
    setBackgroundImageOpacity(65)

    expect(backgroundImagePath.value).toBe('/Users/test/Pictures/aurora.webp')
    expect(backgroundImageOpacity.value).toBe(65)
    expect(localStorage.getItem('backgroundImagePath:v1')).toBe('/Users/test/Pictures/aurora.webp')
    expect(localStorage.getItem('backgroundImageOpacity:v1')).toBe('65')
    expect(document.documentElement.classList.contains('has-custom-background')).toBe(true)
    expect(document.documentElement.style.getPropertyValue('--custom-background-image')).toContain('aurora.webp')
    expect(document.documentElement.style.getPropertyValue('--custom-background-opacity')).toBe('65%')
  })

  it('persists border opacity and applies the wallpaper-only border token', () => {
    setBackgroundBorderOpacity(42)

    expect(backgroundBorderOpacity.value).toBe(42)
    expect(localStorage.getItem('backgroundBorderOpacity:v1')).toBe('42')
    expect(document.documentElement.style.getPropertyValue('--custom-background-border-opacity')).toBe('42%')
  })

  it('removes the image and restores the normal shell state', () => {
    setBackgroundImagePath('/Users/test/Pictures/aurora.webp')
    setBackgroundImagePath(null)

    expect(backgroundImagePath.value).toBeNull()
    expect(localStorage.getItem('backgroundImagePath:v1')).toBeNull()
    expect(document.documentElement.classList.contains('has-custom-background')).toBe(false)
    expect(document.documentElement.style.getPropertyValue('--custom-background-image')).toBe('')
  })

  it('identifies MP4 backgrounds so the app can render a video layer', () => {
    setBackgroundImagePath('/Users/test/Movies/aurora.mp4')

    expect(backgroundIsVideo.value).toBe(true)
    expect(document.documentElement.classList.contains('has-custom-video-background')).toBe(true)
    expect(document.documentElement.classList.contains('has-custom-image-background')).toBe(false)
    expect(document.documentElement.style.getPropertyValue('--custom-background-image')).toBe('')
  })
})

describe('applyTheme', () => {
  it('adds the dark class when the theme is dark', () => {
    setTheme('dark')
    applyTheme()
    expect(document.documentElement.classList.contains(DARK)).toBe(true)
  })

  it('removes the dark class when the theme is light', () => {
    document.documentElement.classList.add(DARK)
    setTheme('light')
    applyTheme()
    expect(document.documentElement.classList.contains(DARK)).toBe(false)
  })

  it('follows the system preference when the theme is system', () => {
    stubMatchMedia(true)
    setTheme('system')
    applyTheme()
    expect(document.documentElement.classList.contains(DARK)).toBe(true)

    stubMatchMedia(false)
    applyTheme()
    expect(document.documentElement.classList.contains(DARK)).toBe(false)
  })

  it('re-applies automatically (via watchEffect) when the theme ref changes', async () => {
    setTheme('dark')
    await nextTick()
    expect(document.documentElement.classList.contains(DARK)).toBe(true)

    setTheme('light')
    await nextTick()
    expect(document.documentElement.classList.contains(DARK)).toBe(false)
  })
})

describe('clearAppCache', () => {
  it('removes project display preferences', () => {
    localStorage.setItem('projPrefs:v1', '{"pinned":[]}')
    localStorage.setItem('projectOrder:v1', '{"claude":["demo"]}')
    clearAppCache()
    expect(localStorage.getItem('projPrefs:v1')).toBeNull()
    expect(localStorage.getItem('projectOrder:v1')).toBeNull()
  })
})

// detectSystemLang is module-private and only runs at import time, so we
// re-import a fresh copy of settings.ts under controlled navigator state.
describe('language detection on first load', () => {
  async function freshLoad(opts: {
    languages?: unknown
    storedLang?: string
    storedTheme?: string
  }) {
    localStorage.clear()
    if (opts.storedLang) localStorage.setItem('lang', opts.storedLang)
    if (opts.storedTheme) localStorage.setItem('theme', opts.storedTheme)
    Object.defineProperty(window.navigator, 'languages', {
      value: opts.languages,
      configurable: true,
    })
    vi.resetModules()
    return import('../src/settings')
  }

  it.each([
    ['zh-Hant-TW', 'zh-TW'],
    ['zh-TW', 'zh-TW'],
    ['zh-HK', 'zh-TW'],
    ['zh-MO', 'zh-TW'],
    ['zh-CN', 'zh'],
    ['zh', 'zh'],
    ['ja-JP', 'ja'],
    ['ja', 'ja'],
    ['en-GB', 'en'],
  ])('maps %s to %s', async (tag, expected) => {
    const mod = await freshLoad({ languages: [tag] })
    expect(mod.lang.value).toBe(expected)
  })

  it('falls back to English for an unsupported language', async () => {
    const mod = await freshLoad({ languages: ['fr-FR'] })
    expect(mod.lang.value).toBe('en')
  })

  it('skips empty entries and uses the first usable tag', async () => {
    const mod = await freshLoad({ languages: ['', 'ja-JP'] })
    expect(mod.lang.value).toBe('ja')
  })

  it('falls back to navigator.language when languages is unavailable', async () => {
    const mod = await freshLoad({ languages: undefined })
    expect(mod.lang.value).toBe('en')
  })

  it('prefers an explicit localStorage language over detection', async () => {
    const mod = await freshLoad({ languages: ['ja-JP'], storedLang: 'zh' })
    expect(mod.lang.value).toBe('zh')
  })

  it('restores a persisted theme, defaulting to system', async () => {
    const stored = await freshLoad({ languages: ['en-US'], storedTheme: 'dark' })
    expect(stored.theme.value).toBe('dark')
    const fallback = await freshLoad({ languages: ['en-US'] })
    expect(fallback.theme.value).toBe('system')
  })
})

describe('stats scope / range persistence', () => {
  async function freshStats(opts: { scope?: string; range?: string }) {
    localStorage.clear()
    if (opts.scope) localStorage.setItem('statsScope:v1', opts.scope)
    if (opts.range) localStorage.setItem('statsRange:v1', opts.range)
    vi.resetModules()
    return import('../src/settings')
  }

  it('defaults to all agents + last 3 months when no preference is stored', async () => {
    const mod = await freshStats({})
    expect(mod.statsScope.value).toBe('all')
    expect(mod.statsRange.value).toBe('months3')
  })

  it('restores a valid persisted scope and range', async () => {
    const mod = await freshStats({ scope: 'grok', range: 'days7' })
    expect(mod.statsScope.value).toBe('grok')
    expect(mod.statsRange.value).toBe('days7')
  })

  it('restores a valid persisted custom date range', async () => {
    const mod = await freshStats({ range: 'custom:2026-01-05:2026-07-05' })
    expect(mod.statsRange.value).toBe('custom:2026-01-05:2026-07-05')
  })

  // 老用户 localStorage 里可能存的 'all'（已废弃）；这里 pin 死回退到 months3
  // 而不是再写 'all'，否则 startAgentStats 会被后端拒掉。
  it('migrates legacy "all" range to months3 (and rejects bogus values)', async () => {
    const mod = await freshStats({ scope: 'bogus', range: 'all' })
    expect(mod.statsScope.value).toBe('all')
    expect(mod.statsRange.value).toBe('months3')
    const mod2 = await freshStats({ range: 'forever' })
    expect(mod2.statsRange.value).toBe('months3')
  })

  it('writes back to localStorage when the ref changes', async () => {
    const mod = await freshStats({})
    mod.statsScope.value = 'codex'
    mod.statsRange.value = 'days30'
    await nextTick()
    expect(localStorage.getItem('statsScope:v1')).toBe('codex')
    expect(localStorage.getItem('statsRange:v1')).toBe('days30')
  })
})

describe('agent visibility (enabledAgents / visibleAgents / setAgentEnabled)', () => {
  async function freshAgents(stored?: string) {
    localStorage.clear()
    if (stored !== undefined) localStorage.setItem('enabledAgents:v1', stored)
    vi.resetModules()
    return import('../src/settings')
  }

  it('defaults to Claude, Codex, Grok Build, and Pi when nothing is stored', async () => {
    const mod = await freshAgents()
    expect(mod.visibleAgents.value).toEqual(['claude', 'codex', 'grok', 'pi'])
  })

  it('restores a persisted subset, preserving the canonical order', async () => {
    const mod = await freshAgents(
      JSON.stringify({ claude: true, codex: false, grok: false, agy: true, opencode: false }),
    )
    expect(mod.visibleAgents.value).toEqual(['claude', 'pi', 'agy'])
  })

  it('treats agents missing from stored data as enabled (new agent rollout)', async () => {
    // 旧版本存的 JSON 没有 Grok Build / opencode 键 —— 升级后它们应默认可见，
    // 其它 agent 的用户选择保持不变。
    const mod = await freshAgents(JSON.stringify({ claude: true, codex: false, agy: false }))
    expect(mod.visibleAgents.value).toEqual(['claude', 'grok', 'pi'])
  })

  it('falls back to the default agents when stored data has every agent off', async () => {
    const mod = await freshAgents(
      JSON.stringify({ claude: false, codex: false, grok: false, kimi: false, agy: false, opencode: false }),
    )
    expect(mod.visibleAgents.value).toEqual(['pi'])
  })

  it('falls back to all-enabled on corrupt JSON', async () => {
    const mod = await freshAgents('{not json')
    expect(mod.visibleAgents.value).toEqual(['claude', 'codex', 'grok', 'pi'])
  })

  it('keeps only the first four enabled agents from an older oversized preference', async () => {
    const mod = await freshAgents(
      JSON.stringify({ claude: true, codex: true, grok: true, kimicode: true, agy: true, opencode: true }),
    )
    expect(mod.visibleAgents.value).toEqual(['claude', 'codex', 'grok', 'kimicode'])
    expect(JSON.parse(localStorage.getItem('enabledAgents:v1')!)).toMatchObject({ agy: false, opencode: false })
    expect(mod.consumeEnabledAgentsTrimmedNotice()).toBe(true)
    expect(mod.consumeEnabledAgentsTrimmedNotice()).toBe(false)
  })

  it('setAgentEnabled disables an agent and persists', async () => {
    const mod = await freshAgents()
    mod.setAgentEnabled('grok', false)
    expect(mod.visibleAgents.value).toEqual(['claude', 'codex', 'pi'])
    expect(JSON.parse(localStorage.getItem('enabledAgents:v1')!).grok).toBe(false)
  })

  it('refuses to disable the last remaining agent', async () => {
    const mod = await freshAgents(
      JSON.stringify({ claude: true, codex: false, grok: false, kimi: false, agy: false, opencode: false }),
    )
    mod.setAgentEnabled('claude', false)
    expect(mod.visibleAgents.value).toEqual(['pi'])
  })

  it('re-enables a previously hidden agent', async () => {
    const mod = await freshAgents(
      JSON.stringify({ claude: true, codex: false, grok: false, kimi: false, agy: false, opencode: false }),
    )
    mod.setAgentEnabled('codex', true)
    expect(mod.visibleAgents.value).toEqual(['claude', 'codex', 'pi'])
  })

  it('refuses to enable a fifth agent until one is disabled', async () => {
    const mod = await freshAgents()
    mod.setAgentEnabled('agy', true)
    expect(mod.visibleAgents.value).toEqual(['claude', 'codex', 'grok', 'pi'])

    mod.setAgentEnabled('grok', false)
    mod.setAgentEnabled('agy', true)
    expect(mod.visibleAgents.value).toEqual(['claude', 'codex', 'pi', 'agy'])
  })

  it('preserves legacy launch args and adds an empty Grok Build value', async () => {
    localStorage.clear()
    localStorage.setItem(
      'launchArgs:v1',
      JSON.stringify({ claude: '--legacy-claude', codex: '--legacy-codex' }),
    )
    vi.resetModules()
    const mod = await import('../src/settings')
    expect(mod.launchArgs.value).toMatchObject({
      claude: '--legacy-claude',
      codex: '--legacy-codex',
      grok: '',
      kimicode: '',
    })
  })

  it('migrates the legacy Kimi launch arguments to kimicode', async () => {
    localStorage.clear()
    localStorage.setItem('launchArgs:v1', JSON.stringify({ kimi: '--model k2' }))
    vi.resetModules()
    const mod = await import('../src/settings')
    expect(mod.launchArgs.value.kimicode).toBe('--model k2')
  })
})
