// Viewer-owned preferences only: never modify agent history, config or SQLite.
// A shared ref keeps all split panes in sync. IDs survive rename/archive/compression;
// ID-less records fall back to their path. Different agents never share a mark.
import { ref } from 'vue'
import type { Agent, SessionMeta } from './types'

type SessionIdentity = Pick<SessionMeta, 'id' | 'path'>
type StarStorage = Pick<Storage, 'getItem' | 'setItem'>
export const SESSION_STARS_KEY = 'sessionStars:v1'

function starKey(agent: Agent, session: SessionIdentity): string {
  return JSON.stringify([agent, session.id ? 'id' : 'path', session.id || session.path])
}

function readStars(storage: StarStorage, strict = false): Set<string> {
  try {
    const data: unknown = JSON.parse(storage.getItem(SESSION_STARS_KEY) || '[]')
    return new Set(Array.isArray(data) ? data.filter((key): key is string => typeof key === 'string') : [])
  } catch (error) {
    if (strict) throw error
    return new Set()
  }
}

/** Storage injection keeps regression tests away from real app preferences. */
export function createSessionStarStore(storage: StarStorage) {
  const keys = ref<ReadonlySet<string>>(readStars(storage))
  function isStarred(agent: Agent, session: SessionIdentity): boolean {
    return keys.value.has(starKey(agent, session))
  }
  function setStarred(agent: Agent, session: SessionIdentity, starred: boolean) {
    // Merge the latest persisted keys to avoid erasing unrelated marks from another window.
    const next = readStars(storage, true)
    const key = starKey(agent, session)
    if (starred) next.add(key)
    else next.delete(key)
    // Commit the UI only after the atomic localStorage write succeeds.
    // Quota/permission failures propagate to the caller's inline error.
    storage.setItem(SESSION_STARS_KEY, JSON.stringify([...next].sort()))
    keys.value = next
  }
  return { isStarred, setStarred }
}

const stars = createSessionStarStore({
  getItem: key => localStorage.getItem(key),
  setItem: (key, value) => localStorage.setItem(key, value),
})
export const isSessionStarred = stars.isStarred
export const setSessionStarred = stars.setStarred
