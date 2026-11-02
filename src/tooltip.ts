// Singleton tooltip implemented as a Vue directive `v-tooltip="text"`.
// 一个全局复用的 DOM 节点，挂在 <body> 上；hover/focus 时定位到目标元素附近。
// 主要替代原生 `title` —— 原生 tooltip 在 macOS WebKit 下风格生硬，且 240ms
// 才出现、字号小、深浅模式无法跟随。
import type { Directive } from 'vue'

type Placement = 'top' | 'bottom' | 'right' | 'auto'

interface BindData {
  text: string
  placement: Placement
  /** Only labels created by this directive may be replaced or removed. */
  autoLabel: string | null
  enter: () => void
  leave: () => void
  focusin: () => void
  focusout: () => void
}

const bindings = new WeakMap<HTMLElement, BindData>()
let tipEl: HTMLDivElement | null = null
let showTimer = 0
let activeEl: HTMLElement | null = null

function ensureTipEl(): HTMLDivElement {
  if (tipEl) return tipEl
  const el = document.createElement('div')
  el.className = 'cv-tooltip'
  el.setAttribute('role', 'tooltip')
  document.body.appendChild(el)
  tipEl = el
  return el
}

/**
 * body 上挂着字号缩放 `zoom`（来自设置，见 settings.ts）。tipEl 是 body 的子节点，
 * 其 `position: fixed` 坐标会被这个 zoom 整体缩放，而我们用 getBoundingClientRect 算出的
 * 是**视觉像素**（已含 zoom）。所以最终写 style 时要 / zoom 抵消，否则 tip 会朝左上角漂。
 */
function currentZoom(): number {
  const z = parseFloat(getComputedStyle(document.body).zoom || '1')
  return Number.isFinite(z) && z > 0 ? z : 1
}

function showFor(target: HTMLElement, text: string, placement: Placement) {
  const el = ensureTipEl()
  el.textContent = text
  // 重置位置以便测量真实尺寸（max-width 由 CSS 控制）
  el.style.left = '0px'
  el.style.top = '0px'
  el.classList.remove('is-visible')
  const targetRect = target.getBoundingClientRect()
  const rect = el.getBoundingClientRect()
  const gap = 6
  const margin = 6
  const zoom = currentZoom()

  // 'right'：浮在目标右侧、垂直居中；右侧放不下则翻到左侧
  if (placement === 'right') {
    let left = targetRect.right + gap
    if (left + rect.width + margin > window.innerWidth) {
      left = targetRect.left - rect.width - gap
    }
    left = Math.max(margin, left)
    let top = targetRect.top + targetRect.height / 2 - rect.height / 2
    top = Math.max(
      margin,
      Math.min(window.innerHeight - rect.height - margin, top),
    )
    el.style.left = `${Math.round(left / zoom)}px`
    el.style.top = `${Math.round(top / zoom)}px`
    el.dataset.placement = 'right'
    requestAnimationFrame(() => el.classList.add('is-visible'))
    return
  }

  // 'top' / 'bottom' 强制方向；'auto' 默认朝下，碰到下边界再翻到上
  let placeAbove =
    placement === 'top'
      ? true
      : placement === 'bottom'
        ? false
        : targetRect.bottom + gap + rect.height + margin > window.innerHeight
  let top = placeAbove
    ? targetRect.top - rect.height - gap
    : targetRect.bottom + gap
  // 强制方向时若越界仍 clamp 到可视区，避免被裁掉
  if (top < margin) {
    top = margin
    placeAbove = false
  } else if (top + rect.height + margin > window.innerHeight) {
    top = window.innerHeight - rect.height - margin
  }
  let left = targetRect.left + targetRect.width / 2 - rect.width / 2
  left = Math.max(margin, Math.min(window.innerWidth - rect.width - margin, left))
  el.style.left = `${Math.round(left / zoom)}px`
  el.style.top = `${Math.round(top / zoom)}px`
  el.dataset.placement = placeAbove ? 'top' : 'bottom'
  requestAnimationFrame(() => el.classList.add('is-visible'))
}

function hide() {
  if (showTimer) {
    clearTimeout(showTimer)
    showTimer = 0
  }
  activeEl = null
  tipEl?.classList.remove('is-visible')
}

// 程序化触发：用于 v-html 动态生成、挂不上指令的元素（如聊天气泡里的命令 token）。
// 复用同一个单例 tip 节点与 250ms 延迟；hideTooltip 收起。
export function showTooltipFor(el: HTMLElement, text: string) {
  if (!text) return
  activeEl = el
  if (showTimer) clearTimeout(showTimer)
  showTimer = window.setTimeout(() => {
    if (activeEl === el) showFor(el, text, 'auto')
  }, 250)
}
export function hideTooltip() {
  hide()
}

function bind(el: HTMLElement, text: string, placement: Placement) {
  const data: BindData = {
    text,
    placement,
    autoLabel: el.hasAttribute('aria-label') ? null : text,
    enter() {
      if (!data.text) return
      activeEl = el
      if (showTimer) clearTimeout(showTimer)
      showTimer = window.setTimeout(() => {
        if (activeEl === el) showFor(el, data.text, data.placement)
      }, 250)
    },
    leave() {
      if (activeEl === el) hide()
    },
    focusin() {
      if (!data.text) return
      activeEl = el
      // 键盘聚焦时不延迟
      showFor(el, data.text, data.placement)
    },
    focusout() {
      if (activeEl === el) hide()
    },
  }
  if (data.autoLabel !== null) el.setAttribute('aria-label', data.autoLabel)
  el.addEventListener('mouseenter', data.enter)
  el.addEventListener('mouseleave', data.leave)
  el.addEventListener('focusin', data.focusin)
  el.addEventListener('focusout', data.focusout)
  bindings.set(el, data)
}

function unbind(el: HTMLElement) {
  const data = bindings.get(el)
  if (!data) return
  el.removeEventListener('mouseenter', data.enter)
  el.removeEventListener('mouseleave', data.leave)
  el.removeEventListener('focusin', data.focusin)
  el.removeEventListener('focusout', data.focusout)
  if (activeEl === el) hide()
  bindings.delete(el)
}

function readPlacement(arg: string | undefined): Placement {
  return arg === 'top' || arg === 'bottom' || arg === 'right' ? arg : 'auto'
}

export const vTooltip: Directive<HTMLElement, string | undefined | null> = {
  mounted(el, binding) {
    const text = typeof binding.value === 'string' ? binding.value : ''
    if (!text) return
    bind(el, text, readPlacement(binding.arg))
  },
  updated(el, binding) {
    const next = typeof binding.value === 'string' ? binding.value : ''
    const placement = readPlacement(binding.arg)
    const prev = bindings.get(el)
    if (next === (prev?.text ?? '') && placement === (prev?.placement ?? 'auto')) return
    if (prev) {
      if (next) {
        prev.text = next
        prev.placement = placement
        if (prev.autoLabel !== null && el.getAttribute('aria-label') === prev.autoLabel) {
          el.setAttribute('aria-label', next)
          prev.autoLabel = next
        } else {
          prev.autoLabel = null
        }
      } else {
        if (prev.autoLabel !== null && el.getAttribute('aria-label') === prev.autoLabel) {
          el.removeAttribute('aria-label')
        }
        unbind(el)
      }
    } else if (next) {
      bind(el, next, placement)
    }
  },
  unmounted(el) {
    unbind(el)
  },
}
