import { describe, expect, it, beforeEach } from 'vitest'
import {
  createViewTab,
  markViewTabViewed,
  removeViewTab,
  viewTabs,
  viewTabStatusKind,
  migrateViewTabsProjectKey,
  visibleViewTabs,
} from '../src/viewTabs'
import { chatSessions, findChatBySourceSession, type ChatSession } from '../src/chatSessions'

function chatSession(over: Partial<ChatSession> = {}): ChatSession {
  return {
    uiId: 1,
    chatId: null,
    agent: 'codex',
    projectKey: 'proj',
    cwd: 'C:\\repo',
    sessionId: 'thread-1',
    title: 'Thread',
    msgs: [],
    status: 'running',
    turnState: 'idle',
    turnStartedAt: 100,
    lastTurnOutcome: null,
    pendingPermissions: [],
    pendingQuestions: [],
    ...over,
  } as ChatSession
}

describe('GUI chat ownership', () => {
  beforeEach(() => {
    viewTabs.value = []
    chatSessions.value = []
  })

  it('stops a live chat when its tab has already switched to read mode', async () => {
    const session = chatSession({ uiId: 7 })
    chatSessions.value = [session]
    const tab = createViewTab({
      type: 'session',
      agent: 'codex',
      projectKey: 'proj',
      chatSession: session,
    })

    await removeViewTab(tab.uiId)

    expect(viewTabs.value).toHaveLength(0)
    expect(chatSessions.value).toHaveLength(0)
  })

  it('finds only a live owner when stale failed sessions share the thread id', () => {
    const failed = chatSession({ uiId: 8, status: 'error' })
    const live = chatSession({ uiId: 9 })
    chatSessions.value = [failed, live]

    expect(findChatBySourceSession('codex', 'thread-1')?.uiId).toBe(live.uiId)
  })
})

describe('migrateViewTabsProjectKey', () => {
  beforeEach(() => {
    viewTabs.value = []
  })

  it('moves every view tab from the old project key to the new one', () => {
    const a = createViewTab({ type: 'session', agent: 'claude', projectKey: 'worktree:/p/wt' })
    const b = createViewTab({ type: 'chat', agent: 'claude', projectKey: 'worktree:/p/wt' })
    const other = createViewTab({ type: 'session', agent: 'claude', projectKey: 'other' })

    migrateViewTabsProjectKey('worktree:/p/wt', 'realkey')

    expect(a.projectKey).toBe('realkey')
    expect(b.projectKey).toBe('realkey')
    expect(other.projectKey).toBe('other') // 无关项目不动
    // 迁移后旧 key 查不到、新 key 查得到 —— 这正是「点 List tab 后标签栏消失」的根因修复。
    expect(visibleViewTabs('claude', 'worktree:/p/wt')).toHaveLength(0)
    expect(visibleViewTabs('claude', 'realkey')).toHaveLength(2)
  })

  it('is a no-op when old and new keys are equal', () => {
    const a = createViewTab({ type: 'session', agent: 'claude', projectKey: 'k' })
    migrateViewTabsProjectKey('k', 'k')
    expect(a.projectKey).toBe('k')
  })
})

describe('GUI chat tab status', () => {
  beforeEach(() => {
    viewTabs.value = []
  })

  it('maps the chat lifecycle to the same visual states as TUI tabs', () => {
    const session = chatSession({ status: 'spawning' })
    const tab = createViewTab({
      type: 'chat',
      agent: 'codex',
      projectKey: 'proj',
      chatSession: session,
    })

    expect(viewTabStatusKind(tab, false)).toBe('none')

    session.status = 'running'
    session.turnState = 'running'
    expect(viewTabStatusKind(tab, false)).toBe('working')

    session.pendingQuestions = [{} as ChatSession['pendingQuestions'][number]]
    expect(viewTabStatusKind(tab, false)).toBe('blocked')

    session.pendingQuestions = []
    expect(viewTabStatusKind(tab, false)).toBe('working')

    session.turnState = 'idle'
    session.lastTurnOutcome = 'failed'
    expect(viewTabStatusKind(tab, false)).toBe('error')

    session.lastTurnOutcome = 'completed'
    session.status = 'exited'
    expect(viewTabStatusKind(tab, false)).toBe('exited')
  })

  it('shows done only for a successful background turn that has not been viewed', () => {
    const session = chatSession({ lastTurnOutcome: 'completed', turnStartedAt: 123 })
    const tab = createViewTab({
      type: 'chat',
      agent: 'codex',
      projectKey: 'proj',
      chatSession: session,
    })

    expect(viewTabStatusKind(tab, false)).toBe('done')
    expect(viewTabStatusKind(tab, true)).toBe('none')

    markViewTabViewed(tab)

    expect(tab.lastViewedChatTurnStartedAt).toBe(123)
    expect(viewTabStatusKind(tab, false)).toBe('none')
  })

  it('does not mark a running turn as viewed when the user switches away', () => {
    const session = chatSession({ turnState: 'running', turnStartedAt: 456 })
    const tab = createViewTab({
      type: 'chat',
      agent: 'codex',
      projectKey: 'proj',
      chatSession: session,
    })

    markViewTabViewed(tab)

    expect(tab.lastViewedChatTurnStartedAt).toBe(0)
    session.turnState = 'idle'
    session.lastTurnOutcome = 'completed'
    expect(viewTabStatusKind(tab, false)).toBe('done')
  })

  it('hides a viewed error until a different error occurs', () => {
    const session = chatSession({
      lastTurnOutcome: 'failed',
      turnStartedAt: 789,
      errorMessage: 'first failure',
    })
    const tab = createViewTab({
      type: 'chat',
      agent: 'codex',
      projectKey: 'proj',
      chatSession: session,
    })

    expect(viewTabStatusKind(tab, false)).toBe('error')
    expect(viewTabStatusKind(tab, true)).toBe('none')

    markViewTabViewed(tab)

    expect(viewTabStatusKind(tab, false)).toBe('none')
    session.errorMessage = 'second failure'
    expect(viewTabStatusKind(tab, false)).toBe('error')
  })
})
