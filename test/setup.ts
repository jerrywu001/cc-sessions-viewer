// Global test setup — runs once before each test file (Vitest `setupFiles`).
//
// jsdom ships neither `matchMedia` nor the Web Animations API, but
// settings.ts touches `matchMedia` at *import time* and flyToTrash.ts calls
// `Element.prototype.animate`. Polyfill both here so importing those modules
// doesn't throw.
import { afterEach, vi } from 'vitest'

// --- localStorage ---------------------------------------------------------
// jsdom's localStorage is disabled by default in newer versions. Provide a
// minimal in-memory implementation for tests that persist state.
if (!globalThis.localStorage) {
  const store: Record<string, string> = {}
  globalThis.localStorage = {
    getItem: (key: string) => store[key] ?? null,
    setItem: (key: string, value: string) => {
      store[key] = value
    },
    removeItem: (key: string) => {
      delete store[key]
    },
    clear: () => {
      Object.keys(store).forEach((key) => delete store[key])
    },
    key: (index: number) => Object.keys(store)[index] ?? null,
    get length() {
      return Object.keys(store).length
    },
  } as Storage
}

// --- window.matchMedia ----------------------------------------------------
// Default to light mode (matches: false). Individual tests override
// `window.matchMedia` with vi.stubGlobal when they need dark mode.
if (!window.matchMedia) {
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: vi.fn(), // deprecated, kept for completeness
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }))
}

// --- ResizeObserver -------------------------------------------------------
// jsdom omits ResizeObserver; CollapsibleBox feature-detects it, so provide a
// no-op class to exercise that branch.
if (!globalThis.ResizeObserver) {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver
}

// --- IntersectionObserver -------------------------------------------------
// jsdom omits this too; SessionsView uses it to lazy-load per-card token
// usage. Tests don't exercise visibility, so a no-op is enough.
if (!globalThis.IntersectionObserver) {
  globalThis.IntersectionObserver = class {
    constructor() {}
    observe() {}
    unobserve() {}
    disconnect() {}
    takeRecords() {
      return []
    }
    root = null
    rootMargin = ''
    thresholds: number[] = []
  } as unknown as typeof IntersectionObserver
}

// --- Element.prototype.animate -------------------------------------------
// Minimal Web Animations API stub: every test that exercises animation only
// needs `.finished` (a resolved promise) and `.cancel()`.
if (!Element.prototype.animate) {
  Element.prototype.animate = vi.fn().mockImplementation(() => ({
    finished: Promise.resolve(),
    cancel: vi.fn(),
    play: vi.fn(),
    pause: vi.fn(),
    onfinish: null,
  })) as unknown as typeof Element.prototype.animate
}

// jsdom 的 getContext() 会打印「Not implemented」后返回 null。xterm 在导入期只
// 用它探测 canvas 是否可用，因此明确返回 null 既符合 jsdom 的能力，也避免测试
// 通过时仍留下三条无意义的 stderr 提示。
Object.defineProperty(HTMLCanvasElement.prototype, 'getContext', {
  configurable: true,
  writable: true,
  value: () => null,
})

// Keep localStorage clean between tests so persisted lang/theme/prefs from
// one test never leak into the next.
afterEach(() => {
  localStorage.clear()
})
