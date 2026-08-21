import type { Agent } from './types'

export interface AgentCapabilities {
  history: boolean
  terminal: boolean
  guiChat: boolean
  worktree: boolean
  hooks: boolean
  stats: boolean
  pricing: boolean
}

export interface AgentMeta {
  label: string
  shortLabel: string
  cliName: string
  capabilities: AgentCapabilities
}

/**
 * Product-level agent metadata. UI entry points should check capabilities
 * instead of growing another `agent === ...` allowlist. Grok intentionally has
 * history/terminal support but no GUI chat in the current integration.
 */
export const AGENT_META: Record<Agent, AgentMeta> = {
  claude: {
    label: 'Claude Code',
    shortLabel: 'Claude',
    cliName: 'claude',
    capabilities: {
      history: true,
      terminal: true,
      guiChat: true,
      worktree: true,
      hooks: true,
      stats: true,
      pricing: true,
    },
  },
  codex: {
    label: 'Codex',
    shortLabel: 'Codex',
    cliName: 'codex',
    capabilities: {
      history: true,
      terminal: true,
      guiChat: true,
      worktree: true,
      hooks: true,
      stats: true,
      pricing: true,
    },
  },
  agy: {
    label: 'Antigravity CLI',
    shortLabel: 'agy',
    cliName: 'agy',
    capabilities: {
      history: true,
      terminal: true,
      guiChat: false,
      worktree: false,
      hooks: true,
      stats: false,
      pricing: false,
    },
  },
  opencode: {
    label: 'opencode',
    shortLabel: 'opencode',
    cliName: 'opencode',
    capabilities: {
      history: true,
      terminal: true,
      guiChat: false,
      worktree: false,
      hooks: false,
      stats: true,
      pricing: true,
    },
  },
  grok: {
    label: 'Grok Build',
    shortLabel: 'Grok Build',
    cliName: 'grok',
    capabilities: {
      history: true,
      terminal: true,
      guiChat: false,
      worktree: true,
      hooks: true,
      stats: true,
      pricing: true,
    },
  },
  kimicode: {
    label: 'Kimi Code',
    shortLabel: 'Kimi Code',
    cliName: 'kimi',
    capabilities: {
      history: true,
      terminal: true,
      guiChat: false,
      worktree: true,
      hooks: true,
      stats: true,
      pricing: true,
    },
  },
}

export function agentLabel(agent: Agent, long = false): string {
  const meta = AGENT_META[agent]
  return long ? meta.label : meta.shortLabel
}

export function agentSupports(
  agent: Agent,
  capability: keyof AgentCapabilities,
): boolean {
  return AGENT_META[agent].capabilities[capability]
}
