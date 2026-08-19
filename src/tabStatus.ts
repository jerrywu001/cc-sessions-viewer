import type { Agent } from './types'

export type TerminalProcessState = 'spawning' | 'alive' | 'exited' | 'error'
export type TerminalTurnState = 'idle' | 'working' | 'blocked' | 'review' | 'error' | 'unknown'
export type TerminalTurnSignalSource =
  | 'session-live-tail'
  | 'pty-input'
  | 'pty-exit'
  | 'hook'

export type TerminalTurnEventState = 'started' | 'completed' | 'blocked' | 'failed'
export type TabStatusKind =
  | 'working'
  | 'done'
  | 'blocked'
  | 'error'
  | 'exited'
  | 'unknown'
  | 'none'

type StatusTab = {
  processState: TerminalProcessState
  status: 'spawning' | 'running' | 'exited' | 'error'
  turnState: TerminalTurnState
  turnStateSource: TerminalTurnSignalSource | null
  turnStateUpdatedAt: number
  turnSignalId?: string | null
  agent: Agent
  sessionPath: string
}

const pendingTurnStates = new Map<
  string,
  {
    state: TerminalTurnEventState
    source: TerminalTurnSignalSource
    signalId?: string
    updatedAt: number
  }
>()

export function normalizeSessionPath(path: string): string {
  if (path.startsWith('\\\\?\\UNC\\')) return `\\\\${path.slice(8)}`
  if (path.startsWith('\\\\?\\')) return path.slice(4)
  return path
}

export function sessionPathsEqual(left: string, right: string): boolean {
  return normalizeSessionPath(left) === normalizeSessionPath(right)
}

function turnStateKey(agent: Agent, sessionPath: string) {
  return `${agent}\0${normalizeSessionPath(sessionPath)}`
}

function completedState(isActive: boolean): TerminalTurnState {
  return isActive ? 'idle' : 'review'
}

export function isSlashCommandInput(line: string): boolean {
  return line.trimStart().startsWith('/')
}

export function shouldTerminalInputStartTurn(agent: Agent, line: string): boolean {
  void agent
  if (isSlashCommandInput(line)) return false
  return line.trim().length > 0
}

export interface TerminalInputState {
  text: string
  cursor: number
  reliable: boolean
}

export function createTerminalInputState(text = ''): TerminalInputState {
  return {
    text,
    cursor: Array.from(text).length,
    reliable: true,
  }
}

const LEFT = new Set(['\x1b[D', '\x1bOD'])
const RIGHT = new Set(['\x1b[C', '\x1bOC'])
const HOME = new Set(['\x1b[H', '\x1bOH'])
const END = new Set(['\x1b[F', '\x1bOF'])
const DELETE = '\x1b[3~'
const PASTE_MARKERS = new Set(['\x1b[200~', '\x1b[201~'])
const FOCUS_MARKERS = new Set(['\x1b[I', '\x1b[O'])
const HISTORY = new Set(['\x1b[A', '\x1b[B', '\x1bOA', '\x1bOB'])

function terminalControlSequenceEnd(data: string, start: number): number {
  const first = data[start]
  if (first === '\x9b') {
    let end = start + 1
    while (end < data.length && !/[\x40-\x7e]/.test(data[end])) end += 1
    return Math.min(end + 1, data.length)
  }

  const next = data[start + 1]
  if (next === '[') {
    let end = start + 2
    while (end < data.length && !/[\x40-\x7e]/.test(data[end])) end += 1
    return Math.min(end + 1, data.length)
  }
  if (next === 'O') return Math.min(start + 3, data.length)
  if (next === ']') {
    let end = start + 2
    while (end < data.length) {
      if (data[end] === '\x07') return end + 1
      if (data[end] === '\x1b' && data[end + 1] === '\\') return end + 2
      end += 1
    }
    return data.length
  }
  if (next && /[PX^_]/.test(next)) {
    let end = start + 2
    while (end < data.length) {
      if (data[end] === '\x1b' && data[end + 1] === '\\') return end + 2
      end += 1
    }
    return data.length
  }
  return Math.min(start + (next ? 2 : 1), data.length)
}

