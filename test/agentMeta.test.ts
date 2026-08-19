import { describe, expect, it } from 'vitest'
import { AGENT_META, agentLabel, agentSupports } from '../src/agentMeta'

describe('agent metadata', () => {
  it('defines Grok Build as history-capable but never GUI-chat-capable', () => {
    expect(agentLabel('grok')).toBe('Grok Build')
    expect(AGENT_META.grok.cliName).toBe('grok')
    expect(agentSupports('grok', 'history')).toBe(true)
    expect(agentSupports('grok', 'guiChat')).toBe(false)
    expect(agentSupports('grok', 'worktree')).toBe(true)
    expect(agentSupports('grok', 'stats')).toBe(true)
    expect(agentSupports('grok', 'pricing')).toBe(true)
  })
})
