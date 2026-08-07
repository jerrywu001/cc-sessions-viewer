// 轻量文本格式化：把会话内容渲染成可读的 HTML（无第三方依赖）。
import { t } from './i18n'
import { renderCodexPluginLinkHtml } from './codexPluginMentions'

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
}

function escapeHtmlAttr(s: string): string {
  return escapeHtml(s).replace(/"/g, '&quot;')
}

// Backtick is excluded so a bare URL never swallows the closing backtick of an
// inline-code span (`https://x`) — that used to desync all later code/strong tags.
const URL_RE = /https?:\/\/[^\s<>&)}\]`]+/g
const MD_LINK_RE = /\[([^\]\n]+)\]\((<[^>\n]+>|[^)\n]+)\)/g

// 「文件引用」inline code：形如 lib/a/b.dart:371、./src/x.ts、/abs/y.rs:3:7、C:\p\z.cs。
// 必须含至少一个路径分隔符（否则 obj.method / package.json 之类会被误判），末段是
// name.ext，可带 :行 或 :行:列。命中后渲染成可点 code —— ChatView 按会话 cwd 解析、
// 在外部编辑器打开。`https://…/a.ts` 这种以 `scheme:` 开头的不会命中（冒号断掉首段）。
const FILE_REF_RE =
  /^(?:~\/|\.{1,2}[\\/]|\/|[A-Za-z]:[\\/])?(?:[\w.@~-]+[\\/])+[\w.@-]+\.[A-Za-z][\w-]*(?::\d+(?::\d+)?)?$/

/**
 * 把文件引用拆成「路径 + 可选 行 / 列」。末尾的 `:行` 或 `:行:列` 是定位信息（点击后用于在
 * 支持跳转的编辑器里定位），路径本身交给后端按 cwd 解析。Windows 盘符冒号（`C:\…`）不在末尾，
 * 不会被误拆。
 */
export function parseFileRef(raw: string): { path: string; line?: number; col?: number } {
  const m = /^(.*?):(\d+)(?::(\d+))?$/.exec(raw)
  if (!m) return { path: raw }
  return { path: m[1], line: Number(m[2]), col: m[3] ? Number(m[3]) : undefined }
}

function isExternalUrl(target: string): boolean {
  return /^https?:\/\//i.test(target)
}

function isAbsoluteLocalPath(target: string): boolean {
  return (
    target.startsWith('/') ||
    /^[A-Za-z]:[\\/]/.test(target) ||
    target.startsWith('\\\\')
  )
}

function renderMarkdownLink(label: string, rawTarget: string): string {
  const target = rawTarget.trim().replace(/^<|>$/g, '')
  const plugin = renderCodexPluginLinkHtml(label, target, escapeHtml)
  if (plugin) return plugin
  const text = escapeHtml(label)
  if (isAbsoluteLocalPath(target)) {
    const escapedTarget = escapeHtmlAttr(target)
    return `<a href="${escapedTarget}" class="local-file-link" data-local-file-link="1" data-local-target="${escapedTarget}" title="${escapedTarget}">${text}</a>`
  }
  if (isExternalUrl(target)) {
    const escapedTarget = escapeHtmlAttr(target)
    return `<a href="${escapedTarget}" target="_blank" rel="noopener">${text}</a>`
  }
  return `<a href="${escapeHtmlAttr(target)}">${text}</a>`
}