export function applyTerminalInputState(
  current: TerminalInputState,
  data: string,
): { nextState: TerminalInputState; submittedLines: string[] } {
  const chars = Array.from(current.text)
  let cursor = Math.min(current.cursor, chars.length)
  let reliable = current.reliable
  const submittedLines: string[] = []

  const insert = (value: string) => {
    const added = Array.from(value)
    chars.splice(cursor, 0, ...added)
    cursor += added.length
  }
  const backspace = () => {
    if (cursor <= 0) return
    chars.splice(cursor - 1, 1)
    cursor -= 1
  }
  const deleteForward = () => {
    if (cursor < chars.length) chars.splice(cursor, 1)
  }
  const reset = () => {
    chars.splice(0)
    cursor = 0
    reliable = true
  }

  for (let index = 0; index < data.length; ) {
    const currentChar = data[index]
    if (currentChar === '\x1b' || currentChar === '\x9b') {
      const end = terminalControlSequenceEnd(data, index)
      const sequence = data.slice(index, end)
      if (LEFT.has(sequence)) cursor = Math.max(0, cursor - 1)
      else if (RIGHT.has(sequence)) cursor = Math.min(chars.length, cursor + 1)
      else if (HOME.has(sequence)) cursor = 0
      else if (END.has(sequence)) cursor = chars.length
      else if (sequence === DELETE) deleteForward()
      else if (HISTORY.has(sequence)) reliable = false
      else if (!PASTE_MARKERS.has(sequence) && !FOCUS_MARKERS.has(sequence)) {
        reliable = false
      }
      index = end
      continue
    }

    const codePoint = data.codePointAt(index)
    if (codePoint === undefined) break
    const value = String.fromCodePoint(codePoint)
    index += value.length

    if (value === '\r') {
      submittedLines.push(chars.join(''))
      reset()
    } else if (value === '\n') {
      reliable = false
    } else if (value === '\b' || value === '\x7f') {
      backspace()
    } else if (value === '\x15') {
      reset()
    } else if (value === '\x01') {
      cursor = 0
    } else if (value === '\x05') {
      cursor = chars.length
    } else if (value >= ' ') {
      insert(value)
    } else {
      reliable = false
    }
  }

  return {
    nextState: { text: chars.join(''), cursor, reliable },
    submittedLines,
  }
}

export function applyTerminalInputLineState(
  current: string,
  data: string,
): { nextLine: string; submittedLines: string[] } {
  const result = applyTerminalInputState(createTerminalInputState(current), data)
  return {
    nextLine: result.nextState.text,
    submittedLines: result.submittedLines,
  }
}

export function statusKind(tab: StatusTab): TabStatusKind {
  if (tab.turnState === 'error' || tab.processState === 'error') return 'error'
  if (tab.processState === 'exited') return 'exited'
  if (tab.turnState === 'blocked') return 'blocked'
  if (tab.processState === 'spawning' || tab.turnState === 'working') return 'working'
  if (tab.turnState === 'review') return 'done'
  if (tab.turnState === 'idle') return 'none'
  return 'unknown'
}

export function setProcessState(tab: StatusTab, state: TerminalProcessState) {
  tab.processState = state
  tab.status = state === 'alive' ? 'running' : state
}

export function setTurnState(
  tab: StatusTab,
  state: TerminalTurnState,
  source: TerminalTurnSignalSource,
  updatedAt = Date.now(),
) {
  tab.turnState = state
  tab.turnStateSource = source
  tab.turnStateUpdatedAt = updatedAt
}

export function applyTurnSignal(
  tab: StatusTab,
  state: TerminalTurnEventState,
  source: TerminalTurnSignalSource,
  isActive: boolean,
  signalId?: string,
) {
  // Grok turn-end hooks are deliberately asynchronous: a cancelled previous
  // turn can report after the next prompt has already started. Associate hook
  // signals with the opaque promptId so that stale completion never clears a
  // newer working tab. Signals without an id (such as idle backstops) settle
  // unconditionally as prescribed by Grok's hook contract.
  if (source === 'hook' && signalId) {
    if (state === 'started') tab.turnSignalId = signalId
    else if (tab.turnSignalId && tab.turnSignalId !== signalId) return
    else tab.turnSignalId = signalId
  }
  if (state === 'started') {
    setTurnState(tab, 'working', source)
    return
  }
  if (state === 'completed') {
    setTurnState(tab, completedState(isActive), source)
    return
  }
  if (state === 'blocked') {
    setTurnState(tab, 'blocked', source)
    return
  }
  setTurnState(tab, 'error', source)
}

export function markSessionActivity(tab: StatusTab) {
  void tab
}

export function clearLocalWorkingTurn(tab: StatusTab, isActive: boolean) {
  if (tab.turnState !== 'working') return
  setTurnState(tab, completedState(isActive), 'pty-input')
}

export function rememberPendingTurnState(
  agent: Agent,
  sessionPath: string,
  state: TerminalTurnEventState,
  source: TerminalTurnSignalSource,
  signalId?: string,
) {
  if (!sessionPath) return
  pendingTurnStates.set(turnStateKey(agent, sessionPath), {
    state,
    source,
    signalId,
    updatedAt: Date.now(),
  })
  if (pendingTurnStates.size > 200) {
    const first = pendingTurnStates.keys().next().value
    if (first) pendingTurnStates.delete(first)
  }
}

export function applyPendingTurnState(tab: StatusTab, isActive: boolean) {
  if (!tab.sessionPath) return
  const key = turnStateKey(tab.agent, tab.sessionPath)
  const pending = pendingTurnStates.get(key)
  if (!pending) return
  applyTurnSignal(tab, pending.state, pending.source, isActive, pending.signalId)
  tab.turnStateUpdatedAt = pending.updatedAt
  pendingTurnStates.delete(key)
}
