import { describe, expect, it, vi } from 'vitest'
import {
  captureStableTerminalCursor,
  codexSgrNormalizer,
  codexResumeConfigHint,
  createTerminalImeInputDeduper,
  handleWindowsTerminalSelectionDelete,
  shouldBlinkTerminalCursor,
  shouldCopyWindowsTerminalSelection,
  shouldBufferTerminalImeSwitch,
  shouldUseBracketedImagePaste,
  shouldUseStableTerminalCursor,
  terminalPtyColumns,
  type TerminalTab,
} from '../src/terminals'
import { setLang } from '../src/settings'
import { humanizeTerminalSessionError } from '../src/sessionError'

// Windows：实测 codex 认不出背景、按深色主题出色 → 浅色主题下镜像前景。
const normalizeLightSgr = codexSgrNormalizer('light', true)
const normalizeDarkSgr = codexSgrNormalizer('dark', true)
const normalizeInheritedBackgroundSgr = codexSgrNormalizer('dark', false, true)
// mac/Linux：未验证 codex 用哪套调色板 → 前景一律不动。
const normalizeLightSgrMac = codexSgrNormalizer('light', false)

// 感知亮度，和 src/terminals.ts 里的权重一致。
function luma(r: number, g: number, b: number) {
  return r * 0.299 + g * 0.587 + b * 0.114
}

function fgRgb(sgr: string | null): [number, number, number] {
  const m = /38;2;(\d+);(\d+);(\d+)/.exec(sgr ?? '')
  if (!m) throw new Error(`expected a truecolor foreground, got ${sgr}`)
  return [Number(m[1]), Number(m[2]), Number(m[3])]
}

function key(over: Partial<KeyboardEvent> = {}) {
  return {
    type: 'keydown',
    key: 'c',
    ctrlKey: true,
    shiftKey: false,
    altKey: false,
    metaKey: false,
    ...over,
  } as KeyboardEvent
}

describe('terminal keyboard handling', () => {
  it('keeps native image labels for CLIs that support bracketed paste', () => {
    expect(shouldUseBracketedImagePaste('claude')).toBe(true)
    expect(shouldUseBracketedImagePaste('opencode')).toBe(true)
    expect(shouldUseBracketedImagePaste('grok')).toBe(true)
    expect(shouldUseBracketedImagePaste('codex')).toBe(true)
    expect(shouldUseBracketedImagePaste('kimicode')).toBe(false)
    expect(shouldUseBracketedImagePaste('pi')).toBe(false)
    expect(shouldUseBracketedImagePaste('agy')).toBe(false)
  })

  it('reserves one Windows PTY column for Kimi and Pi borders only', () => {
    expect(terminalPtyColumns('kimicode', 100, 'Win32')).toBe(99)
    expect(terminalPtyColumns('pi', 100, 'Win32')).toBe(98)
    expect(terminalPtyColumns('grok', 100, 'Win32')).toBe(100)
    expect(terminalPtyColumns('kimicode', 100, 'MacIntel')).toBe(100)
    expect(terminalPtyColumns('pi', 100, 'MacIntel')).toBe(100)
    expect(terminalPtyColumns('grok', 100, 'MacIntel')).toBe(100)
  })

  it('copies terminal selection on Windows Ctrl+C', () => {
    expect(shouldCopyWindowsTerminalSelection(key(), true, 'Win32')).toBe(true)
  })

  it('does not intercept Ctrl+C without a terminal selection', () => {
    expect(shouldCopyWindowsTerminalSelection(key(), false, 'Win32')).toBe(false)
  })

  it('does not intercept non-Windows Ctrl+C', () => {
    expect(shouldCopyWindowsTerminalSelection(key(), true, 'MacIntel')).toBe(false)
  })

  it('does not intercept modified or unrelated keys', () => {
    expect(shouldCopyWindowsTerminalSelection(key({ shiftKey: true }), true, 'Win32')).toBe(false)
    expect(shouldCopyWindowsTerminalSelection(key({ key: 'v' }), true, 'Win32')).toBe(false)
  })

  it('recognizes Windows bare Shift as an IME switch only on Windows', () => {
    expect(shouldBufferTerminalImeSwitch(key({ key: 'Shift', ctrlKey: false }), 'Win32')).toBe(true)
    expect(shouldBufferTerminalImeSwitch(key({ key: '', code: 'ShiftLeft', ctrlKey: false }), 'Win32')).toBe(true)
    expect(shouldBufferTerminalImeSwitch(key({ key: 'Shift', ctrlKey: false }), 'MacIntel')).toBe(false)
    expect(shouldBufferTerminalImeSwitch(key({ key: 'Shift' }), 'Win32')).toBe(false)
  })
})