function inline(text: string): string {
  const links: string[] = []
  let s = text.replace(MD_LINK_RE, (_m, label, target) => {
    const idx = links.push(renderMarkdownLink(label, target)) - 1
    return `\u0001LINK${idx}\u0001`
  })
  // Pull inline-code spans out to placeholders BEFORE the URL / emphasis passes.
  // Their contents must stay literal — a URL or `**` inside backticks must not be
  // linkified/bolded, and (critically) the URL regex must not reach across a code
  // span and swallow its closing backtick, which would split the emitted tags and
  // misnest `<code>`/`<strong>` into every following sibling. The \x01 sentinel
  // (same convention as the link placeholder above) keeps placeholders collision-safe.
  const SENT = String.fromCharCode(1)
  const codes: string[] = []
  s = s.replace(/`([^`\n]+)`/g, (_m, code) => {
    const idx = codes.push(code) - 1
    return `${SENT}CODE${idx}${SENT}`
  })
  // 行内数学 $...$ → 占位（保护内容不被后续 pass 误改）
  s = s.replace(/\$([^\$\n]+?)\$/g, (_m, expr) => {
    const idx = codes.push(`MATH:${expr}`) - 1
    return `${SENT}CODE${idx}${SENT}`
  })
  // 行内代码 ~~~code~~~（opencode 用的非标准语法）
  s = s.replace(/~~~([^~\n]+?)~~~/g, (_m, code) => {
    const idx = codes.push(code) - 1
    return `${SENT}CODE${idx}${SENT}`
  })
  s = escapeHtml(s)
  s = s.replace(URL_RE, (url) => `<a href="${url}" target="_blank" rel="noopener">${url}</a>`)
  s = s.replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>')
  s = s.replace(/(?<![*\\])\*([^*\n]+)\*(?!\*)/g, '<em>$1</em>')
  s = s.replace(/~~([^~\n]+)~~/g, '<del>$1</del>')
  s = s.replace(/==([^=\n]+)==/g, '<mark class="md-mark">$1</mark>')
  s = s.replace(/\^([^\^\s]+)\^/g, '<sup>$1</sup>')
  s = s.replace(/~([^~\s]+)~/g, '<sub>$1</sub>')
  s = s.replace(/\[\^(\w+)\]/g, '<sup class="md-fn-ref">[$1]</sup>')
  s = s.replace(/^######\s+(.+)$/gm, '<h6>$1</h6>')
  s = s.replace(/^#####\s+(.+)$/gm, '<h6>$1</h6>')
  s = s.replace(/^####\s+(.+)$/gm, '<h5>$1</h5>')
  s = s.replace(/^###\s+(.+)$/gm, '<h4>$1</h4>')
  s = s.replace(/^##\s+(.+)$/gm, '<h3>$1</h3>')
  s = s.replace(/^#\s+(.+)$/gm, '<h3>$1</h3>')
  s = s.replace(/\n/g, '<br>')
  s = s.replace(/(?:<br>\s*)+(<h[3-6]>)/g, '$1')
  s = s.replace(/(<\/h[3-6]>)(?:\s*<br>)+/g, '$1')
  // Restore code spans (escaped, so contents stay literal) BEFORE links, so a link
  // placeholder captured inside a code span still gets expanded by the link pass.
  if (codes.length) {
    const codeRe = new RegExp(`${SENT}CODE(\\d+)${SENT}`, 'g')
    s = s.replace(codeRe, (_m, n) => {
      const raw = codes[Number(n)] ?? ''
      if (raw.startsWith('MATH:')) {
        const expr = raw.slice(5)
        return `<span class="md-math-inline" data-math="${escapeHtmlAttr(expr)}">${escapeHtml(expr)}</span>`
      }
      if (FILE_REF_RE.test(raw)) {
        return `<code class="file-ref" data-file-ref="${escapeHtmlAttr(raw)}">${escapeHtml(raw)}</code>`
      }
      return `<code>${escapeHtml(raw)}</code>`
    })
  }
  if (links.length) {
    s = s.replace(/\u0001LINK(\d+)\u0001/g, (_m, n) => links[Number(n)] ?? '')
  }
  return s
}

// Claude Code / Codex inject slash-command markup into the user message as
// pseudo-XML: <command-name>/init</command-name>, <command-message>init</…>,
// <command-args>foo bar</…>. Rendering them literally is ugly.
//
// <command-message> is just the slash command name without the leading "/" —
// fully redundant with <command-name>. We drop it. <command-name> and
// <command-args> get re-emitted as inline <code> chips via a placeholder pass
// so the inner text still goes through escapeHtml safely.
const COMMAND_MESSAGE_RE = /\s*<command-message>[\s\S]*?<\/command-message>\s*/g
const COMMAND_TAG_RE = /<(command-(?:name|args))>([\s\S]*?)<\/\1>/g
// Claude Code injects a `<local-command-caveat>…</local-command-caveat>` user
// message right before every shell-output relay (e.g. when the user types `!ls`).
// It's plumbing for the model and pure noise to humans — hide it everywhere.
const LOCAL_COMMAND_CAVEAT_RE = /^\s*<local-command-caveat>[\s\S]*?<\/local-command-caveat>\s*$/

/** True if a user "Me" message is just a Claude Code local-command caveat
 *  (no other text/image/tool content). Such messages should be hidden in
 *  the chat view and skipped in exports. */
export function isCaveatOnlyMsg(m: { role: string; blocks: Array<{ kind: string; text?: string }> }): boolean {
  if (m.role !== 'user') return false
  if (m.blocks.length === 0) return false
  return m.blocks.every(
    (b) => b.kind === 'text' && LOCAL_COMMAND_CAVEAT_RE.test(b.text ?? ''),
  )
}

// Claude Code wraps various app-level facts in <system-reminder> tags inside a
// synthetic user message. The /rename command shows up as:
//   <system-reminder>
//   The user named this session "批量导入". This may indicate the session's focus or intent.
//   </system-reminder>
// Rendering that verbatim looks like a "Me" said an English meta-line. We turn
// it into a centered, localized system-event line instead.
const SYSTEM_REMINDER_RE = /<system-reminder>([\s\S]*?)<\/system-reminder>/
const RENAME_INNER_RE = /The user named this session\s+"([^"]+)"/i
// Claude Code writes a standalone "[Request interrupted by user]" (optionally
// "… for tool use") user message when you hit Esc mid-turn. It's a system event,
// not prose — render it as a centered line like /rename, not a "Me" bubble.
const INTERRUPT_RE = /^\[Request interrupted by user(?: for tool use)?\]$/

// Claude Code 的 GUI / SDK 测试有时会把「要求模型调用 AskUserQuestion」的指令原样写入
// transcript。真正的问题随后会由 assistant 的 AskUserQuestion tool_use 渲染成提问卡，
// 所以这类纯协议指令不应再占一个用户气泡。只匹配明确的工具测试句式，避免误伤普通
// 提到 AskUserQuestion 的用户讨论。
const ASK_USER_QUESTION_INSTRUCTION_RE =
  /^Use\s+the\s+AskUserQuestion\s+tool\s+exactly\s+once\b[\s\S]*\bAsk\s+(?:ONE|TWO|THREE|FOUR|FIVE|SIX|SEVEN|EIGHT|NINE|\d+)\b[\s\S]*\bquestions?\b[\s\S]*$/i

/** True when a user record is only the explicit instruction that asks Claude to emit questions. */
export function isAskUserQuestionInstructionOnlyMsg(m: {
  role: string
  blocks: Array<{ kind: string; text?: string }>
}): boolean {
  if (m.role !== 'user' || m.blocks.length !== 1 || m.blocks[0].kind !== 'text') return false
  return ASK_USER_QUESTION_INSTRUCTION_RE.test((m.blocks[0].text ?? '').trim())
}

export type SystemEvent = { kind: 'rename'; name: string } | { kind: 'interrupt' }

/** Parse a user message into a SystemEvent if it consists solely of a
 *  recognized marker (rename <system-reminder> or an interrupt line).
 *  Returns null otherwise. */
export function parseSystemEvent(m: {
  role: string
  blocks: Array<{ kind: string; text?: string }>
}): SystemEvent | null {
  if (m.role !== 'user') return null
  if (m.blocks.length !== 1 || m.blocks[0].kind !== 'text') return null
  const text = (m.blocks[0].text ?? '').trim()
  if (INTERRUPT_RE.test(text)) return { kind: 'interrupt' }
  const sr = SYSTEM_REMINDER_RE.exec(text)
  if (!sr) return null
  // The whole message must be just the reminder — no other prose around it.
  if (text.replace(SYSTEM_REMINDER_RE, '').trim() !== '') return null
  const rn = RENAME_INNER_RE.exec(sr[1])
  if (rn) return { kind: 'rename', name: rn[1] }
  return null
}

// ─── 系统注入的 user 记录（metaKind）的展示 ──────────────────────────────
// 后端 claude 源给压缩摘要 / skill 注入 / 任务通知 / 命令输出这类 `type:"user"`
// 记录打了 metaKind 标记。这里决定它们怎么渲染：
//   - compact / meta —— 本身就是 markdown 文本，走 renderText（标题、列表、代码）
//   - 其余（task-notification / system / command-output）—— 是带伪 XML 包裹 +
//     可能含 ANSI 控制码的纯文本输出，去壳后以等宽 <pre> 原样呈现更贴近终端观感
const META_PRE_KINDS = new Set(['task-notification', 'system', 'command-output'])

/** 该 metaKind 是否以等宽 <pre>（而非 markdown）呈现。 */
export function metaKindIsPre(kind: string): boolean {
  return META_PRE_KINDS.has(kind)
}

// 命令输出 / 任务通知外面包着一层 Claude Code 的伪 XML 标签，去掉后正文更干净。
const META_WRAPPER_RE =
  /^\s*<(local-command-stdout|local-command-stderr|bash-stdout|bash-stderr|task-notification|system)>([\s\S]*?)<\/\1>\s*$/
// 终端输出里夹着 ANSI 转义序列（ESC[2m 调暗、ESC[0m 复位）—— 纯噪音，去掉。
// ESC（0x1B）控制字节用 fromCharCode 构造，避免把不可见控制符写进源码。
const ESC = String.fromCharCode(27)
const ANSI_RE = new RegExp(ESC + '\\[[0-9;]*m', 'g')

/** 把 metaKind 记录的正文清理成可读纯文本：剥掉外层伪 XML 包裹标签 + ANSI 控制码。 */
export function cleanMetaText(text: string): string {
  const m = META_WRAPPER_RE.exec(text)
  const inner = m ? m[2] : text
  return inner.replace(ANSI_RE, '').trim()
}

// 任务通知正文是一串伪 XML 字段：<task-id>…</task-id>、<summary>…</summary>、
// <event>…</event> 等。直接展示这些尖括号标签很难读，这里解析成 key/value 对，
// 前端再格式化成「字段名 + 值」两列。
const META_FIELD_RE = /<([a-z][a-z0-9-]*)>([\s\S]*?)<\/\1>/gi

export interface MetaField {
  key: string
  value: string
}

/** 把 metaKind 正文解析成有序的字段列表（仅当正文「全是」<tag>value</tag> 字段、
 *  标签之间只有空白时才算）。命令输出之类的纯文本返回 null（交给 <pre> 渲染）。 */
export function parseMetaFields(text: string): MetaField[] | null {
  const cleaned = cleanMetaText(text)
  const fields: MetaField[] = []
  let lastEnd = 0
  let onlyTags = true
  META_FIELD_RE.lastIndex = 0
  let m: RegExpExecArray | null
  while ((m = META_FIELD_RE.exec(cleaned)) !== null) {
    // 字段之间若夹着非空白文本，说明不是「纯字段」结构，放弃格式化。
    if (cleaned.slice(lastEnd, m.index).trim() !== '') onlyTags = false
    fields.push({ key: m[1], value: m[2].trim() })
    lastEnd = m.index + m[0].length
  }
  if (cleaned.slice(lastEnd).trim() !== '') onlyTags = false
  if (fields.length === 0 || !onlyTags) return null
  return fields
}

// 多 agent 协作时，对方 Claude 会话发来的消息被注入成一条 user 记录，形如：
//   Another Claude session sent a message:
//   <teammate-message teammate_id="x" color="blue">{payload}</teammate-message>
//   ...（一段固定的 harness 说明，纯噪音）
// 这里把每个 <teammate-message> 块抽成「队友 id → payload」字段，丢掉前后的说明文字。
const TEAMMATE_BLOCK_RE = /<teammate-message\s+([^>]*?)>([\s\S]*?)<\/teammate-message>/g
const TEAMMATE_ID_RE = /teammate_id\s*=\s*"([^"]*)"/

/** 解析 teammate-message 注入的正文为「队友 → payload」字段；非该结构返回 null。 */
export function parseTeammateMessage(text: string): MetaField[] | null {
  const fields: MetaField[] = []
  TEAMMATE_BLOCK_RE.lastIndex = 0
  let m: RegExpExecArray | null
  while ((m = TEAMMATE_BLOCK_RE.exec(text)) !== null) {
    const id = TEAMMATE_ID_RE.exec(m[1])?.[1] ?? 'teammate'
    fields.push({ key: id, value: m[2].trim() })
  }
  return fields.length ? fields : null
}
const COMMAND_NAME_RE = /<command-name>([\s\S]*?)<\/command-name>/
const COMMAND_ARGS_RE = /<command-args>([\s\S]*?)<\/command-args>/

/** 把 slash 命令的伪 XML（<command-name>/effort</…> + <command-message>…</…> +
 *  <command-args>…</…>）还原成用户当初敲的那行纯文本（"/effort" 或 "/review src/x"）。
 *  供历史回填 / 导出用 —— 把那坨标签收回成干净命令。非命令标记返回 null。 */
export function commandInputFromMarkup(text: string): string | null {
  const name = COMMAND_NAME_RE.exec(text)?.[1]?.trim()
  if (!name) return null
  const args = COMMAND_ARGS_RE.exec(text)?.[1]?.trim()
  return args ? `${name} ${args}` : name
}

type CmdCode = { isName: boolean; inner: string }
function extractCommandTags(raw: string): { text: string; codes: CmdCode[] } {
  const codes: CmdCode[] = []
  const stripped = raw.replace(COMMAND_MESSAGE_RE, '')
  const text = stripped.replace(COMMAND_TAG_RE, (_m, tag, inner) => {
    // 无参 slash 命令（`/clear` / `/init` 等）会带一个空的 <command-args></…>。
    // 留着就会渲染出一个只有 padding+背景的空 chip —— 像个小色块挂在命令后面。
    // inner 是空 / 全空白时直接吞掉整个标签。
    if (!inner.trim()) return ''
    // 命令名（command-name，形如 /review）单独标记 → 渲染成蓝色；参数（command-args）走普通文本。
    const idx = codes.push({ isName: tag === 'command-name', inner }) - 1
    return `CMD${idx}`
  })
  return { text, codes }
}

// ─── GFM-lite 表格 ──────────────────────────────────────────────────
// 检测 markdown table 并渲染成 <table>。之前用户反馈："table 渲染出来是
// `| 路由 | 路径 | 文件 |\n|---|---|---|` 一坨原始字符 + inline code 把每个
// `|` 单元包成小灰块" —— 完全不能读。这里加最小可用版：
//   - 表头行：`| col | col |`（前后 `|` 可省）
//   - 分隔行：`|---|---|`（可带 `:` 做对齐）
//   - 表体行：跟表头同形态
// 单元格内容仍走 inline()，所以行内强调 / inline code / 链接照常生效。
// 转义 `\|` 不处理（罕见，遇到再加）。
// 分隔格按 GFM：至少 1 个连字符即合法（`-` / `:-` / `-:` / `:-:`）。之前误写成
// `-{3,}`，导致对齐列只用 `--:`（2 个连字符）的表格整张当普通文本渲染。
const TABLE_SEP_CELL_RE = /^\s*:?-+:?\s*$/

function isTableSeparator(line: string): boolean {
  const cells = line.trim().replace(/^\||\|$/g, '').split('|')
  if (cells.length < 1) return false
  return cells.every((c) => TABLE_SEP_CELL_RE.test(c))
}

function splitTableRow(line: string): string[] {
  return line.trim().replace(/^\||\|$/g, '').split('|').map((c) => c.trim())
}

type CellAlign = 'left' | 'center' | 'right' | null
function getAlignments(separator: string): CellAlign[] {
  const cells = separator.trim().replace(/^\||\|$/g, '').split('|')
  return cells.map((c) => {
    const tt = c.trim()
    const l = tt.startsWith(':')
    const r = tt.endsWith(':')
    if (l && r) return 'center'
    if (r) return 'right'
    if (l) return 'left'
    return null
  })
}

function renderTableHtml(
  headerCells: string[],
  alignments: CellAlign[],
  bodyRows: string[][],
): string {
  const cell = (tag: 'th' | 'td', text: string, idx: number) => {
    const a = alignments[idx]
    const style = a ? ` style="text-align:${a}"` : ''
    return `<${tag}${style}>${inline(text)}</${tag}>`
  }
  const head = '<tr>' + headerCells.map((c, i) => cell('th', c, i)).join('') + '</tr>'
  const body = bodyRows
    .map((row) => '<tr>' + row.map((c, i) => cell('td', c, i)).join('') + '</tr>')
    .join('')
  // 外面套一层 .md-table-wrap 提供 overflow-x —— 列多 / 单元格内容长时
  // 整张表才能横向滚动；不套的话 table 要么撑爆父容器要么 cells 被挤换行。
  return `<div class="md-table-wrap"><table class="md-table"><thead>${head}</thead><tbody>${body}</tbody></table></div>`
}

// 主题分隔线（thematic break）：整行只有 3+ 个 - / * / _（可夹空白）。
// 之前没有处理，`---` 会被当字面量渲染成一行 "---"；这里识别出来发 <hr>。
// 注意要排在 table 检测之后：表格分隔行 `|---|---|` 带竖线，不会命中本正则。
const HR_RE = /^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/

const BULLET_ITEM_RE = /^\s*[-*]\s+(.+?)\s*$/
const ORDERED_ITEM_RE = /^\s*\d+[.)]\s+(.+?)\s*$/
const TASK_ITEM_RE = /^\[([xX ])\]\s+(.+)$/
const DEF_TERM_RE = /^\S/
const DEF_LINE_RE = /^:\s+(.+)$/

function isBulletItem(line: string): boolean {
  return BULLET_ITEM_RE.test(line)
}

function isOrderedItem(line: string): boolean {
  return ORDERED_ITEM_RE.test(line)
}

function bulletItemText(line: string): string {
  const m = BULLET_ITEM_RE.exec(line)
  return m?.[1] ?? line.trim()
}

function orderedItemText(line: string): string {
  const m = ORDERED_ITEM_RE.exec(line)
  return m?.[1] ?? line.trim()
}

function renderListItemHtml(text: string): string {
  const task = TASK_ITEM_RE.exec(text)
  if (task) {
    const checked = task[1].toLowerCase() === 'x'
    const icon = checked
      ? '<span class="md-check checked">&#9745;</span>'
      : '<span class="md-check">&#9744;</span>'
    return `<li class="md-task">${icon}${inline(task[2])}</li>`
  }
  return `<li>${inline(text)}</li>`
}

function renderBulletListHtml(items: string[]): string {
  const body = items.map(renderListItemHtml).join('')
  return `<ul class="md-list">${body}</ul>`
}

function renderOrderedListHtml(items: string[]): string {
  const body = items.map(renderListItemHtml).join('')
  return `<ol class="md-list md-list-ol">${body}</ol>`
}

type MdSegment =
  | { kind: 'table'; html: string }
  | { kind: 'list'; html: string }
  | { kind: 'rule' }
  | { kind: 'blockquote'; html: string }
  | { kind: 'text'; text: string }

/** 把一段非代码块文本按 markdown table / bullet list 切片。未命中的部分保留原换行，
 *  之后交由 inline() 处理。 */
function extractMarkdownBlocks(text: string): MdSegment[] {
  const lines = text.split('\n')
  const segs: MdSegment[] = []
  let buf: string[] = []
  const flushBuf = () => {
    if (!buf.length) return
    segs.push({ kind: 'text', text: buf.join('\n') })
    buf = []
  }
  let i = 0
  while (i < lines.length) {
    const line = lines[i]
    // 起点：当前行像数据行（含 `|`）+ 下一行是分隔行（dashes/colons）
    if (
      line.trim().includes('|') &&
      i + 1 < lines.length &&
      isTableSeparator(lines[i + 1])
    ) {
      flushBuf()
      const headerCells = splitTableRow(line)
      const alignments = getAlignments(lines[i + 1])
      const bodyRows: string[][] = []
      let j = i + 2
      while (j < lines.length && lines[j].trim() !== '' && lines[j].trim().includes('|')) {
        bodyRows.push(splitTableRow(lines[j]))
        j++
      }
      segs.push({ kind: 'table', html: renderTableHtml(headerCells, alignments, bodyRows) })
      i = j
      continue
    }
    if (HR_RE.test(line)) {
      flushBuf()
      segs.push({ kind: 'rule' })
      i++
      continue
    }
    if (line.startsWith('> ') || line === '>') {
      flushBuf()
      const qLines: string[] = [line.replace(/^>\s?/, '')]
      let j = i + 1
      while (j < lines.length && (lines[j].startsWith('> ') || lines[j] === '>')) {
        qLines.push(lines[j].replace(/^>\s?/, ''))
        j++
      }
      segs.push({ kind: 'blockquote', html: `<blockquote class="md-quote">${inline(qLines.join('\n'))}</blockquote>` })
      i = j
      continue
    }
    if (isBulletItem(line)) {
      flushBuf()
      const items: string[] = [bulletItemText(line)]
      let j = i + 1
      while (j < lines.length && isBulletItem(lines[j])) {
        items.push(bulletItemText(lines[j]))
        j++
      }
      segs.push({ kind: 'list', html: renderBulletListHtml(items) })
      i = j
      continue
    }
    if (isOrderedItem(line)) {
      flushBuf()
      const items: string[] = [orderedItemText(line)]
      let j = i + 1
      while (j < lines.length && isOrderedItem(lines[j])) {
        items.push(orderedItemText(lines[j]))
        j++
      }
      segs.push({ kind: 'list', html: renderOrderedListHtml(items) })
      i = j
      continue
    }
    // 定义列表: term + `: definition`
    if (DEF_TERM_RE.test(line) && i + 1 < lines.length && DEF_LINE_RE.test(lines[i + 1])) {
      flushBuf()
      let html = '<dl class="md-dl">'
      let j = i
      while (j < lines.length) {
        if (DEF_TERM_RE.test(lines[j]) && j + 1 < lines.length && DEF_LINE_RE.test(lines[j + 1])) {
          html += `<dt>${inline(lines[j])}</dt>`
          j++
          while (j < lines.length && DEF_LINE_RE.test(lines[j])) {
            html += `<dd>${inline(DEF_LINE_RE.exec(lines[j])![1])}</dd>`
            j++
          }
        } else if (lines[j].trim() === '') {
          j++
        } else {
          break
        }
      }
      html += '</dl>'
      segs.push({ kind: 'list', html })
      i = j
      continue
    }
    buf.push(line)
    i++
  }
  flushBuf()
  return segs
}

/** 用户粘贴图片时，各 CLI 会在用户文本里留下 `[Image #1]` 这样的占位符。既然缩略图
 *  已单独渲染在气泡上方，正文里这些占位符就是重复噪音 —— 滤掉它们并清理残留空白。
 *  仅对「带图片块的消息」调用（见 ChatView / export），避免误删正文里对图片的文字引用。 */
export function stripImagePlaceholders(raw: string): string {
  return raw
    .replace(/[ \t]*\[Image #\d+\][ \t]*/gi, ' ') // 占位符 + 紧邻空格 → 单空格
    .replace(/[ \t]+\n/g, '\n') // 行尾残留空格
    .replace(/\n[ \t]+/g, '\n') // 行首残留空格
    .replace(/\n{3,}/g, '\n\n') // 占位符独占行被删后压多余空行
    .trim()
}

// renderText 是纯函数（只依赖 raw）。虚拟滚动下同一条消息会随滚动反复挂载/卸载,每次模板
// v-html 都重跑一遍 markdown 解析 —— 用一个带上限的 LRU 缓存按 raw 记住结果,滚动重入零解析。
const renderTextCache = new Map<string, string>()
const RENDER_CACHE_MAX = 3000

/** 渲染 Markdown 子集：围栏代码块 + 行内强调 + GFM table。 */
export function renderText(raw: string): string {
  const cached = renderTextCache.get(raw)
  if (cached !== undefined) {
    // LRU：命中后挪到末尾（最近使用）。
    renderTextCache.delete(raw)
    renderTextCache.set(raw, cached)
    return cached
  }
  const out = renderTextImpl(raw)
  renderTextCache.set(raw, out)
  if (renderTextCache.size > RENDER_CACHE_MAX) {
    // 淘汰最旧一条（Map 迭代序 = 插入序）。
    const oldest = renderTextCache.keys().next().value
    if (oldest !== undefined) renderTextCache.delete(oldest)
  }
  return out
}

/** 流式预览专用：每一帧文本都是一次性增长前缀，不写 LRU，避免缓存大量中间 HTML。 */
export function renderTextUncached(raw: string): string {
  return renderTextImpl(raw, false)
}

function renderTextImpl(raw: string, cacheNested = true): string {
  const { text: pre, codes } = extractCommandTags(raw)
  // 按行扫围栏，而不是 split('```')：围栏长度由开围栏决定，闭围栏必须 ≥ 开围栏长度，
  // 更短的反引号串算作代码内容。这样 ````markdown 里嵌的 ```js 不会被误判成围栏。
  let html = ''
  const lines = pre.split('\n')
  let textBuf: string[] = []

  const flushText = () => {
    if (!textBuf.length) return
    const part = textBuf.join('\n')
    textBuf = []
    if (!part) return
    for (const seg of extractMarkdownBlocks(part)) {
      if (seg.kind === 'table') html += seg.html
      else if (seg.kind === 'list') html += seg.html
      else if (seg.kind === 'blockquote') html += seg.html
      else if (seg.kind === 'rule') html += '<hr class="md-hr">'
      else if (seg.kind === 'text') {
        // 去掉文本段首尾的空行、并把 3+ 连续空行压成最多一行 —— 标题/代码块/表格
        // 前后常跟着空行，原样转 <br> 会叠成大段空白（用户反馈的「间距过大」）。
        const trimmed = seg.text
          .replace(/^\n+/, '')
          .replace(/\n+$/, '')
          .replace(/\n{3,}/g, '\n\n')
        if (trimmed) html += `<div class="text-run">${inline(trimmed)}</div>`
      }
    }
  }

  let i = 0
  while (i < lines.length) {
    // 块级数学 $$...$$
    if (lines[i].trim() === '$$') {
      const body: string[] = []
      let j = i + 1
      let closed = false
      for (; j < lines.length; j++) {
        if (lines[j].trim() === '$$') { closed = true; break }
        body.push(lines[j])
      }
      flushText()
      const expr = body.join('\n')
      html += `<div class="md-math-block" data-math="${escapeHtmlAttr(expr)}"><pre>${escapeHtml(expr)}</pre></div>`
      i = closed ? j + 1 : j
      continue
    }
    // <details> 折叠
    if (lines[i].trim().startsWith('<details')) {
      const block: string[] = [lines[i]]
      let j = i + 1
      let closed = false
      for (; j < lines.length; j++) {
        block.push(lines[j])
        if (lines[j].trim().includes('</details>')) { closed = true; j++; break }
      }
      if (!closed) j = lines.length
      flushText()
      const raw = block.join('\n')
      const summary = /<summary>([\s\S]*?)<\/summary>/.exec(raw)?.[1]?.trim() ?? ''
      const content = raw
        .replace(/<\/?details[^>]*>/g, '')
        .replace(/<summary>[\s\S]*?<\/summary>/, '')
        .trim()
      const bodyHtml = cacheNested ? renderText(content) : renderTextImpl(content, false)
      html += `<details class="md-details"><summary>${escapeHtml(summary)}</summary><div class="md-details-body">${bodyHtml}</div></details>`
      i = j
      continue
    }
    // 开围栏：缩进 ≤3、≥3 个连续反引号、信息串里不含反引号（CommonMark）。
    const open = lines[i].match(/^( {0,3})(`{3,})(.*)$/)
    if (open && !open[3].includes('`')) {
      const fenceLen = open[2].length
      const lang = open[3].trim().toLowerCase()
      const body: string[] = []
      let j = i + 1
      let closed = false
      for (; j < lines.length; j++) {
        // 闭围栏：纯反引号行（无信息串）、缩进 ≤3、长度 ≥ 开围栏。更短的 ``` 当内容。
        const close = lines[j].match(/^ {0,3}(`{3,})[ \t]*$/)
        if (close && close[1].length >= fenceLen) {
          closed = true
          break
        }
        body.push(lines[j])
      }
      flushText()
      const src = body.join('\n')
      if (lang === 'mermaid') {
        // mermaid 块用占位符发出去，渲染管线（ChatView 那边的 hookMermaidRender）
        // 后置扫描 .md-mermaid 调 mermaid.render() 替换。原文存 data-source，主题
        // 切换时可以重新渲染。
        html += `<div class="md-mermaid" data-source="${encodeURIComponent(src)}"><pre class="md-mermaid-source">${escapeHtml(src)}</pre></div>`
      } else {
        html += `<pre class="code-block"${lang ? ` data-lang="${escapeHtml(lang)}"` : ''}><code>${escapeHtml(src)}</code></pre>`
      }
      i = closed ? j + 1 : j // 未闭合则扫到文件尾（j === lines.length）
      continue
    }
    textBuf.push(lines[i])
    i++
  }
  flushText()
  if (codes.length) {
    html = html.replace(
      /CMD(\d+)/g,
      (_m, n) => {
        const c = codes[Number(n)]
        // 命令名加 cmd-name（蓝色），并补一个真实空格 —— 去灰底后命令名会和参数贴死
        // （/configtui=default）；真实空格而非 margin，复制粘贴也保留分隔。参数只用 cmd-tag。
        if (c.isName) return `<code class="cmd-tag cmd-name">${escapeHtml(c.inner)}</code> `
        return `<code class="cmd-tag">${escapeHtml(c.inner)}</code>`
      },
    )
  }
  return html
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

/** 紧凑 token 数：≤ 999 直接写，1000-999_999 显示 `12.3K`，≥ 1M 显示 `1.2M`。
 *  整 K / 整 M 去掉尾随零（`10K` 而不是 `10.0K`），非整数永远保留 1 位小数
 *  —— 跟 codeburn 一致，否则 `240.5K out` 会被显示成 `241K`，对账时看着像 bug。 */
export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0'
  if (n < 1000) return `${Math.round(n)}`
  const unit = n < 1_000_000 ? 'K' : 'M'
  const scaled = n / (n < 1_000_000 ? 1000 : 1_000_000)
  return `${scaled.toFixed(1).replace(/\.0$/, '')}${unit}`
}

function pad(n: number): string {
  return n < 10 ? `0${n}` : `${n}`
}

/** 把毫秒时间戳或 ISO 字符串格式化为本地时间。 */
export function formatTime(input: number | string | undefined): string {
  if (input === undefined || input === '') return '—'
  const d = new Date(input)
  if (isNaN(d.getTime())) return '—'
  const now = new Date()
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate()
  // 也判断"昨天"，让相对日期更有用
  const ms = 24 * 60 * 60 * 1000
  const yesterday = new Date(now.getTime() - ms)
  const isYesterday =
    d.getFullYear() === yesterday.getFullYear() &&
    d.getMonth() === yesterday.getMonth() &&
    d.getDate() === yesterday.getDate()
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`
  if (sameDay) return `${t('time.today')} ${hm}`
  if (isYesterday) return `${t('time.yesterday')} ${hm}`
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${hm}`
}

/** 把运行秒数压成短标签：7s、16m 40s、1h 02m。 */
export function formatElapsedSeconds(input: number): string {
  if (!Number.isFinite(input) || input <= 0) return '0s'
  const total = Math.floor(input)
  const seconds = total % 60
  const minutesTotal = Math.floor(total / 60)
  if (minutesTotal === 0) return `${seconds}s`
  const minutes = minutesTotal % 60
  const hours = Math.floor(minutesTotal / 60)
  if (hours === 0) return `${minutes}m ${seconds}s`
  return `${hours}h ${pad(minutes)}m`
}

/** 从完整路径取最后一段，作为项目短名。 */
export function shortName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean)
  return parts.length ? parts[parts.length - 1] : path
}

/** 关键词高亮用的文本片段：hit 为 true 的片段是命中段。 */
export interface HlSegment {
  text: string
  hit: boolean
}

/** 把 text 按 query（大小写不敏感）的出现位置切成片段，hit 标记命中段。
 *  query 为空 / text 为空 / 无匹配时返回单段未命中。用 indexOf 而非正则，
 *  天然免疫 query 里的正则特殊字符。供会话列表的关键词高亮使用。 */
export function highlightSegments(text: string, query: string): HlSegment[] {
  const q = query.trim().toLowerCase()
  if (!q || !text) return [{ text, hit: false }]
  const lower = text.toLowerCase()
  const segs: HlSegment[] = []
  let i = 0
  let at = lower.indexOf(q)
  while (at !== -1) {
    if (at > i) segs.push({ text: text.slice(i, at), hit: false })
    segs.push({ text: text.slice(at, at + q.length), hit: true })
    i = at + q.length
    at = lower.indexOf(q, i)
  }
  if (i < text.length) segs.push({ text: text.slice(i), hit: false })
  return segs
}
