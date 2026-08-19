import { beforeEach, describe, expect, it, vi } from 'vitest'

// api.ts is a thin typed wrapper over Tauri's `invoke`; we assert each helper
// maps to the right command name and argument shape.
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

import * as api from '../src/api'

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
})

describe('api wrappers', () => {
  it('listProjects → list_projects', () => {
    api.listProjects('claude')
    expect(invoke).toHaveBeenCalledWith('list_projects', {
      agent: 'claude',
      includeCodexInternal: false,
      includeCodexArchived: false,
    })
  })

  it('listSessions → list_sessions with pagination', () => {
    api.listSessions('codex', 'proj-key', 10, 20)
    expect(invoke).toHaveBeenCalledWith('list_sessions', {
      agent: 'codex',
      projectKey: 'proj-key',
      offset: 10,
      limit: 20,
      includeCodexInternal: false,
      includeCodexArchived: false,
    })
  })

  it('readSession → read_session', () => {
    api.readSession('claude', '/p/s.jsonl')
    expect(invoke).toHaveBeenCalledWith('read_session', {
      agent: 'claude',
      path: '/p/s.jsonl',
    })
  })

  it('sessionUsage → session_usage', () => {
    api.sessionUsage('codex', '/p/s.jsonl')
    expect(invoke).toHaveBeenCalledWith('session_usage', {
      agent: 'codex',
      path: '/p/s.jsonl',
    })
  })

  it('agentStats → agent_stats', () => {
    api.agentStats('claude')
    expect(invoke).toHaveBeenCalledWith('agent_stats', { agent: 'claude' })
  })

  it('startAgentStats → start_agent_stats with scope/range/requestId', () => {
    api.startAgentStats('all', 'days7', 42)
    expect(invoke).toHaveBeenCalledWith('start_agent_stats', {
      scope: 'all',
      range: 'days7',
      requestId: 42,
    })
  })

  it('cancelStats → cancel_stats', () => {
    api.cancelStats()
    expect(invoke).toHaveBeenCalledWith('cancel_stats')
  })

  it('setTrayEnabledAgents → set_tray_enabled_agents', () => {
    api.setTrayEnabledAgents(['claude', 'grok'])
    expect(invoke).toHaveBeenCalledWith('set_tray_enabled_agents', {
      agents: ['claude', 'grok'],
    })
  })

  it('nextStatsRequestId is monotonically increasing', () => {
    const a = api.nextStatsRequestId()
    const b = api.nextStatsRequestId()
    expect(b).toBeGreaterThan(a)
  })

  it('renameSession → rename_session', () => {
    api.renameSession('claude', '/p/s.jsonl', 'New name')
    expect(invoke).toHaveBeenCalledWith('rename_session', {
      agent: 'claude',
      path: '/p/s.jsonl',
      name: 'New name',
    })
  })

  it('softDeleteSession → soft_delete_session', () => {
    api.softDeleteSession('codex', '/p/s.jsonl', 'My Project')
    expect(invoke).toHaveBeenCalledWith('soft_delete_session', {
      agent: 'codex',
      path: '/p/s.jsonl',
      projectLabel: 'My Project',
    })
  })

  it('listTrash → list_trash with no args', () => {
    api.listTrash()
    expect(invoke).toHaveBeenCalledWith('list_trash')
  })

  it('restoreSession → restore_session', () => {
    api.restoreSession('trash-1.jsonl')
    expect(invoke).toHaveBeenCalledWith('restore_session', {
      trashFile: 'trash-1.jsonl',
    })
  })

  it('permanentDeleteTrash → permanent_delete_trash', () => {
    api.permanentDeleteTrash('trash-1.jsonl')
    expect(invoke).toHaveBeenCalledWith('permanent_delete_trash', {
      trashFile: 'trash-1.jsonl',
    })
  })

  it('emptyTrash → empty_trash', () => {
    api.emptyTrash()
    expect(invoke).toHaveBeenCalledWith('empty_trash')
  })

  it('revealInFinder → reveal_in_finder', () => {
    api.revealInFinder('/some/path')
    expect(invoke).toHaveBeenCalledWith('reveal_in_finder', { path: '/some/path' })
  })

  it('writeFile → write_file', () => {
    api.writeFile('/out.md', '# content')
    expect(invoke).toHaveBeenCalledWith('write_file', {
      path: '/out.md',
      content: '# content',
    })
  })

  it('resumeSession → resume_session', () => {
    api.resumeSession('claude', 'abc-123', '/work/dir', '/p/s.jsonl')
    expect(invoke).toHaveBeenCalledWith('resume_session', {
      agent: 'claude',
      sessionId: 'abc-123',
      cwd: '/work/dir',
      path: '/p/s.jsonl',
      extraArgs: '',
      terminalApp: 'terminal',
    })
  })

  it('watchSession → watch_session', () => {
    api.watchSession('claude', '/p/s.jsonl')
    expect(invoke).toHaveBeenCalledWith('watch_session', {
      agent: 'claude',
      path: '/p/s.jsonl',
    })
  })

  it('unwatchSession → unwatch_session with no args', () => {
    api.unwatchSession()
    expect(invoke).toHaveBeenCalledWith('unwatch_session')
  })

  it('installTurnHooks → install_turn_hooks', () => {
    api.installTurnHooks()
    expect(invoke).toHaveBeenCalledWith('install_turn_hooks')
  })

  it('turnHookStatus → turn_hook_status', () => {
    api.turnHookStatus()
    expect(invoke).toHaveBeenCalledWith('turn_hook_status')
  })

  it('appVersion → app_version', () => {
    api.appVersion()
    expect(invoke).toHaveBeenCalledWith('app_version')
  })

  it('passes the invoke result back to the caller', async () => {
    invoke.mockResolvedValue('1.2.3')
    await expect(api.appVersion()).resolves.toBe('1.2.3')
  })
})

