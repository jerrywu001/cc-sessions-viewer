import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}))

import {
  chatEffectiveEffortForTest,
  chatOnDeltaForTest,
  chatOnMsgForTest,
  chatOnResultForTest,
  canSteerQueued,
  enqueuePrompt,
  interruptChat,
  parseRetryLine,
  reconnectChats,
  removeQueued,
  respondPermission,
  respondQuestion,
  startChat,
  steerQueued,
  shouldDropDuplicatePatchOutputForTest,
} from '../src/chatSessions'
import { rememberChatGuiPreference } from '../src/chatGuiPreferences'
import type { ChatPermissionRequest, ChatQuestionRequest, Msg } from '../src/types'

describe('chatSessions streaming delta batching', () => {
  it('publishes accumulated text at most once per 50ms and flushes the tail on stop', () => {
    vi.useFakeTimers()
    const session = {
      live: null,
      retry: null,
      toolActivity: null,
      toolActivities: [],
    } as any

    chatOnDeltaForTest(session, { index: 0, phase: 'start', kind: 'text' })
    chatOnDeltaForTest(session, { index: 0, phase: 'delta', kind: 'text', text: '你' })
    chatOnDeltaForTest(session, { index: 0, phase: 'delta', kind: 'text', text: '好' })
    expect(session.live).toEqual({ kind: 'text', text: '' })

    vi.advanceTimersByTime(49)
    expect(session.live).toEqual({ kind: 'text', text: '' })
    vi.advanceTimersByTime(1)
    expect(session.live).toEqual({ kind: 'text', text: '你好' })

    chatOnDeltaForTest(session, { index: 0, phase: 'delta', kind: 'text', text: '！' })
    chatOnDeltaForTest(session, { index: 0, phase: 'stop', kind: 'text' })
    expect(session.live).toEqual({ kind: 'text', text: '你好！' })
    vi.runOnlyPendingTimers()
    expect(session.live).toEqual({ kind: 'text', text: '你好！' })
  })
})

afterEach(() => {
  vi.useRealTimers()
})

describe('chatSessions Claude API-key compatibility', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('drops Claude effort for API-key sessions', () => {
    expect(
      chatEffectiveEffortForTest({
        agent: 'claude',
        model: 'claude-opus-4-8',
        effort: 'high',
        apiKeySource: 'ANTHROPIC_API_KEY',
      }),
    ).toBeUndefined()
  })

  it('keeps Claude effort for subscription sessions', () => {
    expect(
      chatEffectiveEffortForTest({
        agent: 'claude',
        model: 'claude-opus-4-8',
        effort: 'high',
        apiKeySource: 'none',
      }),
    ).toBe('high')
  })

  it('starts Claude chat without forcing a default model or effort', async () => {
    invokeMock.mockResolvedValueOnce({ chatId: 1, processModel: 'longLivedStdin' })
    const s = await startChat({
      agent: 'claude',
      projectKey: 'proj',
      cwd: '/tmp',
      title: 'Chat',
    })
    expect(s.model).toBeUndefined()
    expect(s.effort).toBeUndefined()
    expect(invokeMock).toHaveBeenCalledWith(
      'agent_chat_start',
      expect.objectContaining({
        agent: 'claude',
        model: undefined,
        effort: undefined,
      }),
    )
  })

  it('interrupts the current Claude turn by restarting the long-lived process with resume', async () => {
    invokeMock.mockResolvedValueOnce(undefined)
    invokeMock.mockResolvedValueOnce({ chatId: 8, processModel: 'longLivedStdin' })
    const session = {
      chatId: 7,
      agent: 'claude',
      cwd: '/tmp',
      sessionId: 'sess-1',
      permissionMode: 'acceptEdits',
      model: undefined,
      effort: undefined,
      apiKeySource: 'none',
      processModel: 'longLivedStdin',
      applied: { permissionMode: 'acceptEdits', model: undefined, effort: undefined },
      status: 'running',
      turnState: 'running',
      turnStartedAt: Date.now(),
      lastTurnMs: 0,
      msgs: [],
      queue: [],
      submittedQueue: [],
      pendingSteerIds: [],
      live: { kind: 'text', text: 'hello' },
      pendingPermissions: [],
      pendingQuestions: [],
    } as any
    await interruptChat(session)
    expect(invokeMock).toHaveBeenNthCalledWith(1, 'agent_chat_stop', { id: 7 })
    expect(invokeMock).toHaveBeenNthCalledWith(2, 'agent_chat_start', {
      agent: 'claude',
      cwd: '/tmp',
      sessionId: 'sess-1',
      permissionMode: 'acceptEdits',
      model: undefined,
      effort: undefined,
      fork: undefined,
      useReclaude: false,
    })
    expect(session.chatId).toBe(8)
    expect(session.status).toBe('running')
    expect(session.turnState).toBe('idle')
    expect(session.live).toBeNull()
    expect(session.msgs).toHaveLength(1)
    expect(session.msgs[0].role).toBe('user')
    expect(session.msgs[0].blocks[0].text).toBe('[Request interrupted by user]')
  })
})

