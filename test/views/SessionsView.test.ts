import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { vTooltip } from '../../src/tooltip'
import { setLang } from '../../src/settings'

// 关键词搜索走后端：mock 掉，让规格用例驱动返回值。
// cancelSearch 在每次新输入时调一次，桩成 no-op 即可。
const { searchMock, cancelMock, usageMock, gitHasRepoMock, gitRepositoryStateMock } = vi.hoisted(() => ({
  searchMock: vi.fn(),
  cancelMock: vi.fn().mockResolvedValue(undefined),
  gitHasRepoMock: vi.fn(),
  gitRepositoryStateMock: vi.fn(),
  // SessionsView wires sessionUsage to an IntersectionObserver — our jsdom stub
  // never reports visibility, so the mock is mostly unused but must exist.
  usageMock: vi.fn().mockResolvedValue({
    inputTokens: 0,
    outputTokens: 0,
    cacheCreationInputTokens: 0,
    cacheCreation1hInputTokens: 0,
    cacheReadInputTokens: 0,
    reasoningOutputTokens: 0,
    total: 0,
  }),
}))
let _id = 0
vi.mock('../../src/api', () => ({
  searchSessions: searchMock,
  cancelSearch: cancelMock,
  nextSearchRequestId: () => ++_id,
  sessionUsage: usageMock,
  gitHasRepo: gitHasRepoMock,
  gitRepositoryState: gitRepositoryStateMock,
  gitSwitchBranch: vi.fn(),
  gitDeleteBranch: vi.fn(),
  gitCreateBranch: vi.fn(),
}))

import SessionsView from '../../src/views/SessionsView.vue'
import {
  resetSessionsToolbar,
  selectedSessions,
  sessionSearch,
  sessionSelectMode,
  sessionSort,
} from '../../src/sessionsToolbar'
import type { ProjectInfo, SearchHit, SessionMeta } from '../../src/types'
import { PaneActionsKey, type PaneActions } from '../../src/paneActions'

const stubPaneActions = {} as PaneActions

// 每个 case 后卸载它挂载的 wrapper。否则旧实例的 watch 仍订阅 sessionSearch，
// 一旦设值，所有历史实例都会一起调 searchSessions，把 mockResolvedValueOnce
// 提前消费掉，当前 case 拿到 undefined → 渲染空列表。
enableAutoUnmount(afterEach)

beforeEach(() => {
  setLang('en')
  resetSessionsToolbar()
  searchMock.mockReset()
  cancelMock.mockClear()
  cancelMock.mockResolvedValue(undefined)
  gitHasRepoMock.mockReset()
  gitHasRepoMock.mockResolvedValue(true)
  gitRepositoryStateMock.mockReset()
  gitRepositoryStateMock.mockResolvedValue({
    branch: 'main', branches: ['main'], changeCount: 0,
  })
  _id = 0
})

// 防抖 ≥ 280ms + 后端 promise；给个 320ms 余量再 flush，等 visibleSessions 切到 searchHits。
async function waitForSearchSettle() {
  await new Promise((r) => setTimeout(r, 320))
  await flushPromises()
}

// 把元数据数组包成后端会返回的 SearchHit 形状。
const toHits = (sessions: SessionMeta[]): SearchHit[] =>
  sessions.map((s) => ({
    projectKey: 'proj',
    projectDisplay: '/work/proj',
    matchedField: 'title' as const,
    snippet: s.title,
    session: s,
  }))

const project: ProjectInfo = {
  dirName: 'proj',
  displayPath: '/work/proj',
  sessionCount: 1,
  lastModified: 0,
  exists: true,
}

const session = (over: Partial<SessionMeta> = {}): SessionMeta => ({
  id: 'sess-abcdef12',
  fileName: 's.jsonl',
  path: '/work/proj/s.jsonl',
  title: 'A session',
  modified: 0,
  size: 1024,
  messageCount: 3,
  ...over,
})

type Props = InstanceType<typeof SessionsView>['$props']
const factory = (sessions: SessionMeta[] = [session()]) =>
  mount(SessionsView, {
    props: {
      agent: 'claude',
      project,
      sessions,
      sessionTotal: sessions.length,
      loading: false,
      loadingMore: false,
    } as Props,
    global: {
      directives: { tooltip: vTooltip },
      provide: { [PaneActionsKey as symbol]: stubPaneActions },
    },
  })