describe('checkUpdate', () => {
  // checkUpdate is the one wrapper that doesn't just forward to invoke —
  // it calls app_version through invoke, then fetches GitHub's
  // /releases/latest endpoint and compares tag_name with the local version.
  beforeEach(() => {
    invoke.mockReset()
  })

  function mockFetch(impl: typeof globalThis.fetch) {
    vi.stubGlobal('fetch', vi.fn(impl))
  }

  function releaseJson(version: string, htmlUrl = `https://github.com/jerrywu001/cc-sessions-viewer/releases/tag/v${version}`) {
    return { tag_name: `v${version}`, html_url: htmlUrl }
  }

  function jsonResponse(body: unknown, init: { status?: number } = {}) {
    return {
      ok: (init.status ?? 200) < 400,
      status: init.status ?? 200,
      json: async () => body,
    } as Response
  }

  it('reports hasUpdate=true when remote tag is newer', async () => {
    invoke.mockResolvedValueOnce('0.1.1')
    mockFetch(async () => jsonResponse(releaseJson('0.2.0')))
    const r = await api.checkUpdate()
    expect(r).toEqual({
      current: '0.1.1',
      latest: '0.2.0',
      hasUpdate: true,
      htmlUrl: 'https://github.com/jerrywu001/cc-sessions-viewer/releases/tag/v0.2.0',
    })
  })

  it('reports hasUpdate=false when versions match', async () => {
    invoke.mockResolvedValueOnce('0.1.1')
    mockFetch(async () => jsonResponse(releaseJson('0.1.1')))
    const r = await api.checkUpdate()
    expect(r.hasUpdate).toBe(false)
    expect(r.latest).toBe('0.1.1')
  })

  it('treats 404 as an error (GitHub releases unavailable)', async () => {
    invoke.mockResolvedValueOnce('0.1.0')
    mockFetch(async () => jsonResponse({ message: 'Not Found' }, { status: 404 }))
    await expect(api.checkUpdate()).rejects.toThrow(/404/)
  })

  it('throws on other HTTP errors so the caller can surface the failure', async () => {
    invoke.mockResolvedValueOnce('0.1.0')
    mockFetch(async () => jsonResponse({ message: 'rate limited' }, { status: 503 }))
    await expect(api.checkUpdate()).rejects.toThrow(/503/)
  })
})
