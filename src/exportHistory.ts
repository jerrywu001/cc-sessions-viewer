import { ref } from 'vue'
import type { Agent } from './types'

// 「已导出会话」历史 —— 记录被导出过的原始会话，按时间倒序存 localStorage。
//
// 关键点：这里存的是**原始会话**的引用（agent + 原始 JSONL 路径），不是导出产物。
// 点开一条历史 = 用平时查看会话的同一套逻辑（read_session）重新打开那个原始 transcript，
// 跟落盘的 md/html/json 文件没有任何关系。
// 局限：1) 只记录加这个功能之后导出过的会话。
//       2) 原始文件被移动/删除后条目会失效 —— 打开时后端报错，列表项提供"移除"。

const KEY = 'exportHistory:v1'
const CAP = 50
const VALID_AGENTS: ReadonlySet<Agent> = new Set([
  'claude',
  'codex',
  'agy',
  'opencode',
  'grok',
  'kimicode',
])

export interface ExportRecord {
  /** 原始会话 JSONL 的绝对路径 —— 既是打开入口，也是去重键。 */
  path: string
  title: string
  agent: Agent
  sessionId: string
  cwd?: string
  /** 导出时刻（Date.now()），列表按它倒序。 */
  exportedAt: number
}

function load(): ExportRecord[] {
  try {
    const arr = JSON.parse(localStorage.getItem(KEY) ?? '[]')
    if (!Array.isArray(arr)) return []
    // 历史版本曾按导出文件路径（filePath）存；现在按原始会话（path）存。
    // 旧数据可能还缺 title/sessionId/exportedAt，逐条补齐而不是让一条旧记录
    // 影响整个历史页渲染。filePath-only 的记录无法重新打开原 transcript，丢弃。
    return arr.flatMap((raw): ExportRecord[] => {
      if (!raw || typeof raw !== 'object') return []
      const r = raw as Partial<ExportRecord>
      const storedAgent = (r as { agent?: unknown }).agent
      const agent = storedAgent === 'kimi' ? 'kimicode' : storedAgent
      if (
        !r.path ||
        typeof r.path !== 'string' ||
        !agent ||
        typeof agent !== 'string' ||
        !VALID_AGENTS.has(agent as Agent)
      ) {
        return []
      }
      return [{
        path: r.path,
        agent: agent as Agent,
        title: typeof r.title === 'string' ? r.title : '',
        sessionId: typeof r.sessionId === 'string' ? r.sessionId : '',
        cwd: typeof r.cwd === 'string' ? r.cwd : undefined,
        exportedAt: typeof r.exportedAt === 'number' && Number.isFinite(r.exportedAt)
          ? r.exportedAt
          : 0,
      }]
    })
  } catch {
    return []
  }
}

function persist() {
  // 导出已经成功，历史写入失败（例如浏览器禁用存储）不应反过来让导出流程报错。
  try {
    localStorage.setItem(KEY, JSON.stringify(history.value))
  } catch {}
}

/** 响应式快照：历史页读它；写操作整体替换以触发刷新。 */
export const history = ref<ExportRecord[]>(load())

/** 记录一次导出：同一原始会话去重提到队首，截断到 CAP。 */
export function recordExport(rec: ExportRecord) {
  const next = [rec, ...history.value.filter((r) => r.path !== rec.path)].slice(
    0,
    CAP,
  )
  history.value = next
  persist()
}

/** 从历史里移除一条（原始文件已失效 / 用户手动删）。 */
export function removeExport(path: string) {
  if (!history.value.some((r) => r.path === path)) return
  history.value = history.value.filter((r) => r.path !== path)
  persist()
}

/** 清空整个导出历史。 */
export function clearExportHistory() {
  if (!history.value.length) return
  history.value = []
  persist()
}
