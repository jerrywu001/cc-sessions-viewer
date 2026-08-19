import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import WelcomeView from '../../src/views/WelcomeView.vue'
import { vTooltip } from '../../src/tooltip'
import { setLang } from '../../src/settings'
import { recents } from '../../src/recents'
import type { ProjectInfo } from '../../src/types'

beforeEach(() => {
  setLang('en')
  localStorage.clear()
  recents.value = {}
})

const proj = (over: Partial<ProjectInfo> = {}): ProjectInfo => ({
  dirName: 'd',
  displayPath: '/work/d',
  sessionCount: 3,
  lastModified: 0,
  exists: true,
  ...over,
})

const factory = (projects: ProjectInfo[] = []) =>
  mount(WelcomeView, {
    props: { agent: 'claude', projects },
    global: { directives: { tooltip: vTooltip } },
  })

describe('WelcomeView', () => {
  it('falls back to the pick-a-project hint when there are no recents', () => {
    const wrapper = factory([proj()])
    expect(wrapper.find('.welcome-recents').exists()).toBe(false)
    expect(wrapper.find('.welcome-hint').text()).toContain('Select a Claude project')
  })

  it('uses the Grok Build label instead of falling back to Claude', () => {
    const wrapper = mount(WelcomeView, {
      props: { agent: 'grok', projects: [] },
      global: { directives: { tooltip: vTooltip } },
    })
    expect(wrapper.find('.welcome-hint').text()).toContain('Select a Grok Build project')
  })

  it('renders the current agent in the global search label', () => {
    setLang('zh')
    const wrapper = factory()

    expect(wrapper.find('.welcome-search-label').text()).toBe('搜索 Claude 会话')
  })

  it('lists recently opened projects most-recent first', () => {
    recents.value = { claude: ['b', 'a'] }
    const wrapper = factory([
      proj({ dirName: 'a', displayPath: '/work/a' }),
      proj({ dirName: 'b', displayPath: '/work/b' }),
    ])
    const names = wrapper.findAll('.welcome-recent-name').map((n) => n.text())
    expect(names).toEqual(['b', 'a'])
  })

  it('drops recents that are no longer in the project list', () => {
    recents.value = { claude: ['gone', 'here'] }
    const wrapper = factory([proj({ dirName: 'here', displayPath: '/work/here' })])
    const names = wrapper.findAll('.welcome-recent-name').map((n) => n.text())
    expect(names).toEqual(['here'])
  })

  it('emits "select-project" when a recent project is clicked', async () => {
    recents.value = { claude: ['a'] }
    const wrapper = factory([proj({ dirName: 'a', displayPath: '/work/a' })])
    await wrapper.find('.welcome-recent').trigger('click')
    expect(wrapper.emitted('select-project')).toEqual([['a']])
  })

  it('emits "switch-agent" from the agent toggle', async () => {
    const wrapper = factory()
    await wrapper.findAll('.welcome-agent')[1].trigger('click')
    expect(wrapper.emitted('switch-agent')).toEqual([['codex']])
  })

  it('marks the active agent button', () => {
    const buttons = factory().findAll('.welcome-agent')
    expect(buttons[0].classes()).toContain('active')
    expect(buttons[1].classes()).not.toContain('active')
  })

  it('emits "open-repo" when the GitHub button is clicked', async () => {
    const wrapper = factory()
    await wrapper.find('.welcome-github').trigger('click')
    expect(wrapper.emitted('open-repo')).toHaveLength(1)
  })
})
