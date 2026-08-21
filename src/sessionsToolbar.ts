// 会话列表工具栏与 SessionsView 之间的共享状态。
//
// SessionsTopbar 住在 App.vue 的顶栏 slot 里、SessionsView 住在 main 区域，二者
// 并不共享父模板。与 chatToolbar.ts / trashToolbar.ts 同理，这里用一个轻量模块
// （refs + 纯函数）做胶水：
//   - SessionsTopbar 写：sessionSearch / sessionSort
//   - SessionsView 读 filterSessions() 拿到过滤 + 排序后的列表
// 切换项目时 App.vue 调 resetSessionsToolbar() 清状态；任一筛选被激活时
// （sessionsFilterActive）App.vue 会一次性把整个项目的会话都加载进来，
// 避免分页窗口让搜索 / 排序只覆盖已加载的一页。

import { computed, ref } from 'vue'
import type { SessionMeta } from './types'

/** 排序方式：更新时间最新 / 最早、创建时间最新 / 最早、体积、消息数。 */
export type SessionSort =
  | 'recent'
  | 'oldest'
  | 'createdRecent'
  | 'createdOldest'
  | 'size'
  | 'messages'

/** 搜索关键词：匹配标题 + 会话 ID，空串表示未搜索。 */
export const sessionSearch = ref('')
/** 当前排序方式，默认与后端分页一致（时间最新在前）。 */
export const sessionSort = ref<SessionSort>('recent')

/** 批量选择模式开关（与回收站 trashToolbar 平行）。 */
export const sessionSelectMode = ref(false)
/** 已勾选的会话 path 集合（仅 sessionSelectMode 下有意义）。 */
export const selectedSessions = ref<Set<string>>(new Set())

/** 工具栏是否处于「非默认」状态。App.vue watch 此值决定是否加载全部会话。 */
export const sessionsFilterActive = computed(
  () =>
    sessionSearch.value.trim().length > 0 ||
    sessionSort.value !== 'recent',
)

/** 勾选 / 取消勾选一个会话。整体替换 Set 以触发响应式更新。 */
export function toggleSessionSelected(path: string) {
  const next = new Set(selectedSessions.value)
  if (next.has(path)) next.delete(path)
  else next.add(path)
  selectedSessions.value = next
}

/** 退出批量模式：关掉 sessionSelectMode 并清空选择。 */
export function exitSessionSelectMode() {
  sessionSelectMode.value = false
  selectedSessions.value = new Set()
}

/** 切换项目时把工具栏状态归零。 */
export function resetSessionsToolbar() {
  sessionSearch.value = ''
  sessionSort.value = 'recent'
  sessionSelectMode.value = false
  selectedSessions.value = new Set()
}

/** 应用排序。不再做关键词匹配 —— 关键词搜索现在走后端
 *  `searchSessions(projectKey)`，能命中会话标题 + 用户消息正文，而本地的元数据
 *  只够匹配 title / id 两列。读取模块 refs，故在 computed 里调用即响应式；
 *  返回新数组，不改动入参。体积 / 消息数排序在并列时回退到「时间最新」以保证稳定。 */
function sessionCreatedTime(session: SessionMeta): number {
  const created = session.created
    ? (/^\d+$/.test(session.created.trim())
      ? Number(session.created)
      : Date.parse(session.created))
    : Number.NaN
  return Number.isFinite(created) ? created : session.modified
}

export function filterSessions(sessions: SessionMeta[]): SessionMeta[] {
  const out = sessions.slice()
  const byRecent = (a: SessionMeta, b: SessionMeta) => b.modified - a.modified
  out.sort((a, b) => {
    switch (sessionSort.value) {
      case 'oldest':
        return a.modified - b.modified
      case 'createdRecent':
        return sessionCreatedTime(b) - sessionCreatedTime(a) || byRecent(a, b)
      case 'createdOldest':
        return sessionCreatedTime(a) - sessionCreatedTime(b) || byRecent(a, b)
      case 'size':
        return b.size - a.size || byRecent(a, b)
      case 'messages':
        return b.messageCount - a.messageCount || byRecent(a, b)
      default:
        return byRecent(a, b)
    }
  })
  return out
}