describe('chatSessions live tool result routing', () => {
  it('drops duplicate apply_patch success output after a structured file change', () => {
    const existing: Msg[] = [{
      role: 'assistant',
      blocks: [{
        kind: 'tool_result',
        isError: false,
        toolId: 'file-change-1',
        filePath: 'test-88.md',
        diff: [{
          oldStart: 0,
          newStart: 1,
          lines: [{ kind: 'add', oldNo: null, newNo: 1, text: '1111===2222' }],
        }],
      }],
    } as Msg]
    const incoming = {
      role: 'user',
      blocks: [{
        kind: 'tool_result',
        isError: false,
        toolId: 'call-1',
        text: 'Exit code: 0\nWall time: 0.1 seconds\nOutput:\nSuccess. Updated the following files:\nA test-88.md\n',
      }],
    } as Msg

    expect(shouldDropDuplicatePatchOutputForTest(existing, incoming)).toBe(true)
  })

  it('keeps ordinary tool output after a structured file change', () => {
    const existing: Msg[] = [{
      role: 'assistant',
      blocks: [{
        kind: 'tool_result',
        isError: false,
        toolId: 'file-change-1',
        filePath: 'test-88.md',
        diff: [{
          oldStart: 0,
          newStart: 1,
          lines: [{ kind: 'add', oldNo: null, newNo: 1, text: '1111===2222' }],
        }],
      }],
    } as Msg]
    const incoming = {
      role: 'user',
      blocks: [{
        kind: 'tool_result',
        isError: false,
        toolId: 'call-2',
        text: 'hello.md  7B\n',
      }],
    } as Msg

    expect(shouldDropDuplicatePatchOutputForTest(existing, incoming)).toBe(false)
  })
})

describe('chatSessions compact live tool activity', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('tracks tool calling, result, and failure without retaining tool output', async () => {
    invokeMock.mockResolvedValueOnce({ chatId: 81, processModel: 'longLivedStdin' })
    const session = await startChat({ agent: 'claude', projectKey: 'p', cwd: '/tmp', title: 'C' })
    session.turnState = 'running'

    chatOnMsgForTest(session, {
      role: 'assistant',
      sidechain: false,
      blocks: [{ kind: 'tool_use', toolName: 'Bash', toolId: 'bash-1', toolInput: '{"command":"pwd"}', isError: false }],
    })
    expect(session.toolActivity).toMatchObject({
      toolName: 'Bash',
      toolId: 'bash-1',
      phase: 'calling',
      summary: { kind: 'runCommand' },
    })

    chatOnMsgForTest(session, {
      role: 'user',
      sidechain: false,
      blocks: [{ kind: 'tool_result', toolId: 'bash-1', text: '/tmp', isError: false }],
    })
    expect(session.toolActivity).toMatchObject({
      toolName: 'Bash',
      toolId: 'bash-1',
      phase: 'result',
      summary: { kind: 'runCommand' },
    })

    chatOnMsgForTest(session, {
      role: 'assistant',
      sidechain: false,
      blocks: [{ kind: 'text', text: 'Done', isError: false }],
    })
    expect(session.toolActivity).toMatchObject({ toolName: 'Bash', phase: 'result' })

    chatOnMsgForTest(session, {
      role: 'user',
      sidechain: false,
      blocks: [{ kind: 'tool_result', toolId: 'bash-1', text: 'permission denied', isError: true }],
    })
    expect(session.toolActivity).toMatchObject({ toolName: 'Bash', toolId: 'bash-1', phase: 'failed' })
  })

  it('keeps parallel tool calls available after one result arrives', async () => {
    invokeMock.mockResolvedValueOnce({ chatId: 84, processModel: 'longLivedStdin' })
    const session = await startChat({ agent: 'codex', projectKey: 'p', cwd: '/tmp', title: 'C' })
    session.turnState = 'running'

    chatOnMsgForTest(session, {
      role: 'assistant',
      sidechain: false,
      blocks: [
        { kind: 'tool_use', toolName: 'shell', toolId: 'shell-1', toolInput: 'rg TODO src', isError: false },
        { kind: 'tool_use', toolName: 'shell', toolId: 'shell-2', toolInput: 'git status', isError: false },
      ],
    })
    expect(session.toolActivities).toHaveLength(2)
    expect(session.toolActivity?.toolId).toBe('shell-2')

    chatOnMsgForTest(session, {
      role: 'user',
      sidechain: false,
      blocks: [{ kind: 'tool_result', toolId: 'shell-1', text: 'src/a.ts', isError: false }],
    })
    expect(session.toolActivities).toHaveLength(1)
    expect(session.toolActivities[0].toolId).toBe('shell-2')
    expect(session.toolActivity).toMatchObject({ toolId: 'shell-1', phase: 'result' })
  })

  it('marks a completed turn so the UI can play its completion feedback', async () => {
    invokeMock.mockResolvedValueOnce({ chatId: 82, processModel: 'longLivedStdin' })
    const session = await startChat({ agent: 'claude', projectKey: 'p', cwd: '/tmp', title: 'C' })
    session.turnState = 'running'

    chatOnResultForTest(session, { chatId: 82, ok: true })

    expect(session.lastTurnOutcome).toBe('completed')
    expect(session.turnState).toBe('idle')
  })

  it('marks an unsuccessful result as failed rather than completed', async () => {
    invokeMock.mockResolvedValueOnce({ chatId: 83, processModel: 'longLivedStdin' })
    const session = await startChat({ agent: 'claude', projectKey: 'p', cwd: '/tmp', title: 'C' })
    session.turnState = 'running'

    chatOnResultForTest(session, { chatId: 83, ok: false })

    expect(session.lastTurnOutcome).toBe('failed')
  })
})

