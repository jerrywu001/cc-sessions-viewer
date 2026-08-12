import { describe, expect, it } from 'vitest'
import {
  bindInlineImagePlaceholders,
  bindInlineImagePlaceholdersAtAttachmentPositions,
  inlineImagePlaceholders,
  nextInlineImageNumber,
} from '../src/inlineImages'

describe('inline image placeholders', () => {
  it('finds unique placeholders in their text order', () => {
    expect(inlineImagePlaceholders('a [Image #2] b [Image #1] c [Image #2]')).toEqual([
      '[Image #2]',
      '[Image #1]',
    ])
  })

  it('allocates after the highest existing number', () => {
    expect(nextInlineImageNumber('old [Image #4]')).toBe(5)
    expect(nextInlineImageNumber('')).toBe(1)
  })

  it('binds legacy image blocks to the visible placeholder order', () => {
    const images = [{ data: 'a' }, { data: 'b' }]
    expect(bindInlineImagePlaceholders('x [Image #2] y [Image #1]', images)).toEqual([
      { data: 'a', inlinePlaceholder: '[Image #2]' },
      { data: 'b', inlinePlaceholder: '[Image #1]' },
    ])
  })

  it('does not assign placeholders to ordinary attachments in a mixed draft', () => {
    const images = [{ data: 'ordinary' }, { data: 'inline', inlinePlaceholder: '[Image #1]' }]
    expect(bindInlineImagePlaceholders('[Image #1]', images)).toEqual(images)
  })

  it('binds legacy mixed attachments by the overall attachment position', () => {
    const images = [{ data: 'ordinary png' }, { data: 'pasted png' }]
    expect(bindInlineImagePlaceholdersAtAttachmentPositions('[Image #3]', images, [2, 3])).toEqual([
      { data: 'ordinary png' },
      { data: 'pasted png', inlinePlaceholder: '[Image #3]' },
    ])
  })
})
