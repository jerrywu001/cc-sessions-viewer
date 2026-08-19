import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Block, Msg, SessionMeta } from '../src/types'

// Tauri's save dialog and the filesystem command are unavailable in jsdom —
// stub them so the落盘 path (exportMarkdown / exportHtml) is testable.
const { saveMock, writeFileMock } = vi.hoisted(() => ({
  saveMock: vi.fn(),
  writeFileMock: vi.fn(),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: saveMock }))
vi.mock('../src/api', () => ({ writeFile: writeFileMock }))

import {
  batchExportFolderName,
  buildExportEnvelope,
  exportHtml,
  exportJson,
  exportJsonToDir,
  exportMarkdown,
  messagesToHtml,
  messagesToMarkdown,
} from '../src/export'
import { setLang } from '../src/settings'

beforeEach(() => {
  setLang('en')
  saveMock.mockReset()
  writeFileMock.mockReset()
})
afterEach(() => {
  document.documentElement.classList.remove('theme-dark')
})

// ---- factories -----------------------------------------------------------
function blk(over: Partial<Block> & { kind: Block['kind'] }): Block {
  return { isError: false, ...over }
}
function msg(
  role: Msg['role'],
  blocks: Block[],
  over: Partial<Msg> = {},
): Msg {
  return { role, sidechain: false, blocks, ...over }
}
function session(over: Partial<SessionMeta> = {}): SessionMeta {
  return {
    id: 'sess-1',
    fileName: 's.jsonl',
    path: '/p/s.jsonl',
    title: 'My Session',
    modified: 0,
    size: 100,
    messageCount: 5,
    ...over,
  }
}

const text = (t: string) => blk({ kind: 'text', text: t })