describe('chatSessions Codex custom provider compatibility', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('uses remembered GUI permission, model, and effort for a new Codex chat', async () => {
    rememberChatGuiPreference('codex', {
      permissionMode: 'approve',
      model: 'gpt-5.6-sol',
      effort: 'ultra',
    })
    invokeMock.mockImplementation((command: string) => {
      if (command === 'codex_runtime_info') return Promise.resolve({ usesApiKey: false })
      if (command === 'agent_chat_start') return Promise.resolve({ chatId: 5, processModel: 'codexAppServer' })
      return Promise.resolve(undefined)
    })

    const s = await startChat({
      agent: 'codex',
      projectKey: 'proj',
      cwd: '/tmp',
      title: 'Codex',
    })

    expect(s.permissionMode).toBe('approve')
    expect(s.model).toBe('gpt-5.6-sol')
    expect(s.effort).toBe('ultra')
    expect(invokeMock).toHaveBeenCalledWith(
      'agent_chat_start',
      expect.objectContaining({
        agent: 'codex',
        permissionMode: 'approve',
        model: 'gpt-5.6-sol',
        effort: 'ultra',
      }),
    )
  })

  it('uses config.toml Codex model and effort for custom providers', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'codex_runtime_info') return Promise.resolve({ usesApiKey: true, model: 'gpt-5.6-sol', effort: 'high' })
      if (command === 'agent_chat_start') return Promise.resolve({ chatId: 2, processModel: 'codexAppServer' })
      return Promise.resolve(undefined)
    })

    const s = await startChat({
      agent: 'codex',
      projectKey: 'proj',
      cwd: '/tmp',
      title: 'Codex',
    })

    expect(s.model).toBe('gpt-5.6-sol')
    expect(s.effort).toBe('high')
    expect(s.lastModel).toBe('gpt-5.6-sol')
    expect(invokeMock).toHaveBeenCalledWith('codex_runtime_info')
    expect(invokeMock).toHaveBeenCalledWith(
      'agent_chat_start',
      expect.objectContaining({
        agent: 'codex',
        model: 'gpt-5.6-sol',
        effort: 'high',
      }),
    )
  })

  it('preserves explicitly provided Codex custom-provider model and effort', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'codex_runtime_info') return Promise.resolve({ usesApiKey: true })
      if (command === 'agent_chat_start') return Promise.resolve({ chatId: 3, processModel: 'codexAppServer' })
      return Promise.resolve(undefined)
    })

    const s = await startChat({
      agent: 'codex',
      projectKey: 'proj',
      cwd: '/tmp',
      title: 'Codex',
      model: 'custom-model',
      effort: 'high',
    })

    expect(s.model).toBe('custom-model')
    expect(s.effort).toBe('high')
    expect(invokeMock).toHaveBeenCalledWith(
      'agent_chat_start',
      expect.objectContaining({
        agent: 'codex',
        model: 'custom-model',
        effort: 'high',
      }),
    )
  })

  it('does not apply global Codex config when resuming an existing chat', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'codex_runtime_info') return Promise.resolve({ usesApiKey: true, model: 'gpt-5.6-sol', effort: 'high' })
      if (command === 'agent_chat_start') return Promise.resolve({ chatId: 4, processModel: 'codexAppServer' })
      return Promise.resolve(undefined)
    })

    const s = await startChat({
      agent: 'codex',
      projectKey: 'proj',
      cwd: '/tmp',
      title: 'Existing',
      sessionId: 'thread-1',
      preloadMsgs: [
        {
          role: 'assistant',
          sidechain: false,
          model: 'gpt-5.4',
          blocks: [{ kind: 'text', text: 'older', isError: false }],
        },
      ],
    })

    expect(s.model).toBe('gpt-5.5')
    expect(s.effort).toBe('high')
    expect(s.lastModel).toBe('gpt-5.4')
    expect(invokeMock).not.toHaveBeenCalledWith('codex_runtime_info')
    expect(invokeMock).toHaveBeenCalledWith(
      'agent_chat_start',
      expect.objectContaining({
        agent: 'codex',
        sessionId: 'thread-1',
        model: 'gpt-5.5',
        effort: 'high',
      }),
    )
  })
})