describe('terminal IME input', () => {
  it('forwards a composition result once when switching from Chinese IME to English', () => {
    vi.useFakeTimers()
    try {
      const deduper = createTerminalImeInputDeduper()
      deduper.onCompositionEnd()

      // macOS may produce insertText immediately, then xterm's delayed composition result.
      expect(deduper.consume('g ro k')).toBe('grok')
      expect(deduper.consume('g ro k')).toBeNull()

      const merged = createTerminalImeInputDeduper()
      merged.onCompositionEnd()
      expect(merged.consume('g ro kg ro k')).toBe('grok')

      const compact = createTerminalImeInputDeduper()
      compact.onCompositionEnd()
      expect(compact.consume('grokgrok')).toBe('grok')
    } finally {
      vi.useRealTimers()
    }
  })

  it('does not suppress distinct text after a composition ends', () => {
    vi.useFakeTimers()
    try {
      const deduper = createTerminalImeInputDeduper()
      deduper.onCompositionEnd()

      expect(deduper.consume('果肉可')).toBe('果肉可')
      expect(deduper.consume('!')).toBe('!')
    } finally {
      vi.useRealTimers()
    }
  })

  it('does not affect ordinary input after the composition event loop', () => {
    vi.useFakeTimers()
    try {
      const deduper = createTerminalImeInputDeduper()
      deduper.onCompositionEnd()
      vi.runAllTimers()

      expect(deduper.consume('grok')).toBe('grok')
      expect(deduper.consume('grok')).toBe('grok')
    } finally {
      vi.useRealTimers()
    }
  })

  it('filters a delayed duplicate caused by Caps Lock within the debounce window', () => {
    vi.useFakeTimers()
    try {
      const deduper = createTerminalImeInputDeduper()
      deduper.onCompositionEnd()

      expect(deduper.consume('grok')).toBe('grok')
      vi.advanceTimersByTime(80)
      expect(deduper.consume('grok')).toBeNull()
    } finally {
      vi.useRealTimers()
    }
  })

  it('buffers all Caps Lock IME fragments and flushes one normalized result', () => {
    vi.useFakeTimers()
    try {
      const flushed: string[] = []
      const deduper = createTerminalImeInputDeduper()
      deduper.setFlushHandler((data) => flushed.push(data))
      deduper.onInputMethodSwitch()

      expect(deduper.consume('g')).toBeNull()
      expect(deduper.consume('ro')).toBeNull()
      expect(deduper.consume('kgrok')).toBeNull()
      vi.advanceTimersByTime(120)

      expect(flushed).toEqual(['grok'])
    } finally {
      vi.useRealTimers()
    }
  })

  it('keeps the Caps Lock buffer active when compositionend follows the keydown', () => {
    vi.useFakeTimers()
    try {
      const flushed: string[] = []
      const deduper = createTerminalImeInputDeduper()
      deduper.setFlushHandler((data) => flushed.push(data))
      deduper.onInputMethodSwitch()
      deduper.onCompositionEnd()

      expect(deduper.consume('g')).toBeNull()
      expect(deduper.consume('ro')).toBeNull()
      expect(deduper.consume('k')).toBeNull()
      vi.advanceTimersByTime(120)

      expect(flushed).toEqual(['grok'])
    } finally {
      vi.useRealTimers()
    }
  })

  it('accepts the same text as a new input after the debounce window', () => {
    vi.useFakeTimers()
    try {
      const deduper = createTerminalImeInputDeduper()
      deduper.onCompositionEnd()

      expect(deduper.consume('grok')).toBe('grok')
      vi.advanceTimersByTime(121)
      expect(deduper.consume('grok')).toBe('grok')
    } finally {
      vi.useRealTimers()
    }
  })
})

