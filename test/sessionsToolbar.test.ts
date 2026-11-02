import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import {
  filterSessions,
  resetSessionsToolbar,
  sessionSearch,
  sessionSort,
  sessionsFilterActive,
  sessionStarredOnly,
  sessionSelectMode,
  selectedSessions,
  toggleStarredOnly,
  createSessionFilterLoadQueue,
} from '../src/sessionsToolbar'
import { setSessionStarred } from '../src/sessionStars'
import type { SessionMeta } from '../src/types'

const session = (over: Partial<SessionMeta> & { path: string }): SessionMeta => ({
  id: 'sess-abcdef12',
  fileName: 's.jsonl',
  title: 'A session',
  modified: 0,
  size: 100,
  messageCount: 3,
  ...over,
})

// sessionsToolbar holds module-level state; reset it around every test.
beforeEach(() => resetSessionsToolbar())
afterEach(() => resetSessionsToolbar())

describe('filterSessions', () => {
  const items = [
    session({ path: 'a', title: 'Refactor parser', id: 'id-a', modified: 300, size: 50, messageCount: 9 }),
    session({ path: 'b', title: 'Fix login bug', id: '', modified: 100, size: 300, messageCount: 1 }),
    session({ path: 'c', title: 'Add tests', id: 'id-c', modified: 200, size: 150, messageCount: 5 }),
  ]

  it('returns every item, newest first, with no filters', () => {
    expect(filterSessions(items).map((s) => s.path)).toEqual(['a', 'c', 'b'])
  })

  it('sorts oldest first', () => {
    sessionSort.value = 'oldest'
    expect(filterSessions(items).map((s) => s.path)).toEqual(['b', 'c', 'a'])
  })

  it('sorts by creation time, newest first', () => {
    const created = [
      session({ path: 'a', created: '2026-01-02T00:00:00Z', modified: 300 }),
      session({ path: 'b', created: '2026-01-03T00:00:00Z', modified: 100 }),
      session({ path: 'c', created: '2026-01-01T00:00:00Z', modified: 200 }),
    ]
    sessionSort.value = 'createdRecent'
    expect(filterSessions(created).map((s) => s.path)).toEqual(['b', 'a', 'c'])
  })

  it('sorts by creation time, oldest first', () => {
    const created = [
      session({ path: 'a', created: '2026-01-02T00:00:00Z', modified: 300 }),
      session({ path: 'b', created: '2026-01-03T00:00:00Z', modified: 100 }),
      session({ path: 'c', created: '2026-01-01T00:00:00Z', modified: 200 }),
    ]
    sessionSort.value = 'createdOldest'
    expect(filterSessions(created).map((s) => s.path)).toEqual(['c', 'a', 'b'])
  })

  it('falls back to modified for missing or invalid creation times', () => {
    const created = [
      session({ path: 'valid', created: '1970-01-01T00:00:00.020Z', modified: 5 }),
      session({ path: 'missing', modified: 30 }),
      session({ path: 'invalid', created: 'not-a-date', modified: 10 }),
    ]
    sessionSort.value = 'createdRecent'
    expect(filterSessions(created).map((s) => s.path)).toEqual([
      'missing',
      'valid',
      'invalid',
    ])
  })

  it('breaks creation-time ties by newest modified', () => {
    const tied = [
      session({ path: 'old-update', created: '2026-01-01T00:00:00Z', modified: 10 }),
      session({ path: 'new-update', created: '2026-01-01T00:00:00Z', modified: 20 }),
    ]
    sessionSort.value = 'createdRecent'
    expect(filterSessions(tied).map((s) => s.path)).toEqual(['new-update', 'old-update'])
  })

  it('sorts by size, largest first', () => {
    sessionSort.value = 'size'
    expect(filterSessions(items).map((s) => s.path)).toEqual(['b', 'c', 'a'])
  })

  it('sorts by message count, most first', () => {
    sessionSort.value = 'messages'
    expect(filterSessions(items).map((s) => s.path)).toEqual(['a', 'c', 'b'])
  })

  it('breaks size ties by newest modified', () => {
    const tied = [
      session({ path: 'old', size: 100, modified: 10 }),
      session({ path: 'new', size: 100, modified: 20 }),
    ]
    sessionSort.value = 'size'
    expect(filterSessions(tied).map((s) => s.path)).toEqual(['new', 'old'])
  })

  // 关键词搜索现在走后端（searchSessions），filterSessions 不再做 title/id 文本匹配 ——
  // 留 sessionSearch 这个开关只为让 sessionsFilterActive 触发 App.vue 一次性加载整个项目。
  it('ignores sessionSearch — text matching moved to the backend', () => {
    sessionSearch.value = 'LOGIN'
    expect(filterSessions(items).map((s) => s.path)).toEqual(['a', 'c', 'b'])
  })

  it('does not mutate the input array', () => {
    sessionSort.value = 'oldest'
    const input = [...items]
    filterSessions(input)
    expect(input.map((s) => s.path)).toEqual(['a', 'b', 'c'])
  })
})

