// 模型价格表（models.dev 上游）加载状态 —— 给 StatsView 决定渲染什么。
//
// 后端 `stats::pricing::init()` 在 app 启动时后台线程拉一份，本模块通过
// `invoke('pricing_status')` 读那份状态。三种情况：
//
//   1. loaded=true：内存表里有数据（可能是 24h 内的本地 cache）→ 正常渲染。
//   2. loaded=false && lastError=null：还在拉 → 显示 loading 占位。
//   3. loaded=false && lastError!=null：拉失败 + 没有兜底 cache → 显示 error placeholder。
//
// app 启动后第一次进 StatsView 时，后端 fetch 可能还没结束 —— `watchUntilReady`
// 做一个 1s 间隔、最多 20 次的轻量 poll，命中 loaded 或拿到 lastError 后就停。
// 没用 backend event 是为了减少 init wiring（AppHandle 注入到 stats 模块里很重）。

import { invoke } from '@tauri-apps/api/core'
import { ref } from 'vue'

export interface PricingStatus {
  /** 内存价格表里至少有 1 条；前端可以放心渲染 cost。 */
  loaded: boolean
  /** 当前正在跑一次拉取（启动期或用户手动触发）。 */
  fetching: boolean
  /** 上次拉取失败时的错误描述；成功 / 还没拉过都是 null。 */
  lastError: string | null
  modelCount: number
}

const initial: PricingStatus = {
  loaded: false,
  fetching: false,
  lastError: null,
  modelCount: 0,
}

export const pricingStatus = ref<PricingStatus>(initial)

export async function refreshStatus(): Promise<PricingStatus> {
  try {
    const s = await invoke<PricingStatus>('pricing_status')
    pricingStatus.value = s
    return s
  } catch {
    // 后端命令调用失败本身就是异常态；保持当前 ref，让上层用现有 lastError 渲染。
    return pricingStatus.value
  }
}

let pollTimer: number | null = null

/** 启动期轮询：每秒 refresh 一次，直到 loaded 或 lastError 出现、最多 20 次。
 *  多次调用是幂等的 —— 已有 poll 在跑就直接返回。 */
export function watchUntilReady(): void {
  if (pollTimer !== null) return
  if (pricingStatus.value.loaded || pricingStatus.value.lastError) return

  let ticks = 0
  pollTimer = window.setInterval(async () => {
    ticks += 1
    await refreshStatus()
    if (pricingStatus.value.loaded || pricingStatus.value.lastError || ticks >= 20) {
      if (pollTimer !== null) {
        clearInterval(pollTimer)
        pollTimer = null
      }
    }
  }, 1000)
}

/** 用户手动「重试 / 立即刷新」：同步 invoke 后端，刷新 status。
 *  成功返回入表条数；失败往上抛错误（前端按需展示 toast）。 */
export async function forceRefresh(): Promise<number> {
  try {
    const n = await invoke<number>('refresh_pricing')
    await refreshStatus()
    return n
  } catch (e) {
    await refreshStatus()
    throw e
  }
}

/** 价格表里单条模型 entry —— 给 PricingView 渲染。
 *  价格字段都是 $/token（不是 $/Mtok），与后端 ModelCosts 同形；UI 乘 1e6 再展示。 */
export interface PricingEntry {
  name: string
  family: 'claude' | 'codex' | 'grok' | 'agy' | 'opencode'
  input: number
  output: number
  cacheWrite: number
  cacheRead: number
  /** Context window (max input tokens). 0 if upstream doesn't list it. */
  context: number
}

/** 拉当前价格表 —— 后端已按 family + input 升序排好，前端可直接 group_by。 */
export async function listPricing(): Promise<PricingEntry[]> {
  try {
    return await invoke<PricingEntry[]>('list_pricing')
  } catch {
    return []
  }
}
