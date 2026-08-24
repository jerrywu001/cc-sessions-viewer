import { describe, expect, it } from 'vitest'
import {
  isPiSkillReadPath,
  parsePiReadArgs,
  parsePiTodo,
  parsePiTodoSummary,
  piTodoSummaryMarkdown,
} from '../src/piToolRendering'

describe('Pi tool rendering', () => {
  it('turns todo create and update results into status views', () => {
    const input = JSON.stringify({ action: 'create', subject: '熟悉结算需求文档' })
    expect(parsePiTodo(input, 'Created #1: 熟悉结算需求文档 (pending)')).toMatchObject({
      subject: '熟悉结算需求文档',
      status: 'pending',
      statusLabel: 'pending',
      symbol: '○',
    })
    expect(parsePiTodo(JSON.stringify({ action: 'update', id: 1 }), 'Updated #1 (pending → in_progress)')).toMatchObject({
      subject: '#1',
      status: 'in_progress',
      statusLabel: 'in progress',
      symbol: '◐',
    })
  })

  it('recognizes a skill read path', () => {
    expect(isPiSkillReadPath('/project/.agents/skills/demo/SKILL.md')).toBe(true)
    expect(isPiSkillReadPath('/project/README.md')).toBe(false)
    expect(parsePiReadArgs(JSON.stringify({ path: '/project/.agents/skills/demo/SKILL.md', offset: 1, limit: 2000 }))).toEqual({
      path: '/project/.agents/skills/demo/SKILL.md',
    })
  })

  it('renders Pi todo result details as a Markdown task list', () => {
    const summary = parsePiTodoSummary({
      completed: 2,
      total: 3,
      tasks: [
        { subject: '第一项', status: 'completed' },
        { subject: '第二项', status: 'in_progress' },
        { subject: '第三项', status: 'pending' },
      ],
    })
    expect(summary).not.toBeNull()
    expect(piTodoSummaryMarkdown(summary!)).toBe(
      '**○ Todos (2/3)**\n\n- [x] 第一项\n- [ ] 第二项 _(in progress)_\n- [ ] 第三项',
    )
  })

})
