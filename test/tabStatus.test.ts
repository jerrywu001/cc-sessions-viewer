import { describe, expect, it } from 'vitest'
import {
  applyTurnSignal,
  applyTerminalInputLineState,
  applyTerminalInputState,
  createTerminalInputState,
  isSlashCommandInput,
  normalizeSessionPath,
  sessionPathsEqual,
  shouldTerminalInputStartTurn,
} from '../src/tabStatus'

describe('terminal input status inference', () => {
  it('matches Windows verbatim hook paths to normal tab paths', () => {
    const normal = 'C:\\Users\\me\\.codex\\sessions\\rollout.jsonl'
    const verbatim = '\\\\?\\C:\\Users\\me\\.codex\\sessions\\rollout.jsonl'

    expect(normalizeSessionPath(verbatim)).toBe(normal)
    expect(sessionPathsEqual(verbatim, normal)).toBe(true)
    expect(normalizeSessionPath('\\\\?\\UNC\\server\\share\\rollout.jsonl')).toBe(
      '\\\\server\\share\\rollout.jsonl',
    )
  })

  it('does not mark known slash commands as a user turn', () => {
    for (const input of [
      '/copy',
      '/status',
      '/diff',
      '/model gpt-5',
      '/permissions',
      '/plan',
      '/goal pause',
      '/side quick question',
      '/btw quick question',
      '  /theme',
    ]) {
      expect(isSlashCommandInput(input)).toBe(true)
      expect(shouldTerminalInputStartTurn('codex', input)).toBe(false)
      expect(shouldTerminalInputStartTurn('claude', input)).toBe(false)
    }
  })

  it('does not optimistically start turns for any slash input', () => {
    expect(isSlashCommandInput('/unknown maybe a future command')).toBe(true)
    expect(shouldTerminalInputStartTurn('codex', '/unknown maybe a future command')).toBe(false)
  })

  it('keeps normal prompts eligible to start a turn', () => {
    expect(shouldTerminalInputStartTurn('codex', 'fix this bug')).toBe(true)
    expect(shouldTerminalInputStartTurn('claude', 'fix this bug')).toBe(true)
  })

  it('ignores empty terminal input', () => {
    expect(shouldTerminalInputStartTurn('codex', '')).toBe(false)
    expect(shouldTerminalInputStartTurn('codex', '   ')).toBe(false)
  })

  it('extracts submitted terminal lines from chunked and pasted input', () => {
    expect(applyTerminalInputLineState('/cop', 'y\r')).toEqual({
      nextLine: '',
      submittedLines: ['/copy'],
    })
    expect(applyTerminalInputLineState('', 'fix bug\r')).toEqual({
      nextLine: '',
      submittedLines: ['fix bug'],
    })
  })

  it('tracks basic terminal line editing before submit', () => {
    expect(applyTerminalInputLineState('/stats', '\b\b\batus\r')).toEqual({
      nextLine: '',
      submittedLines: ['/status'],
    })
    expect(applyTerminalInputLineState('/copy', '\x15/status\r')).toEqual({
      nextLine: '',
      submittedLines: ['/status'],
    })
  })

  it('ignores terminal control sequences before an empty submit', () => {
    expect(applyTerminalInputLineState('', '\x1b[I\r')).toEqual({
      nextLine: '',
      submittedLines: [''],
    })
    expect(applyTerminalInputLineState('', '\x1b[200~\x1b[201~\r')).toEqual({
      nextLine: '',
      submittedLines: [''],
    })
    expect(applyTerminalInputLineState('', '\x1b[A\r')).toEqual({
      nextLine: '',
      submittedLines: [''],
    })
  })

  it('lets a new hook turn resume a blocked tab', () => {
    const tab = {
      processState: 'alive' as const,
      status: 'running' as const,
      turnState: 'blocked' as const,
      turnStateSource: 'hook' as const,
      turnStateUpdatedAt: 0,
      agent: 'codex' as const,
      sessionPath: '/tmp/session.jsonl',
    }

    applyTurnSignal(tab, 'started', 'hook', true)

    expect(tab.turnState).toBe('working')
    expect(tab.turnStateSource).toBe('hook')
  })

  it('lets a new hook turn recover from a failed prior turn', () => {
    const tab = {
      processState: 'alive' as const,
      status: 'running' as const,
      turnState: 'error' as 'error' | 'working',
      turnStateSource: 'hook' as const,
      turnStateUpdatedAt: 0,
      agent: 'claude' as const,
      sessionPath: '/tmp/session.jsonl',
    }

    applyTurnSignal(tab, 'started', 'hook', true)

    expect(tab.turnState).toBe('working')
  })

  it('ignores a delayed Grok completion from an older prompt', () => {
    const tab = {
      processState: 'alive' as const,
      status: 'running' as const,
      turnState: 'working' as const,
      turnStateSource: 'hook' as const,
      turnStateUpdatedAt: 0,
      turnSignalId: 'prompt-new',
      agent: 'grok' as const,
      sessionPath: '/tmp/session/updates.jsonl',
    }

    applyTurnSignal(tab, 'completed', 'hook', true, 'prompt-old')
    expect(tab.turnState).toBe('working')

    applyTurnSignal(tab, 'completed', 'hook', true, 'prompt-new')
    expect(tab.turnState).toBe('idle')
  })
})

describe('cursor-aware terminal input state', () => {
  it('inserts text at the logical cursor after left movement', () => {
    const current = createTerminalInputState('abcd')
    expect(applyTerminalInputState(current, '\x1b[D\x1b[DZ')).toEqual({
      nextState: { text: 'abZcd', cursor: 3, reliable: true },
      submittedLines: [],
    })
  })

  it('applies Backspace and Delete at the logical cursor', () => {
    const current = { text: '你好ab', cursor: 3, reliable: true }
    expect(applyTerminalInputState(current, '\x7f\x1b[3~')).toEqual({
      nextState: { text: '你好', cursor: 2, reliable: true },
      submittedLines: [],
    })
  })

  it('supports Home, End, and bracketed paste without losing reliability', () => {
    const current = createTerminalInputState('ab')
    expect(
      applyTerminalInputState(current, '\x1b[HZ\x1b[F\x1b[200~你\x1b[201~'),
    ).toEqual({
      nextState: { text: 'Zab你', cursor: 4, reliable: true },
      submittedLines: [],
    })
  })

  it('marks history navigation unreliable and resets after submit', () => {
    const unreliable = applyTerminalInputState(
      createTerminalInputState('draft'),
      '\x1b[A',
    ).nextState
    expect(unreliable).toEqual({
      text: 'draft',
      cursor: 5,
      reliable: false,
    })
    expect(applyTerminalInputState(unreliable, '\r').nextState).toEqual({
      text: '',
      cursor: 0,
      reliable: true,
    })
  })

  it('marks a Shift+Enter line break unreliable until the prompt is submitted', () => {
    const multiline = applyTerminalInputState(
      createTerminalInputState('draft'),
      '\n',
    )
    expect(multiline).toEqual({
      nextState: { text: 'draft', cursor: 5, reliable: false },
      submittedLines: [],
    })
    expect(applyTerminalInputState(multiline.nextState, 'more\r')).toEqual({
      nextState: { text: '', cursor: 0, reliable: true },
      submittedLines: ['draftmore'],
    })
  })
})
