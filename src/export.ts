// 把 `Msg[]` 序列化成 Markdown / HTML，弹出原生 Save As 对话框让用户选位置，
// 然后通过 Tauri 命令把字节落到选中的路径。
//
// 不要用 Blob + <a download> —— Tauri 的 WKWebView（macOS）不识别 download
// 属性，blob URL 直接被吞（dev mode 浏览器里看上去正常，原生包里完全没反应）。
// 走 dialog.save() + write_file 是稳的路径，同时让用户能选目录/改文件名。

import type { Msg, Block, SessionMeta, Agent, DiffHunk } from './types'
import { writeFile } from './api'
import { save as saveDialog, open as openDialog } from '@tauri-apps/plugin-dialog'
import { t } from './i18n'
import { formatTime, isCaveatOnlyMsg, parseSystemEvent, renderText, cleanMetaText, metaKindIsPre, parseMetaFields, parseTeammateMessage, stripImagePlaceholders } from './format'
import {
  highlightJsonInPlace,
  looksLikeJson,
  prettifyAndHighlightJson,
} from './jsonHighlight'
import { highlightDiff, looksLikeDiff } from './diffHighlight'
import { shouldAttachToolResult, shouldPreferToolResult } from './toolResultRouting'
import { renderCodexPluginMentionHtmlText } from './codexPluginMentions'
import { agentLabel } from './agentMeta'