describe('Codex terminal resume hints', () => {
  it('explains how to recover when another writer owns the session', () => {
    setLang('zh')
    expect(
      codexResumeConfigHint(
        'thread/resume failed: thread 019 already has an active writer (code -32600)',
      ),
    ).toBe('这个会话正在被其他 Codex 进程或客户端占用。请先关闭外部终端或 Codex 客户端中的该会话，然后关闭当前内嵌终端，再重新打开会话。')
  })
})

describe('embedded terminal session errors', () => {
  it('uses terminal-specific recovery instructions for an in-app Chat lock', () => {
    setLang('zh')
    expect(
      humanizeTerminalSessionError(new Error('Session is already open in GUI chat')),
    ).toBe('这个会话已在应用内 Chat 中打开。请先关闭那个 Chat，然后关闭当前内嵌终端，再重新打开会话。')
  })
})

function deleteKey(over: Partial<KeyboardEvent> = {}) {
  return {
    type: 'keydown',
    key: 'Delete',
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    metaKey: false,
    preventDefault: vi.fn(),
    stopImmediatePropagation: vi.fn(),
    ...over,
  } as unknown as KeyboardEvent
}

function selectionTarget(selectedText = '34', row = 5) {
  return {
    hasSelection: () => true,
    getSelection: () => selectedText,
    getSelectionPosition: () => ({
      start: { x: 2, y: row },
      end: { x: 4, y: row },
    }),
    getActiveCursorRow: () => 5,
    clearSelection: vi.fn(),
    input: vi.fn(),
  }
}

describe('terminal selection Delete integration', () => {
  it('clears a valid selection and feeds the edit through xterm input', () => {
    const target = selectionTarget()
    const event = deleteKey()

    expect(
      handleWindowsTerminalSelectionDelete(
        target,
        { text: '123456', cursor: 6, reliable: true },
        event,
        true,
        'Win32',
      ),
    ).toBe(true)
    expect(target.clearSelection).toHaveBeenCalledOnce()
    expect(target.input).toHaveBeenCalledWith(
      '\x1b[D'.repeat(4) + '\x1b[3~'.repeat(2),
      true,
    )
    expect(event.preventDefault).toHaveBeenCalledOnce()
  })

  it('deletes a valid selection when the user presses Backspace', () => {
    const target = selectionTarget()
    const event = deleteKey({ key: 'Backspace' })

    expect(
      handleWindowsTerminalSelectionDelete(
        target,
        { text: '123456', cursor: 6, reliable: true },
        event,
        true,
        'Win32',
      ),
    ).toBe(true)
    expect(target.clearSelection).toHaveBeenCalledOnce()
    expect(target.input).toHaveBeenCalledWith(
      '\x1b[D'.repeat(4) + '\x1b[3~'.repeat(2),
      true,
    )
    expect(event.preventDefault).toHaveBeenCalledOnce()
  })

  it('consumes an unsafe selection without clearing or editing it', () => {
    const target = selectionTarget('34', 3)
    expect(
      handleWindowsTerminalSelectionDelete(
        target,
        { text: '123456', cursor: 6, reliable: true },
        deleteKey(),
        true,
        'Win32',
      ),
    ).toBe(true)
    expect(target.clearSelection).not.toHaveBeenCalled()
    expect(target.input).not.toHaveBeenCalled()
  })

  it('leaves ordinary Delete and dead PTYs to the existing path', () => {
    const target = selectionTarget()
    expect(
      handleWindowsTerminalSelectionDelete(
        { ...target, hasSelection: () => false },
        { text: '123456', cursor: 6, reliable: true },
        deleteKey(),
        true,
        'Win32',
      ),
    ).toBe(false)
    expect(
      handleWindowsTerminalSelectionDelete(
        target,
        { text: '123456', cursor: 6, reliable: true },
        deleteKey(),
        false,
        'Win32',
      ),
    ).toBe(false)
  })
})