describe('sessionsFilterActive', () => {
  it('is false in the default state', () => {
    expect(sessionsFilterActive.value).toBe(false)
  })

  it('is true once a search term is entered', () => {
    sessionSearch.value = 'x'
    expect(sessionsFilterActive.value).toBe(true)
  })

  it('treats a whitespace-only search as inactive', () => {
    sessionSearch.value = '   '
    expect(sessionsFilterActive.value).toBe(false)
  })

  it('is true for any non-default sort', () => {
    sessionSort.value = 'size'
    expect(sessionsFilterActive.value).toBe(true)
  })

  it('activates full-project loading when starred-only is enabled', () => {
    sessionStarredOnly.value = true
    expect(sessionsFilterActive.value).toBe(true)
  })

  it('clears hidden bulk selections when toggling the star filter', () => {
    sessionSelectMode.value = true
    selectedSessions.value = new Set(['a', 'b'])
    toggleStarredOnly()
    expect(sessionStarredOnly.value).toBe(true)
    expect(sessionSelectMode.value).toBe(false)
    expect(selectedSessions.value.size).toBe(0)
  })

  it('filters stars independently of sort order and agent scope', () => {
    const a = session({ path: 'star-a', id: 'toolbar-star-a', modified: 2 })
    const b = session({ path: 'star-b', id: 'toolbar-star-b', modified: 1 })
    try {
      setSessionStarred('claude', b, true)
      sessionStarredOnly.value = true
      sessionSort.value = 'oldest'
      expect(filterSessions([a, b], 'claude')).toEqual([b])
      expect(filterSessions([a, b], 'codex')).toEqual([])
    } finally { setSessionStarred('claude', b, false) }
  })

  it('is true for creation-time sorting', () => {
    sessionSort.value = 'createdRecent'
    expect(sessionsFilterActive.value).toBe(true)
  })
})

describe('full-project filter loading queue', () => {
  it('loads once when filtering starts, not on its own busy transitions', () => {
    const queue = createSessionFilterLoadQueue()
    expect(queue(false, false, false, 300)).toBe(false)
    expect(queue(true, false, false, 300)).toBe(true)
    expect(queue(true, false, true, 300)).toBe(false)
    expect(queue(true, false, false, 300)).toBe(false)
  })

  it('waits for the initial page before fetching old starred sessions', () => {
    const queue = createSessionFilterLoadQueue()
    expect(queue(true, true, false, 0)).toBe(false)
    expect(queue(true, true, false, 300)).toBe(false)
    expect(queue(true, false, false, 300)).toBe(true)
    expect(queue(true, false, false, 300)).toBe(false)
  })

  it('waits for in-flight scrolling and then loads the remaining pages', () => {
    const queue = createSessionFilterLoadQueue()
    expect(queue(false, false, true, 300)).toBe(false)
    expect(queue(true, false, true, 300)).toBe(false)
    expect(queue(true, false, false, 300)).toBe(true)
  })

  it('cancels the queued load when the filter is cleared', () => {
    const queue = createSessionFilterLoadQueue()
    queue(true, true, false, 300)
    expect(queue(false, true, false, 300)).toBe(false)
    expect(queue(false, false, false, 300)).toBe(false)
  })

  it('does not retry a failure forever, but allows a manual refresh', () => {
    const queue = createSessionFilterLoadQueue()
    expect(queue(true, false, false, 300)).toBe(true)
    queue(true, false, true, 300)
    expect(queue(true, false, false, 300)).toBe(false)
    expect(queue(true, true, false, 300)).toBe(false)
    expect(queue(true, false, false, 300)).toBe(true)
  })

  it('includes new totals rather than dropping older marked sessions on refresh', () => {
    const queue = createSessionFilterLoadQueue()
    expect(queue(true, false, false, 300)).toBe(true)
    expect(queue(true, false, false, 301)).toBe(true)
    expect(queue(true, false, false, 301)).toBe(false)
  })
})

describe('resetSessionsToolbar', () => {
  it('restores every field to its default', () => {
    sessionSearch.value = 'q'
    sessionSort.value = 'messages'
    sessionStarredOnly.value = true

    resetSessionsToolbar()

    expect(sessionSearch.value).toBe('')
    expect(sessionSort.value).toBe('recent')
    expect(sessionStarredOnly.value).toBe(false)
    expect(sessionsFilterActive.value).toBe(false)
  })
})
