import { computed } from 'vue'
import { describe, expect, it, vi } from 'vitest'
import { createSessionStarStore, SESSION_STARS_KEY } from '../src/sessionStars'

function fixture(initial: string | null = null) {
  const values = new Map<string, string>()
  if (initial !== null) values.set(SESSION_STARS_KEY, initial)
  const storage = {
    getItem: vi.fn((key: string) => values.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => { values.set(key, value) }),
  }
  return { values, storage, stars: createSessionStarStore(storage) }
}
const a = { id: 'same-id', path: '/sessions/a.jsonl' }
const b = { id: 'another-id', path: '/sessions/b.jsonl' }

describe('Viewer-owned session stars', () => {
  it('starts empty and persists adding/removing without duplicate marks', () => {
    const { stars, storage, values } = fixture()
    expect(stars.isStarred('claude', a)).toBe(false)
    stars.setStarred('claude', a, true)
    stars.setStarred('claude', a, true)
    expect(stars.isStarred('claude', a)).toBe(true)
    expect(JSON.parse(values.get(SESSION_STARS_KEY)!)).toHaveLength(1)
    expect(createSessionStarStore(storage).isStarred('claude', a)).toBe(true)
    stars.setStarred('claude', a, false)
    expect(createSessionStarStore(storage).isStarred('claude', a)).toBe(false)
    expect([...values.keys()]).toEqual([SESSION_STARS_KEY])
  })

  it('isolates different agents with identical IDs and paths', () => {
    const { stars } = fixture()
    stars.setStarred('claude', a, true)
    expect(stars.isStarred('codex', a)).toBe(false)
    expect(stars.isStarred('opencode', a)).toBe(false)
    stars.setStarred('codex', a, true)
    stars.setStarred('claude', a, false)
    expect(stars.isStarred('codex', a)).toBe(true)
  })

  it('keeps stable-ID marks when renamed, archived or compressed', () => {
    const { stars } = fixture()
    stars.setStarred('codex', a, true)
    expect(stars.isStarred('codex', { ...a, path: '/archived/a.jsonl.zst' })).toBe(true)
    expect(stars.isStarred('codex', { ...a, path: 'codex://same-id' })).toBe(true)
    expect(stars.isStarred('codex', { ...a, id: 'a-new-fork-id' })).toBe(false)
  })

  it('supports database virtual paths and path fallback for ID-less records', () => {
    const { stars } = fixture()
    stars.setStarred('opencode', { id: 'ses_test', path: 'opencode://ses_test' }, true)
    expect(stars.isStarred('opencode', { id: 'ses_test', path: 'opencode://ses_test' })).toBe(true)
    const noId = { id: '', path: '/old/legacy.jsonl' }
    stars.setStarred('pi', noId, true)
    expect(stars.isStarred('pi', noId)).toBe(true)
    expect(stars.isStarred('pi', { ...noId, path: '/old/other.jsonl' })).toBe(false)
  })

  it('keeps separator-like IDs unambiguous', () => {
    const { stars } = fixture()
    stars.setStarred('claude', { id: 'id::path::x', path: '/one' }, true)
    expect(stars.isStarred('claude', { id: '', path: 'id::path::x' })).toBe(false)
  })

  it.each(['{bad', '{}', 'null', 'false', '[null,42,false]'])('does not crash on malformed stored preferences: %s', (raw) => {
    const { stars } = fixture(raw)
    expect(stars.isStarred('claude', a)).toBe(false)
  })

  it('renders safely if storage cannot be read', () => {
    const stars = createSessionStarStore({ getItem: () => { throw Error('blocked') }, setItem: vi.fn() })
    expect(stars.isStarred('claude', a)).toBe(false)
  })

  it('does not overwrite unknown marks if reading storage fails during save', () => {
    const { stars, storage, values } = fixture()
    stars.setStarred('claude', a, true)
    const before = values.get(SESSION_STARS_KEY)
    storage.getItem.mockImplementationOnce(() => { throw Error('blocked') })
    expect(() => stars.setStarred('claude', b, true)).toThrow('blocked')
    expect(values.get(SESSION_STARS_KEY)).toBe(before)
    expect(stars.isStarred('claude', a)).toBe(true)
    expect(stars.isStarred('claude', b)).toBe(false)
  })

  it('does not claim a mark was saved when storage rejects the write', () => {
    const { stars, storage } = fixture()
    const visibleInBothPanes = computed(() => stars.isStarred('claude', a))
    storage.setItem.mockImplementationOnce(() => { throw Error('quota') })
    expect(() => stars.setStarred('claude', a, true)).toThrow('quota')
    expect(visibleInBothPanes.value).toBe(false)
    stars.setStarred('claude', a, true)
    expect(visibleInBothPanes.value).toBe(true)
    storage.setItem.mockImplementationOnce(() => { throw Error('quota') })
    expect(() => stars.setStarred('claude', a, false)).toThrow('quota')
    expect(visibleInBothPanes.value).toBe(true)
  })

  it('merges the latest storage rather than erasing another store’s marks', () => {
    const { stars, storage } = fixture()
    const second = createSessionStarStore(storage)
    stars.setStarred('claude', a, true)
    second.setStarred('claude', b, true)
    const restored = createSessionStarStore(storage)
    expect(restored.isStarred('claude', a)).toBe(true)
    expect(restored.isStarred('claude', b)).toBe(true)
  })
})
