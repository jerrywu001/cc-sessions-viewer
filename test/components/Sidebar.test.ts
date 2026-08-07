import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import Sidebar from '../../src/components/Sidebar.vue'
import { vTooltip } from '../../src/tooltip'
import { setLang } from '../../src/settings'
import type { ProjectInfo } from '../../src/types'

beforeEach(() => setLang('en'))

const project = (over: Partial<ProjectInfo> & { dirName: string }): ProjectInfo => ({
  displayPath: `/projects/${over.dirName}`,
  sessionCount: 1,
  lastModified: 0,
  exists: true,
  ...over,
})

type Props = InstanceType<typeof Sidebar>['$props']
const factory = (props: Partial<Props> = {}) =>
  mount(Sidebar, {
    props: {
      agent: 'claude',
      projects: [],
      activeDir: null,
      showTrash: false,
      projPrefs: {},
      ...props,
    } as Props,
    global: { directives: { tooltip: vTooltip } },
  })

describe('Sidebar', () => {
  it('shows the agent name in the sub-header', () => {
    expect(factory({ agent: 'claude' }).find('.sidebar-sub').text()).toContain('Claude')
    expect(factory({ agent: 'codex' }).find('.sidebar-sub').text()).toContain('Codex')
  })

  it('renders one row per project', () => {
    const wrapper = factory({
      projects: [project({ dirName: 'a' }), project({ dirName: 'b' })],
    })
    expect(wrapper.findAll('.proj-item')).toHaveLength(2)
  })

  it('shows the empty-state message when there are no projects', () => {
    const wrapper = factory({ projects: [] })
    expect(wrapper.findAll('.proj-item')).toHaveLength(0)
    expect(wrapper.text()).toContain('No Claude sessions')
  })

  it('emits switch-agent when an agent tab is clicked', async () => {
    const wrapper = factory({ agent: 'claude' })
    await wrapper.findAll('.agent-switch button')[1].trigger('click')
    expect(wrapper.emitted('switch-agent')![0]).toEqual(['codex'])
  })

  it('emits select-project with the project dirName', async () => {
    const wrapper = factory({ projects: [project({ dirName: 'proj-x' })] })
    await wrapper.find('.proj-item').trigger('click')
    expect(wrapper.emitted('select-project')![0]).toEqual(['proj-x'])
  })

  it('emits context-menu on right-click', async () => {
    const wrapper = factory({ projects: [project({ dirName: 'p' })] })
    await wrapper.find('.proj-item').trigger('contextmenu')
    expect(wrapper.emitted('context-menu')).toHaveLength(1)
  })

  it('emits open-settings from the footer button', async () => {
    const wrapper = factory()
    await wrapper.find('.trash-tab').trigger('click')
    expect(wrapper.emitted('open-settings')).toHaveLength(1)
  })

  it('orders pinned projects first and sunk projects last', () => {
    const wrapper = factory({
      projects: [
        project({ dirName: 'normal' }),
        project({ dirName: 'pinned' }),
        project({ dirName: 'sunk' }),
      ],
      projPrefs: { 'claude::pinned': 'pinned', 'claude::sunk': 'sunk' },
    })
    const names = wrapper.findAll('.proj-name').map((n) => n.text())
    expect(names).toEqual(['pinned', 'normal', 'sunk'])
  })

  it('uses a persisted drag order within each pin group', () => {
    const wrapper = factory({
      projects: [
        project({ dirName: 'first' }),
        project({ dirName: 'second' }),
        project({ dirName: 'pinned' }),
      ],
      projPrefs: { 'claude::pinned': 'pinned' },
      projectOrder: ['second', 'first', 'pinned'],
    })
    expect(wrapper.findAll('.proj-name').map((node) => node.text())).toEqual(['pinned', 'second', 'first'])
  })

  it('keeps empty bookmarks first until a manual order is saved', () => {
    const wrapper = factory({
      projects: [
        project({ dirName: 'project' }),
        project({ dirName: 'bookmark', bookmarked: true, sessionCount: 0 }),
      ],
    })

    expect(wrapper.findAll('.proj-name').map((node) => node.text())).toEqual(['bookmark', 'project'])
  })

  it('lets persisted order place a project before an empty bookmark', () => {
    const wrapper = factory({
      projects: [
        project({ dirName: 'bookmark', bookmarked: true, sessionCount: 0 }),
        project({ dirName: 'project' }),
      ],
      projectOrder: ['project', 'bookmark'],
    })

    expect(wrapper.findAll('.proj-name').map((node) => node.text())).toEqual(['project', 'bookmark'])
  })

  it('emits the reordered root projects after a drag and drop', async () => {
    const wrapper = factory({
      projects: [project({ dirName: 'first' }), project({ dirName: 'second' })],
    })
    const [first, second] = wrapper.findAll('.proj-item')
    Object.defineProperty(second.element, 'getBoundingClientRect', {
      value: () => ({ top: 0, height: 20 }),
    })
    Object.defineProperty(document, 'elementFromPoint', {
      configurable: true,
      value: vi.fn(() => second.element),
    })

    await first.trigger('pointerenter')
    first.find('.proj-drag-handle').element.dispatchEvent(
      new MouseEvent('pointerdown', { button: 0, clientX: 0, clientY: 0, bubbles: true, cancelable: true }),
    )
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 8, clientY: 18, bubbles: true, cancelable: true }))
    await wrapper.vm.$nextTick()
    expect(second.classes()).toContain('drop-after')
    window.dispatchEvent(new MouseEvent('pointerup', { clientX: 8, clientY: 18, bubbles: true, cancelable: true }))

    expect(wrapper.emitted('reorder-projects')![0]).toEqual([['second', 'first']])
  })

  it('allows dragging a project above an empty bookmark', async () => {
    const wrapper = factory({
      projects: [
        project({ dirName: 'project' }),
        project({ dirName: 'bookmark', bookmarked: true, sessionCount: 0 }),
      ],
    })
    const [bookmark, regularProject] = wrapper.findAll('.proj-item')
    Object.defineProperty(bookmark.element, 'getBoundingClientRect', {
      value: () => ({ top: 0, height: 20 }),
    })
    Object.defineProperty(regularProject.element, 'getBoundingClientRect', {
      value: () => ({ left: 0, top: 21, width: 240, height: 20 }),
    })
    Object.defineProperty(document, 'elementFromPoint', {
      configurable: true,
      value: vi.fn(() => bookmark.element),
    })

    await regularProject.trigger('pointerenter')
    regularProject.find('.proj-drag-handle').element.dispatchEvent(
      new MouseEvent('pointerdown', { button: 0, clientX: 8, clientY: 30, bubbles: true, cancelable: true }),
    )
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 8, clientY: 3, bubbles: true, cancelable: true }))
    await wrapper.vm.$nextTick()

    expect(bookmark.classes()).toContain('drop-before')
    window.dispatchEvent(new MouseEvent('pointerup', { clientX: 8, clientY: 3, bubbles: true, cancelable: true }))
    expect(wrapper.emitted('reorder-projects')![0]).toEqual([['project', 'bookmark']])
  })

  it('shows a floating project preview while dragging', async () => {
    const wrapper = factory({
      projects: [project({ dirName: 'first' }), project({ dirName: 'second' })],
    })
    const [first, second] = wrapper.findAll('.proj-item')
    Object.defineProperty(first.element, 'getBoundingClientRect', {
      value: () => ({ left: 10, top: 20, width: 240, height: 30 }),
    })
    Object.defineProperty(second.element, 'getBoundingClientRect', {
      value: () => ({ top: 50, height: 30 }),
    })
    Object.defineProperty(document, 'elementFromPoint', {
      configurable: true,
      value: vi.fn(() => second.element),
    })

    await first.trigger('pointerenter')
    first.find('.proj-drag-handle').element.dispatchEvent(
      new MouseEvent('pointerdown', { button: 0, clientX: 20, clientY: 30, bubbles: true, cancelable: true }),
    )
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 30, clientY: 60, bubbles: true, cancelable: true }))
    await wrapper.vm.$nextTick()

    const preview = document.body.querySelector<HTMLElement>('.proj-item-drag-preview')
    expect(preview?.textContent).toContain('first')
    expect(preview?.style.left).toBe('20px')
    expect(preview?.style.top).toBe('50px')
    expect(preview?.style.width).toBe('240px')
    expect(first.classes()).toContain('dragging')

    window.dispatchEvent(new MouseEvent('pointerup', { clientX: 30, clientY: 60, bubbles: true, cancelable: true }))
    await wrapper.vm.$nextTick()
    expect(document.body.querySelector('.proj-item-drag-preview')).toBeNull()
  })

  it('renders the drag handle only while a project row is hovered', async () => {
    const wrapper = factory({
      projects: [project({ dirName: 'first' }), project({ dirName: 'second' })],
    })
    const [first, second] = wrapper.findAll('.proj-item')

    expect(first.find('.proj-drag-handle').exists()).toBe(false)
    expect(first.find('.proj-name').element.previousElementSibling).toBeNull()
    await second.trigger('pointerenter')
    expect(second.find('.proj-drag-handle').exists()).toBe(true)
    expect(second.find('.proj-name').element.previousElementSibling).toBe(second.find('.proj-drag-handle').element)
    await second.trigger('pointerleave')
    expect(second.find('.proj-drag-handle').exists()).toBe(false)
  })

  it('keeps the insertion indicator visible while the pointer stays over a project row', async () => {
    const wrapper = factory({
      projects: [project({ dirName: 'first' }), project({ dirName: 'second' })],
    })
    const [first, second] = wrapper.findAll('.proj-item')
    Object.defineProperty(second.element, 'getBoundingClientRect', {
      value: () => ({ top: 0, height: 20 }),
    })
    Object.defineProperty(document, 'elementFromPoint', {
      configurable: true,
      value: vi.fn(() => second.find('.proj-name').element),
    })

    await first.trigger('pointerenter')
    first.find('.proj-drag-handle').element.dispatchEvent(
      new MouseEvent('pointerdown', { button: 0, clientX: 0, clientY: 0, bubbles: true, cancelable: true }),
    )
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 8, clientY: 3, bubbles: true, cancelable: true }))
    await wrapper.vm.$nextTick()
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 10, clientY: 4, bubbles: true, cancelable: true }))
    await wrapper.vm.$nextTick()

    expect(second.classes()).toContain('drop-before')
    window.dispatchEvent(new MouseEvent('pointerup', { clientX: 10, clientY: 4, bubbles: true, cancelable: true }))
    expect(wrapper.emitted('reorder-projects')).toBeUndefined()
  })

  it('renders a pin dot only for pinned projects', () => {
    const wrapper = factory({
      projects: [project({ dirName: 'p' }), project({ dirName: 'q' })],
      projPrefs: { 'claude::p': 'pinned' },
    })
    const items = wrapper.findAll('.proj-item')
    expect(items[0].find('.pin-dot').exists()).toBe(true)
    expect(items[1].find('.pin-dot').exists()).toBe(false)
  })

  it('marks the active project, but not while the trash view is open', () => {
    const projects = [project({ dirName: 'here' })]
    expect(
      factory({ projects, activeDir: 'here', showTrash: false }).find('.proj-item').classes(),
    ).toContain('active')
    expect(
      factory({ projects, activeDir: 'here', showTrash: true }).find('.proj-item').classes(),
    ).not.toContain('active')
  })

  it('flags a project whose directory no longer exists', () => {
    const wrapper = factory({ projects: [project({ dirName: 'gone', exists: false })] })
    expect(wrapper.find('.proj-item').classes()).toContain('missing')
  })

  it('shows the session count and the short project name', () => {
    const wrapper = factory({
      projects: [project({ dirName: 'x', displayPath: '/a/b/my-proj', sessionCount: 12 })],
    })
    expect(wrapper.find('.proj-name').text()).toBe('my-proj')
    expect(wrapper.find('.proj-count').text()).toBe('12')
  })
})