// codex-cli 0.144.4 在 Windows 上真实吐出的整套前景色，由
// src-tauri/examples/codex_color_probe.rs 在真 PTY 里抓下来。
const CODEX_FG = {
  body: '204;204;204',
  text: '187;187;187',
  secondary: '144;144;144',
  dim: '90;90;90',
  rule: '47;47;47',
  faintRule: '31;31;31',
  green: '171;223;167',
  cream: '246;226;183',
}

describe('SGR foreground normalization (light theme)', () => {
  it('mirrors codex dark-theme greys into their light-theme twins', () => {
    expect(fgRgb(normalizeLightSgr(`38;2;${CODEX_FG.body}`))).toEqual([51, 51, 51])
    expect(fgRgb(normalizeLightSgr(`38;2;${CODEX_FG.secondary}`))).toEqual([111, 111, 111])
    expect(fgRgb(normalizeLightSgr(`38;2;${CODEX_FG.faintRule}`))).toEqual([224, 224, 224])
  })

  it('keeps the whole brightness ladder ordered instead of flattening it', () => {
    // codex 的层级：正文 > 文字 > 次要 > 暗 > 分隔线。翻成浅色后顺序必须整体反过来，
    // 即正文最深、分隔线最浅 —— 之前的写法把所有 accent 夹到同一亮度，层级就没了。
    const order = ['body', 'text', 'secondary', 'dim', 'rule', 'faintRule'] as const
    const lumas = order.map((k) => luma(...fgRgb(normalizeLightSgr(`38;2;${CODEX_FG[k]}`))))
    for (let i = 1; i < lumas.length; i++) expect(lumas[i]).toBeGreaterThan(lumas[i - 1])
  })

  it('turns codex faint separators light, not stark black on white', () => {
    // 回归：分隔线本来是「深色底上的极淡线」，只修浅色会把它原样留成白底上的死黑线。
    const [r] = fgRgb(normalizeLightSgr(`38;2;${CODEX_FG.faintRule}`))
    expect(r).toBeGreaterThan(200)
    expect(luma(...fgRgb(normalizeLightSgr(`38;2;${CODEX_FG.dim}`)))).toBeGreaterThan(128)
  })

  it('keeps accent hue and saturation, only flipping lightness', () => {
    const green = fgRgb(normalizeLightSgr(`38;2;${CODEX_FG.green}`))
    const cream = fgRgb(normalizeLightSgr(`38;2;${CODEX_FG.cream}`))
    // 浅绿仍是绿（G 最大），奶油仍是暖色（R 最大）——不是糊成一团灰。
    expect(green[1]).toBeGreaterThan(green[0])
    expect(green[1]).toBeGreaterThan(green[2])
    expect(cream[0]).toBeGreaterThan(cream[1])
    expect(cream[1]).toBeGreaterThan(cream[2])
    // 两个 accent 的明暗差异要保留，不能被夹到同一档。
    expect(luma(...green)).not.toBeCloseTo(luma(...cream), 0)
  })

  it('is an involution: mirroring twice returns the original color', () => {
    const once = fgRgb(normalizeLightSgr(`38;2;${CODEX_FG.green}`))
    const twice = fgRgb(normalizeLightSgr(`38;2;${once.join(';')}`))
    for (let i = 0; i < 3; i++) expect(twice[i]).toBeCloseTo(Number(CODEX_FG.green.split(';')[i]), -0.5)
  })

  it('leaves 16-color foregrounds to the xterm theme palette', () => {
    expect(normalizeLightSgr('37')).toBeNull()
    expect(normalizeLightSgr('97')).toBeNull()
    expect(normalizeLightSgr('30')).toBeNull()
    expect(normalizeLightSgr('38;5;6')).toBeNull() // codex 真的会发这个
    expect(normalizeLightSgr('38;5;15')).toBeNull()
  })

  it('resolves the 256-color cube through the same path', () => {
    // 231 = 立方体里的白 (255,255,255) → 镜像成黑。
    expect(fgRgb(normalizeLightSgr('38;5;231'))).toEqual([0, 0, 0])
  })
})

