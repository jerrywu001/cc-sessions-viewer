import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import ExportHistoryView from '../src/views/ExportHistoryView.vue'
import {
  clearExportHistory,
  history,
  recordExport,
} from '../src/exportHistory'

beforeEach(() => {
  localStorage.clear()
  history.value = []
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('export history persistence', () => {
  it('records and persists a Grok Build export', () => {
    recordExport({
      path: '/Users/me/.grok/projects/demo/sessions/abc.jsonl',
      title: 'Fix export history',
      agent: 'grok',
      sessionId: 'abc',
      cwd: '/Users/me/demo',
      exportedAt: 1_700_000_000_000,
    })

    expect(history.value).toHaveLength(1)
    expect(history.value[0].agent).toBe('grok')
    expect(JSON.parse(localStorage.getItem('exportHistory:v1')!)).toMatchObject([
      { agent: 'grok', path: '/Users/me/.grok/projects/demo/sessions/abc.jsonl' },
    ])
  })

  it('normalizes incomplete legacy records without dropping valid exports', async () => {
    localStorage.setItem('exportHistory:v1', JSON.stringify([
      { path: '/sessions/legacy.jsonl', agent: 'grok' },
      { filePath: '/exports/no-source.md', agent: 'grok' },
      { path: '/sessions/unknown.jsonl', agent: 'future-agent' },
    ]))
    vi.resetModules()
    const mod = await import('../src/exportHistory')

    expect(mod.history.value).toEqual([{
      path: '/sessions/legacy.jsonl',
      agent: 'grok',
      title: '',
      sessionId: '',
      cwd: undefined,
      exportedAt: 0,
    }])
  })
})

describe('ExportHistoryView', () => {
  it('renders an explicit empty state instead of a blank view', () => {
    const wrapper = mount(ExportHistoryView, {
      global: { directives: { tooltip: () => {} } },
    })

    expect(wrapper.classes()).toContain('export-history-root')
    expect(wrapper.find('.empty').text()).toContain('No exports yet')
  })

  it('renders a saved Grok Build record', () => {
    recordExport({
      path: '/sessions/grok.jsonl',
      title: 'Grok export',
      agent: 'grok',
      sessionId: 'grok',
      exportedAt: 1_700_000_000_000,
    })
    const wrapper = mount(ExportHistoryView, {
      global: { directives: { tooltip: () => {} } },
    })

    expect(wrapper.findAll('.session-card')).toHaveLength(1)
    expect(wrapper.text()).toContain('Grok Build')
    expect(wrapper.text()).toContain('Grok export')
  })

  it('emits the selected record when a history card is clicked', async () => {
    const record = {
      path: '/sessions/click.jsonl',
      title: 'Clickable export',
      agent: 'codex' as const,
      sessionId: 'click',
      exportedAt: 1_700_000_000_000,
    }
    recordExport(record)
    const wrapper = mount(ExportHistoryView, {
      global: { directives: { tooltip: () => {} } },
    })

    await wrapper.find('.session-card').trigger('click')
    expect(wrapper.emitted('open')?.[0]?.[0]).toEqual(record)
  })
})

afterEach(() => clearExportHistory())
