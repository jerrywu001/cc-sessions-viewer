import { beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  cleanupRuntimeChildren: vi.fn<() => Promise<void>>(),
  relaunch: vi.fn<() => Promise<void>>(),
}))

vi.mock('../src/api', () => ({
  appVersion: vi.fn(),
  checkUpdate: vi.fn(),
  cleanupRuntimeChildren: mocks.cleanupRuntimeChildren,
  openUrl: vi.fn(),
}))

vi.mock('@tauri-apps/plugin-process', () => ({
  relaunch: mocks.relaunch,
}))

vi.mock('@tauri-apps/plugin-updater', () => ({
  check: vi.fn(),
}))

import { relaunchApp } from '../src/updateCheck'

describe('relaunchApp', () => {
  beforeEach(() => {
    mocks.cleanupRuntimeChildren.mockReset().mockResolvedValue()
    mocks.relaunch.mockReset().mockResolvedValue()
  })

  it('waits for managed child processes to stop before relaunching', async () => {
    await relaunchApp()

    expect(mocks.cleanupRuntimeChildren).toHaveBeenCalledOnce()
    expect(mocks.relaunch).toHaveBeenCalledOnce()
    expect(mocks.cleanupRuntimeChildren.mock.invocationCallOrder[0])
      .toBeLessThan(mocks.relaunch.mock.invocationCallOrder[0])
  })

  it('does not relaunch when child-process cleanup fails', async () => {
    mocks.cleanupRuntimeChildren.mockRejectedValueOnce(new Error('cleanup failed'))

    await expect(relaunchApp()).rejects.toThrow('cleanup failed')
    expect(mocks.relaunch).not.toHaveBeenCalled()
  })
})