describe('messagesToMarkdown', () => {
  it('emits a title heading and the meta block', () => {
    const md = messagesToMarkdown(session({ cwd: '/work', id: 'abc' }), [], 'claude')
    expect(md).toContain('# My Session')
    expect(md).toContain('- Agent: `claude`')
    expect(md).toContain('- cwd: `/work`')
    expect(md).toContain('- ID: `abc`')
    expect(md).toContain('\n---\n')
  })

  it('omits the cwd and id lines when they are absent', () => {
    const md = messagesToMarkdown(session({ cwd: undefined, id: '' }), [], 'claude')
    expect(md).not.toContain('cwd:')
    expect(md).not.toContain('ID:')
  })

  it('renders a user text block under a "Me" heading', () => {
    const md = messagesToMarkdown(session(), [msg('user', [text('Hello world')])], 'claude')
    expect(md).toContain('## Me')
    expect(md).toContain('Hello world')
  })

  it('labels Grok Build assistant messages rather than Claude', async () => {
    const messages = [msg('assistant', [text('Grok Build response')])]
    expect(messagesToMarkdown(session(), messages, 'grok')).toContain('## Grok Build')
    expect(await messagesToHtml(session(), messages, 'grok')).toContain('Grok Build')
  })

  it('renders a thinking block inside a <details> element', () => {
    const md = messagesToMarkdown(
      session(),
      [msg('assistant', [blk({ kind: 'thinking', text: 'pondering' })])],
      'claude',
    )
    expect(md).toContain('<summary>🧠 Thinking</summary>')
    expect(md).toContain('pondering')
  })

  it('renders a tool_use with its JSON arguments', () => {
    const md = messagesToMarkdown(
      session(),
      [msg('assistant', [blk({ kind: 'tool_use', toolName: 'Read', toolInput: '{"file":"x"}' })])],
      'claude',
    )
    expect(md).toContain('Tool call · Read')
    expect(md).toContain('```json')
    expect(md).toContain('{"file":"x"}')
  })

  it('inlines a non-file-mutating tool_result under its tool_use', () => {
    const messages = [
      msg('assistant', [blk({ kind: 'tool_use', toolName: 'Read', toolId: 't1', toolInput: '{}' })]),
      msg('user', [blk({ kind: 'tool_result', toolId: 't1', text: 'file body' })]),
    ]
    const md = messagesToMarkdown(session(), messages, 'claude')
    expect(md).toContain('file body')
    // the tool-result message is fully absorbed — no standalone "## Tool"
    expect(md).not.toContain('## Tool')
  })

  it('renders a file-mutating tool_result as its own diff block', () => {
    const messages = [
      msg('assistant', [blk({ kind: 'tool_use', toolName: 'Write', toolId: 't2', toolInput: '{}' })]),
      msg('user', [
        blk({
          kind: 'tool_result',
          toolId: 't2',
          filePath: '/x/y.ts',
          diff: [
            {
              oldStart: 1,
              newStart: 1,
              lines: [
                { kind: 'ctx', oldNo: 1, newNo: 1, text: 'keep' },
                { kind: 'add', oldNo: null, newNo: 2, text: 'added' },
                { kind: 'del', oldNo: 2, newNo: null, text: 'removed' },
              ],
            },
          ],
        }),
      ]),
    ]
    const md = messagesToMarkdown(session(), messages, 'claude')
    expect(md).toContain('File change · /x/y.ts')
    expect(md).toContain('```diff')
    expect(md).toContain('+added')
    expect(md).toContain('-removed')
  })

  it('marks an error tool_result', () => {
    const md = messagesToMarkdown(
      session(),
      [msg('assistant', [blk({ kind: 'tool_result', text: 'boom', isError: true })])],
      'claude',
    )
    expect(md).toContain('Tool result · error')
  })

  it('renders an image block', () => {
    const md = messagesToMarkdown(
      session(),
      [msg('user', [blk({ kind: 'image', imageSrc: 'data:image/png;base64,AAA' })])],
      'claude',
    )
    expect(md).toContain('![image](data:image/png;base64,AAA)')
  })

  it('drops local-command-caveat messages', () => {
    const messages = [
      msg('user', [text('<local-command-caveat>noise</local-command-caveat>')]),
      msg('user', [text('real prompt')]),
    ]
    const md = messagesToMarkdown(session(), messages, 'claude')
    expect(md).not.toContain('noise')
    expect(md).toContain('real prompt')
  })

  it('renders a /rename system event as an italic line', () => {
    const messages = [
      msg('user', [
        text('<system-reminder>The user named this session "新名字". x</system-reminder>'),
      ]),
    ]
    const md = messagesToMarkdown(session(), messages, 'claude')
    expect(md).toContain('_User renamed this session to "新名字"')
  })

  it('labels a tool-only user message as "Tool"', () => {
    const messages = [
      msg('assistant', [blk({ kind: 'tool_use', toolName: 'Write', toolId: 't9', toolInput: '{}' })]),
      msg('user', [blk({ kind: 'tool_result', toolId: 't9', filePath: '/a.ts', text: 'done' })]),
    ]
    const md = messagesToMarkdown(session(), messages, 'claude')
    expect(md).toContain('## Tool')
  })

  it('counts prompts and replies in the stats line', () => {
    const messages = [
      msg('user', [text('q1')]),
      msg('assistant', [text('a1')]),
      msg('assistant', [text('a2')]),
      msg('user', [text('<local-command-caveat>x</local-command-caveat>')]),
    ]
    const md = messagesToMarkdown(session(), messages, 'claude')
    expect(md).toContain('1 prompts · 2 replies')
  })

  it('uses the agent-specific assistant label', () => {
    const md = messagesToMarkdown(session(), [msg('assistant', [text('hi')])], 'codex')
    expect(md).toContain('## Codex')
    expect(md).toContain('- Agent: `codex`')
  })

})

