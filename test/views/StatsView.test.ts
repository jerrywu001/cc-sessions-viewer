import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'

const { startMock } = vi.hoisted(() => ({ startMock: vi.fn() }))

vi.mock('../../src/stats', async () => {
  const { ref, shallowRef } = await import('vue')
  return {
    useStatsStream: () => ({
      stats: shallowRef({
        scope: 'grok',
        sessionCount: 1,
        messageCount: 2,
        callCount: 3,
        daysActive: 1,
        usage: {
          inputTokens: 100,
          outputTokens: 20,
          cacheCreationInputTokens: 0,
          cacheCreation1hInputTokens: 0,
          cacheReadInputTokens: 30,
          reasoningOutputTokens: 5,
          total: 155,
        },
        costUsd: 1.23,
        unpricedCallCount: 0,
        estimatedCallCount: 3,
        cacheHitRate: 0.23,
        projects: [],
        dailyActivity: [],
        topSessions: [{
          agent: 'grok',
          sessionId: 'safe-test-session',
          path: '/safe/test/updates.jsonl',
          projectDisplay: '/safe/test',
          title: 'Grok Build test',
          lastModified: 1,
          callCount: 3,
          usage: {
            inputTokens: 100,
            outputTokens: 20,
            cacheCreationInputTokens: 0,
            cacheCreation1hInputTokens: 0,
            cacheReadInputTokens: 30,
            reasoningOutputTokens: 5,
            total: 155,
          },
          costUsd: 0,
        }],
        byModel: [{
          model: 'custom/grok-test',
          label: 'custom/grok-test',
          callCount: 3,
          usage: {
            inputTokens: 100,
            outputTokens: 20,
            cacheCreationInputTokens: 0,
            cacheCreation1hInputTokens: 0,
            cacheReadInputTokens: 30,
            reasoningOutputTokens: 5,
            total: 155,
          },
          costUsd: 1.23,
          unpricedCallCount: 0,
          estimatedCallCount: 3,
          cacheHitRate: 0.23,
        }],
        byTool: [],
        byShell: [],
        byMcp: [],
        byActivity: [],
      }),
      stage: ref('done'),
      progress: ref({ processed: 1, total: 1 }),
      error: ref(''),
      start: startMock,
      cancel: vi.fn(),
    }),
  }
})

vi.mock('../../src/pricing', async () => {
  const { ref } = await import('vue')
  return {
    pricingStatus: ref({ loaded: true, fetching: false, lastError: null, modelCount: 1 }),
    refreshStatus: vi.fn().mockResolvedValue(undefined),
    watchUntilReady: vi.fn(),
    forceRefresh: vi.fn().mockResolvedValue(1),
  }
})

import StatsView from '../../src/views/StatsView.vue'
import { setLang, statsScope } from '../../src/settings'
import { vTooltip } from '../../src/tooltip'

describe('StatsView Grok integration', () => {
  beforeEach(() => {
    setLang('en')
    statsScope.value = 'all'
    startMock.mockClear()
  })

  it('shows Grok Build scope, marks fallback pricing, and preserves Grok session agent', async () => {
    const wrapper = mount(StatsView, {
      global: {
        directives: { tooltip: vTooltip },
        stubs: {
          StatsDailyChart: true,
          StatsModelChart: true,
          StatsActivityChart: true,
        },
      },
    })

    expect(wrapper.find('.stats-pill-agent img[alt="Grok Build"]').exists()).toBe(true)
    expect(wrapper.find('.kpi-card--brand .kpi-card-num').text()).toBe('$1.23~')
    expect(wrapper.find('.stats-hero-warning.estimated').text()).toContain('3 calls use an official Grok Build price estimate')
    expect(wrapper.find('.stats-pricing-missing.estimated').text()).toContain('1 models use estimated pricing')

    await wrapper.find('.bar-list-sessions .bar-row').trigger('click')
    expect(wrapper.emitted('open-session')?.[0]).toEqual([
      'grok',
      '/safe/test/updates.jsonl',
      'Grok Build test',
    ])
  })
})