function sanitizeFilename(name: string): string {
  const cleaned = name.replace(/[\\/:*?"<>|\n\r\t]/g, '_').trim()
  return (cleaned.slice(0, 80) || 'session').replace(/\s+/g, ' ')
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

// 头像 SVG（与会话详情用的图标字典对齐：claude/codex 取自 iconify 在 src/components/icons.ts
// 的同名导入；user/tool 用 lucide 标准路径）。导出 HTML 是离线静态文件，
// 不能依赖 Vue runtime，所以这里直接内联 SVG 字符串。
const AVATAR_SVG = {
  claude: '<svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><g fill="#ff7043"><path d="m14.375 6.48l.49.28v.209l-.14.489l-5.937 1.397l-.558-1.387zm0 0"/><path d="m12.155 2.373l.683.143l.182.224l.173.535l-.072.342l-3.983 5.447L7.81 7.737l3.673-4.82z"/><path d="m8.719 1.522l.419-.28l.349.14l.349.49l-.957 5.748l-.65-.441l-.279-.769l.49-4.33z"/><path d="m4.239 1.614l.43-.55L4.95 1l.558.081l.275.216l2.004 4.442l.724 2.11l-.848.471l-3.231-5.864z"/><path d="m2.154 4.665l-.14-.56l.42-.488l.488.07h.14l2.933 2.165l.908.698l1.257.978l-.698 1.187l-.629-.489l-.419-.419l-4.05-2.863z"/><path d="M1.316 8.296L1 7.946v-.31l.316-.108l3.562.21l3.491.279l-.113.695l-6.66-.346z"/><path d="M3.411 11.931h-.698l-.278-.32v-.382l1.186-.838l4.82-3.068l.487.833z"/><path d="m4.738 13.883l-.28.07l-.418-.21l.07-.35l4.12-5.446l.558.768l-3.072 4.05z"/><path d="m8.23 14.581l-.21.28l-.419.14l-.349-.28l-.21-.42L8.09 8.646l.629.07z"/><path d="M11.791 13.045v.558l-.07.21l-.279.14l-.489-.066l-3.356-4.996l1.331-1.014l1.117 2.025l.105.733z"/><path d="m13.398 12.207l.07.349l-.21.279l-.21-.07l-1.187-.838l-1.815-1.606l-1.397-.978l.419-1.326l.698.419l.42.768z"/><path d="m12.49 8.645l1.746.14l.419.28l.279.418v.302l-.768.327l-3.911-.978l-1.606-.07l.419-1.466l1.117.838z"/></g></svg>',
  codex: '<svg viewBox="0 0 48 48" width="18" height="18" aria-hidden="true"><g fill="none" stroke="currentColor" stroke-width="3" stroke-linejoin="round"><path d="M18.38 27.94v-14.4l11.19-6.46c6.2-3.58 17.3 5.25 12.64 13.33"/><path d="m18.38 20.94l12.47-7.2l11.19 6.46c6.2 3.58 4.1 17.61-5.23 17.61"/><path d="m24.44 17.44l12.47 7.2v12.93c0 7.16-13.2 12.36-17.86 4.28"/><path d="M30.5 21.2v14.14L19.31 41.8c-6.2 3.58-17.3-5.25-12.64-13.33"/><path d="m30.5 27.94l-12.47 7.2l-11.19-6.46c-6.21-3.59-4.11-17.61 5.22-17.61"/><path d="m24.44 31.44l-12.47-7.2V11.31c0-7.16 13.2-12.36 17.86-4.28"/></g></svg>',
  grok: '<svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 5l14 14"/><path d="M19 5L5 19"/><circle cx="12" cy="12" r="9"/></svg>',
  user: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="8" r="5"/><path d="M20 21a8 8 0 0 0-16 0"/></svg>',
  tool: '<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>',
  arrowUp: '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 19V5"/><path d="m5 12 7-7 7 7"/></svg>',
  arrowDown: '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 5v14"/><path d="m19 12-7 7-7-7"/></svg>',
} as const

function avatarSvg(role: string, agent: Agent): string {
  if (role === 'tool') return AVATAR_SVG.tool
  if (role === 'user') return AVATAR_SVG.user
  if (agent === 'codex') return AVATAR_SVG.codex
  if (agent === 'grok') return AVATAR_SVG.grok
  if (agent === 'agy') return AVATAR_SVG.claude // agy 暂复用 claude 的 SVG 头像
  if (agent === 'opencode') return AVATAR_SVG.claude // opencode 同样复用
  return AVATAR_SVG.claude
}

function roleLabel(role: string, agent: Agent): string {
  if (role === 'tool') return t('chat.role.tool')
  if (role === 'user') return t('chat.role.me')
  return agentLabel(agent)
}

// 在 Claude 的 JSONL 中，tool_result 块被装在 role:"user" 的消息里
// （表示用户"把"工具输出回传给模型）。视觉上这其实是 agent 这边的产物，
// 跟 ChatView.isToolOnly 一致：整条消息都是 tool_result 时不算作用户输入。
function isToolOnly(m: Msg): boolean {
  return m.role === 'user' && m.blocks.length > 0 && m.blocks.every((b) => b.kind === 'tool_result')
}

// 把 system event（目前只有 /rename）翻成本地化的句子；非 system event 返回 null。
function systemEventText(m: Msg): string | null {
  const ev = parseSystemEvent(m)
  if (!ev) return null
  if (ev.kind === 'rename') return t('chat.systemEvent.rename', { name: ev.name })
  if (ev.kind === 'interrupt') return t('chat.systemEvent.interrupted')
  return null
}

// 系统注入的 user 记录（metaKind）的本地化标题 —— 与 ChatView 一致，导出时也不
// 把它们标成「Me」。
const META_KIND_KEY: Record<string, string> = {
  compact: 'chat.metaKind.compact',
  meta: 'chat.metaKind.meta',
  'task-notification': 'chat.metaKind.taskNotification',
  system: 'chat.metaKind.system',
  'command-output': 'chat.metaKind.commandOutput',
  'teammate-message': 'chat.metaKind.teammateMessage',
}
// metaKind 正文 → key/value 字段：通用 <tag>value</tag>（任务通知）优先，
// 再试 teammate-message（多 agent 消息）；纯文本返回 null。与 ChatView.metaFieldsOf 一致。
function metaFields(text: string) {
  return parseMetaFields(text) ?? parseTeammateMessage(text)
}
function metaKindLabelText(kind: string): string {
  return t(META_KIND_KEY[kind] ?? 'chat.metaKind.system')
}

// 与 ChatView.stats 同步：u = 真正的用户消息条数（排除 tool-only / caveat-only /
// system-event），a = 助手消息条数。
function computeStats(messages: Msg[]): { u: number; a: number } {
  let u = 0
  let a = 0
  for (const m of messages) {
    if (
      m.role === 'user' &&
      !m.metaKind &&
      !isToolOnly(m) &&
      !isCaveatOnlyMsg(m) &&
      !systemEventText(m)
    ) {
      u++
    } else if (m.role === 'assistant') a++
  }
  return { u, a }
}

// 把 toolId 对应的 tool_result 找出来；普通工具输出内联到 tool_use 里展示，
// 文件改动结果则保留为独立块，和 ChatView 的 live GUI 渲染一致。
function buildInlinedResults(messages: Msg[]): {
  resultByToolId: Map<string, Block>
  inlinedIds: Set<string>
} {
  const resultByToolId = new Map<string, Block>()
  for (const m of messages) {
    for (const b of m.blocks) {
      if (b.kind === 'tool_result' && b.toolId && shouldPreferToolResult(b, resultByToolId.get(b.toolId))) {
        resultByToolId.set(b.toolId, b)
      }
    }
  }
  const inlinedIds = new Set<string>()
  for (const m of messages) {
    for (const b of m.blocks) {
      if (
        b.kind === 'tool_use' &&
        b.toolId &&
        resultByToolId.has(b.toolId) &&
        !shouldAttachToolResult(b, resultByToolId.get(b.toolId))
      ) {
        inlinedIds.add(b.toolId)
      }
    }
  }
  return { resultByToolId, inlinedIds }
}

function diffToText(hunks: DiffHunk[]): string {
  const lines: string[] = []
  for (const h of hunks) {
    lines.push(`@@ -${h.oldStart},_ +${h.newStart},_ @@`)
    for (const l of h.lines) {
      const prefix = l.kind === 'add' ? '+' : l.kind === 'del' ? '-' : ' '
      lines.push(`${prefix}${l.text}`)
    }
  }
  return lines.join('\n')
}

// ============================ Markdown ============================

function toolResultMd(b: Block): string {
  const head = b.filePath
    ? `> 📄 **${t('tool.resultDiff', { file: b.filePath })}**`
    : b.isError
      ? `> ⚠️ **${t('tool.resultError')}**`
      : `> 📤 **${t('tool.result')}**`
  if (b.diff && b.diff.length) {
    return [head, '', '```diff', diffToText(b.diff), '```'].join('\n')
  }
  const txt = (b.text ?? '').trim()
  if (!txt) return head
  return [head, '', '```', txt, '```'].join('\n')
}

function blockToMd(b: Block, ctx: { resultByToolId: Map<string, Block>; inlinedIds: Set<string> }): string {
  switch (b.kind) {
    case 'text':
      return (b.text ?? '').trim()
    case 'thinking':
      return [
        '<details>',
        `<summary>🧠 ${t('tool.thinking')}</summary>`,
        '',
        (b.text ?? '').trim(),
        '',
        '</details>',
      ].join('\n')
    case 'tool_use': {
      const head = `> 🔧 **${t('tool.call', { name: b.toolName ?? '' })}**`
      const args = (b.toolInput ?? '').trim()
      const lines = [head]
      if (args) lines.push('', '```json', args, '```')
      // 把对应的非 file-mutating result 内联在 tool_use 下方
      if (b.toolId && ctx.inlinedIds.has(b.toolId)) {
        const r = ctx.resultByToolId.get(b.toolId)
        if (r) lines.push('', toolResultMd(r))
      }
      return lines.join('\n')
    }
    case 'tool_result': {
      // 被 tool_use 吸收的不再单独输出
      if (b.toolId && ctx.inlinedIds.has(b.toolId)) return ''
      return toolResultMd(b)
    }
    case 'image':
      return b.imageSrc ? `![image](${b.imageSrc})` : ''
    case 'file':
      return b.filePath ? `📎 [${b.filePath.split(/[/\\]/).pop() || b.filePath}](${b.filePath})` : ''
    default:
      return ''
  }
}

function msgToMd(
  m: Msg,
  agent: Agent,
  ctx: { resultByToolId: Map<string, Block>; inlinedIds: Set<string> },
): string {
  // System event (e.g. /rename) — emit as a horizontal-rule-bracketed italic line.
  const sysText = systemEventText(m)
  if (sysText) {
    const ts = m.timestamp ? ` · ${formatTime(m.timestamp)}` : ''
    return `_${sysText}${ts}_`
  }
  // System-injected user records (compaction summary, skill, command output, …):
  // labeled by kind, never "Me". Notification-style pseudo-XML → key/value list;
  // other pre kinds → code fence; markdown kinds → raw markdown.
  if (m.metaKind) {
    const ts = m.timestamp ? ` · ${formatTime(m.timestamp)}` : ''
    const head = `## ${roleLabel('assistant', agent)} · ${metaKindLabelText(m.metaKind)}${ts}`
    const pre = metaKindIsPre(m.metaKind)
    const body = m.blocks
      .filter((b) => b.kind === 'text')
      .map((b) => {
        const fields = metaFields(b.text ?? '')
        if (fields) return fields.map((f) => `- **${f.key}**: ${f.value}`).join('\n')
        return pre ? '```\n' + cleanMetaText(b.text ?? '') + '\n```' : (b.text ?? '').trim()
      })
      .filter(Boolean)
      .join('\n\n')
    return body ? `${head}\n\n${body}` : head
  }
  const ts = m.timestamp ? ` · ${formatTime(m.timestamp)}` : ''
  const model = m.model ? ` · ${m.model}` : ''
  const displayRole = isToolOnly(m) ? 'tool' : m.role
  const head = `## ${roleLabel(displayRole, agent)}${model}${ts}`
  // 带图消息：正文滤掉 [Image #n] 占位符（图片本身已用 ![image](src) 表达）。
  const hasImgs = m.blocks.some((b) => b.kind === 'image' && b.imageSrc)
  const body = m.blocks
    .map((b) =>
      b.kind === 'text' && hasImgs
        ? blockToMd({ ...b, text: stripImagePlaceholders(b.text ?? '') }, ctx)
        : blockToMd(b, ctx),
    )
    .filter(Boolean)
    .join('\n\n')
  return body ? `${head}\n\n${body}` : head
}

export function messagesToMarkdown(
  session: SessionMeta,
  messages: Msg[],
  agent: Agent,
): string {
  const ctx = buildInlinedResults(messages)
  const { u, a } = computeStats(messages)
  const statsLine = t('chat.stats', {
    u,
    a,
    time: session.created ? formatTime(session.created) : '—',
  })
  const meta = [
    `# ${session.title}`,
    '',
    `- ${statsLine}`,
    `- ${t('export.meta.agent')}: \`${agent}\``,
    session.cwd ? `- ${t('export.meta.cwd')}: \`${session.cwd}\`` : '',
    session.id ? `- ${t('export.meta.id')}: \`${session.id}\`` : '',
    '',
    '---',
  ]
    .filter(Boolean)
    .join('\n')
  // 过滤：1) 整条都是被内联 tool_result 的行（避免空 "## Tool"）
  //       2) Claude Code 的 local-command-caveat 噪音
  const visible = messages.filter((m) => {
    if (isCaveatOnlyMsg(m)) return false
    const blocks = m.blocks.map((b) => blockToMd(b, ctx)).filter(Boolean)
    return blocks.length > 0 || !isToolOnly(m)
  })
  const body = visible.map((m) => msgToMd(m, agent, ctx)).join('\n\n')
  return `${meta}\n\n${body}\n`
}

// ============================ HTML ============================

// Geist-style tokens. The light/dark palettes mirror src/style.css so exported
// transcripts look like the app. `data-theme="dark"` on <html> picks dark; the
// in-page toggle button flips that attribute (and persists to localStorage for
// the standalone file).
const HTML_STYLE = `
:root {
  color-scheme: light dark;
  --bg: hsl(0 0% 100%);
  --surface: hsl(0 0% 100%);
  --surface-2: hsl(0 0% 98%);
  --surface-hover: hsl(0 0% 95%);
  --border: hsl(0 0% 92%);
  --border-strong: hsl(0 0% 79%);
  --text: hsl(0 0% 9%);
  --text-dim: hsl(0 0% 30%);
  --text-mute: hsl(0 0% 56%);
  --user-bg: hsl(0 0% 96%);
  --code-bg: hsl(0 0% 96%);
  --diff-add: rgba(22, 163, 74, 0.14);
  --diff-del: rgba(220, 38, 38, 0.14);
  --link: hsl(212 100% 48%);
}
:root[data-theme="dark"] {
  --bg: hsl(0 0% 4%);
  --surface: hsl(0 0% 4%);
  --surface-2: hsl(0 0% 0%);
  --surface-hover: hsl(0 0% 10%);
  --border: hsl(0 0% 16%);
  --border-strong: hsl(0 0% 27%);
  --text: hsl(0 0% 93%);
  --text-dim: hsl(0 0% 63%);
  --text-mute: hsl(0 0% 53%);
  --user-bg: hsl(0 0% 8%);
  --code-bg: hsl(0 0% 10%);
  --diff-add: rgba(22, 163, 74, 0.22);
  --diff-del: rgba(220, 38, 38, 0.22);
  --link: hsl(210 100% 66%);
}
* { box-sizing: border-box; }
body {
  font: 14px/1.6 'Inter', -apple-system, BlinkMacSystemFont, 'SF Pro Text', 'PingFang SC', 'Helvetica Neue', Arial, sans-serif;
  max-width: 1200px; margin: 0 auto; padding: 0 24px 80px;
  color: var(--text); background: var(--bg);
  font-feature-settings: 'cv11', 'ss01';
}
a { color: var(--link); text-decoration: none; }
a:hover { text-decoration: underline; }
/* Sticky title + meta strip. We keep it inside the 1200px max-width column
   so it lines up with the body; background must be opaque to mask scrolling
   content underneath. The thin bottom border doubles as the meta divider. */
.sticky-head {
  position: sticky; top: 0; z-index: 20;
  background: var(--bg);
  border-bottom: 1px solid var(--border);
  margin: 0 -24px 24px; padding: 24px 24px 16px;
}
.header {
  display: flex; align-items: center;
  gap: 8px; margin: 0 0 12px;
}
h1 { font-size: 22px; font-weight: 600; margin: 0; letter-spacing: -0.01em; flex: 1; min-width: 0; }
.theme-toggle {
  appearance: none; background: var(--surface); color: var(--text-dim);
  border: 1px solid var(--border); border-radius: 8px;
  padding: 6px 12px; font: inherit; font-size: 12px; cursor: pointer;
  display: inline-flex; align-items: center; gap: 6px;
  transition: background .15s, color .15s, border-color .15s;
}
.theme-toggle:hover { background: var(--surface-hover); color: var(--text); border-color: var(--border-strong); }
.meta { color: var(--text-mute); font-size: 12px; }
.meta code { background: transparent; padding: 0; color: var(--text-dim); }

/* WeChat-style chat layout: user on the right, assistant on the left.
   Avatar + bubble side-by-side; bubble has an asymmetric corner pointing
   toward the avatar to mimic the speech-bubble tail. */
.msg {
  display: flex; align-items: flex-start; gap: 10px;
  margin: 18px 0;
}
.msg.user { flex-direction: row-reverse; }
.avatar {
  flex: 0 0 32px; width: 32px; height: 32px;
  border-radius: 50%; background: var(--surface-2);
  border: 1px solid var(--border);
  display: inline-flex; align-items: center; justify-content: center;
  color: var(--text-dim);
  user-select: none;
}
.avatar svg { display: block; }
.msg.user .avatar { color: var(--text); }
.bubble {
  max-width: min(75%, 880px);
  padding: 12px 16px;
  border: 1px solid var(--border);
  border-radius: 14px;
  background: var(--surface);
}
.msg.user .bubble {
  background: var(--user-bg);
  border-top-right-radius: 4px;
}
.msg.assistant .bubble {
  border-top-left-radius: 4px;
}
.msg.tool .bubble {
  background: var(--surface-2);
  border-top-left-radius: 4px;
}
.msg.tool .avatar {
  background: var(--surface-2); color: var(--text-mute);
}
.tool-result-inline {
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px dashed var(--border);
}
/* System events (e.g. /rename) render as a small centered meta line. */
.msg.system { justify-content: center; margin: 14px 0; }
.system-event {
  color: var(--text-mute);
  font-size: 12px;
  text-align: center;
  padding: 2px 12px;
}
/* System-injected records (compaction summary, skill, command output,
   task-notification, teammate-message): agent prefix + a collapsed labeled
   card, clearly not a "Me" bubble. */
.msg.meta { justify-content: flex-start; }
.msg.meta .bubble.meta-msg { background: transparent; padding: 0; border: 0; }
/* Collapsed tool-call-style card; summary is the uppercase kind label. Reuses
   the shared details chrome (border / chevron / surface-2 bg). */
.meta-details { margin: 8px 0 0; }
.meta-details > summary {
  text-transform: uppercase;
  letter-spacing: 0.05em;
  font-weight: 600;
  font-size: 11px;
  color: var(--text-mute);
}
.meta-pre {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: ui-monospace, 'SF Mono', Menlo, monospace;
  font-size: 12px;
  line-height: 1.6;
}
.meta-fields {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: 3px 14px;
  margin: 0;
  align-items: baseline;
}
.meta-field-key {
  margin: 0;
  color: var(--text-mute);
  font-size: 11px;
  font-family: ui-monospace, 'SF Mono', Menlo, monospace;
  white-space: nowrap;
}
.meta-field-val {
  margin: 0;
  word-break: break-word;
  white-space: pre-wrap;
  line-height: 1.55;
}
.role-tag {
  font-size: 11px; color: var(--text-mute);
  text-transform: uppercase; letter-spacing: 0.08em;
  margin-bottom: 8px; font-weight: 500;
}
.msg.user .role-tag { text-align: right; }
.text { white-space: pre-wrap; word-break: break-word; }
.text-run { white-space: pre-wrap; word-break: break-word; }
.text-run h3 { font-size: 15px; font-weight: 600; margin: 14px 0 6px; }
.text-run h4 { font-size: 13.5px; font-weight: 600; margin: 10px 0 4px; }
/* renderText emit 的 fenced code 块。沿用上面 pre / code 的样式，class 留作钩子。 */
.code-block { display: block; }
/* GFM 表格 —— 行容器 .md-table-wrap 提供横向滚动；表格本身用 design tokens 上色。 */
.md-table-wrap {
  max-width: 100%; overflow-x: auto; margin: 10px 0;
  border: 1px solid var(--border); border-radius: 8px;
  -webkit-overflow-scrolling: touch;
  /* 浅色模式下默认 native scrollbar 几乎是黑色，把 thumb 改成跟正文同色 22% 透明 */
  scrollbar-width: thin;
  scrollbar-color: color-mix(in srgb, var(--text) 22%, transparent) transparent;
}
.md-table-wrap::-webkit-scrollbar { height: 7px; width: 7px; }
.md-table-wrap::-webkit-scrollbar-track { background: transparent; }
.md-table-wrap::-webkit-scrollbar-thumb {
  background: color-mix(in srgb, var(--text) 22%, transparent);
  border-radius: 999px;
}
.md-table-wrap::-webkit-scrollbar-thumb:hover {
  background: color-mix(in srgb, var(--text) 38%, transparent);
}
.md-table {
  width: max-content; min-width: 100%;
  border-collapse: separate; border-spacing: 0;
  font-size: 13px; line-height: 1.5; background: var(--surface);
}
.md-table thead { background: var(--surface-2); }
.md-table th, .md-table td {
  padding: 7px 12px; text-align: left; vertical-align: top;
  border-bottom: 1px solid var(--border);
}
.md-table th { font-weight: 600; font-size: 12px; }
.md-table tbody tr:last-child td { border-bottom: 0; }
.md-table tbody tr:hover td { background: var(--surface-hover); }
.md-table code { font-size: 12px; }
/* Mermaid 流程图 —— 导出时已经 prerender 成 SVG 烤进 HTML，离线可看。
 * 主题以导出时刻的 app 主题为准（mermaid SVG 颜色烤死），切换 HTML 主题时
 * 其它元素跟着变，mermaid 图保持不变。 */
.md-mermaid {
  display: block; margin: 10px 0; padding: 12px;
  border: 1px solid var(--border); border-radius: 8px;
  background: var(--surface); overflow-x: auto; text-align: center;
}
.md-mermaid svg { max-width: 100%; height: auto; }
.md-mermaid-source {
  margin: 0; padding: 10px 12px; background: var(--code-bg);
  border-radius: 6px; font-size: 12px; white-space: pre; overflow-x: auto;
  text-align: left;
  font-family: 'SF Mono', 'JetBrains Mono', Menlo, Consolas, monospace;
}
.md-mermaid-error { border-color: hsl(0 70% 60% / 0.5); }
.md-mermaid-errmsg {
  font-size: 12px; color: hsl(0 70% 50%);
  margin-bottom: 8px; text-align: left;
  font-family: 'SF Mono', 'JetBrains Mono', Menlo, Consolas, monospace;
}
.cmd-tag { background: var(--surface-hover); }
.bubble .cmd-name {
  color: #2563eb;
}
:root[data-theme="dark"] .bubble .cmd-name {
  color: #60a5fa;
}
.bubble .codex-plugin-ref {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  max-width: 100%;
  vertical-align: -3px;
  color: var(--text);
  background: var(--surface);
  border: 1px solid color-mix(in srgb, var(--border) 72%, transparent);
  border-radius: 8px;
  padding: 4px 9px;
  font-weight: 500;
  white-space: nowrap;
}
.bubble .codex-plugin-ref-ic {
  flex: none;
  width: 16px;
  height: 16px;
  color: var(--text-dim);
}
.bubble .codex-plugin-ref-ic svg {
  display: block;
  width: 16px;
  height: 16px;
  stroke: currentColor;
  stroke-width: 1.7;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.bubble .codex-plugin-ref-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  color: #2563eb;
}
:root[data-theme="dark"] .bubble .codex-plugin-ref-name {
  color: #60a5fa;
}
/* JSON syntax highlight：tool_use args 与 JSON 形态的 tool_result。 */
.lang-json .json-key { color: hsl(214 65% 42%); }
.lang-json .json-string { color: hsl(140 50% 32%); }
.lang-json .json-num { color: hsl(280 55% 45%); }
.lang-json .json-bool { color: hsl(14 75% 45%); font-weight: 500; }
.lang-json .json-null { color: var(--text-mute); font-style: italic; }
:root[data-theme="dark"] .lang-json .json-key { color: hsl(214 80% 70%); }
:root[data-theme="dark"] .lang-json .json-string { color: hsl(140 50% 65%); }
:root[data-theme="dark"] .lang-json .json-num { color: hsl(280 70% 75%); }
:root[data-theme="dark"] .lang-json .json-bool { color: hsl(14 75% 65%); }
/* Unified diff syntax highlight：Bash 跑 git diff / 工具吐 patch 等文本形态 diff。
   颜色复用 DiffBlock 的 add/del 语义，不画底色（避免和外层 pre 背景打架）。 */
.lang-diff { display: block; }
.lang-diff .diff-file { color: var(--text); font-weight: 600; }
.lang-diff .diff-meta { color: var(--text-mute); }
.lang-diff .diff-hunk { color: hsl(214 50% 45%); font-weight: 500; }
.lang-diff .diff-add { color: hsl(140 55% 32%); background: color-mix(in srgb, hsl(140 55% 45%) 12%, transparent); display: block; }
.lang-diff .diff-del { color: hsl(0 65% 42%); background: color-mix(in srgb, hsl(0 65% 50%) 12%, transparent); display: block; }
.lang-diff .diff-ctx { color: var(--text); }
:root[data-theme="dark"] .lang-diff .diff-hunk { color: hsl(214 70% 70%); }
:root[data-theme="dark"] .lang-diff .diff-add { color: hsl(140 55% 70%); background: color-mix(in srgb, hsl(140 55% 50%) 18%, transparent); }
:root[data-theme="dark"] .lang-diff .diff-del { color: hsl(0 70% 72%); background: color-mix(in srgb, hsl(0 70% 55%) 18%, transparent); }
pre {
  background: var(--code-bg); padding: 12px 14px; border-radius: 8px;
  border: 1px solid var(--border);
  overflow-x: auto;
  font: 12.5px/1.55 'SF Mono', 'JetBrains Mono', Menlo, Consolas, monospace;
  white-space: pre-wrap; word-break: break-word;
  color: var(--text);
}
code {
  background: var(--code-bg); padding: 1px 6px; border-radius: 4px;
  font: 0.92em 'SF Mono', 'JetBrains Mono', Menlo, Consolas, monospace;
  border: 1px solid var(--border);
}
pre code { background: transparent; padding: 0; border: 0; }
details {
  margin: 10px 0; border: 1px solid var(--border); border-radius: 8px;
  padding: 8px 12px; background: var(--surface-2);
}
details > summary {
  cursor: pointer; font-size: 12px; color: var(--text-dim);
  list-style: none; user-select: none;
}
details > summary::-webkit-details-marker { display: none; }
details > summary::before {
  content: '›'; display: inline-block; margin-right: 6px;
  transition: transform .15s; color: var(--text-mute);
}
details[open] > summary::before { transform: rotate(90deg); }
details[open] > summary { margin-bottom: 10px; }
img { max-width: 100%; border-radius: 6px; border: 1px solid var(--border); }
/* 消息内容列：缩略图行 + 气泡竖排（缩略图浮在气泡上方，不进气泡）。 */
.msg-content { display: flex; flex-direction: column; min-width: 0; max-width: min(75%, 880px); }
.msg.user .msg-content { align-items: flex-end; }
.msg.assistant .msg-content, .msg.tool .msg-content { align-items: flex-start; }
.msg-content > .bubble { max-width: 100%; }
/* 图片成排小缩略图（自适应比例），浮在气泡上方。 */
.msg-images { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 8px; margin-bottom: 6px; }
.msg.user .msg-images { justify-content: flex-end; }
/* 图片可点击放大 —— 见 blockToHtml case 'image' 的 onclick + lightbox runtime。 */
img.msg-image {
  cursor: zoom-in;
  width: auto; height: auto;
  max-width: 200px; max-height: 160px;
  border-radius: 10px; object-fit: contain;
}
img.msg-image:hover { border-color: var(--border-strong); }
/* Lightbox：fixed 覆盖层，不开就 display:none；img 居中且按视口尺寸限缩。 */
.csv-lightbox {
  position: fixed; inset: 0;
  display: none;
  align-items: center; justify-content: center;
  background: rgba(0, 0, 0, 0.78);
  z-index: 9999;
  cursor: zoom-out;
  padding: 32px;
}
.csv-lightbox.open { display: flex; }
.csv-lightbox img {
  max-width: 100%; max-height: 100%;
  border-radius: 6px; border: none;
  box-shadow: 0 16px 48px rgba(0, 0, 0, 0.45);
  cursor: default;
}
/* 多图时左右翻看的箭头 + 计数；单图时 JS 隐藏。 */
.csv-lb-nav {
  position: absolute; top: 50%; transform: translateY(-50%);
  width: 44px; height: 66px;
  display: flex; align-items: center; justify-content: center;
  border: none; border-radius: 8px;
  background: rgba(0, 0, 0, 0.4); color: #fff;
  font-size: 30px; line-height: 1; cursor: pointer;
  -webkit-user-select: none; user-select: none;
}
.csv-lb-nav:hover { background: rgba(0, 0, 0, 0.66); }
.csv-lb-prev { left: 20px; }
.csv-lb-next { right: 20px; }
.csv-lb-count {
  position: absolute; bottom: 24px; left: 50%; transform: translateX(-50%);
  color: rgba(255, 255, 255, 0.85); font-size: 13px;
  background: rgba(0, 0, 0, 0.4); padding: 4px 10px; border-radius: 99px;
}
.diff {
  background: var(--surface-2); border: 1px solid var(--border);
  border-radius: 8px; padding: 10px 12px;
  font: 12px/1.55 'SF Mono', Menlo, Consolas, monospace; overflow-x: auto;
}
.diff .add { background: var(--diff-add); display: block; }
.diff .del { background: var(--diff-del); display: block; }
.diff .ctx { display: block; color: var(--text-mute); }

/* Long blocks keep a stable max height and scroll vertically. */
.collapsible-box {
  position: relative; max-height: 320px; overflow-y: auto; overflow-x: hidden;
  overscroll-behavior: contain; scrollbar-width: none; -ms-overflow-style: none;
}
.collapsible-box::-webkit-scrollbar { width: 0; height: 0; display: none; }

/* Scroll-to-top / scroll-to-bottom floating buttons (mirrors ChatView FABs).
   Hidden when at the corresponding edge with an 8px tolerance. */
.fabs {
  position: fixed; right: 24px; bottom: 24px; z-index: 30;
  display: flex; flex-direction: column; gap: 10px;
  pointer-events: none;
}
.fab {
  pointer-events: auto;
  width: 36px; height: 36px; border-radius: 50%;
  background: var(--surface); color: var(--text-dim);
  border: 1px solid var(--border);
  display: inline-flex; align-items: center; justify-content: center;
  cursor: pointer; padding: 0;
  box-shadow: 0 1px 3px rgba(0,0,0,0.08);
  transition: opacity .18s, transform .18s, background .12s, color .12s, border-color .12s;
}
.fab:hover { background: var(--surface-hover); color: var(--text); border-color: var(--border-strong); }
.fab[data-hidden="1"] { opacity: 0; pointer-events: none; transform: translateY(8px); }

/* ---- Message hide / context menu ---- */
.msg[data-hidden] { opacity: 0.35; }
.msg[data-hidden]:not([data-show-hidden]) { display: none !important; }
.csv-ctx-menu {
  position: fixed; z-index: 80; min-width: 176px; padding: 4px;
  background: var(--surface); border: 1px solid var(--border);
  border-radius: 10px; box-shadow: 0 8px 24px rgba(0,0,0,0.12);
  display: none; flex-direction: column; gap: 1px; user-select: none;
}
.csv-ctx-menu.open { display: flex; }
.csv-ctx-item {
  display: flex; align-items: center; gap: 9px;
  padding: 7px 10px; border-radius: 6px;
  font-size: 12.5px; color: var(--text); text-align: left;
  background: none; border: 0; cursor: pointer;
  transition: background 0.1s;
}
.csv-ctx-item:hover { background: var(--surface-hover); }
.csv-ctx-item svg { width: 13px; height: 13px; color: var(--text-mute); flex-shrink: 0; }
.hide-toggle {
  appearance: none; background: var(--surface); color: var(--text-dim);
  border: 1px solid var(--border); border-radius: 8px;
  padding: 6px 12px; font: inherit; font-size: 12px; cursor: pointer;
  display: none; align-items: center; gap: 6px;
  transition: background .15s, color .15s, border-color .15s;
}
.hide-toggle[data-count]:not([data-count="0"]) { display: inline-flex; }
.hide-toggle:hover { background: var(--surface-hover); color: var(--text); border-color: var(--border-strong); }
.hide-toggle.active { color: var(--text); border-color: var(--border-strong); }

/* ---- Jump-to-prompt locate menu ---- */
.locate-wrap { position: relative; display: inline-flex; }
.locate-btn {
  appearance: none; background: var(--surface); color: var(--text-dim);
  border: 1px solid var(--border); border-radius: 8px;
  padding: 6px 12px; font: inherit; font-size: 12px; cursor: pointer;
  display: inline-flex; align-items: center; gap: 6px;
  transition: background .15s, color .15s, border-color .15s;
}
.locate-btn:hover { background: var(--surface-hover); color: var(--text); border-color: var(--border-strong); }
.locate-btn.active { color: var(--text); border-color: var(--border-strong); }
.locate-btn svg { width: 14px; height: 14px; }
.locate-panel {
  position: absolute; top: calc(100% + 6px); right: 0; z-index: 50;
  width: 360px; max-height: 420px;
  background: var(--surface); border: 1px solid var(--border);
  border-radius: 8px; box-shadow: 0 4px 14px rgba(0,0,0,0.08);
  display: none; flex-direction: column; overflow: hidden;
}
.locate-panel.open { display: flex; }
.locate-panel-search {
  padding: 8px; border-bottom: 1px solid var(--border); flex-shrink: 0;
}
.locate-panel-input {
  width: 100%; appearance: none; border: 1px solid var(--border);
  border-radius: 6px; padding: 5px 8px; font: inherit; font-size: 13px;
  color: var(--text); background: var(--surface-2); outline: none;
}
.locate-panel-input:focus { border-color: var(--border-strong); }
.locate-panel-list {
  overflow-y: auto; padding: 4px;
  scrollbar-width: thin;
  scrollbar-color: color-mix(in srgb, var(--text) 22%, transparent) transparent;
}
.locate-panel-list::-webkit-scrollbar { width: 7px; height: 7px; }
.locate-panel-list::-webkit-scrollbar-track { background: transparent; }
.locate-panel-list::-webkit-scrollbar-thumb {
  background: color-mix(in srgb, var(--text) 22%, transparent);
  border-radius: 999px;
}
.locate-panel-list::-webkit-scrollbar-thumb:hover {
  background: color-mix(in srgb, var(--text) 38%, transparent);
}
.locate-panel-item {
  appearance: none; background: transparent; border: 0; color: var(--text);
  font: inherit; text-align: left; padding: 6px 10px; border-radius: 6px;
  cursor: pointer; display: flex; align-items: baseline; gap: 8px; width: 100%;
  transition: background 0.12s;
}
.locate-panel-item:hover { background: var(--surface-hover); }
.locate-panel-idx { flex-shrink: 0; font-size: 11px; font-weight: 600; color: var(--text-mute); font-variant-numeric: tabular-nums; }
.locate-panel-text { flex: 1; min-width: 0; font-size: 13px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.locate-panel-time { flex-shrink: 0; font-size: 11px; color: var(--text-mute); font-variant-numeric: tabular-nums; }
.locate-panel-empty { padding: 16px; text-align: center; color: var(--text-mute); font-size: 13px; }
mark.locate-hl { background: rgba(255,213,79,0.55); color: inherit; border-radius: 2px; padding: 0 1px; }
:root[data-theme="dark"] mark.locate-hl { background: rgba(255,213,79,0.35); }

/* ---- Flash animation for jump-to-prompt ---- */
.msg.msg-flash > .bubble {
  animation: msg-flash-glow 1.4s ease;
  border-radius: 14px;
}
@keyframes msg-flash-glow {
  0% { box-shadow: 0 0 0 0 rgba(255, 178, 71, 0); }
  18% { box-shadow: 0 0 0 6px rgba(255, 178, 71, 0.32); }
  100% { box-shadow: 0 0 0 0 rgba(255, 178, 71, 0); }
}
`

function buildRuntimeScript(labels: {
  themeLight: string
  themeDark: string
  hideLabel: string
  unhideLabel: string
  hideMsgLabel: string
  unhideMsgLabel: string
  initialHidden: string[]
  emptyLabel: string
}): string {
  const L_LIGHT = JSON.stringify(`☀ ${labels.themeLight}`)
  const L_DARK = JSON.stringify(`☾ ${labels.themeDark}`)
  const L_HIDE = JSON.stringify(labels.hideLabel)
  const L_UNHIDE = JSON.stringify(labels.unhideLabel)
  const L_HIDE_MSG = JSON.stringify(labels.hideMsgLabel)
  const L_UNHIDE_MSG = JSON.stringify(labels.unhideMsgLabel)
  return `
(function () {
  var KEY = 'csv-export-theme';
  var root = document.documentElement;
  var stored = null;
  try { stored = localStorage.getItem(KEY); } catch (_) {}
  if (stored === 'light' || stored === 'dark') {
    root.setAttribute('data-theme', stored);
  }
  var THEME_LIGHT = ${L_LIGHT};
  var THEME_DARK = ${L_DARK};
  function paintTheme() {
    var btn = document.getElementById('theme-toggle');
    if (!btn) return;
    var dark = root.getAttribute('data-theme') === 'dark';
    // Button shows the *destination* theme — clicking it switches you there.
    btn.textContent = dark ? THEME_LIGHT : THEME_DARK;
  }
  document.addEventListener('DOMContentLoaded', function () {
    paintTheme();
    var btn = document.getElementById('theme-toggle');
    if (btn) btn.addEventListener('click', function () {
      var dark = root.getAttribute('data-theme') === 'dark';
      var next = dark ? 'light' : 'dark';
      root.setAttribute('data-theme', next);
      try { localStorage.setItem(KEY, next); } catch (_) {}
      paintTheme();
    });
    // ----- smooth scroll FABs (mirrors ChatView.scrollToTop / ToBottom) -----
    var fabTop = document.getElementById('fab-top');
    var fabBottom = document.getElementById('fab-bottom');
    var rafScroll = 0;
    function cancelScroll() {
      if (rafScroll) { cancelAnimationFrame(rafScroll); rafScroll = 0; }
    }
    function smoothScrollTo(target) {
      cancelScroll();
      var start = window.scrollY;
      var max = Math.max(0, document.documentElement.scrollHeight - window.innerHeight);
      var dest = Math.max(0, Math.min(target, max));
      var dist = dest - start;
      if (Math.abs(dist) < 2) { window.scrollTo(0, dest); return; }
      var duration = Math.min(360, 180 + Math.abs(dist) * 0.05);
      var t0 = performance.now();
      function ease(p) { return 1 - Math.pow(1 - p, 3); }
      function step(now) {
        var p = Math.min(1, (now - t0) / duration);
        window.scrollTo(0, start + dist * ease(p));
        if (p < 1) rafScroll = requestAnimationFrame(step); else rafScroll = 0;
      }
      function onUserScroll() {
        cancelScroll();
        window.removeEventListener('wheel', onUserScroll);
        window.removeEventListener('touchmove', onUserScroll);
      }
      window.addEventListener('wheel', onUserScroll, { passive: true, once: true });
      window.addEventListener('touchmove', onUserScroll, { passive: true, once: true });
      rafScroll = requestAnimationFrame(step);
    }
    if (fabTop) fabTop.addEventListener('click', function () { smoothScrollTo(0); });
    if (fabBottom) fabBottom.addEventListener('click', function () {
      smoothScrollTo(document.documentElement.scrollHeight);
    });
    function updateEdges() {
      var y = window.scrollY;
      var max = document.documentElement.scrollHeight - window.innerHeight;
      var atTop = y <= 8;
      var atBottom = y >= max - 8;
      if (fabTop) fabTop.setAttribute('data-hidden', atTop ? '1' : '0');
      if (fabBottom) fabBottom.setAttribute('data-hidden', atBottom ? '1' : '0');
    }
    var rafEdge = 0;
    window.addEventListener('scroll', function () {
      if (rafEdge) return;
      rafEdge = requestAnimationFrame(function () { rafEdge = 0; updateEdges(); });
    }, { passive: true });
    window.addEventListener('resize', updateEdges);
    updateEdges();

    // ----- image lightbox -----
    // 同页放大查看。data: URL 无法走 window.open（Chrome 阻断顶层导航到 data:），
    // 改成 fixed 覆盖层。点遮罩 / 按 Esc 关闭。
    var lb = document.createElement('div');
    lb.id = 'csv-lightbox';
    lb.className = 'csv-lightbox';
    var lbPrev = document.createElement('button');
    lbPrev.type = 'button'; lbPrev.className = 'csv-lb-nav csv-lb-prev'; lbPrev.innerHTML = '‹';
    var lbNext = document.createElement('button');
    lbNext.type = 'button'; lbNext.className = 'csv-lb-nav csv-lb-next'; lbNext.innerHTML = '›';
    var lbImg = document.createElement('img');
    var lbCount = document.createElement('div');
    lbCount.className = 'csv-lb-count';
    lb.appendChild(lbPrev); lb.appendChild(lbImg); lb.appendChild(lbNext); lb.appendChild(lbCount);
    document.body.appendChild(lb);
    var lbList = [];
    var lbIdx = 0;
    function renderLb() {
      if (!lbList.length) return;
      lbImg.src = lbList[lbIdx];
      var multi = lbList.length > 1;
      lbPrev.style.display = multi ? '' : 'none';
      lbNext.style.display = multi ? '' : 'none';
      lbCount.style.display = multi ? '' : 'none';
      lbCount.textContent = (lbIdx + 1) + ' / ' + lbList.length;
    }
    function stepLb(d) {
      if (lbList.length < 2) return;
      lbIdx = (lbIdx + d + lbList.length) % lbList.length;
      renderLb();
    }
    function closeLb() { lb.classList.remove('open'); lbImg.removeAttribute('src'); lbList = []; }
    // 点击某张图，取同一条消息 .msg-images 里的全部图片成组，从点中的那张开始翻看。
    function openLb(el) {
      if (!el || el.tagName !== 'IMG') return;
      var box = el.closest('.msg-images');
      var imgs = box ? [].slice.call(box.querySelectorAll('img')) : [el];
      lbList = imgs.map(function (im) { return im.getAttribute('src'); });
      lbIdx = Math.max(0, imgs.indexOf(el));
      if (!lbList.length) return;
      lb.classList.add('open');
      renderLb();
    }
    lb.addEventListener('click', function (e) {
      if (e.target === lbPrev || e.target === lbNext || e.target === lbImg) return;
      closeLb();
    });
    lbImg.addEventListener('click', function (e) { e.stopPropagation(); });
    lbPrev.addEventListener('click', function (e) { e.stopPropagation(); stepLb(-1); });
    lbNext.addEventListener('click', function (e) { e.stopPropagation(); stepLb(1); });
    document.addEventListener('keydown', function (e) {
      if (!lb.classList.contains('open')) return;
      if (e.key === 'Escape') closeLb();
      else if (e.key === 'ArrowLeft') stepLb(-1);
      else if (e.key === 'ArrowRight') stepLb(1);
    });
    window.__csvLightbox = openLb;

    // ----- message hide / context menu -----
    var HIDE_KEY = 'csv-hidden';
    var L_SHOW_HIDDEN = ${L_HIDE};
    var L_HIDE_HIDDEN = ${L_UNHIDE};
    var L_HIDE_MSG = ${L_HIDE_MSG};
    var L_UNHIDE_MSG = ${L_UNHIDE_MSG};
    var INITIAL_HIDDEN = ${JSON.stringify(labels.initialHidden)};
    var hiddenSet = {};
    try { var raw = localStorage.getItem(HIDE_KEY); if (raw) hiddenSet = JSON.parse(raw); } catch (_) {}
    for (var hi = 0; hi < INITIAL_HIDDEN.length; hi++) { if (!hiddenSet[INITIAL_HIDDEN[hi]]) hiddenSet[INITIAL_HIDDEN[hi]] = 1; }
    var showHidden = false;
    var ctxMenu = document.getElementById('csv-ctx-menu');
    var ctxLabel = document.getElementById('csv-ctx-label');
    var ctxToggleBtn = document.getElementById('csv-ctx-toggle');
    var hideToggle = document.getElementById('hide-toggle');
    var ctxTarget = null;

    function hiddenCount() {
      var n = 0; for (var k in hiddenSet) if (hiddenSet[k]) n++; return n;
    }
    function saveHidden() {
      try { localStorage.setItem(HIDE_KEY, JSON.stringify(hiddenSet)); } catch (_) {}
    }
    function refreshHiddenUI() {
      var count = hiddenCount();
      if (hideToggle) {
        hideToggle.setAttribute('data-count', String(count));
        hideToggle.textContent = (showHidden ? '\\u25C9 ' : '\\u25CE ') + count + ' hidden';
        if (showHidden) hideToggle.classList.add('active');
        else hideToggle.classList.remove('active');
      }
      var msgs = document.querySelectorAll('.msg[data-msg-key]');
      for (var i = 0; i < msgs.length; i++) {
        var el = msgs[i];
        var key = el.getAttribute('data-msg-key');
        if (hiddenSet[key]) {
          el.setAttribute('data-hidden', '1');
          if (showHidden) el.setAttribute('data-show-hidden', '1');
          else el.removeAttribute('data-show-hidden');
        } else {
          el.removeAttribute('data-hidden');
          el.removeAttribute('data-show-hidden');
        }
      }
    }
    function closeCtx() { if (ctxMenu) ctxMenu.classList.remove('open'); ctxTarget = null; }
    if (hideToggle) {
      hideToggle.addEventListener('click', function () {
        showHidden = !showHidden;
        refreshHiddenUI();
      });
    }
    if (ctxToggleBtn) {
      ctxToggleBtn.addEventListener('click', function () {
        if (!ctxTarget) return;
        var key = ctxTarget.getAttribute('data-msg-key');
        if (hiddenSet[key]) delete hiddenSet[key];
        else hiddenSet[key] = 1;
        saveHidden();
        closeCtx();
        refreshHiddenUI();
      });
    }
    document.addEventListener('contextmenu', function (e) {
      var msgEl = e.target.closest('.msg[data-msg-key]');
      if (!msgEl || msgEl.classList.contains('system')) return;
      e.preventDefault();
      ctxTarget = msgEl;
      var key = msgEl.getAttribute('data-msg-key');
      var isHidden = !!hiddenSet[key];
      if (ctxLabel) ctxLabel.textContent = isHidden ? L_UNHIDE_MSG : L_HIDE_MSG;
      if (ctxMenu) {
        var W = 180, H = 44;
        var x = Math.min(e.clientX, window.innerWidth - W - 8);
        var y = Math.min(e.clientY, window.innerHeight - H - 8);
        ctxMenu.style.left = x + 'px';
        ctxMenu.style.top = y + 'px';
        ctxMenu.classList.add('open');
      }
    });
    document.addEventListener('mousedown', function (e) {
      if (ctxMenu && ctxMenu.classList.contains('open') && !ctxMenu.contains(e.target)) closeCtx();
    });
    document.addEventListener('keydown', function (e) {
      if (e.key === 'Escape') closeCtx();
    });
    document.addEventListener('scroll', closeCtx, { passive: true });
    refreshHiddenUI();

    // ----- jump-to-prompt locate menu -----
    var locateBtn = document.getElementById('locate-btn');
    var locatePanel = document.getElementById('locate-panel');
    var locateInput = document.getElementById('locate-input');
    var locateList = document.getElementById('locate-list');
    var L_EMPTY = ${JSON.stringify(labels.emptyLabel)};
    var prompts = [];
    (function buildPrompts() {
      var userMsgs = document.querySelectorAll('.msg.user[data-msg-key]');
      var seq = 0;
      for (var i = 0; i < userMsgs.length; i++) {
        var el = userMsgs[i];
        var roleTag = el.querySelector('.role-tag');
        var timePart = '';
        if (roleTag) {
          var parts = roleTag.textContent.split('·');
          if (parts.length > 1) timePart = parts[parts.length - 1].trim();
        }
        var bubble = el.querySelector('.bubble');
        if (!bubble) continue;
        var textRun = bubble.querySelector('.text-run, .text');
        var raw = textRun ? textRun.textContent || '' : '';
        var plain = raw.replace(/<[^>]*>/g, '').trim();
        if (!plain) continue;
        seq++;
        var text = plain.length > 80 ? plain.slice(0, 80) + '…' : plain;
        prompts.push({ el: el, seq: seq, text: text, time: timePart });
      }
    })();
    function escHtml(s) {
      return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
    }
    function renderLocateList(q) {
      if (!locateList) return;
      var lower = (q || '').toLowerCase();
      var items = lower ? prompts.filter(function (p) { return p.text.toLowerCase().indexOf(lower) >= 0; }) : prompts;
      if (!items.length) {
        locateList.innerHTML = '<div class="locate-panel-empty">' + escHtml(L_EMPTY) + '</div>';
        return;
      }
      var html = '';
      for (var i = 0; i < items.length; i++) {
        var p = items[i];
        var label = escHtml(p.text);
        if (lower) {
          var esc = lower.replace(/[-\\/\\\\^$*+?.()|[\\]{}]/g, '\\\\$&');
          var re = new RegExp('(' + esc + ')', 'gi');
          label = label.replace(re, '<mark class="locate-hl">$1</mark>');
        }
        html += '<button class="locate-panel-item" data-locate-idx="' + i + '">' +
          '<span class="locate-panel-idx">#' + p.seq + '</span>' +
          '<span class="locate-panel-text">' + label + '</span>' +
          '<span class="locate-panel-time">' + escHtml(p.time) + '</span>' +
          '</button>';
      }
      locateList.innerHTML = html;
      // bind click handlers
      var btns = locateList.querySelectorAll('.locate-panel-item');
      for (var j = 0; j < btns.length; j++) {
        (function (idx) {
          btns[idx].addEventListener('click', function () {
            closeLocate();
            items[idx].el.scrollIntoView({ behavior: 'smooth', block: 'center' });
            items[idx].el.classList.add('msg-flash');
            setTimeout(function () { items[idx].el.classList.remove('msg-flash'); }, 1400);
          });
        })(j);
      }
    }
    function closeLocate() {
      if (locatePanel) locatePanel.classList.remove('open');
      if (locateBtn) locateBtn.classList.remove('active');
    }
    function toggleLocate() {
      if (!locatePanel) return;
      var open = locatePanel.classList.contains('open');
      if (open) { closeLocate(); return; }
      locatePanel.classList.add('open');
      if (locateBtn) locateBtn.classList.add('active');
      if (locateInput) { locateInput.value = ''; locateInput.focus(); }
      renderLocateList('');
    }
    if (locateBtn) locateBtn.addEventListener('click', function (e) { e.stopPropagation(); toggleLocate(); });
    if (locateInput) locateInput.addEventListener('input', function () { renderLocateList(locateInput.value); });
    if (locateInput) locateInput.addEventListener('keydown', function (e) { if (e.key === 'Escape') { e.stopPropagation(); closeLocate(); } });
    document.addEventListener('mousedown', function (e) {
      if (locatePanel && locatePanel.classList.contains('open')) {
        var wrap = locateBtn ? locateBtn.parentElement : null;
        if (wrap && !wrap.contains(e.target)) closeLocate();
      }
    });
  });
})();
`
}

function diffToHtml(hunks: DiffHunk[]): string {
  const rows: string[] = []
  for (const h of hunks) {
    rows.push(
      `<span class="ctx">@@ -${h.oldStart}, +${h.newStart} @@</span>`,
    )
    for (const l of h.lines) {
      const cls = l.kind === 'add' ? 'add' : l.kind === 'del' ? 'del' : 'ctx'
      const sign = l.kind === 'add' ? '+' : l.kind === 'del' ? '-' : ' '
      rows.push(`<span class="${cls}">${escapeHtml(sign + l.text)}</span>`)
    }
  }
  return `<div class="diff">${rows.join('\n')}</div>`
}

function toolResultBodyHtml(b: Block): string {
  if (b.diff && b.diff.length) {
    return `<div class="collapsible-box" data-collapsible>${diffToHtml(b.diff)}</div>`
  }
  const txt = b.text ?? ''
  if (!txt) return ''
  // 渲染优先级：unified diff（`git diff` / patch 文本）→ JSON → 原样。
  // diff 必须先判，因为 JSON 文件的 diff 既像 diff 又像 JSON，应该按 diff 渲染。
  let pre: string
  if (looksLikeDiff(txt)) {
    pre = `<pre class="lang-diff">${highlightDiff(txt)}</pre>`
  } else if (looksLikeJson(txt)) {
    pre = `<pre class="lang-json">${highlightJsonInPlace(txt)}</pre>`
  } else {
    pre = `<pre>${escapeHtml(txt)}</pre>`
  }
  return `<div class="collapsible-box" data-collapsible>${pre}</div>`
}

// tool.resultDiff = "File change · {file}" / "文件改动 · {file}". Split out the
// {file} slot so the path can render as a <code> chip in HTML.
function splitDiffLabel(filePath: string): string {
  const SENTINEL = '__CSV_FILE__'
  const tmpl = t('tool.resultDiff', { file: SENTINEL })
  const idx = tmpl.indexOf(SENTINEL)
  if (idx < 0) return `${escapeHtml(tmpl)} <code>${escapeHtml(filePath)}</code>`
  const pre = escapeHtml(tmpl.slice(0, idx))
  const post = escapeHtml(tmpl.slice(idx + SENTINEL.length))
  return `${pre}<code>${escapeHtml(filePath)}</code>${post}`
}

function toolResultLabel(b: Block): string {
  if (b.filePath) return `📄 ${splitDiffLabel(b.filePath)}`
  if (b.isError) return `⚠️ ${escapeHtml(t('tool.resultError'))}`
  return `📤 ${escapeHtml(t('tool.result'))}`
}

function blockToHtml(
  b: Block,
  ctx: { resultByToolId: Map<string, Block>; inlinedIds: Set<string>; renderCodexPluginMentions?: boolean },
): string {
  switch (b.kind) {
    case 'text':
      // 跟聊天界面一致：renderText() 给出表格 / fenced code / 行内强调 + 一个 mermaid
      // 占位符（<div class="md-mermaid" data-source="..."/>）。占位符在 messagesToHtml
      // 收尾阶段统一被 prerenderMermaidInHtml 替换成 SVG（一次性烤进 HTML，不依赖运行时 JS）。
      return ctx.renderCodexPluginMentions
        ? renderCodexPluginMentionHtmlText(renderText(b.text ?? ''))
        : renderText(b.text ?? '')
    case 'thinking':
      return `<details><summary>🧠 ${escapeHtml(t('tool.thinking'))}</summary><pre>${escapeHtml(b.text ?? '')}</pre></details>`
    case 'tool_use': {
      const label = escapeHtml(t('tool.call', { name: b.toolName ?? '' }))
      // Tool args 永远当 JSON 试 —— prettify + 上色；parse 失败也只是上 token 色，
      // 总比裸 escapeHtml 强。
      const args = prettifyAndHighlightJson(b.toolInput ?? '')
      let inner = `<pre class="lang-json">${args}</pre>`
      if (b.toolId && ctx.inlinedIds.has(b.toolId)) {
        const r = ctx.resultByToolId.get(b.toolId)
        if (r) {
          const body = toolResultBodyHtml(r)
          if (body) inner += `<div class="tool-result-inline">${body}</div>`
        }
      }
      return `<details><summary>🔧 ${label}</summary>${inner}</details>`
    }
    case 'tool_result': {
      // 已被 tool_use 吸收的不再单独出现
      if (b.toolId && ctx.inlinedIds.has(b.toolId)) return ''
      const label = toolResultLabel(b)
      const body = toolResultBodyHtml(b)
      // File change（有 diff 或 filePath）默认展开，跟会话详情一致
      const open = b.filePath || (b.diff && b.diff.length) ? ' open' : ''
      return body
        ? `<details${open}><summary>${label}</summary>${body}</details>`
        : `<details${open}><summary>${label}</summary></details>`
    }
    case 'image':
      // 导出的 HTML 里图片默认按文本宽度缩放，看不清细节；点击 → 同页 lightbox
      // 放大查看。原本用 window.open(this.src) 但 Chrome 拒绝从 window.open
      // 顶层导航到 data: URL（数据 URL 是 base64 图片，被 Block 成 about:blank）。
      // lightbox 在同页 fixed 覆盖，没有跨源 / 顶层导航问题。传 this（<img> 元素），
      // runtime 从同一条消息的 .msg-images 里取出整组图片，可左右翻看。
      return b.imageSrc
        ? `<img src="${escapeHtml(b.imageSrc)}" alt="" class="msg-image" onclick="window.__csvLightbox&amp;&amp;window.__csvLightbox(this)">`
        : ''
    case 'file':
      return b.filePath
        ? `<div class="msg-file">📎 ${escapeHtml(b.filePath.split(/[/\\]/).pop() || b.filePath)}</div>`
        : ''
    default:
      return ''
  }
}

function msgToHtml(
  m: Msg,
  idx: number,
  agent: Agent,
  ctx: { resultByToolId: Map<string, Block>; inlinedIds: Set<string> },
): string {
  const key = m.uuid || `idx:${idx}`
  // System event row — centered, no avatar, no bubble.
  const sysText = systemEventText(m)
  if (sysText) {
    const ts = m.timestamp ? ` · ${escapeHtml(formatTime(m.timestamp))}` : ''
    return `<div class="msg system" data-msg-key="${escapeHtml(key)}"><div class="system-event">${escapeHtml(sysText)}${ts}</div></div>`
  }
  // System-injected user records: labeled tag chip + formatted body, not a "Me"
  // bubble. Notification-style pseudo-XML is rendered as a key/value list.
  if (m.metaKind) {
    const ts = m.timestamp ? escapeHtml(formatTime(m.timestamp)) : ''
    const label = escapeHtml(metaKindLabelText(m.metaKind))
    const pre = metaKindIsPre(m.metaKind)
    const body = m.blocks
      .filter((b) => b.kind === 'text')
      .map((b) => {
        const fields = metaFields(b.text ?? '')
        if (fields) {
          const rows = fields
            .map(
              (f) =>
                `<dt class="meta-field-key">${escapeHtml(f.key)}</dt><dd class="meta-field-val">${escapeHtml(f.value)}</dd>`,
            )
            .join('')
          return `<dl class="meta-fields">${rows}</dl>`
        }
        return pre
          ? `<pre class="meta-pre">${escapeHtml(cleanMetaText(b.text ?? ''))}</pre>`
          : renderText(b.text ?? '')
      })
      .join('\n')
    if (!body) return ''
    const name = escapeHtml(roleLabel('assistant', agent))
    // Agent prefix on top, then a collapsed tool-call-style card whose summary
    // is the uppercase kind label — mirrors ChatView's meta rendering.
    return `<div class="msg meta" data-msg-key="${escapeHtml(key)}">
  <div class="avatar">${avatarSvg('assistant', agent)}</div>
  <div class="bubble meta-msg">
    <div class="role-tag"><span class="name">${name}</span>${ts ? ` <span>${ts}</span>` : ''}</div>
    <details class="meta-details"><summary>${label}</summary>${body}</details>
  </div>
</div>`
  }
  const displayRole = isToolOnly(m) ? 'tool' : m.role
  const tag = [
    roleLabel(displayRole, agent),
    m.model ? escapeHtml(m.model) : '',
    m.timestamp ? escapeHtml(formatTime(m.timestamp)) : '',
  ]
    .filter(Boolean)
    .join(' · ')
  // 跟 ChatView 一致：图片缩略图浮在气泡上方（不进气泡），正文滤掉 [Image #n] 占位符；
  // 纯图片消息不渲染空气泡。
  const imgs = m.blocks.filter((b) => b.kind === 'image' && b.imageSrc)
  const imagesHtml = imgs.length
    ? `<div class="msg-images">${imgs.map((b) => blockToHtml(b, ctx)).join('')}</div>`
    : ''
  const blockCtx = {
    ...ctx,
    renderCodexPluginMentions: displayRole === 'user' && agent === 'codex',
  }
  const body = m.blocks
    .filter((b) => b.kind !== 'image')
    .map((b) =>
      b.kind === 'text' && imgs.length
        ? blockToHtml({ ...b, text: stripImagePlaceholders(b.text ?? '') }, blockCtx)
        : blockToHtml(b, blockCtx),
    )
    .filter(Boolean)
    .join('\n')
  if (!body && !imagesHtml) return ''
  // 跟 ChatView 一致：只有用户消息整体走 CollapsibleBox，超过 320px 才折叠+显示更多
  const wrappedBody =
    displayRole === 'user'
      ? `<div class="collapsible-box" data-collapsible>${body}</div>`
      : body
  const bubbleHtml = body
    ? `<div class="bubble"><div class="role-tag">${tag}</div>${wrappedBody}</div>`
    : ''
  return `<div class="msg ${displayRole}" data-msg-key="${escapeHtml(key)}">
  <div class="avatar">${avatarSvg(displayRole, agent)}</div>
  <div class="msg-content">${imagesHtml}${bubbleHtml}</div>
</div>`
}

function currentTheme(): 'light' | 'dark' {
  return document.documentElement.classList.contains('theme-dark') ? 'dark' : 'light'
}

/** 扫一遍 HTML 把 renderText 留下的 .md-mermaid 占位符替换成真 SVG。
 *  让导出 HTML 完全离线可看（不依赖运行时 mermaid.js）。
 *  - 一次性 dynamic-import mermaid；同一 source 二次出现复用上次的 SVG 不重画。
 *  - 渲染失败：保留占位符 + 一行错误提示 + 源码，跟聊天里的兜底一致。
 *  - 主题：用当前 app 的 theme（light/dark），SVG 颜色烤死；HTML 的 theme toggle 切
 *    其它元素的色，mermaid SVG 保持不变（mermaid 不支持运行时切主题）。 */
async function prerenderMermaidInHtml(html: string): Promise<string> {
  // 没占位符就别动 mermaid，避免给纯文本会话加 600KB 的解析开销。
  if (!html.includes('class="md-mermaid"')) return html
  let mermaid: typeof import('mermaid').default
  try {
    mermaid = (await import('mermaid')).default
  } catch (e) {
    // 拉不到 mermaid（离线 / 安装损坏）—— 直接交回带占位符的 HTML，源码 fallback 还在。
    console.warn('[export] mermaid load failed:', e)
    return html
  }
  mermaid.initialize({
    startOnLoad: false,
    securityLevel: 'strict',
    theme: currentTheme() === 'dark' ? 'dark' : 'default',
    fontFamily:
      '-apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif',
  })
  const cache = new Map<string, { ok: true; svg: string } | { ok: false; err: string }>()
  // \s\S 跨行匹配占位符里的 fallback <pre>；同一占位符 div 不嵌套，懒匹配安全。
  const RE = /<div class="md-mermaid" data-source="([^"]*)">[\s\S]*?<\/div>/g
  const sources = new Set<string>()
  for (const m of html.matchAll(RE)) sources.add(m[1])
  let counter = 0
  for (const enc of sources) {
    counter += 1
    const src = decodeURIComponent(enc)
    try {
      const { svg } = await mermaid.render(`md-mermaid-export-${counter}`, src)
      cache.set(enc, { ok: true, svg })
    } catch (e) {
      cache.set(enc, { ok: false, err: (e as Error)?.message ?? String(e) })
    }
  }
  return html.replace(RE, (_, enc) => {
    const hit = cache.get(enc)
    const src = decodeURIComponent(enc)
    if (!hit) return _
    if (hit.ok) {
      return `<div class="md-mermaid" data-rendered>${hit.svg}</div>`
    }
    return (
      `<div class="md-mermaid md-mermaid-error" data-rendered>` +
      `<div class="md-mermaid-errmsg">mermaid: ${escapeHtml(hit.err)}</div>` +
      `<pre class="md-mermaid-source">${escapeHtml(src)}</pre>` +
      `</div>`
    )
  })
}

export async function messagesToHtml(
  session: SessionMeta,
  messages: Msg[],
  agent: Agent,
  hiddenKeys?: string[],
): Promise<string> {
  const title = escapeHtml(session.title)
  const { u, a } = computeStats(messages)
  const statsLine = escapeHtml(
    t('chat.stats', {
      u,
      a,
      time: session.created ? formatTime(session.created) : '—',
    }),
  )
  const meta = [
    statsLine,
    `${escapeHtml(t('export.meta.agent'))}: <code>${agent}</code>`,
    session.cwd ? `${escapeHtml(t('export.meta.cwd'))}: <code>${escapeHtml(session.cwd)}</code>` : '',
    session.id ? `${escapeHtml(t('export.meta.id'))}: <code>${escapeHtml(session.id)}</code>` : '',
  ]
    .filter(Boolean)
    .join(' &middot; ')
  const ctx = buildInlinedResults(messages)
  // 先生成 raw body（含 .md-mermaid 占位符），再一次性烤 SVG 进去。
  // 收尾才烤可以让多个 mermaid 块共用同一个 mermaid runtime 初始化。
  const rawBody = messages
    .map((m, i) => isCaveatOnlyMsg(m) ? '' : msgToHtml(m, i, agent, ctx))
    .filter(Boolean)
    .join('\n')
  const body = await prerenderMermaidInHtml(rawBody)
  const theme = currentTheme()
  const themeLight = t('export.theme.light')
  const themeDark = t('export.theme.dark')
  const hideLabel = t('chat.action.showHidden')
  const unhideLabel = t('chat.action.hideHidden')
  const hideMsgLabel = t('chat.action.hideMsg')
  const unhideMsgLabel = t('chat.action.unhideMsg')
  const runtimeScript = buildRuntimeScript({
    themeLight,
    themeDark,
    hideLabel,
    unhideLabel,
    hideMsgLabel,
    unhideMsgLabel,
    initialHidden: hiddenKeys ?? [],
    emptyLabel: t('chat.empty'),
  })
  const initialBtnLabel = theme === 'dark' ? `☀ ${escapeHtml(themeLight)}` : `☾ ${escapeHtml(themeDark)}`
  const topLabel = escapeHtml(t('chat.action.top'))
  const bottomLabel = escapeHtml(t('chat.action.bottom'))
  return `<!doctype html>
<html lang="en" data-theme="${theme}">
<head>
<meta charset="utf-8">
<title>${title}</title>
<style>${HTML_STYLE}</style>
</head>
<body>
<div class="sticky-head">
  <div class="header">
    <h1>${title}</h1>
    <div class="locate-wrap">
      <button id="locate-btn" class="locate-btn" type="button">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="22" y1="12" x2="18" y2="12"/><line x1="6" y1="12" x2="2" y2="12"/><line x1="12" y1="6" x2="12" y2="2"/><line x1="12" y1="22" x2="12" y2="18"/></svg>
      </button>
      <div id="locate-panel" class="locate-panel">
        <div class="locate-panel-search">
          <input id="locate-input" class="locate-panel-input" type="text" placeholder="${escapeHtml(t('chat.tb.locate.placeholder'))}">
        </div>
        <div id="locate-list" class="locate-panel-list"></div>
      </div>
    </div>
    <button id="hide-toggle" class="hide-toggle" type="button" data-count="0"></button>
    <button id="theme-toggle" class="theme-toggle" type="button" aria-label="Toggle theme">${initialBtnLabel}</button>
  </div>
  <div class="meta">${meta}</div>
</div>
${body}
<div class="fabs">
  <button id="fab-top" class="fab" type="button" aria-label="${topLabel}" title="${topLabel}" data-hidden="1">${AVATAR_SVG.arrowUp}</button>
  <button id="fab-bottom" class="fab" type="button" aria-label="${bottomLabel}" title="${bottomLabel}">${AVATAR_SVG.arrowDown}</button>
</div>
<div id="csv-ctx-menu" class="csv-ctx-menu">
  <button id="csv-ctx-toggle" class="csv-ctx-item" type="button">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49"/><path d="M14.084 14.158a3 3 0 0 1-4.242-4.242"/><path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143"/><path d="m2 2 20 20"/></svg>
    <span id="csv-ctx-label"></span>
  </button>
</div>
<script>${runtimeScript}</script>
</body>
</html>
`
}

// ============================ 落盘 ============================
// 弹原生 Save As 让用户选位置，再写盘。返回最终路径以便提示/打开访达。
// 用户取消对话框时返回 null（调用方据此跳过 toast 与 reveal）。

export type ExportKind = 'md' | 'html' | 'json'

const EXPORT_FILTERS: Record<ExportKind, { name: string; extensions: string[] }> = {
  md: { name: 'Markdown', extensions: ['md'] },
  html: { name: 'HTML', extensions: ['html'] },
  json: { name: 'JSON', extensions: ['json'] },
}

async function pickAndWrite(
  content: string,
  defaultName: string,
  kind: ExportKind,
): Promise<string | null> {
  const chosen = await saveDialog({
    defaultPath: defaultName,
    filters: [EXPORT_FILTERS[kind]],
  })
  if (!chosen) return null
  return writeFile(chosen, content)
}

/** 无损 JSON 导出的信封：自包含（带 messages），可在任意机器上重新导入还原。
 *  `__type` 是导入端识别本格式的标记；`version` 留给以后格式演进。 */
export function buildExportEnvelope(
  session: SessionMeta,
  messages: Msg[],
  agent: Agent,
): string {
  return JSON.stringify(
    { __type: 'cc-session-viewer-export', version: 1, agent, session, messages },
    null,
    2,
  )
}

export function exportMarkdown(
  session: SessionMeta,
  messages: Msg[],
  agent: Agent,
): Promise<string | null> {
  const md = messagesToMarkdown(session, messages, agent)
  return pickAndWrite(md, `${sanitizeFilename(session.title)}.md`, 'md')
}

export async function exportHtml(
  session: SessionMeta,
  messages: Msg[],
  agent: Agent,
  hiddenKeys?: string[],
): Promise<string | null> {
  const html = await messagesToHtml(session, messages, agent, hiddenKeys)
  return pickAndWrite(html, `${sanitizeFilename(session.title)}.html`, 'html')
}

export function exportJson(
  session: SessionMeta,
  messages: Msg[],
  agent: Agent,
): Promise<string | null> {
  const json = buildExportEnvelope(session, messages, agent)
  return pickAndWrite(json, `${sanitizeFilename(session.title)}.json`, 'json')
}

// ============================ 批量导出 ============================
// 批量场景：让用户挑一个目标目录，所有会话以 `<title>-<id8>.<ext>` 落进去。
// 用 `/` 拼接：Rust 端走 `PathBuf::from`，Windows 也能接受正斜杠。

/** 弹原生 Open 目录选择器；取消返回 null。 */
export async function pickExportDir(): Promise<string | null> {
  const r = await openDialog({ directory: true, multiple: false })
  // open() 在「单选 + directory」下返回字符串或 null（与平台/插件版本相关）。
  return typeof r === 'string' ? r : null
}

/** 批量导出的子目录名：`export-YYYYMMDD-HHMMSS-<md|html>`。
 *  本地时间，便于人在 Finder 里直观分辨；多次导出不会撞名。
 *  `now` 形参只用于测试；生产路径走默认值 `new Date()`。 */
export function batchExportFolderName(kind: ExportKind, now: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, '0')
  const dt = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`
  return `export-${dt}-${kind}`
}

/** 文件名：`<sanitized-title>-<id8>.<ext>`；标题相同的两条会话不会互相覆盖。 */
function batchFileName(session: SessionMeta, ext: ExportKind): string {
  const title = sanitizeFilename(session.title)
  const tag = (session.id || '').slice(0, 8) || 'session'
  return `${title}-${tag}.${ext}`
}

/** 把一条会话以 Markdown 写到目录里，返回最终绝对路径。 */
export async function exportMarkdownToDir(
  session: SessionMeta,
  messages: Msg[],
  agent: Agent,
  dir: string,
): Promise<string> {
  const md = messagesToMarkdown(session, messages, agent)
  return writeFile(`${dir}/${batchFileName(session, 'md')}`, md)
}

/** 把一条会话以 HTML 写到目录里，返回最终绝对路径。 */
export async function exportHtmlToDir(
  session: SessionMeta,
  messages: Msg[],
  agent: Agent,
  dir: string,
): Promise<string> {
  const html = await messagesToHtml(session, messages, agent)
  return writeFile(`${dir}/${batchFileName(session, 'html')}`, html)
}

/** 把一条会话以无损 JSON 写到目录里，返回最终绝对路径。 */
export async function exportJsonToDir(
  session: SessionMeta,
  messages: Msg[],
  agent: Agent,
  dir: string,
): Promise<string> {
  const json = buildExportEnvelope(session, messages, agent)
  return writeFile(`${dir}/${batchFileName(session, 'json')}`, json)
}
