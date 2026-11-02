import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import SessionsTopbar from '../../src/components/topbar/SessionsTopbar.vue'
import { vTooltip } from '../../src/tooltip'
import { setLang } from '../../src/settings'
import {
  resetSessionsToolbar,
  sessionSearch,
  sessionSort,
} from '../../src/sessionsToolbar'
import type { SessionMeta } from '../../src/types'

// Only the search bar + sort dropdown still live in the topbar; the with-id
// filter, select-mode entry, and batch operations were moved into
// SessionsView's body header (see `list-head-actions` in SessionsView.test.ts)
// to reduce the "two parallel icon rows" density at the top of the window.

beforeEach(() => {
  setLang('en')
  resetSessionsToolbar()
})

const session = (over: Partial<SessionMeta> = {}): SessionMeta => ({
  path: '/p/a.jsonl',
  fileName: 'a.jsonl',
  id: 'aaaa1111-bbbb-2222-cccc-333344445555',
  title: 'A session',
  cwd: '/p',
  size: 100,
  messageCount: 1,
  modified: 0,
  ...over,
})

const factory = (sessions: SessionMeta[] = [session(), session({ path: '/p/b.jsonl' })]) =>
  mount(SessionsTopbar, {
    props: { sessions },
    global: { directives: { tooltip: vTooltip } },
  })

describe('SessionsTopbar', () => {
  it('binds the search box to the shared search ref (debounced)', async () => {
    const wrapper = factory()
    await wrapper.find('.ct-search-input').setValue('parser')
    // 防抖：打字立即落到本地 draft，~220ms 后才同步到共享 ref
    expect(sessionSearch.value).toBe('')
    await new Promise((r) => setTimeout(r, 280))
    expect(sessionSearch.value).toBe('parser')
  })

  it('clears the search from the clear button', async () => {
    sessionSearch.value = 'parser'
    const wrapper = factory()
    await wrapper.find('.ct-search .ct-btn').trigger('click')
    expect(sessionSearch.value).toBe('')
  })

  it('lists every sort option and applies creation-time sorting', async () => {
    const wrapper = factory()
    await wrapper.find('.ct-scope-btn').trigger('click')
    const items = wrapper.findAll('.ct-scope-item')
    expect(items).toHaveLength(6)

    await items[2].trigger('click')
    expect(sessionSort.value).toBe('createdRecent')
  })

  it('focuses the search box on the ⌘F / Ctrl+F shortcut', () => {
    const wrapper = mount(SessionsTopbar, {
      props: { sessions: [session(), session({ path: '/p/b.jsonl' })] },
      global: { directives: { tooltip: vTooltip } },
      attachTo: document.body,
    })
    const isMac = /Mac/i.test(navigator.platform)
    window.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'f', metaKey: isMac, ctrlKey: !isMac }),
    )
    expect(document.activeElement).toBe(wrapper.find('.ct-search-input').element)
    wrapper.unmount()
  })

  it('filters to starred sessions with one accessible toggle', async () => {
    const wrapper = factory()
    const toggle = wrapper.find('.session-star-filter')
    expect(toggle.exists()).toBe(true)
    expect(toggle.attributes('aria-pressed')).toBe('false')
    expect(toggle.attributes('aria-label')).toBe('Starred only')
    await toggle.trigger('click')
    expect(toggle.attributes('aria-pressed')).toBe('true')
    await toggle.trigger('click')
    expect(toggle.attributes('aria-pressed')).toBe('false')
  })

  it('does not render the action bar — those controls live in SessionsView now', () => {
    const wrapper = factory()
    expect(wrapper.find('.ct-actions').exists()).toBe(false)
  })
})
