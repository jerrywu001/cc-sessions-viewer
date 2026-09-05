import { describe, expect, it, vi } from 'vitest'
import {
  pasteIntentForEvent,
  runEmbeddedCliPaste,
  type PasteKeyEvent,
} from '../src/embeddedCliPaste'

const key = (overrides: Partial<PasteKeyEvent> = {}): PasteKeyEvent => ({
  type: 'keydown',
  key: 'v',
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  metaKey: false,
  ...overrides,
})

describe('embedded CLI paste intents', () => {
  it('uses Ctrl+V as the unified Windows/Linux entry', () => {
    expect(pasteIntentForEvent(key({ ctrlKey: true }), 'Win32')).toBe('unified')
    expect(pasteIntentForEvent(key({ ctrlKey: true }), 'Linux')).toBe('unified')
    expect(pasteIntentForEvent(key({ ctrlKey: true, altKey: true }), 'Win32')).toBeNull()
  })

  it('uses macOS Control+V for images and Command+V as the normal unified paste', () => {
    expect(pasteIntentForEvent(key({ ctrlKey: true }), 'MacIntel')).toBe('image')
    expect(pasteIntentForEvent(key({ metaKey: true }), 'MacIntel')).toBe('unified')
    expect(pasteIntentForEvent(key({ ctrlKey: true, metaKey: true }), 'MacIntel')).toBeNull()
    expect(pasteIntentForEvent(key({ altKey: true, metaKey: true }), 'MacIntel')).toBeNull()
  })

  it('does not assign an Alt+V intent', () => {
    expect(pasteIntentForEvent(key({ altKey: true }), 'Win32')).toBeNull()
  })
})

describe('embedded CLI paste execution', () => {
  it('prefers an image over text for the unified shortcut', async () => {
    const paste = vi.fn()
    const pasteImage = vi.fn()
    const saveImage = vi.fn(async () => 'clipboard.png')
    const result = await runEmbeddedCliPaste(
      'unified',
      { paste, pasteImage },
      {
        readImages: async () => [{ data: 'png-data', mediaType: 'image/png' }],
        readText: async () => 'fallback text',
      },
      { saveImage },
    )
    expect(result).toBe('handled-image')
    expect(saveImage).toHaveBeenCalledWith('png-data', 'image/png')
    expect(paste).toHaveBeenCalledWith('clipboard.png')
    expect(pasteImage).not.toHaveBeenCalled()
  })

  it('uses the direct image channel only for the macOS image shortcut', async () => {
    const paste = vi.fn()
    const pasteImage = vi.fn()
    const result = await runEmbeddedCliPaste(
      'image',
      { paste, pasteImage },
      {
        readImages: async () => [{ data: 'png-data', mediaType: 'image/png' }],
        readText: async () => 'fallback text',
      },
      { saveImage: vi.fn(async () => 'clipboard.png') },
    )
    expect(result).toBe('handled-image')
    expect(pasteImage).toHaveBeenCalledWith('clipboard.png')
    expect(paste).not.toHaveBeenCalled()
  })

  it('writes text when no image is available', async () => {
    const paste = vi.fn()
    const result = await runEmbeddedCliPaste(
      'unified',
      { paste },
      { readImages: async () => [], readText: async () => '多行\ntext' },
      { saveImage: vi.fn() },
    )
    expect(result).toBe('handled-text')
    expect(paste).toHaveBeenCalledWith('多行\ntext')
  })

  it('falls back for an empty clipboard and does not write', async () => {
    const paste = vi.fn()
    const result = await runEmbeddedCliPaste(
      'unified',
      { paste },
      { readImages: async () => [], readText: async () => '' },
      { saveImage: vi.fn() },
    )
    expect(result).toBe('fallback')
    expect(paste).not.toHaveBeenCalled()
  })

  it('does not downgrade a macOS image shortcut into text', async () => {
    const paste = vi.fn()
    const result = await runEmbeddedCliPaste(
      'image',
      { paste },
      { readImages: async () => [], readText: async () => 'text' },
      { saveImage: vi.fn() },
    )
    expect(result).toBe('fallback')
    expect(paste).not.toHaveBeenCalled()
  })

  it('keeps the text shortcut text-only', async () => {
    const paste = vi.fn()
    const result = await runEmbeddedCliPaste(
      'text',
      { paste },
      {
        readImages: async () => { throw new Error('must not read images') },
        readText: async () => 'plain text',
      },
      { saveImage: vi.fn() },
    )
    expect(result).toBe('handled-text')
    expect(paste).toHaveBeenCalledWith('plain text')
  })

  it('uses the native image path channel when provided', async () => {
    const paste = vi.fn()
    const pasteImage = vi.fn()
    const readImages = vi.fn(async () => [{ data: 'wrong', mediaType: 'image/png' }])
    const saveImagePath = vi.fn(async () => '/tmp/clipboard.png')
    const result = await runEmbeddedCliPaste(
      'image',
      { paste, pasteImage },
      { readImages, readText: async () => '' },
      { saveImage: vi.fn(), saveImagePath },
    )
    expect(result).toBe('handled-image')
    expect(saveImagePath).toHaveBeenCalledOnce()
    expect(readImages).not.toHaveBeenCalled()
    expect(pasteImage).toHaveBeenCalledWith('/tmp/clipboard.png')
  })

  it('uses the native image path for macOS unified paste before falling back to text', async () => {
    const paste = vi.fn()
    const pasteImage = vi.fn()
    const saveImagePath = vi.fn(async () => '/tmp/clipboard.png')
    const result = await runEmbeddedCliPaste(
      'unified',
      { paste, pasteImage },
      { readImages: async () => { throw new Error('must not use WebView image read') }, readText: async () => 'text' },
      { saveImage: vi.fn(), saveImagePath },
    )
    expect(result).toBe('handled-image')
    expect(saveImagePath).toHaveBeenCalledOnce()
    expect(pasteImage).toHaveBeenCalledWith('/tmp/clipboard.png')
    expect(paste).not.toHaveBeenCalled()
  })

  it('falls back from a native unified image read to text', async () => {
    const paste = vi.fn()
    const saveImagePath = vi.fn(async () => null)
    const result = await runEmbeddedCliPaste(
      'unified',
      { paste },
      { readImages: async () => { throw new Error('must not use WebView image read') }, readText: async () => 'plain text' },
      { saveImage: vi.fn(), saveImagePath },
    )
    expect(result).toBe('handled-text')
    expect(paste).toHaveBeenCalledWith('plain text')
  })

  it('keeps unified paste usable when the native image command is unavailable', async () => {
    const paste = vi.fn()
    const result = await runEmbeddedCliPaste(
      'unified',
      { paste },
      { readText: async () => 'plain text' },
      { saveImage: vi.fn(), saveImagePath: async () => { throw new Error('unknown command') } },
    )
    expect(result).toBe('handled-text')
    expect(paste).toHaveBeenCalledWith('plain text')
  })

  it('does not append text after image reading fails', async () => {
    const paste = vi.fn()
    const result = await runEmbeddedCliPaste(
      'unified',
      { paste },
      { readImages: async () => { throw new Error('clipboard denied') }, readText: async () => 'text' },
      { saveImage: vi.fn() },
    )
    expect(result).toBe('failed')
    expect(paste).not.toHaveBeenCalled()
  })
})