describe('reconnectChats — restored live messages', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('normalizes missing timestamps and assistant model labels', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-07-09T12:58:00.000Z'))
    invokeMock.mockResolvedValueOnce([
      {
        chatId: 77,
        agent: 'codex',
        projectKey: 'proj',
        cwd: '/tmp',
        sessionId: 'thread-1',
        messages: [
          {
            role: 'assistant',
            sidechain: false,
            timestamp: null,
            blocks: [{ kind: 'text', text: 'hi', isError: false }],
          },
        ],
        turnState: 'idle',
        turnStartedAtMs: null,
        permissionMode: 'approve',
        model: 'gpt-5.4',
        effort: 'high',
        processModel: 'codexAppServer',
      },
    ])

    const [session] = await reconnectChats()

    expect(session.msgs[0].timestamp).toBe('2026-07-09T12:58:00.000Z')
    expect(session.msgs[0].model).toBe('gpt-5.4')
    expect(session.lastModel).toBe('gpt-5.4')
  })
})

describe('parseRetryLine — network-retry detection from CLI stderr', () => {
  it('extracts attempt/max from "(N/M)" form', () => {
    expect(parseRetryLine('Request failed · retrying (4/10) · 24s')).toEqual({ attempt: 4, max: 10 })
    expect(parseRetryLine('Reconnecting... 3/5 (7m 27s · esc to interrupt)')).toEqual({ attempt: 3, max: 5 })
  })

  it('extracts attempt/max from "N of M" form', () => {
    expect(parseRetryLine('API error, retrying 2 of 5...')).toEqual({ attempt: 2, max: 5 })
  })

  it('matches transient-error keywords without a count → empty object', () => {
    expect(parseRetryLine('Overloaded, backing off')).toEqual({})
    expect(parseRetryLine('fetch failed: ECONNRESET')).toEqual({})
    expect(parseRetryLine('socket hang up')).toEqual({})
    expect(parseRetryLine('Reconnecting...')).toEqual({})
    expect(parseRetryLine('Service Unavailable: Service temporarily unavailable')).toEqual({})
  })

  it('is case-insensitive', () => {
    expect(parseRetryLine('RETRYING request')).toEqual({})
  })

  it('returns null for unrelated stderr lines', () => {
    expect(parseRetryLine('[debug] loaded 3 of 4 plugins')).toBeNull()
    expect(parseRetryLine('Reading config from ~/.claude')).toBeNull()
    expect(parseRetryLine('')).toBeNull()
  })
})

describe('respondPermission — interactive tool-permission reply', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  const permReq = (over: Partial<ChatPermissionRequest> = {}): ChatPermissionRequest => ({
    requestId: 'req-1',
    toolName: 'Bash',
    input: { command: 'ls' },
    ...over,
  })

  it('writes the decision back to the matching chat and dequeues the request', async () => {
    invokeMock.mockResolvedValueOnce({ chatId: 42, processModel: 'longLivedStdin' })
    const s = await startChat({ agent: 'claude', projectKey: 'p', cwd: '/tmp', title: 'C' })
    const r = permReq()
    s.pendingPermissions = [r]
    invokeMock.mockReset()
    invokeMock.mockResolvedValueOnce(undefined)

    await respondPermission(s, r, 'allow-once')

    expect(s.pendingPermissions).toHaveLength(0)
    expect(invokeMock).toHaveBeenCalledWith('agent_chat_respond_permission', {
      id: 42,
      requestId: 'req-1',
      decision: { behavior: 'allow', updatedInput: { command: 'ls' } },
    })
  })

  it('dequeues only the answered request, leaving others pending', async () => {
    invokeMock.mockResolvedValueOnce({ chatId: 7, processModel: 'longLivedStdin' })
    const s = await startChat({ agent: 'claude', projectKey: 'p', cwd: '/tmp', title: 'C' })
    const a = permReq({ requestId: 'a' })
    const b = permReq({ requestId: 'b' })
    s.pendingPermissions = [a, b]
    invokeMock.mockReset()
    invokeMock.mockResolvedValueOnce(undefined)

    await respondPermission(s, a, 'deny')

    expect(s.pendingPermissions.map((p) => p.requestId)).toEqual(['b'])
  })

  it('exits Claude plan mode after allowing ExitPlanMode', async () => {
    invokeMock.mockResolvedValueOnce({ chatId: 8, processModel: 'longLivedStdin' })
    const s = await startChat({ agent: 'claude', projectKey: 'p', cwd: '/tmp', title: 'C', permissionMode: 'plan' })
    const r = permReq({
      toolName: 'ExitPlanMode',
      input: { plan: '# Plan\n\n- Create file' },
    })
    s.pendingPermissions = [r]
    invokeMock.mockReset()
    invokeMock.mockResolvedValueOnce(undefined)

    await respondPermission(s, r, 'allow-once')

    expect(s.permissionMode).toBe('bypassPermissions')
    expect(invokeMock).toHaveBeenCalledWith('agent_chat_respond_permission', {
      id: 8,
      requestId: 'req-1',
      decision: { behavior: 'allow', updatedInput: { plan: '# Plan\n\n- Create file' } },
    })
  })
})