describe('SGR foreground normalization (dark theme)', () => {
  it('leaves every codex foreground alone — its palette already assumes a dark background', () => {
    for (const [, rgb] of Object.entries(CODEX_FG)) {
      expect(normalizeDarkSgr(`38;2;${rgb}`)).toBeNull()
    }
    expect(normalizeDarkSgr('30')).toBeNull()
    expect(normalizeDarkSgr('38;5;0')).toBeNull()
  })
})

describe('SGR foreground normalization (platforms other than Windows)', () => {
  // 只有 Windows 上确认了 codex 会误用深色调色板。mac/Linux 上它可能问得出背景色、
  // 直接出浅色主题的深色字；那时再镜像就会把深字翻成浅字、在白底上彻底看不见。
  it('never touches foregrounds, so codex keeps whatever palette it chose', () => {
    for (const [, rgb] of Object.entries(CODEX_FG)) {
      expect(normalizeLightSgrMac(`38;2;${rgb}`)).toBeNull()
    }
    // 假如 codex 在 mac 上真的出浅色主题（深字配浅底），深字必须原样留着。
    expect(normalizeLightSgrMac('38;2;23;23;23')).toBeNull()
    expect(normalizeLightSgrMac('38;5;231')).toBeNull()
  })

  it('still strips dark backgrounds — that behaviour predates the mirror and stays cross-platform', () => {
    expect(normalizeLightSgrMac('48;2;41;41;41')).toBe('49')
    expect(normalizeLightSgrMac('40')).toBe('49')
  })
})

describe('SGR background normalization', () => {
  it('drops codex panel background under the light theme', () => {
    expect(normalizeLightSgr('48;2;41;41;41')).toBe('49') // codex 唯一用到的背景
    expect(normalizeLightSgr('40')).toBe('49')
    expect(normalizeLightSgr('48;5;0')).toBe('49')
  })

  it('drops light backgrounds under the dark theme', () => {
    expect(normalizeDarkSgr('107')).toBe('49')
    expect(normalizeDarkSgr('48;2;255;255;255')).toBe('49')
  })

  it('does not let extended-color params be mistaken for their own SGR codes', () => {
    // 回归：`38;2;40;…` 里的 40 曾被当成「黑底」改写成 49（`38;5;40` 里的 40 同理），
    // 参数被当成独立 SGR 码，颜色被悄悄改坏。深绿 (40,100,47) 应整段当颜色处理。
    expect(normalizeLightSgr('38;2;40;100;47')).toBe('38;2;155;215;162') // 镜像成浅绿，G 仍最大
    expect(normalizeLightSgr('38;5;40')).not.toContain('38;5;49')
    expect(normalizeDarkSgr('38;2;40;100;47')).toBeNull()
    expect(normalizeDarkSgr('48;2;30;30;30')).toBeNull()
  })

  it('strips every explicit background when a CLI should inherit the app terminal theme', () => {
    expect(normalizeInheritedBackgroundSgr('40')).toBe('49')
    expect(normalizeInheritedBackgroundSgr('107')).toBe('49')
    expect(normalizeInheritedBackgroundSgr('48;2;0;0;0')).toBe('49')
    expect(normalizeInheritedBackgroundSgr('1;38;2;255;255;255;48;5;0;22')).toBe(
      '1;38;2;255;255;255;49;22',
    )
  })
})