describe('messagesToHtml', () => {
  it('produces a full HTML document', async () => {
    const html = await messagesToHtml(session(), [], 'claude')
    expect(html.startsWith('<!doctype html>')).toBe(true)
    expect(html).toContain('<title>My Session</title>')
    expect(html).toContain('</html>')
  })

  it('escapes HTML-significant characters in the title', async () => {
    const html = await messagesToHtml(session({ title: '<script>' }), [], 'claude')
    expect(html).toContain('<title>&lt;script&gt;</title>')
  })

  it('wraps a user message body in a collapsible box', async () => {
    const html = await messagesToHtml(session(), [msg('user', [text('hi')])], 'claude')
    expect(html).toContain('class="msg user"')
    expect(html).toContain('collapsible-box')
  })

  it('renders Codex plugin mentions with bounded icons in HTML export', async () => {
    const html = await messagesToHtml(
      session(),
      [
        msg('user', [text('@Visualize hi')]),
        msg('user', [text('[@template-creator](plugin://template-creator@openai-primary-runtime) 创建一个readme模版')]),
      ],
      'codex',
    )
    expect(html).toContain('class="codex-plugin-ref cmd-name"')
    expect(html).toContain('codex-plugin-ref-ic--visualize')
    expect(html).toContain('codex-plugin-ref-ic--template')
    expect(html).toContain('<span class="codex-plugin-ref-name">Visualize</span>')
    expect(html).toContain('<span class="codex-plugin-ref-name">Template Creator</span>')
    expect(html).toContain('.bubble .codex-plugin-ref-ic svg')
    expect(html).toContain('width: 16px;')
    expect(html).not.toContain('plugin://template-creator')
  })

  it('styles the prompt locator scrollbar for light HTML exports', async () => {
    const html = await messagesToHtml(session(), [msg('user', [text('hi')])], 'claude')
    expect(html).toContain('.locate-panel-list::-webkit-scrollbar')
    expect(html).toContain('.locate-panel-list::-webkit-scrollbar-thumb')
    expect(html).toContain('scrollbar-color: color-mix(in srgb, var(--text) 22%, transparent) transparent')
    expect(html).toContain('width: 7px;')
  })

  it('converts newlines in assistant text to <br>', async () => {
    const html = await messagesToHtml(session(), [msg('assistant', [text('a\nb')])], 'claude')
    expect(html).toContain('a<br>b')
  })

  it('renders a file-change result as an open <details>', async () => {
    const messages = [
      msg('assistant', [
        blk({ kind: 'tool_result', filePath: '/f.ts', text: 'patched' }),
      ]),
    ]
    const html = await messagesToHtml(session(), messages, 'claude')
    expect(html).toContain('<details open>')
    expect(html).toContain('<code>/f.ts</code>')
  })

  it('renders a thinking block as a <details> element', async () => {
    const html = await messagesToHtml(
      session(),
      [msg('assistant', [blk({ kind: 'thinking', text: 'reasoning' })])],
      'claude',
    )
    expect(html).toContain('🧠')
    expect(html).toContain('<details><summary>')
    expect(html).toContain('reasoning')
  })

  it('renders a tool_use with its arguments', async () => {
    const html = await messagesToHtml(
      session(),
      [msg('assistant', [blk({ kind: 'tool_use', toolName: 'Bash', toolInput: 'ls -la' })])],
      'claude',
    )
    expect(html).toContain('🔧')
    expect(html).toContain('Tool call · Bash')
    expect(html).toContain('ls -la')
  })

  it('inlines a non-file-mutating tool_result inside its tool_use', async () => {
    const messages = [
      msg('assistant', [blk({ kind: 'tool_use', toolName: 'Read', toolId: 'r1', toolInput: '{}' })]),
      msg('user', [blk({ kind: 'tool_result', toolId: 'r1', text: 'file contents' })]),
    ]
    const html = await messagesToHtml(session(), messages, 'claude')
    expect(html).toContain('tool-result-inline')
    expect(html).toContain('file contents')
  })

  it('renders a structured diff result with add/del rows', async () => {
    const messages = [
      msg('assistant', [
        blk({
          kind: 'tool_result',
          filePath: '/d.ts',
          diff: [
            {
              oldStart: 2,
              newStart: 2,
              lines: [
                { kind: 'ctx', oldNo: 2, newNo: 2, text: 'unchanged' },
                { kind: 'add', oldNo: null, newNo: 3, text: 'new line' },
                { kind: 'del', oldNo: 3, newNo: null, text: 'old line' },
              ],
            },
          ],
        }),
      ]),
    ]
    const html = await messagesToHtml(session(), messages, 'claude')
    expect(html).toContain('<div class="diff">')
    expect(html).toContain('<span class="add">+new line</span>')
    expect(html).toContain('<span class="del">-old line</span>')
  })

  it('marks a standalone error tool_result', async () => {
    const html = await messagesToHtml(
      session(),
      [msg('assistant', [blk({ kind: 'tool_result', text: 'failure', isError: true })])],
      'claude',
    )
    expect(html).toContain('⚠️')
    expect(html).toContain('Tool result · error')
  })

  it('labels a tool-only user message with the tool avatar', async () => {
    const messages = [
      msg('assistant', [blk({ kind: 'tool_use', toolName: 'Write', toolId: 'w1', toolInput: '{}' })]),
      msg('user', [blk({ kind: 'tool_result', toolId: 'w1', filePath: '/a.ts', text: 'done' })]),
    ]
    const html = await messagesToHtml(session(), messages, 'claude')
    expect(html).toContain('class="msg tool"')
  })

  it('renders an image as an <img> tag', async () => {
    const html = await messagesToHtml(
      session(),
      [msg('user', [blk({ kind: 'image', imageSrc: 'data:x' })])],
      'claude',
    )
    expect(html).toContain('<img src="data:x"')
    // 图片点击 → 同页 lightbox（不用 window.open，Chrome 拒绝顶层导航到 data:URL）。
    expect(html).toContain('class="msg-image"')
    expect(html).toMatch(/onclick="window\.__csvLightbox/)
    // lightbox 容器 + runtime 入口在导出 HTML 里要存在。
    expect(html).toContain('csv-lightbox')
    expect(html).toContain('window.__csvLightbox = openLb')
  })

  it('drops local-command-caveat messages', async () => {
    const html = await messagesToHtml(
      session(),
      [msg('user', [text('<local-command-caveat>noise</local-command-caveat>')])],
      'claude',
    )
    expect(html).not.toContain('noise')
  })

  it('renders a system event as a centered row', async () => {
    const html = await messagesToHtml(
      session(),
      [msg('user', [text('<system-reminder>The user named this session "X". y</system-reminder>')])],
      'claude',
    )
    expect(html).toContain('class="msg system"')
  })

  it('renders an interrupt marker as a centered system row, not a Me bubble', async () => {
    const html = await messagesToHtml(
      session(),
      [msg('user', [text('[Request interrupted by user]')])],
      'claude',
    )
    expect(html).toContain('class="msg system"')
    expect(html).toContain('Request interrupted by user')
    expect(html).not.toContain('class="msg user"')
  })

  it('renders a teammate-message metaKind as a collapsed labeled card, not a Me bubble', async () => {
    const raw =
      'Another Claude session sent a message:\n' +
      '<teammate-message teammate_id="flow_reader" color="blue">\n' +
      '{"type":"idle_notification"}\n' +
      '</teammate-message>\n\n' +
      'This came from another Claude session — boilerplate that should be dropped.'
    const html = await messagesToHtml(
      session(),
      [msg('user', [text(raw)], { metaKind: 'teammate-message' })],
      'claude',
    )
    // Meta block, not a "Me" bubble, with the agent prefix + collapsed card.
    expect(html).toContain('class="msg meta"')
    expect(html).not.toContain('class="msg user"')
    expect(html).toContain('<details class="meta-details"><summary>Teammate message</summary>')
    // teammate id → payload rendered as a field row; boilerplate dropped.
    expect(html).toContain('flow_reader')
    expect(html).toContain('idle_notification')
    expect(html).not.toContain('boilerplate that should be dropped')
  })

  it('reflects the active theme in the data-theme attribute', async () => {
    expect(await messagesToHtml(session(), [], 'claude')).toContain('data-theme="light"')
    document.documentElement.classList.add('theme-dark')
    expect(await messagesToHtml(session(), [], 'claude')).toContain('data-theme="dark"')
  })

  // 用户反馈：HTML 导出之前是裸 escapeHtml + <br>，markdown 元素全不渲染。
  // 现在切到 renderText()，table / mermaid / inline 强调都要在导出里正确呈现。
  it('renders markdown tables in HTML export', async () => {
    const html = await messagesToHtml(
      session(),
      [msg('assistant', [text('| A | B |\n|---|---|\n| 1 | 2 |')])],
      'claude',
    )
    expect(html).toContain('<div class="md-table-wrap">')
    expect(html).toContain('<table class="md-table">')
    expect(html).toContain('<th>A</th>')
    expect(html).toContain('<td>2</td>')
    // 原始 `|` 分隔符不应该泄漏
    expect(html).not.toContain('| A | B |')
  })

  it('renders inline markdown (bold / inline code) in HTML export', async () => {
    const html = await messagesToHtml(
      session(),
      [msg('assistant', [text('hello **world** with `code`')])],
      'claude',
    )
    expect(html).toContain('<strong>world</strong>')
    expect(html).toContain('<code>code</code>')
  })

  // mermaid prerender —— 不论成功（SVG）/ 失败（errmsg + 源码）/ mermaid 加载失败
  // （留占位符），都不应该让 mermaid 块在导出 HTML 里彻底消失。最低保障：源码
  // 字符串至少要出现在 HTML 里某处（rendered 节点里、error fallback 里、或原始占位符里）。
  it('preserves mermaid blocks in HTML export (rendered or fallback)', async () => {
    const html = await messagesToHtml(
      session(),
      [msg('assistant', [text('```mermaid\nNOTAVALIDGRAPH\n```')])],
      'claude',
    )
    // 必有 md-mermaid 容器（class 或在 error / rendered 状态）
    expect(/class="md-mermaid/.test(html)).toBe(true)
  })

})

describe('exportMarkdown / exportHtml', () => {
  it('returns null when the save dialog is cancelled', async () => {
    saveMock.mockResolvedValue(null)
    const result = await exportMarkdown(session(), [], 'claude')
    expect(result).toBeNull()
    expect(writeFileMock).not.toHaveBeenCalled()
  })

  it('writes the markdown file and returns the final path', async () => {
    saveMock.mockResolvedValue('/Users/me/out.md')
    writeFileMock.mockResolvedValue('/Users/me/out.md')
    const result = await exportMarkdown(session(), [msg('user', [text('hi')])], 'claude')
    expect(result).toBe('/Users/me/out.md')
    expect(writeFileMock).toHaveBeenCalledWith('/Users/me/out.md', expect.stringContaining('# My Session'))
  })

  it('writes the html file and returns the final path', async () => {
    saveMock.mockResolvedValue('/Users/me/out.html')
    writeFileMock.mockResolvedValue('/Users/me/out.html')
    const result = await exportHtml(session(), [], 'claude')
    expect(result).toBe('/Users/me/out.html')
    expect(writeFileMock).toHaveBeenCalledWith('/Users/me/out.html', expect.stringContaining('<!doctype html>'))
  })

  it('writes a lossless JSON envelope that round-trips agent + session + messages', async () => {
    saveMock.mockResolvedValue('/Users/me/out.json')
    writeFileMock.mockResolvedValue('/Users/me/out.json')
    const msgs = [msg('user', [text('hi')]), msg('assistant', [text('yo')])]
    const result = await exportJson(session(), msgs, 'codex')
    expect(result).toBe('/Users/me/out.json')
    const written = writeFileMock.mock.calls[0][1]
    const parsed = JSON.parse(written)
    expect(parsed.__type).toBe('cc-session-viewer-export')
    expect(parsed.version).toBe(1)
    expect(parsed.agent).toBe('codex')
    expect(parsed.session.id).toBe('sess-1')
    expect(parsed.messages).toHaveLength(2)
    expect(parsed.messages[0].blocks[0].text).toBe('hi')
  })

  it('preserves Grok Build in JSON and batch exports', async () => {
    saveMock.mockResolvedValue('/Users/me/grok.json')
    writeFileMock.mockResolvedValue('/exports/My Session-sess-1.json')
    const messages = [msg('assistant', [text('Grok Build response')])]

    await exportJson(session(), messages, 'grok')
    expect(JSON.parse(writeFileMock.mock.calls[0][1]).agent).toBe('grok')

    await exportJsonToDir(session(), messages, 'grok', '/exports')
    expect(writeFileMock).toHaveBeenLastCalledWith(
      '/exports/My Session-sess-1.json',
      expect.stringContaining('"agent": "grok"'),
    )
  })
})

describe('buildExportEnvelope', () => {
  it('tags the format and embeds the full payload', () => {
    const env = JSON.parse(buildExportEnvelope(session(), [msg('user', [text('hi')])], 'codex'))
    expect(env.__type).toBe('cc-session-viewer-export')
    expect(env.agent).toBe('codex')
    expect(env.session.title).toBe('My Session')
    expect(env.messages[0].role).toBe('user')
  })

  it('sanitizes illegal characters out of the default filename', async () => {
    saveMock.mockResolvedValue(null)
    await exportMarkdown(session({ title: 'a/b:c*?' }), [], 'claude')
    expect(saveMock.mock.calls[0][0].defaultPath).toBe('a_b_c__.md')
  })

  it('falls back to "session" when the title sanitizes to empty', async () => {
    saveMock.mockResolvedValue(null)
    await exportMarkdown(session({ title: '   ' }), [], 'claude')
    expect(saveMock.mock.calls[0][0].defaultPath).toBe('session.md')
  })
})

describe('batchExportFolderName', () => {
  it('formats the local date and time and the kind suffix', () => {
    // 2026-05-23T08:09:07 (local) → `export-20260523-080907-md`
    const now = new Date(2026, 4, 23, 8, 9, 7)
    expect(batchExportFolderName('md', now)).toBe('export-20260523-080907-md')
    expect(batchExportFolderName('html', now)).toBe('export-20260523-080907-html')
  })

  it('zero-pads every numeric segment', () => {
    const now = new Date(2026, 0, 5, 3, 4, 5)
    expect(batchExportFolderName('md', now)).toBe('export-20260105-030405-md')
  })
})
