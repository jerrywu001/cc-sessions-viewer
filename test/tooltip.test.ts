import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ObjectDirective } from 'vue'
import { vTooltip } from '../src/tooltip'

// vTooltip is declared as the Directive union; narrow it to the object form
// so the lifecycle hooks (mounted/updated/unmounted) are reachable.
const dir = vTooltip as unknown as Required<
  Pick<ObjectDirective<HTMLElement>, 'mounted' | 'updated' | 'unmounted'>
>

// Builds a minimal directive binding; the directive only reads value + arg.
const binding = (value: unknown, arg?: string) =>
  ({ value, arg, oldValue: null, modifiers: {}, instance: null, dir: vTooltip }) as never

const noVnode = null as never

function makeEl(): HTMLElement {
  const el = document.createElement('button')
  document.body.appendChild(el)
  return el
}

function tip(): HTMLElement | null {
  return document.querySelector<HTMLElement>('.cv-tooltip')
}
function isVisible(): boolean {
  const t = tip()
  return !!t && t.classList.contains('is-visible')
}

beforeEach(() => {
  vi.useFakeTimers()
  // The tooltip DOM node is a module-level singleton; reset its visible state
  // so leftover state from a previous test doesn't bleed in.
  tip()?.classList.remove('is-visible')
})
afterEach(() => {
  vi.useRealTimers()
  // Remove only the target buttons — the .cv-tooltip node is a module-level
  // singleton that ensureTipEl() caches and never re-appends, so wiping it
  // from <body> would orphan every later showFor() call.
  document.querySelectorAll('button').forEach((b) => b.remove())
})

describe('vTooltip.mounted', () => {
  it('mirrors the text into aria-label', () => {
    const el = makeEl()
    dir.mounted(el, binding('Save file'), noVnode, noVnode)
    expect(el.getAttribute('aria-label')).toBe('Save file')
  })

  it('preserves an explicit accessible name for a toggle button', () => {
    const el = makeEl()
    el.setAttribute('aria-label', 'Star session: Task')
    dir.mounted(el, binding('Remove star'), noVnode, noVnode)
    expect(el.getAttribute('aria-label')).toBe('Star session: Task')
    el.dispatchEvent(new FocusEvent('focusin'))
    expect(tip()!.textContent).toBe('Remove star')
  })

  it('binds nothing for an empty value', () => {
    const el = makeEl()
    dir.mounted(el, binding(''), noVnode, noVnode)
    expect(el.getAttribute('aria-label')).toBeNull()
    el.dispatchEvent(new MouseEvent('mouseenter'))
    vi.advanceTimersByTime(400)
    expect(isVisible()).toBe(false)
  })
})

describe('hover interaction', () => {
  it('shows after the 250ms delay and hides on mouseleave', () => {
    const el = makeEl()
    dir.mounted(el, binding('Hover text'), noVnode, noVnode)

    el.dispatchEvent(new MouseEvent('mouseenter'))
    expect(isVisible()).toBe(false) // not yet — still within the delay

    vi.advanceTimersByTime(400)
    expect(isVisible()).toBe(true)
    expect(tip()!.textContent).toBe('Hover text')

    el.dispatchEvent(new MouseEvent('mouseleave'))
    expect(isVisible()).toBe(false)
  })

  it('shows immediately on focus (no delay) and hides on blur', () => {
    const el = makeEl()
    dir.mounted(el, binding('Focus text'), noVnode, noVnode)

    el.dispatchEvent(new FocusEvent('focusin'))
    vi.advanceTimersByTime(20) // only the rAF that flips is-visible
    expect(isVisible()).toBe(true)
    expect(tip()!.textContent).toBe('Focus text')

    el.dispatchEvent(new FocusEvent('focusout'))
    expect(isVisible()).toBe(false)
  })
})

describe('placement', () => {
  it('honours an explicit "right" placement', () => {
    const el = makeEl()
    dir.mounted(el, binding('Side', 'right'), noVnode, noVnode)
    el.dispatchEvent(new FocusEvent('focusin'))
    expect(tip()!.dataset.placement).toBe('right')
  })

  it('defaults an unknown arg to auto placement', () => {
    const el = makeEl()
    dir.mounted(el, binding('Auto', 'sideways'), noVnode, noVnode)
    el.dispatchEvent(new FocusEvent('focusin'))
    expect(tip()!.dataset.placement).toBe('bottom')
  })
})

describe('vTooltip.updated', () => {
  it('swaps the tooltip text in place', () => {
    const el = makeEl()
    dir.mounted(el, binding('Old'), noVnode, noVnode)
    dir.updated(el, binding('New'), noVnode, noVnode)
    expect(el.getAttribute('aria-label')).toBe('New')

    el.dispatchEvent(new FocusEvent('focusin'))
    expect(tip()!.textContent).toBe('New')
  })

  it('updates or clears only its own label, not an explicit component label', () => {
    const el = makeEl()
    el.setAttribute('aria-label', 'Star session: Task')
    dir.mounted(el, binding('Star'), noVnode, noVnode)
    dir.updated(el, binding('Unstar'), noVnode, noVnode)
    expect(el.getAttribute('aria-label')).toBe('Star session: Task')
    dir.updated(el, binding(''), noVnode, noVnode)
    expect(el.getAttribute('aria-label')).toBe('Star session: Task')
  })

  it('relinquishes label ownership when a component supplies a new name', () => {
    const el = makeEl()
    dir.mounted(el, binding('Old tooltip'), noVnode, noVnode)
    el.setAttribute('aria-label', 'Explicit accessible name')
    dir.updated(el, binding('New tooltip'), noVnode, noVnode)
    expect(el.getAttribute('aria-label')).toBe('Explicit accessible name')
    dir.updated(el, binding(''), noVnode, noVnode)
    expect(el.getAttribute('aria-label')).toBe('Explicit accessible name')
  })

  it('unbinds when the value becomes empty', () => {
    const el = makeEl()
    dir.mounted(el, binding('Text'), noVnode, noVnode)
    dir.updated(el, binding(''), noVnode, noVnode)
    expect(el.getAttribute('aria-label')).toBeNull()
  })

  it('binds late when an element gained a value after mount', () => {
    const el = makeEl()
    dir.mounted(el, binding(''), noVnode, noVnode)
    dir.updated(el, binding('Appeared'), noVnode, noVnode)
    expect(el.getAttribute('aria-label')).toBe('Appeared')

    el.dispatchEvent(new FocusEvent('focusin'))
    expect(tip()!.textContent).toBe('Appeared')
  })
})

describe('vTooltip.unmounted', () => {
  it('detaches listeners so the tooltip no longer shows', () => {
    const el = makeEl()
    dir.mounted(el, binding('Gone soon'), noVnode, noVnode)
    dir.unmounted(el, binding('Gone soon'), noVnode, noVnode)

    el.dispatchEvent(new MouseEvent('mouseenter'))
    vi.advanceTimersByTime(400)
    expect(isVisible()).toBe(false)
  })
})