describe('SGR normalization plumbing', () => {
  it('normalizes colon-form extended colors', () => {
    expect(normalizeLightSgr('1;38:2:255:255:255;4')).toBe('1;38:2:0:0:0;4')
    expect(normalizeLightSgr('48:5:0')).toBe('49')
  })

  it('preserves surrounding attributes and reports no-ops as null', () => {
    expect(normalizeLightSgr('0')).toBeNull()
    expect(normalizeLightSgr('1;3;23')).toBeNull() // codex 真的会发 3 / 23（斜体）
    expect(normalizeLightSgr('1;38;2;255;255;255;22')).toBe('1;38;2;0;0;0;22')
  })
})

describe('terminal cursor rendering', () => {
  it('keeps the Windows Codex cursor steady', () => {
    expect(shouldBlinkTerminalCursor('codex', 'Win32')).toBe(false)
  })

  it('preserves cursor blinking for other terminals', () => {
    expect(shouldBlinkTerminalCursor('claude', 'Win32')).toBe(true)
    expect(shouldBlinkTerminalCursor('codex', 'MacIntel')).toBe(true)
  })

  it('uses a static cursor only while Windows Codex is working', () => {
    expect(shouldUseStableTerminalCursor('codex', 'working', false, 'Win32')).toBe(true)
    expect(shouldUseStableTerminalCursor('codex', 'idle', false, 'Win32')).toBe(false)
    expect(shouldUseStableTerminalCursor('claude', 'working', false, 'Win32')).toBe(false)
    expect(shouldUseStableTerminalCursor('codex', 'working', false, 'MacIntel')).toBe(false)
    expect(shouldUseStableTerminalCursor('codex', 'working', true, 'Win32')).toBe(false)
  })

  it('tracks the real cursor across wrapped and pasted composer lines', () => {
    const lines = [
      { text: '› [Image #1] curl command', isWrapped: false },
      { text: 'wrapped argument', isWrapped: true },
      { text: '  --insecure', isWrapped: false },
      { text: 'gpt-5.6-sol · C:/workspace', isWrapped: false },
      { text: 'working status', isWrapped: false },
    ]
    const buffer = {
      cursorX: 12,
      cursorY: 2,
      viewportY: 0,
      getLine: (row: number) => lines[row]
        ? {
            isWrapped: lines[row].isWrapped,
            translateToString: () => lines[row].text,
          }
        : undefined,
    }
    const tab = {
      term: {
        buffer: { active: buffer },
        cols: 80,
        rows: lines.length,
      },
    } as unknown as TerminalTab

    captureStableTerminalCursor(tab, true)
    expect(tab.stableCursorX).toBe(12)
    expect(tab.stableCursorRowFromBottom).toBe(2)

    buffer.cursorX = 6
    buffer.cursorY = 1
    captureStableTerminalCursor(tab, true)
    expect(tab.stableCursorX).toBe(6)
    expect(tab.stableCursorRowFromBottom).toBe(3)
  })

  it('keeps the previous multi-line cursor while Codex refreshes its status row', () => {
    const lines = [
      { text: '› [Image #1] curl command', isWrapped: false },
      { text: '  --insecure', isWrapped: false },
      { text: 'gpt-5.6-sol · C:/workspace', isWrapped: false },
      { text: 'working status', isWrapped: false },
    ]
    const buffer = {
      cursorX: 20,
      cursorY: 3,
      viewportY: 0,
      getLine: (row: number) => lines[row]
        ? {
            isWrapped: lines[row].isWrapped,
            translateToString: () => lines[row].text,
          }
        : undefined,
    }
    const tab = {
      term: {
        buffer: { active: buffer },
        cols: 80,
        rows: lines.length,
      },
      stableCursorX: 11,
      stableCursorRowFromBottom: 2,
    } as unknown as TerminalTab

    captureStableTerminalCursor(tab, true)
    expect(tab.stableCursorX).toBe(11)
    expect(tab.stableCursorRowFromBottom).toBe(2)
  })
})
