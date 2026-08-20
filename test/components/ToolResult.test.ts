import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import ToolResult from '../../src/components/ToolResult.vue'
import type { Block } from '../../src/types'
import { setLang } from '../../src/settings'

beforeEach(() => setLang('en'))

function blk(over: Partial<Block> & { kind: Block['kind'] }): Block {
  return { isError: false, ...over }
}

describe('ToolResult', () => {
  it('labels a plain result and stays collapsed', () => {
    const wrapper = mount(ToolResult, {
      props: { block: blk({ kind: 'tool_result', text: 'output' }) },
    })
    expect(wrapper.find('.label').text()).toBe('Tool result')
    expect(wrapper.find('details').attributes('open')).toBeUndefined()
    expect(wrapper.find('pre').text()).toBe('output')
  })

  it('marks an error result', () => {
    const wrapper = mount(ToolResult, {
      props: { block: blk({ kind: 'tool_result', text: 'bad', isError: true }) },
    })
    expect(wrapper.find('.thinking-label').text()).toBe('Tool result · error')
    expect(wrapper.find('details').classes()).toContain('thinking-block')
    expect(wrapper.find('details').classes()).toContain('tool-result-error')
    expect(wrapper.find('.thinking-icon').exists()).toBe(true)
  })

  it('renders a Bash textual unified diff as an expanded file-change card', () => {
    const wrapper = mount(ToolResult, {
      props: {
        block: blk({
          kind: 'tool_result',
          text: 'diff --git a/lib/routes/example.dart b/lib/routes/example.dart\nindex abc..def 100644\n--- a/lib/routes/example.dart\n+++ b/lib/routes/example.dart\n@@ -1 +1,2 @@\n old\n+new',
        }),
      },
    })

    expect(wrapper.find('.label').text()).toBe('File change · example.dart')
    expect(wrapper.find('details').attributes('open')).toBeDefined()
    expect(wrapper.find('details').classes()).toContain('text-diff-result')
    expect(wrapper.find('.diff-stat').text()).toBe('+1 −0')
    expect(wrapper.find('.diff-add').text()).toBe('+new')
  })

  it('renders a diff result as a codex patch card', () => {
    const wrapper = mount(ToolResult, {
      props: {
        block: blk({
          kind: 'tool_result',
          filePath: '/deep/nested/file.ts',
          fileChangeType: 'delete',
          diff: [
            {
              oldStart: 1,
              newStart: 1,
              lines: [
                { kind: 'add', oldNo: null, newNo: 1, text: 'x' },
                { kind: 'add', oldNo: null, newNo: 2, text: 'y' },
                { kind: 'del', oldNo: 1, newNo: null, text: 'z' },
              ],
            },
          ],
        }),
      },
    })
    expect(wrapper.find('details').exists()).toBe(false)
    expect(wrapper.find('.codex-patch-file').exists()).toBe(true)
    expect(wrapper.find('.codex-patch-path').text()).toBe('/deep/nested/file.ts')
    expect(wrapper.find('.codex-patch-op').text()).toBe('Deleted')
    expect(wrapper.find('.codex-patch-stat').text()).toBe('+2-1')
    expect(wrapper.find('.codex-patch-line.add').exists()).toBe(true)
    expect(wrapper.find('.codex-patch-line.del').exists()).toBe(true)
  })

  it('renders a filePath-only result as an empty codex patch card', () => {
    const wrapper = mount(ToolResult, {
      props: {
        block: blk({
          kind: 'tool_result',
          filePath: '/deep/nested/empty.md',
          fileChangeType: 'delete',
          text: 'File changed.',
        }),
      },
    })
    expect(wrapper.find('details').exists()).toBe(false)
    expect(wrapper.find('.codex-patch-path').text()).toBe('/deep/nested/empty.md')
    expect(wrapper.find('.codex-patch-op').text()).toBe('Deleted')
    expect(wrapper.text()).not.toContain('File changed.')
  })

  it('omits the diff-stat element when there is no diff', () => {
    const wrapper = mount(ToolResult, {
      props: { block: blk({ kind: 'tool_result', text: 'plain' }) },
    })
    expect(wrapper.find('.diff-stat').exists()).toBe(false)
  })

  it('adds the in-user modifier class when inUser is set', () => {
    const wrapper = mount(ToolResult, {
      props: { block: blk({ kind: 'tool_result', text: 'o' }), inUser: true },
    })
    expect(wrapper.find('details').classes()).toContain('in-user')
  })
})
