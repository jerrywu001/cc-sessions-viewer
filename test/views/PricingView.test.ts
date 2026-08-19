import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

import PricingView from '../../src/views/PricingView.vue'
import { enabledAgents, setLang } from '../../src/settings'
import { vTooltip } from '../../src/tooltip'

describe('PricingView Grok family', () => {
  beforeEach(() => {
    setLang('en')
    invokeMock.mockReset()
    invokeMock.mockImplementation((command: string) => {
      if (command === 'pricing_status') {
        return Promise.resolve({ loaded: true, fetching: false, lastError: null, modelCount: 1 })
      }
      if (command === 'list_pricing') {
        return Promise.resolve([
          {
            name: 'grok-test-priced',
            family: 'grok',
            input: 2e-6,
            output: 10e-6,
            cacheWrite: 2.5e-6,
            cacheRead: 0.2e-6,
            context: 131_072,
          },
        ])
      }
      return Promise.resolve(undefined)
    })
  })

  it('renders the xAI family, Grok Build icon anchor, and model pricing row when Grok is hidden', async () => {
    const originalAgents = { ...enabledAgents.value }
    enabledAgents.value = { ...enabledAgents.value, grok: false }
    const wrapper = mount(PricingView, {
      global: { directives: { tooltip: vTooltip } },
    })
    try {
      await flushPromises()

      const grokSection = wrapper.findAll('.pricing-family').find((section) =>
        section.text().includes('Grok Build / xAI'),
      )
      expect(grokSection).toBeDefined()
      expect(grokSection?.text()).toContain('grok-test-priced')
      expect(grokSection?.text()).toContain('$2.00')
      expect(wrapper.find('.pricing-anchor.agent-grok img[alt="Grok Build"]').exists()).toBe(true)
    } finally {
      wrapper.unmount()
      enabledAgents.value = originalAgents
    }
  })
})
