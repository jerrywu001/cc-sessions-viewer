import { describe, expect, it } from 'vitest'
import {
  formatInlineFileMention,
  inlineFileMentionTextElements,
  inlineFileMentions,
} from '../src/inlineFileMentions'

describe('inline file mentions', () => {
  it('parses quoted paths with spaces and unquoted paths', () => {
    expect(inlineFileMentions('检查 @"src/my file.ts" 和 @src/app.ts')).toMatchObject([
      { path: 'src/my file.ts', token: '@"src/my file.ts"' },
      { path: 'src/app.ts', token: '@src/app.ts' },
    ])
  })

  it('does not treat ordinary @ mentions as files', () => {
    expect(inlineFileMentions('@Computer 打开 @alice')).toEqual([])
  })

  it('calculates Codex ranges in UTF-8 bytes', () => {
    const text = '请检查 @src/a.ts 和 @"中文 file.md"'
    const elements = inlineFileMentionTextElements(text)
    expect(elements.map((element) => element.byteRange)).toEqual([
      { start: new TextEncoder().encode('请检查 ').length, end: new TextEncoder().encode('请检查 @src/a.ts').length },
      { start: new TextEncoder().encode('请检查 @src/a.ts 和 ').length, end: new TextEncoder().encode(text).length },
    ])
  })

  it('quotes paths containing whitespace', () => {
    expect(formatInlineFileMention('src/my file.ts')).toBe('@"src/my file.ts"')
    expect(formatInlineFileMention('src/app.ts')).toBe('@src/app.ts')
  })
})