describe('respondQuestion — structured AskUserQuestion reply', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  const qReq = (over: Partial<ChatQuestionRequest> = {}): ChatQuestionRequest => ({
    requestId: 'q-1',
    questions: [{ question: 'Pick one', options: [{ label: 'A' }, { label: 'B' }] }],
    ...over,
  })

  it('writes an allow decision with the answers map and dequeues the question', async () => {
    invokeMock.mockResolvedValueOnce({ chatId: 42, processModel: 'longLivedStdin' })
    const s = await startChat({ agent: 'claude', projectKey: 'p', cwd: '/tmp', title: 'C' })
    const r = qReq()
    s.pendingQuestions = [r]
    invokeMock.mockReset()
    invokeMock.mockResolvedValueOnce(undefined)

    await respondQuestion(s, r, [{ labels: ['B'] }])

    expect(s.pendingQuestions).toHaveLength(0)
    expect(invokeMock).toHaveBeenCalledWith('agent_chat_respond_question', {
      id: 42,
      requestId: 'q-1',
      decision: {
        behavior: 'allow',
        updatedInput: { questions: r.questions, answers: { 'Pick one': 'B' } },
      },
    })
  })

  it('writes a deny decision when the user cancels (null selections)', async () => {
    invokeMock.mockResolvedValueOnce({ chatId: 9, processModel: 'longLivedStdin' })
    const s = await startChat({ agent: 'claude', projectKey: 'p', cwd: '/tmp', title: 'C' })
    const r = qReq()
    s.pendingQuestions = [r]
    invokeMock.mockReset()
    invokeMock.mockResolvedValueOnce(undefined)

    await respondQuestion(s, r, null)

    expect(s.pendingQuestions).toHaveLength(0)
    expect(invokeMock).toHaveBeenCalledWith('agent_chat_respond_question', {
      id: 9,
      requestId: 'q-1',
      decision: { behavior: 'deny', message: 'The user declined to answer the question.', interrupt: false },
    })
  })

  it('dequeues only the answered question, leaving others pending', async () => {
    invokeMock.mockResolvedValueOnce({ chatId: 7, processModel: 'longLivedStdin' })
    const s = await startChat({ agent: 'claude', projectKey: 'p', cwd: '/tmp', title: 'C' })
    const a = qReq({ requestId: 'a' })
    const b = qReq({ requestId: 'b' })
    s.pendingQuestions = [a, b]
    invokeMock.mockReset()
    invokeMock.mockResolvedValueOnce(undefined)

    await respondQuestion(s, a, [{ labels: ['A'] }])

    expect(s.pendingQuestions.map((q) => q.requestId)).toEqual(['b'])
  })

  it('exits Codex plan mode after accepting plan implementation', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'codex_runtime_info') return Promise.resolve({ usesApiKey: false })
      if (command === 'agent_chat_start') return Promise.resolve({ chatId: 11, processModel: 'codexAppServer' })
      return Promise.resolve(undefined)
    })
    const s = await startChat({ agent: 'codex', projectKey: 'p', cwd: '/tmp', title: 'C', permissionMode: 'plan' })
    const r = qReq({
      questions: [{
        question: 'Implement this plan?',
        allowOther: false,
        options: [{ label: 'Yes, implement this plan' }, { label: 'No, stay in Plan mode' }],
      }],
    })
    s.pendingQuestions = [r]
    invokeMock.mockReset()
    invokeMock.mockResolvedValueOnce(undefined)

    await respondQuestion(s, r, [{ labels: ['Yes, implement this plan'] }])

    expect(s.permissionMode).toBe('fullAccess')
  })

  it('keeps Codex in plan mode when implementation is declined', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'codex_runtime_info') return Promise.resolve({ usesApiKey: false })
      if (command === 'agent_chat_start') return Promise.resolve({ chatId: 12, processModel: 'codexAppServer' })
      return Promise.resolve(undefined)
    })
    const s = await startChat({ agent: 'codex', projectKey: 'p', cwd: '/tmp', title: 'C', permissionMode: 'plan' })
    const r = qReq({
      questions: [{
        question: 'Implement this plan?',
        allowOther: false,
        options: [{ label: 'Yes, implement this plan' }, { label: 'No, stay in Plan mode' }],
      }],
    })
    s.pendingQuestions = [r]
    invokeMock.mockReset()
    invokeMock.mockResolvedValueOnce(undefined)

    await respondQuestion(s, r, [{ labels: ['No, stay in Plan mode'] }])

    expect(s.permissionMode).toBe('plan')
  })

  it('creates a local Codex plan confirmation when app-server only emits a plan item', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'codex_runtime_info') return Promise.resolve({ usesApiKey: false })
      if (command === 'agent_chat_start') return Promise.resolve({ chatId: 15, processModel: 'codexAppServer' })
      return Promise.resolve(undefined)
    })
    const s = await startChat({ agent: 'codex', projectKey: 'p', cwd: '/tmp', title: 'C', permissionMode: 'plan' })
    s.turnState = 'running'
    s.msgs = [
      {
        role: 'assistant',
        sidechain: false,
        blocks: [{ kind: 'text', text: '## Proposed Plan\n\n- [ ] Create plan-mode-test.txt', isError: false }],
      },
    ]

    chatOnResultForTest(s, { chatId: 15, ok: true })

    expect(s.pendingQuestions).toHaveLength(1)
    expect(s.pendingQuestions[0].localCodexPlanPrompt).toBe(true)
    expect(s.pendingQuestions[0].keepAfterTurn).toBe(true)
    expect(s.pendingQuestions[0].questions[0].question).toBe('Implement this plan?')
  })

  it('answers local Codex plan confirmation by switching modes and sending implementation prompt', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'codex_runtime_info') return Promise.resolve({ usesApiKey: false })
      if (command === 'agent_chat_start') return Promise.resolve({ chatId: 16, processModel: 'codexAppServer' })
      return Promise.resolve(undefined)
    })
    const s = await startChat({ agent: 'codex', projectKey: 'p', cwd: '/tmp', title: 'C', permissionMode: 'plan' })
    const r = qReq({
      requestId: 'local-codex-plan-1-0',
      keepAfterTurn: true,
      localCodexPlanPrompt: true,
      questions: [{
        question: 'Implement this plan?',
        allowOther: false,
        options: [{ label: 'Yes, implement this plan' }, { label: 'No, stay in Plan mode' }],
      }],
    })
    s.pendingQuestions = [r]
    invokeMock.mockReset()
    invokeMock.mockImplementation((command: string) => {
      if (command === 'agent_chat_stop') return Promise.resolve(undefined)
      if (command === 'agent_chat_start') return Promise.resolve({ chatId: 17, processModel: 'codexAppServer' })
      if (command === 'agent_chat_send') return Promise.resolve(undefined)
      return Promise.resolve(undefined)
    })

    await respondQuestion(s, r, [{ labels: ['Yes, implement this plan'] }])

    expect(s.permissionMode).toBe('fullAccess')
    expect(invokeMock).not.toHaveBeenCalledWith('agent_chat_respond_question', expect.anything())
    expect(invokeMock).toHaveBeenCalledWith('agent_chat_send', expect.objectContaining({
      id: 17,
      text: 'Implement the plan.',
      permissionMode: 'fullAccess',
    }))
  })

  it('preserves Codex app-server questions across turn result', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'codex_runtime_info') return Promise.resolve({ usesApiKey: false })
      if (command === 'agent_chat_start') return Promise.resolve({ chatId: 13, processModel: 'codexAppServer' })
      return Promise.resolve(undefined)
    })
    const s = await startChat({ agent: 'codex', projectKey: 'p', cwd: '/tmp', title: 'C' })
    const r = qReq({ requestId: 'keep', keepAfterTurn: true })
    s.turnState = 'running'
    s.pendingQuestions = [r]

    chatOnResultForTest(s, { chatId: 13, ok: true })

    expect(s.pendingQuestions.map((q) => q.requestId)).toEqual(['keep'])
    expect(s.turnState).toBe('idle')
  })

  it('clears normal questions across turn result', async () => {
    invokeMock.mockResolvedValueOnce({ chatId: 14, processModel: 'longLivedStdin' })
    const s = await startChat({ agent: 'claude', projectKey: 'p', cwd: '/tmp', title: 'C' })
    s.turnState = 'running'
    s.pendingQuestions = [qReq({ requestId: 'drop' })]

    chatOnResultForTest(s, { chatId: 14, ok: true })

    expect(s.pendingQuestions).toHaveLength(0)
    expect(s.turnState).toBe('idle')
  })
})

