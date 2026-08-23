import { describe, expect, it } from 'vitest'
import {
  bindInlineImagePlaceholders,
  bindInlineImagePlaceholdersAtAttachmentPositions,
  inlineImagePlaceholders,
  nextInlineImageNumber,
} from '../src/inlineImages'

type TestImage = { data: string; inlinePlaceholder?: string }

describe('inline image placeholders', () => {
  it('finds unique placeholders in their text order', () => {
    expect(inlineImagePlaceholders('a [Image #2] b [Image #1] c [Image #2]')).toEqual([
      '[Image #2]',
      '[Image #1]',
    ])
  })

  it('preserves Pi-style image placeholders when binding history images', () => {
    const images: TestImage[] = [{ data: 'first' }, { data: 'second' }]
    expect(bindInlineImagePlaceholdersAtAttachmentPositions('[#image 1] and [#image 2]', images, [1, 2])).toEqual([
      { data: 'first', inlinePlaceholder: '[#image 1]' },
      { data: 'second', inlinePlaceholder: '[#image 2]' },
    ])
  })

  it('allocates after the highest existing number', () => {
    expect(nextInlineImageNumber('old [Image #4]')).toBe(5)
    expect(nextInlineImageNumber('')).toBe(1)
  })

  it('binds legacy image blocks to the visible placeholder order', () => {
    const images: TestImage[] = [{ data: 'a' }, { data: 'b' }]
    expect(bindInlineImagePlaceholders('x [Image #2] y [Image #1]', images)).toEqual([
      { data: 'a', inlinePlaceholder: '[Image #2]' },
      { data: 'b', inlinePlaceholder: '[Image #1]' },
    ])
  })

  it('does not assign placeholders to ordinary attachments in a mixed draft', () => {
    const images: TestImage[] = [{ data: 'ordinary' }, { data: 'inline', inlinePlaceholder: '[Image #1]' }]
    expect(bindInlineImagePlaceholders('[Image #1]', images)).toEqual(images)
  })

  it('binds legacy mixed attachments by the overall attachment position', () => {
    const images: TestImage[] = [{ data: 'ordinary png' }, { data: 'pasted png' }]
    expect(bindInlineImagePlaceholdersAtAttachmentPositions('[Image #3]', images, [2, 3])).toEqual([
      { data: 'ordinary png' },
      { data: 'pasted png', inlinePlaceholder: '[Image #3]' },
    ])
  })
})
