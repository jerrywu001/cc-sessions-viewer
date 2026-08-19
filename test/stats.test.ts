import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, nextTick } from 'vue'
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'

// 关键 mock：@tauri-apps/api/{core,event}。listen 注册回调到一个本地分发器，
// 让我们可以"伪造"后端 emit `stats://progress` / `stats://done` 事件。
const { invokeMock, listeners, unlistenMock } = vi.hoisted(() => {
  const ls = new Map<string, ((e: { payload: unknown }) => void)[]>()
  return {
    invokeMock: vi.fn().mockResolvedValue(undefined),
    listeners: ls,
    unlistenMock: vi.fn(),
  }
})

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    const list = listeners.get(name) ?? []
    list.push(cb)
    listeners.set(name, list)
    return () => {
      unlistenMock(name)
      const arr = listeners.get(name) ?? []
      const idx = arr.indexOf(cb)
      if (idx >= 0) arr.splice(idx, 1)
    }
  },
}))

import { useStatsStream } from '../src/stats'
import type { AgentStats } from '../src/types'

function emit(event: string, payload: unknown) {
  for (const cb of listeners.get(event) ?? []) cb({ payload })
}

function emptyStats(): AgentStats {
  return {
    scope: 'all',
    sessionCount: 0,
    messageCount: 0,
    callCount: 0,
    daysActive: 0,
    usage: {
      inputTokens: 0,
      outputTokens: 0,
      cacheCreationInputTokens: 0,
      cacheCreation1hInputTokens: 0,
      cacheReadInputTokens: 0,
      reasoningOutputTokens: 0,
      total: 0,
    },
    costUsd: 0,
    unpricedCallCount: 0,
    estimatedCallCount: 0,
    cacheHitRate: 0,
    projects: [],
    dailyActivity: [],
    topSessions: [],
    byModel: [],
    byTool: [],
    byShell: [],
    byMcp: [],
    byActivity: [],
  }
}

// 把 useStatsStream 包成一个最小组件（onUnmounted 需要 setup 上下文）。
// 用 defineExpose 把整个 stream 对象（含 refs）挂到 vm.stream 上，避免
// setup return 触发 Vue 的自动 unref —— 我们要在测试里读 .value。
const Harness = defineComponent({
  setup(_, { expose }) {
    const stream = useStatsStream()
    expose({ stream })
    return () => null
  },
})

function getStream(wrapper: ReturnType<typeof mount>) {
  return (wrapper.vm as unknown as { stream: ReturnType<typeof useStatsStream> }).stream
}

enableAutoUnmount(afterEach)

beforeEach(() => {
  invokeMock.mockReset()
  invokeMock.mockResolvedValue(undefined)
  listeners.clear()
  unlistenMock.mockClear()
})

describe('useStatsStream', () => {
  it('starts a stream and accepts progress + done events for the active requestId', async () => {
    const wrapper = mount(Harness)
    const stream = getStream(wrapper)
    await stream.start('all', 'days30')
    await flushPromises()

    // start_agent_stats 被调用，requestId 单调递增（这是该测试中的第一次）
    const startCall = invokeMock.mock.calls.find((c) => c[0] === 'start_agent_stats')
    expect(startCall).toBeTruthy()
    const args = startCall![1] as { scope: string; range: string; requestId: number }
    expect(args.scope).toBe('all')
    expect(args.range).toBe('days30')
    const rid = args.requestId

    // 进度事件 → partial 写入
    const partial = { ...emptyStats(), sessionCount: 5 }
    emit('stats://progress', { requestId: rid, processed: 3, total: 10, partial })
    await nextTick()
    expect(stream.stats.value?.sessionCount).toBe(5)
    expect(stream.progress.value).toEqual({ processed: 3, total: 10 })
    expect(stream.stage.value).toBe('computing')

    // done 事件 → 最终态
    const final = { ...emptyStats(), sessionCount: 42 }
    emit('stats://done', { requestId: rid, stats: final })
    await nextTick()
    expect(stream.stats.value?.sessionCount).toBe(42)
    expect(stream.stage.value).toBe('done')
  })

  it('drops events whose requestId does not match the active stream', async () => {
    const wrapper = mount(Harness)
    const stream = getStream(wrapper)
    await stream.start('claude', 'months6')
    await flushPromises()
    const args = invokeMock.mock.calls.find((c) => c[0] === 'start_agent_stats')![1] as {
      requestId: number
    }
    const rid = args.requestId

    emit('stats://progress', {
      requestId: rid - 1,
      processed: 1,
      total: 1,
      partial: { ...emptyStats(), sessionCount: 999 },
    })
    await nextTick()
    expect(stream.stats.value).toBeNull()
  })

  it('surfaces error event onto the error ref', async () => {
    const wrapper = mount(Harness)
    const stream = getStream(wrapper)
    await stream.start('all', 'months6')
    await flushPromises()
    const rid = (
      invokeMock.mock.calls.find((c) => c[0] === 'start_agent_stats')![1] as {
        requestId: number
      }
    ).requestId
    emit('stats://error', { requestId: rid, error: 'boom' })
    await nextTick()
    expect(stream.error.value).toBe('boom')
    expect(stream.stage.value).toBe('error')
  })

  it('cancels the in-flight stream when starting a new one', async () => {
    const wrapper = mount(Harness)
    const stream = getStream(wrapper)
    await stream.start('all', 'today')
    await stream.start('codex', 'days7')
    await flushPromises()
    const cancelCalls = invokeMock.mock.calls.filter((c) => c[0] === 'cancel_stats')
    expect(cancelCalls.length).toBeGreaterThanOrEqual(1)
    const startCalls = invokeMock.mock.calls.filter((c) => c[0] === 'start_agent_stats')
    expect(startCalls.length).toBe(2)
    const rid1 = (startCalls[0][1] as { requestId: number }).requestId
    const rid2 = (startCalls[1][1] as { requestId: number }).requestId
    expect(rid2).toBeGreaterThan(rid1)
  })

  it('unmount triggers a cancel_stats request', async () => {
    const wrapper = mount(Harness)
    const stream = getStream(wrapper)
    await stream.start('all', 'months6')
    await flushPromises()
    wrapper.unmount()
    await flushPromises()
    expect(invokeMock.mock.calls.some((c) => c[0] === 'cancel_stats')).toBe(true)
  })
})
