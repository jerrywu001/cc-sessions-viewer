import type { PiTodoSummary } from './types'
export type { PiTodoSummary } from './types'

export type PiTodoStatus = 'pending' | 'in_progress' | 'completed'

export interface PiTodoView {
  action: string
  subject: string
  status: PiTodoStatus
  statusLabel: string
  symbol: string
}

export interface PiReadArgs {
  path: string
}

type RecordValue = Record<string, unknown>

function asRecord(value: unknown): RecordValue | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as RecordValue
    : null
}

function parseInput(input: string): RecordValue | null {
  try {
    return asRecord(JSON.parse(input))
  } catch {
    return null
  }
}

function stringValue(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined
}

function normalizeStatus(value: unknown): PiTodoStatus | undefined {
  const raw = stringValue(value)?.toLowerCase().replace(/[- ]+/g, '_')
  if (!raw) return undefined
  if (raw === 'pending' || raw === 'in_progress' || raw === 'completed') {
    return raw
  }
  return undefined
}

function statusFromResult(text: string): PiTodoStatus | undefined {
  const match = /(?:→|\()\s*(pending|in_progress|completed)\)?\s*$/i.exec(text.trim())
  return normalizeStatus(match?.[1])
}

function subjectFromResult(text: string): string | undefined {
  const match = /Created\s+#\d+:\s*(.*?)\s*\(pending\)\s*$/i.exec(text.trim())
  return match?.[1]?.trim() || undefined
}

export function parsePiTodo(input: string, resultText = ''): PiTodoView | null {
  const args = parseInput(input)
  if (!args) return null
  const action = stringValue(args.action) ?? 'update'
  const subject = stringValue(args.subject) ?? subjectFromResult(resultText) ?? (args.id != null ? `#${String(args.id)}` : '')
  if (!subject && action !== 'clear') return null
  const status = normalizeStatus(args.status)
    ?? statusFromResult(resultText)
    ?? (action === 'create' ? 'pending' : 'in_progress')
  const statusLabel = status.replace('_', ' ')
  const symbol = status === 'pending' ? '○' : status === 'in_progress' ? '◐' : '●'
  return {
    action,
    subject: subject || 'tasks',
    status,
    statusLabel,
    symbol,
  }
}

export function parsePiTodoSummary(value: unknown): PiTodoSummary | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const raw = value as Record<string, unknown>
  const tasks = Array.isArray(raw.tasks)
    ? raw.tasks.flatMap((task) => {
        if (!task || typeof task !== 'object' || Array.isArray(task)) return []
        const item = task as Record<string, unknown>
        const subject = stringValue(item.subject)
        const status = stringValue(item.status)
        return subject && status ? [{ subject, status }] : []
      })
    : []
  if (!tasks.length) return null
  const total = Number.isFinite(Number(raw.total)) ? Number(raw.total) : tasks.length
  const completed = Number.isFinite(Number(raw.completed))
    ? Number(raw.completed)
    : tasks.filter((task) => task.status.toLowerCase() === 'completed').length
  return {
    completed: Math.max(0, Math.min(completed, tasks.length)),
    total: Math.max(tasks.length, total),
    tasks,
  }
}

export function piTodoSummaryMarkdown(summary: PiTodoSummary): string {
  const items = summary.tasks
    .map((task) => {
      const checked = task.status.toLowerCase() === 'completed' ? 'x' : ' '
      const status = task.status.toLowerCase() === 'in_progress' ? ' _(in progress)_' : ''
      return `- [${checked}] ${task.subject}${status}`
    })
    .join('\n')
  return `**○ Todos (${summary.completed}/${summary.total})**\n\n${items}`
}

export function parsePiReadArgs(input: string): PiReadArgs | null {
  const args = parseInput(input)
  const path = stringValue(args?.path)
  if (!path) return null
  return { path }
}

export function isPiSkillReadPath(path: string): boolean {
  return /(?:^|[\\/])SKILL\.md$/i.test(path)
}
