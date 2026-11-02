// CSS contract regression: actual browser geometry is checked separately.
// Keep the source rule order, including the later generic 28px .icon-btn rule.
import { readFileSync } from 'node:fs'
import { afterEach, describe, expect, it } from 'vitest'

const source = readFileSync('src/style.css', 'utf8')
const selectors = new Set([
  '.session-card .session-star-btn',
  '.session-card.is-hover .session-star-btn',
  '.session-card:focus-within .session-star-btn',
  '.session-title',
  '.icon-btn',
])
const rules = [...source.replace(/\/\*[\s\S]*?\*\//g, '').matchAll(/([^{}]+)\{([^{}]*)\}/g)]
  .filter(match => match[1].split(',').some(selector => selectors.has(selector.trim())))
  .map(match => match[0]).join('\n')
const owned: HTMLElement[] = []
function fixture(starred = false) {
  const style = document.createElement('style')
  style.textContent = rules
  document.head.appendChild(style)
  const row = document.createElement('div')
  row.className = 'session-card'
  row.innerHTML = `<div class="session-title"><button class="icon-btn session-star-btn${starred ? ' active' : ''}"></button><span>Task title</span></div>`
  document.body.appendChild(row)
  owned.push(style, row)
  return { row, title: row.querySelector<HTMLElement>('.session-title')!, star: row.querySelector<HTMLElement>('button')! }
}
afterEach(() => { for (const node of owned.splice(0)) node.remove() })

describe('session star layout', () => {
  it.each([false, true])('takes no space while hidden (starred=%s)', (starred) => {
    const { star } = fixture(starred)
    expect(getComputedStyle(star).display).toBe('none')
  })

  it('keeps the revealed button within the fixed title line despite generic icon rules', () => {
    const { row, title, star } = fixture()
    const before = getComputedStyle(title).height
    row.classList.add('is-hover')
    expect(getComputedStyle(star).display).toBe('inline-flex')
    expect(getComputedStyle(star).height).toBe('20px')
    expect(getComputedStyle(title).height).toBe(before)
    expect(getComputedStyle(title).lineHeight).toBe('20px')
    expect(Number.parseFloat(getComputedStyle(star).height)).toBeLessThanOrEqual(Number.parseFloat(before))
  })

  it('removes the button without changing the title height on mouse leave', () => {
    const { row, title, star } = fixture(true)
    row.classList.add('is-hover')
    const height = getComputedStyle(title).height
    row.classList.remove('is-hover')
    expect(getComputedStyle(star).display).toBe('none')
    expect(getComputedStyle(title).height).toBe(height)
  })
})
