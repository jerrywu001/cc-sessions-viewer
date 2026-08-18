import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { setLang } from '../../src/settings'
import ChatRail from '../../src/components/ChatRail.vue'
import type { RailEntry } from '../../src/components/ChatRail.vue'

beforeEach(() => {
  setLang('en')
})

const entry = (over: Partial<RailEntry> = {}): RailEntry => ({
  idx: 0,
  seq: 1,
  uuid: 'u1',
  text: 'Hello there',
  summary: 'A short assistant reply.',
  ...over,
})

function factory(entries: RailEntry[], activeIndex: number | null = null) {
  return mount(ChatRail, {
    props: { entries, activeIndex },
  })
}

describe('ChatRail', () => {
  it('renders a dot per prompt and reveals the matching title and assistant summary on focus', async () => {
    const wrapper = factory([
      entry({ idx: 3, seq: 1, text: 'First question' }),
      entry({ idx: 7, seq: 2, uuid: 'u2', text: 'Second question' }),
    ])
    expect(wrapper.findAll('.chat-rail-item')).toHaveLength(2)

    await wrapper.findAll('.chat-rail-item')[0].trigger('focus')
    const preview = wrapper.find('.chat-rail-preview')
    expect(preview.exists()).toBe(true)
    expect(preview.find('.chat-rail-preview-title').text()).toBe('First question')
    expect(preview.find('.chat-rail-preview-summary').text()).toBe('A short assistant reply.')
  })

  it('emits jump with the entry index and uuid when a gutter dot is clicked', async () => {
    const wrapper = factory([
      entry({ idx: 3, seq: 1, uuid: 'u1', text: 'A' }),
      entry({ idx: 7, seq: 2, uuid: 'u2', text: 'B' }),
    ])
    await wrapper.findAll('.chat-rail-item')[1].trigger('click')
    expect(wrapper.emitted('jump')).toEqual([[7, 'u2']])
  })

  it('emits jump when its expanded message preview is clicked', async () => {
    const wrapper = factory([
      entry({ idx: 3, seq: 1, uuid: 'u1', text: 'A' }),
      entry({ idx: 7, seq: 2, uuid: 'u2', text: 'B' }),
    ])
    await wrapper.findAll('.chat-rail-item')[0].trigger('focus')
    await wrapper.find('.chat-rail-preview').trigger('click')
    expect(wrapper.emitted('jump')).toEqual([[3, 'u1']])
  })

  it('marks the active entry by its message index', () => {
    const wrapper = factory(
      [entry({ idx: 3, seq: 1 }), entry({ idx: 7, seq: 2, uuid: 'u2' })],
      7,
    )
    const actives = wrapper.findAll('.chat-rail-item.active')
    expect(actives).toHaveLength(1)
  })

  it('renders nothing when there are no prompts', () => {
    const wrapper = factory([])
    expect(wrapper.find('.chat-rail').exists()).toBe(false)
  })

  it('clears the hovered marker when the rail list scrolls', async () => {
    const wrapper = factory([
      entry({ idx: 3, seq: 1, text: 'First question' }),
      entry({ idx: 7, seq: 2, uuid: 'u2', text: 'Second question' }),
    ])
    const item = wrapper.find('.chat-rail-item')
    await item.trigger('pointermove')
    expect(item.classes()).toContain('hovered')

    await wrapper.find('.chat-rail-list').trigger('scroll')
    expect(item.classes()).not.toContain('hovered')
  })
})
