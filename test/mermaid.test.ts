import { beforeEach, describe, expect, it, vi } from 'vitest'
import { setTheme } from '../src/settings'

const mocks = vi.hoisted(() => ({
  initialize: vi.fn(),
  render: vi.fn(),
}))

vi.mock('mermaid', () => ({
  default: {
    initialize: mocks.initialize,
    render: mocks.render,
  },
}))

import { renderAllMermaid } from '../src/mermaid'

function mermaidPlaceholder(source: string): string {
  return `<div class="md-mermaid" data-source="${encodeURIComponent(source)}"></div>`
}

describe('renderAllMermaid', () => {
  beforeEach(() => {
    mocks.initialize.mockReset()
    mocks.render.mockReset()
    mocks.render.mockImplementation(async (_id: string, source: string) => ({
      svg: `<svg viewBox="0 0 120 40"><text>${source}</text></svg>`,
    }))
    setTheme('light')
  })

  it('renders the replacement placeholder when its rich-text lifecycle runs after a tab change', async () => {
    const root = document.createElement('div')
    root.innerHTML = mermaidPlaceholder('flowchart TD\nA-->B')

    const firstRender = renderAllMermaid(root)
    // v-rich-html runs again after Vue replaces the session HTML in this node.
    root.innerHTML = mermaidPlaceholder('flowchart TD\nC-->D')
    const currentRender = renderAllMermaid(root)
    await Promise.all([firstRender, currentRender])

    expect(mocks.render).toHaveBeenCalledTimes(2)
    expect(mocks.render).toHaveBeenLastCalledWith(
      expect.any(String),
      'flowchart TD\nC-->D',
    )
    expect(root.querySelector('.md-mermaid')?.dataset.rendered).toBe('1')
    expect(root.querySelector('svg')).not.toBeNull()
  })

  it('reuses a rendered SVG when virtual scrolling recreates the same message row', async () => {
    const source = 'flowchart TD\nVirtualized-->Cached'
    const root = document.createElement('div')
    root.innerHTML = mermaidPlaceholder(source)

    await renderAllMermaid(root)
    root.innerHTML = mermaidPlaceholder(source)
    await renderAllMermaid(root)

    expect(mocks.render).toHaveBeenCalledTimes(1)
    expect(root.querySelector('.md-mermaid')?.dataset.rendered).toBe('1')
    expect(root.querySelector('svg')).not.toBeNull()
  })
})