describe('SessionsView', () => {
  it('shows the project Git branch in the header action area', async () => {
    const wrapper = factory()
    await flushPromises()

    expect(wrapper.find('.list-head-branch .git-branch-name').text()).toBe('main')
    expect(wrapper.find('.list-head-branch-divider').exists()).toBe(true)
  })

  it('emits "open" when a session card is clicked', async () => {
    const wrapper = factory()
    await wrapper.find('.session-card').trigger('click')
    expect(wrapper.emitted('open')).toHaveLength(1)
  })

  it('opens the export menu without navigating into the session', async () => {
    const wrapper = factory()
    await wrapper.find('.export-menu-wrap .icon-btn').trigger('click')
    expect(wrapper.find('.export-menu').exists()).toBe(true)
    expect(wrapper.emitted('open')).toBeUndefined()
  })

  // Regression: clicking the menu's padding/gap (the container, not an item)
  // used to bubble to the .session-card and open the session.
  it('does not navigate when the export menu padding is clicked', async () => {
    const wrapper = factory()
    await wrapper.find('.export-menu-wrap .icon-btn').trigger('click')
    await wrapper.find('.export-menu').trigger('click')
    expect(wrapper.emitted('open')).toBeUndefined()
  })

  it('emits "export" — and not "open" — when a menu item is clicked', async () => {
    const wrapper = factory()
    await wrapper.find('.export-menu-wrap .icon-btn').trigger('click')
    await wrapper.findAll('.export-menu-item')[0].trigger('click')

    const exported = wrapper.emitted('export')
    expect(exported).toHaveLength(1)
    expect(exported![0][1]).toBe('md')
    expect(wrapper.emitted('open')).toBeUndefined()
  })

  describe('toolbar filters', () => {
    it('renders only the sessions returned by the backend search', async () => {
      const a = session({ path: 'a', title: 'Refactor parser' })
      const b = session({ path: 'b', title: 'Fix login bug' })
      searchMock.mockResolvedValueOnce(toHits([a]))
      const wrapper = factory([a, b])
      sessionSearch.value = 'parser'
      await waitForSearchSettle()
      expect(wrapper.findAll('.session-card')).toHaveLength(1)
      expect(wrapper.text()).toContain('Refactor parser')
      expect(searchMock).toHaveBeenCalledWith(
        'claude',
        'parser',
        expect.any(Number),
        'proj',
        'keyword',
      )
    })

    it('shows the no-match state when the backend search returns nothing', async () => {
      searchMock.mockResolvedValueOnce([])
      const wrapper = factory([session({ path: 'a', title: 'Refactor parser' })])
      sessionSearch.value = 'zzznoop'
      await waitForSearchSettle()
      expect(wrapper.findAll('.session-card')).toHaveLength(0)
      expect(wrapper.text()).toContain('No sessions match')
    })

    it('keeps the project-empty state separate from the no-match state', () => {
      expect(factory([]).text()).toContain('No sessions in this project')
    })
  })

  describe('keyword highlight', () => {
    it('wraps the matched keyword in the title in a .kw-hit', async () => {
      const s = session({ path: 'a', title: 'workflow with obsidian' })
      searchMock.mockResolvedValueOnce(toHits([s]))
      const wrapper = factory([s])
      sessionSearch.value = 'obsidian'
      await waitForSearchSettle()
      const hits = wrapper.findAll('.session-title-text .kw-hit')
      expect(hits).toHaveLength(1)
      expect(hits[0].text()).toBe('obsidian')
    })

    it('highlights a match in the session ID', async () => {
      const s = session({ path: 'a', title: 'no match here', id: 'abcdef12' })
      searchMock.mockResolvedValueOnce(toHits([s]))
      const wrapper = factory([s])
      sessionSearch.value = 'abcd'
      await waitForSearchSettle()
      const hits = wrapper.findAll('.session-id-text .kw-hit')
      expect(hits).toHaveLength(1)
      expect(hits[0].text()).toBe('abcd')
    })

    it('renders no highlight when there is no active search', () => {
      const wrapper = factory([
        session({ path: 'a', title: 'workflow with obsidian' }),
      ])
      expect(wrapper.find('.kw-hit').exists()).toBe(false)
      // 标题文本仍完整无缺
      expect(wrapper.find('.session-title-text').text()).toBe('workflow with obsidian')
    })
  })

  describe('header actions', () => {
    // 用 aria-label 找按钮，避免依赖 list-head-actions 里按钮的位置 ——
    // 现在那行里同时有 hash 过滤 / 批量选择入口 / 新建 / 刷新 / 删除项目。
    const findByLabel = (wrapper: ReturnType<typeof factory>, label: string) =>
      wrapper.findAll('.list-head-actions .icon-btn').find((b) =>
        b.attributes('aria-label')?.startsWith(label),
      )!

    it('emits "new-session" (TUI) when the first menu item is clicked', async () => {
      const wrapper = factory()
      await flushPromises()
      // Click the "+" button to open the dropdown
      await findByLabel(wrapper, 'New session').trigger('click')
      // Menu items: TUI / GUI / Terminal / Git Changes / Split H / Split V
      const items = wrapper.findAll('.new-menu-item')
      expect(items.length).toBe(6)
      await items[0].trigger('click')
      expect(wrapper.emitted('new-session')).toHaveLength(1)
    })

    it('emits "new-gui-session" when the GUI menu item is clicked', async () => {
      const wrapper = factory()
      await flushPromises()
      await findByLabel(wrapper, 'New session').trigger('click')
      const items = wrapper.findAll('.new-menu-item')
      await items[1].trigger('click')
      expect(wrapper.emitted('new-gui-session')).toHaveLength(1)
    })

    it('emits "new-shell" when the terminal menu item is clicked', async () => {
      const wrapper = factory()
      await flushPromises()
      await findByLabel(wrapper, 'New session').trigger('click')
      const items = wrapper.findAll('.new-menu-item')
      await items[2].trigger('click')
      expect(wrapper.emitted('new-shell')).toHaveLength(1)
    })

    it('hides new-session and refresh when the project directory is missing', () => {
      const wrapper = mount(SessionsView, {
        props: {
          agent: 'claude',
          project: { ...project, exists: false },
          sessions: [],
          sessionTotal: 0,
          loading: false,
          loadingMore: false,
        } as Props,
        global: {
          directives: { tooltip: vTooltip },
          provide: { [PaneActionsKey as symbol]: stubPaneActions },
        },
      })
      // 目录已不存在 → 新建会话 / 刷新都没意义；单格（showExitPane 未传）也无「退出分屏」。
      // 没有会话 (sessions=[]) → 「批量选择」入口也不渲染 → 顶栏动作区为空。
      expect(wrapper.findAll('.list-head-actions .icon-btn')).toHaveLength(0)
      expect(wrapper.find('.list-head-actions .new-menu-wrap').exists()).toBe(false)
      expect(wrapper.find('.list-head-actions .icon-btn[aria-label^="Reload"]').exists()).toBe(
        false,
      )
    })

    it('emits "refresh" when the header refresh button is clicked', async () => {
      const wrapper = factory()
      await findByLabel(wrapper, 'Reload').trigger('click')
      expect(wrapper.emitted('refresh')).toHaveLength(1)
    })

    it('shows the exit-pane button only when showExitPane is set, and emits "exit-pane"', async () => {
      const wrapper = factory()
      // 单格默认（showExitPane 未传）不显示「退出分屏」按钮
      expect(findByLabel(wrapper, 'Exit split pane')).toBeUndefined()
      await wrapper.setProps({ showExitPane: true })
      await findByLabel(wrapper, 'Exit split pane').trigger('click')
      expect(wrapper.emitted('exit-pane')).toHaveLength(1)
    })

    it('shows the "select multiple" entry only when there are 2+ sessions', () => {
      const w1 = factory([session()])
      expect(
        w1.find('.list-head-actions .icon-btn[aria-label^="Select multiple"]').exists(),
      ).toBe(false)
      const w2 = factory([session(), session({ path: '/work/proj/b.jsonl' })])
      expect(
        w2.find('.list-head-actions .icon-btn[aria-label^="Select multiple"]').exists(),
      ).toBe(true)
    })

    it('shows the creation sort shortcut only when there are 2+ sessions', () => {
      const one = factory([session()])
      expect(findByLabel(one, 'Sort by creation time')).toBeUndefined()

      const two = factory([session(), session({ path: '/work/proj/b.jsonl' })])
      expect(findByLabel(two, 'Sort by creation time')).toBeDefined()
    })

    it('colors only the active creation-time direction without an active button background', async () => {
      const wrapper = factory([session(), session({ path: '/work/proj/b.jsonl' })])
      const button = findByLabel(wrapper, 'Sort by creation time')

      expect(button.classes()).not.toContain('active')
      expect(button.find('.creation-sort-icon__arrow--up').classes()).not.toContain('is-active')
      expect(button.find('.creation-sort-icon__arrow--down').classes()).not.toContain('is-active')

      await button.trigger('click')
      expect(sessionSort.value).toBe('createdRecent')
      expect(button.classes()).not.toContain('active')
      expect(button.attributes('aria-label')).toBe('Created: newest first')
      expect(button.find('.creation-sort-icon__arrow--down').classes()).toContain('is-active')
      expect(button.find('.creation-sort-icon__arrow--up').classes()).not.toContain('is-active')

      await button.trigger('click')
      expect(sessionSort.value).toBe('createdOldest')
      expect(button.classes()).not.toContain('active')
      expect(button.attributes('aria-label')).toBe('Created: oldest first')
      expect(button.find('.creation-sort-icon__arrow--up').classes()).toContain('is-active')
      expect(button.find('.creation-sort-icon__arrow--down').classes()).not.toContain('is-active')
    })

    it('hides the creation sort shortcut in select mode', () => {
      sessionSelectMode.value = true
      const wrapper = factory([session(), session({ path: '/work/proj/b.jsonl' })])
      expect(findByLabel(wrapper, 'Sort by creation time')).toBeUndefined()
    })

    it('flips into select mode from the entry button', async () => {
      const wrapper = factory([session(), session({ path: '/work/proj/b.jsonl' })])
      await findByLabel(wrapper, 'Select multiple').trigger('click')
      expect(sessionSelectMode.value).toBe(true)
    })

    it('emits batch-delete from the danger button in select mode', async () => {
      sessionSelectMode.value = true
      selectedSessions.value = new Set(['/work/proj/s.jsonl'])
      const wrapper = factory()
      await wrapper.find('.list-head-actions .icon-btn.danger').trigger('click')
      expect(wrapper.emitted('batch-delete')).toHaveLength(1)
    })

    it('emits batch-export with the picked format in select mode', async () => {
      sessionSelectMode.value = true
      selectedSessions.value = new Set(['/work/proj/s.jsonl'])
      const wrapper = factory()
      await wrapper.find('.list-head-actions .export-menu-wrap .icon-btn').trigger('click')
      const items = wrapper.findAll('.list-head-actions .export-menu-item')
      expect(items).toHaveLength(3) // md / html / json
      await items[1].trigger('click') // HTML
      expect(wrapper.emitted('batch-export')).toEqual([['html']])
    })
  })

  describe('missing-directory tag', () => {
    it('shows the tag when the project directory no longer exists', () => {
      const wrapper = mount(SessionsView, {
        props: {
          agent: 'claude',
          project: { ...project, exists: false },
          sessions: [],
          sessionTotal: 0,
          loading: false,
          loadingMore: false,
        } as Props,
        global: {
          directives: { tooltip: vTooltip },
          provide: { [PaneActionsKey as symbol]: stubPaneActions },
        },
      })
      expect(wrapper.find('.dir-missing-tag').exists()).toBe(true)
    })

    it('hides the tag when the directory exists', () => {
      expect(factory().find('.dir-missing-tag').exists()).toBe(false)
    })

    // 目录已不存在 → 恢复 / 刷新 这些依赖项目目录的卡片操作没有意义，隐藏。
    // 重命名只动 ~/.claude/projects 下的 JSONL，与项目目录无关 —— 保留。
    it('hides chat and resume on session cards when the directory is missing', () => {
      const wrapper = mount(SessionsView, {
        props: {
          agent: 'claude',
          project: { ...project, exists: false },
          sessions: [session()],
          sessionTotal: 1,
          loading: false,
          loadingMore: false,
        } as Props,
        global: {
          directives: { tooltip: vTooltip },
          provide: { [PaneActionsKey as symbol]: stubPaneActions },
        },
      })
      expect(wrapper.find('.title-rename-ic').exists()).toBe(true)
      // 只剩 在文件管理器中显示 / 导出 / 置顶 / 沉底 / 删除
      expect(wrapper.findAll('.session-actions .icon-btn')).toHaveLength(5)
    })

    it('keeps every card action when the directory exists', () => {
      const wrapper = factory()
      expect(wrapper.find('.title-rename-ic').exists()).toBe(true)
      // chat(claude) / resume / reveal / export / pin / sink / delete
      expect(wrapper.findAll('.session-actions .icon-btn')).toHaveLength(7)
    })
  })

  describe('select mode', () => {
    it('shows a checkbox on each card and hides the row actions', () => {
      sessionSelectMode.value = true
      const wrapper = factory([session({ path: 'a' })])
      expect(wrapper.find('.list-check').exists()).toBe(true)
      expect(wrapper.find('.session-actions').exists()).toBe(false)
      expect(wrapper.find('.title-rename-ic').exists()).toBe(false)
      expect(wrapper.find('.session-id-copy').exists()).toBe(false)
    })

    it('toggles selection — and does not open — when a card is clicked', async () => {
      sessionSelectMode.value = true
      const wrapper = factory([session({ path: 'a' })])

      await wrapper.find('.session-card').trigger('click')
      expect(selectedSessions.value.has('a')).toBe(true)
      expect(wrapper.emitted('open')).toBeUndefined()

      await wrapper.find('.session-card').trigger('click')
      expect(selectedSessions.value.has('a')).toBe(false)
    })

    it('marks the row as selected via the list-selected class', async () => {
      sessionSelectMode.value = true
      selectedSessions.value = new Set(['a'])
      const wrapper = factory([session({ path: 'a' })])
      expect(wrapper.find('.session-card').classes()).toContain('list-selected')
    })
  })
})