describe('chatSessions message queue (type-while-running)', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  // 起一个空闲的 Claude 长驻会话；之后的 invoke（send / stop）一律 resolve。
  async function startClaude() {
    invokeMock.mockResolvedValueOnce({ chatId: 1, processModel: 'longLivedStdin' })
    const s = await startChat({ agent: 'claude', projectKey: 'p', cwd: '/tmp', title: 'C' })
    invokeMock.mockResolvedValue(undefined)
    return s
  }

  async function startCodex() {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'codex_runtime_info') return Promise.resolve({ usesApiKey: false })
      if (command === 'agent_chat_start') return Promise.resolve({ chatId: 2, processModel: 'codexAppServer' })
      return Promise.resolve(undefined)
    })
    const s = await startChat({ agent: 'codex', projectKey: 'p', cwd: '/tmp', title: 'C' })
    invokeMock.mockReset()
    invokeMock.mockResolvedValue(undefined)
    return s
  }

  it('sends immediately when idle and the queue is empty', async () => {
    const s = await startClaude()
    enqueuePrompt(s, 'hello')
    await Promise.resolve()
    expect(s.queue).toHaveLength(0)
    expect(s.turnState).toBe('running')
    expect(invokeMock).toHaveBeenCalledWith(
      'agent_chat_send',
      expect.objectContaining({ id: 1, text: 'hello' }),
    )
  })

  it('sends Codex app-server plugin mentions as structured text elements', async () => {
    const s = await startCodex()
    enqueuePrompt(s, '@Computer 打开todesk')
    await Promise.resolve()

    expect(invokeMock).toHaveBeenCalledWith(
      'agent_chat_send',
      expect.objectContaining({
        id: 2,
        text: '@Computer 打开todesk',
        textElements: [
          expect.objectContaining({
            type: 'mention',
            byteRange: { start: 0, end: 9 },
            path: 'plugin://computer-use@openai-bundled',
            name: 'Computer',
            placeholder: '@Computer',
          }),
        ],
      }),
    )
    const msg = s.msgs[s.msgs.length - 1]
    const block = msg?.blocks[msg.blocks.length - 1]
    expect(block).toMatchObject({
      kind: 'text',
      text: '@Computer 打开todesk',
    })
  })

  it('restarts Codex app-server before sending when permission mode changes', async () => {
    const s = await startCodex()
    s.sessionId = 'thread-1'
    s.permissionMode = 'plan'
    invokeMock.mockImplementation((command: string) => {
      if (command === 'agent_chat_stop') return Promise.resolve(undefined)
      if (command === 'agent_chat_start') return Promise.resolve({ chatId: 3, processModel: 'codexAppServer' })
      if (command === 'agent_chat_send') return Promise.resolve(undefined)
      return Promise.resolve(undefined)
    })

    enqueuePrompt(s, '创建一个文件 plan-mode-test.txt，内容写 hello')
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()

    expect(invokeMock).toHaveBeenCalledWith('agent_chat_stop', { id: 2 })
    expect(invokeMock).toHaveBeenCalledWith(
      'agent_chat_start',
      expect.objectContaining({
        agent: 'codex',
        sessionId: 'thread-1',
        permissionMode: 'plan',
      }),
    )
    expect(invokeMock).toHaveBeenCalledWith(
      'agent_chat_send',
      expect.objectContaining({
        id: 3,
        permissionMode: 'plan',
        text: '创建一个文件 plan-mode-test.txt，内容写 hello',
      }),
    )
    expect(s.chatId).toBe(3)
    expect(s.applied?.permissionMode).toBe('plan')
  })

  it('expands Codex one-shot plugin mentions before sending while keeping the local bubble short', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'codex_runtime_info') return Promise.resolve({ usesApiKey: false })
      if (command === 'agent_chat_start') return Promise.resolve({ chatId: 3, processModel: 'oneShotResume' })
      return Promise.resolve(undefined)
    })
    const s = await startChat({ agent: 'codex', projectKey: 'p', cwd: '/tmp', title: 'C' })
    invokeMock.mockReset()
    invokeMock.mockResolvedValue(undefined)

    enqueuePrompt(s, '@Chrome 打开bing.com')
    await Promise.resolve()

    expect(invokeMock).toHaveBeenCalledWith(
      'agent_chat_send',
      expect.objectContaining({
        id: 3,
        text: '[@Chrome](plugin://chrome@openai-bundled) 打开bing.com',
        textElements: [],
      }),
    )
    const msg = s.msgs[s.msgs.length - 1]
    const block = msg?.blocks[msg.blocks.length - 1]
    expect(block).toMatchObject({
      kind: 'text',
      text: '@Chrome 打开bing.com',
    })
  })

  it('queues instead of sending while a turn is running, preserving FIFO order and attachments', async () => {
    const s = await startClaude()
    s.turnState = 'running' // 模拟一轮进行中
    enqueuePrompt(s, 'first')
    enqueuePrompt(s, 'second', [{ dataUrl: 'd', mediaType: 'image/png', data: 'x' }] as never)
    await Promise.resolve()
    expect(s.queue.map((q) => q.text)).toEqual(['first', 'second'])
    expect(s.queue[1].images).toHaveLength(1)
    expect(invokeMock).not.toHaveBeenCalledWith('agent_chat_send', expect.anything())
  })

  it('removeQueued drops a pending message by id', async () => {
    const s = await startClaude()
    s.turnState = 'running'
    enqueuePrompt(s, 'a')
    enqueuePrompt(s, 'b')
    removeQueued(s, s.queue[0].id)
    expect(s.queue.map((q) => q.text)).toEqual(['b'])
  })

  it('does not guide queued messages for non-Codex sessions', async () => {
    const s = await startClaude()
    s.turnState = 'running'
    enqueuePrompt(s, 'wait for this')

    expect(canSteerQueued(s)).toBe(false)
    await steerQueued(s, s.queue[0].id)

    expect(s.queue.map((q) => q.text)).toEqual(['wait for this'])
    expect(s.submittedQueue).toEqual([])
    expect(invokeMock).not.toHaveBeenCalledWith('agent_chat_steer', expect.anything())
  })

  it('guides exactly one Codex app-server queue item into the active turn', async () => {
    const s = await startCodex()
    s.turnState = 'running'
    enqueuePrompt(s, 'first')
    enqueuePrompt(s, 'second')

    expect(canSteerQueued(s)).toBe(true)
    await steerQueued(s, s.queue[1].id)

    expect(invokeMock).toHaveBeenCalledWith(
      'agent_chat_steer',
      expect.objectContaining({
        id: 2,
        text: expect.stringContaining('second'),
        textElements: [],
      }),
    )
    expect(invokeMock).toHaveBeenCalledWith(
      'agent_chat_steer',
      expect.objectContaining({ text: expect.stringContaining('fully complete the original task') }),
    )
    expect(s.queue.map((q) => q.text)).toEqual(['first'])
    expect(s.submittedQueue.map((q) => q.text)).toEqual(['second'])
    expect(s.turnState).toBe('running')
    expect(s.msgs[s.msgs.length - 1]?.blocks).toMatchObject([
      { kind: 'text', text: 'second' },
    ])
  })

  it('allows several queued messages to be guided while earlier steer requests await confirmation', async () => {
    const s = await startCodex()
    s.turnState = 'running'
    enqueuePrompt(s, 'first guide')
    enqueuePrompt(s, 'second guide')
    const firstId = s.queue[0].id
    const secondId = s.queue[1].id
    const resolveSteers: Array<() => void> = []
    invokeMock.mockImplementation((command: string) => {
      if (command === 'agent_chat_steer') {
        return new Promise<void>((resolve) => resolveSteers.push(resolve))
      }
      return Promise.resolve(undefined)
    })

    const first = steerQueued(s, firstId)
    await Promise.resolve()
    await Promise.resolve()
    expect(canSteerQueued(s, secondId)).toBe(true)
    const second = steerQueued(s, secondId)
    await Promise.resolve()
    await Promise.resolve()

    expect(invokeMock).toHaveBeenCalledTimes(2)
    resolveSteers.forEach((resolve) => resolve())
    await Promise.all([first, second])

    expect(s.queue).toEqual([])
    expect(s.submittedQueue.map((q) => q.text)).toEqual(['first guide', 'second guide'])
  })

  it('keeps a Codex queue item pending when app-server steering fails', async () => {
    const s = await startCodex()
    s.turnState = 'running'
    enqueuePrompt(s, 'keep this queued')
    invokeMock.mockRejectedValueOnce(new Error('active turn is gone'))

    await steerQueued(s, s.queue[0].id)

    expect(s.queue.map((q) => q.text)).toEqual(['keep this queued'])
    expect(s.submittedQueue).toEqual([])
    expect(s.msgs).toEqual([])
  })

  it('clears submitted queue items when the active turn completes, then drains FIFO normally', async () => {
    const s = await startCodex()
    s.turnState = 'running'
    enqueuePrompt(s, 'steer this')
    enqueuePrompt(s, 'send next')
    await steerQueued(s, s.queue[0].id)
    invokeMock.mockClear()

    chatOnResultForTest(s, { chatId: 2, ok: true })
    await Promise.resolve()

    expect(s.submittedQueue).toEqual([])
    expect(s.queue).toEqual([])
    expect(invokeMock).toHaveBeenCalledWith(
      'agent_chat_send',
      expect.objectContaining({ id: 2, text: 'send next' }),
    )
  })

  it('ignores empty messages (no text / images / files)', async () => {
    const s = await startClaude()
    s.turnState = 'running'
    enqueuePrompt(s, '   ')
    expect(s.queue).toHaveLength(0)
  })

  it('does not queue or send once the session has ended', async () => {
    const s = await startClaude()
    s.status = 'exited'
    enqueuePrompt(s, 'hello')
    expect(s.queue).toHaveLength(0)
    expect(invokeMock).not.toHaveBeenCalledWith('agent_chat_send', expect.anything())
  })

  it('preserves the queue when the current turn is interrupted and drains next', async () => {
    const s = await startClaude()
    s.turnState = 'running'
    enqueuePrompt(s, 'pending-1')
    enqueuePrompt(s, 'pending-2')
    expect(s.queue).toHaveLength(2)
    // 中断（长驻 = stop + restart）：先 stop 旧进程，再 start 新进程。
    invokeMock.mockReset()
    invokeMock.mockResolvedValueOnce(undefined) // stop
    invokeMock.mockResolvedValueOnce({ chatId: 2, processModel: 'longLivedStdin' }) // start
    invokeMock.mockResolvedValue(undefined) // drain sends
    await interruptChat(s)
    // pending-1 被 drain 发出，pending-2 还在队列
    expect(s.queue).toHaveLength(1)
    expect(s.queue[0].text).toBe('pending-2')
  })
})
