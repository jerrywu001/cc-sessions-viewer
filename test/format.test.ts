import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  cleanMetaText,
  formatElapsedSeconds,
  formatSize,
  formatTime,
  formatTokens,
  highlightSegments,
  isAskUserQuestionInstructionOnlyMsg,
  isCaveatOnlyMsg,
  metaKindIsPre,
  parseFileRef,
  parseMetaFields,
  parseSystemEvent,
  parseTeammateMessage,
  renderText,
  renderTextUncached,
  shortName,
} from '../src/format'
import { setLang } from '../src/settings'

// format.ts pulls localized strings via t(); pin the language so assertions
// don't depend on the host machine's locale.
beforeEach(() => setLang('en'))

// Convenience builders for the structural-message shapes the helpers accept.
const block = (kind: string, text?: string) => ({ kind, text })
const userMsg = (...blocks: Array<{ kind: string; text?: string }>) => ({
  role: 'user',
  blocks,
})

describe('parseFileRef', () => {
  it('splits off a trailing :line and :line:col', () => {
    expect(parseFileRef('lib/a/b.dart')).toEqual({ path: 'lib/a/b.dart' })
    expect(parseFileRef('lib/a/b.dart:371')).toEqual({
      path: 'lib/a/b.dart',
      line: 371,
      col: undefined,
    })
    expect(parseFileRef('src/x.ts:10:5')).toEqual({ path: 'src/x.ts', line: 10, col: 5 })
  })

  it('does not mistake a Windows drive colon for a line number', () => {
    expect(parseFileRef('C:\\proj\\x.ts')).toEqual({ path: 'C:\\proj\\x.ts' })
    expect(parseFileRef('C:\\proj\\x.ts:42')).toEqual({
      path: 'C:\\proj\\x.ts',
      line: 42,
      col: undefined,
    })
  })
})

describe('isAskUserQuestionInstructionOnlyMsg', () => {
  it('hides an explicit tool-test instruction because the assistant card is the visible question', () => {
    expect(
      isAskUserQuestionInstructionOnlyMsg(
        userMsg({
          kind: 'text',
          text: 'Use the AskUserQuestion tool exactly once and NOTHING else. Ask TWO questions in a single call.',
        }),
      ),
    ).toBe(true)
  })

  it('hides a single-select instruction with an inline question description', () => {
    expect(
      isAskUserQuestionInstructionOnlyMsg(
        userMsg({
          kind: 'text',
          text: 'Use the AskUserQuestion tool exactly once and NOTHING else. Ask ONE single-select question (multiSelect false): header "布局", question "选择一个页面布局".',
        }),
      ),
    ).toBe(true)
  })

  it('keeps ordinary discussion about the tool visible', () => {
    expect(
      isAskUserQuestionInstructionOnlyMsg(
        userMsg({ kind: 'text', text: 'What is the AskUserQuestion tool used for?' }),
      ),
    ).toBe(false)
  })
})

describe('renderText', () => {
  it('renders uncached streaming text identically, including nested details', () => {
    const raw = '<details>\n<summary>More</summary>\n**streaming**\n</details>'
    expect(renderTextUncached(raw)).toBe(renderText(raw))
  })

  it('escapes HTML special characters', () => {
    expect(renderText('<b> & </b>')).toContain('&lt;b&gt; &amp; &lt;/b&gt;')
  })

  it('renders inline code, bold and headings', () => {
    expect(renderText('`code`')).toContain('<code>code</code>')
    expect(renderText('**bold**')).toContain('<strong>bold</strong>')
    expect(renderText('# Title')).toContain('<h3>Title</h3>')
    expect(renderText('## Sub')).toContain('<h3>Sub</h3>')
    expect(renderText('### Deep')).toContain('<h4>Deep</h4>')
  })

  it('converts newlines inside a text run to <br>', () => {
    expect(renderText('line1\nline2')).toContain('line1<br>line2')
  })

  it('linkifies a bare URL but keeps a backtick-wrapped URL literal', () => {
    expect(renderText('see https://x.com here')).toContain(
      '<a href="https://x.com" target="_blank" rel="noopener">https://x.com</a>',
    )
    // A URL inside backticks is a literal code span — not linkified, not split.
    expect(renderText('see `https://x.com/` here')).toContain('<code>https://x.com/</code>')
    expect(renderText('see `https://x.com/` here')).not.toContain('<code><a')
  })

  // Regression: a backtick-wrapped URL used to let the URL regex swallow the
  // closing backtick, splitting the <a> tag with a <code> and leaving an unclosed
  // <code>/<strong> that leaked into every following sibling (shrinking all later
  // messages via `code { font: 0.92em }`). Output must be well-nested.
  it('produces well-nested tags for a bullet item with a backtick-wrapped URL', () => {
    const html = renderText('- Self-test address: `https://localhost:1021/`')
    // balanced code/strong/li tags
    const open = (re: RegExp) => (html.match(re) ?? []).length
    expect(open(/<code(?![a-z])/g)).toBe(open(/<\/code>/g))
    expect(open(/<li(?![a-z])/g)).toBe(open(/<\/li>/g))
    // the <code> closes before the </li> — no leaked formatting tag
    expect(html).toContain('<code>https://localhost:1021/</code></li>')
    expect(html).not.toContain('</code>"')
  })

  // 文件路径形态的 inline code 渲染成可点 .file-ref（ChatView 委托点击在外部编辑器打开）。
  it('turns a file-path inline code into a clickable file-ref', () => {
    const html = renderText('see `lib/pages/home/todo_workbench_screen.dart:371` here')
    expect(html).toContain('class="file-ref"')
    expect(html).toContain('data-file-ref="lib/pages/home/todo_workbench_screen.dart:371"')
    expect(html).toContain('>lib/pages/home/todo_workbench_screen.dart:371</code>')
  })

  it('treats absolute paths and :line:col suffixes as file-refs', () => {
    expect(renderText('`/Users/me/proj/src/x.ts:5:3`')).toContain(
      'data-file-ref="/Users/me/proj/src/x.ts:5:3"',
    )
    expect(renderText('`./src/index.ts`')).toContain('data-file-ref="./src/index.ts"')
  })

  it('does not treat object.method, bare filenames or dirs as file-refs', () => {
    // 无路径分隔符 → 普通 code（避免 obj.method / package.json 误判）。
    expect(renderText('`array.map`')).toContain('<code>array.map</code>')
    expect(renderText('`array.map`')).not.toContain('file-ref')
    expect(renderText('`package.json`')).not.toContain('file-ref')
    // 末段无扩展名（目录）→ 不是文件引用。
    expect(renderText('`src/components`')).not.toContain('file-ref')
  })

  it('does not treat a URL inside backticks as a file-ref', () => {
    expect(renderText('`https://x.com/a.ts`')).not.toContain('file-ref')
    expect(renderText('`https://x.com/a.ts`')).toContain('<code>https://x.com/a.ts</code>')
  })

  it('renders a fenced code block with a language line', () => {
    const html = renderText('```js\nconst x = 1\n```')
    expect(html).toContain('<pre class="code-block" data-lang="js"><code>const x = 1</code></pre>')
  })

  it('renders a fenced code block with no language line', () => {
    expect(renderText('```\nplain\n```')).toContain('<code>plain</code>')
  })

  it('escapes HTML inside fenced code blocks', () => {
    expect(renderText('```\n<a> & b\n```')).toContain('&lt;a&gt; &amp; b')
  })

  // 回归用户反馈：外层用 4 个反引号包住内含 ```js 围栏的 markdown 时，内层的 ``` 被
  // 当成围栏拆掉了。围栏长度应由开围栏决定，更短的反引号串只是代码内容。
  it('keeps inner ``` as content inside a longer 4-backtick fence', () => {
    const html = renderText('````markdown\n```js\nx()\n```\n````')
    // 整段是一个 markdown 代码块
    expect(html).toContain('<pre class="code-block" data-lang="markdown">')
    // 内层围栏作为文本内容保留（不另起 code-block）
    expect(html).toContain('```js')
    // 只有一个 code-block —— 内层没有被错误拆成第二个
    expect(html.match(/class="code-block"/g)?.length).toBe(1)
  })

  // 未闭合围栏：从开围栏一直吃到文末，仍算一个代码块（与旧 split 行为一致）。
  it('treats an unclosed fence as a single code block to end of text', () => {
    const html = renderText('```js\nconst x = 1')
    expect(html).toContain('<pre class="code-block" data-lang="js"><code>const x = 1</code></pre>')
  })

  it('wraps plain prose in a text-run div', () => {
    expect(renderText('hello')).toBe('<div class="text-run">hello</div>')
  })

  // GFM table 渲染 —— 回归用户反馈："table 渲染出来是 `| 路由 | 路径 | 文件 |\n|---|---|---|`
  // 一坨原始字符 + 每个 `|` 单元被 inline code 包成小灰块"。
  it('renders a GFM table with header, separator and body into a <table>', () => {
    const html = renderText('| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |')
    // 外层 .md-table-wrap 提供横向滚动（列多时不撑爆气泡），里头才是 <table>。
    expect(html).toContain('<div class="md-table-wrap"><table class="md-table">')
    expect(html).toContain('<th>A</th>')
    expect(html).toContain('<th>B</th>')
    expect(html).toContain('<td>1</td>')
    expect(html).toContain('<td>4</td>')
    // 不能再有原始的 `|` 分隔符泄漏出来
    expect(html).not.toContain('| A | B |')
  })

  it('applies column alignment from the separator row colons', () => {
    const html = renderText('| L | C | R |\n|:---|:---:|---:|\n| a | b | c |')
    expect(html).toContain('text-align:left')
    expect(html).toContain('text-align:center')
    expect(html).toContain('text-align:right')
  })

  // 回归：分隔格按 GFM 只需 ≥1 个连字符，对齐列常写成 `--:`（2 个）。之前要求
  // `-{3,}` 会让整张表当普通文本渲染（用户反馈的 "table 没渲染成功"）。
  it('renders a table whose alignment cells use fewer than three dashes', () => {
    const html = renderText('| # | Lang |\n|--:|------|\n| 1 | Rust |')
    expect(html).toContain('<table class="md-table">')
    expect(html).toContain('text-align:right')
    expect(html).toContain('<th style="text-align:right">#</th>')
    expect(html).not.toContain('|--:|')
  })

  it('honors inline formatting inside table cells', () => {
    const html = renderText('| name | path |\n|---|---|\n| **bold** | `code` |')
    expect(html).toContain('<td><strong>bold</strong></td>')
    expect(html).toContain('<td><code>code</code></td>')
  })

  it('renders markdown bullet lists as <ul><li>', () => {
    const html = renderText('- tool_use directly rendered\n- paired tool result hidden')
    expect(html).toContain('<ul class="md-list">')
    expect(html).toContain('<li>tool_use directly rendered</li>')
    expect(html).toContain('<li>paired tool result hidden</li>')
    expect(html).not.toContain('<div class="text-run">- tool_use directly rendered')
  })

  it('renders absolute local markdown links as clickable file links', () => {
    const html = renderText(
      'See [src/views/ChatView.vue](/Users/wuchao/apps/claude-session-viewer/src/views/ChatView.vue:97).',
    )
    expect(html).toContain('class="local-file-link"')
    expect(html).toContain(
      'data-local-target="/Users/wuchao/apps/claude-session-viewer/src/views/ChatView.vue:97"',
    )
    expect(html).toContain('>src/views/ChatView.vue<')
  })

  // Mermaid 块：emit 占位符给 ChatView 后置 mermaid.render() 替换；fallback 露源码。
  it('emits a mermaid placeholder with encoded source for ```mermaid blocks', () => {
    const html = renderText('```mermaid\nflowchart TD\n  A --> B\n```')
    expect(html).toContain('<div class="md-mermaid"')
    expect(html).toContain('data-source="')
    expect(html).toContain(encodeURIComponent('flowchart TD\n  A --> B'))
    // 渲染前先露源码 fallback
    expect(html).toContain('<pre class="md-mermaid-source">flowchart TD')
    // 不应该走普通 code-block 分支
    expect(html).not.toContain('<pre class="code-block">')
  })

  it('still emits a regular code-block for non-mermaid fenced code', () => {
    const html = renderText('```js\nconst x = 1\n```')
    expect(html).toContain('<pre class="code-block" data-lang="js">')
    expect(html).not.toContain('md-mermaid')
  })

  // 非 table 文本和 table 混合时，前后文本各自走原来的 .text-run 渲染。
  it('keeps surrounding prose around an inline table', () => {
    const html = renderText('before\n\n| A |\n|---|\n| 1 |\n\nafter')
    expect(html).toContain('<div class="text-run">before')
    expect(html).toContain('<table class="md-table">')
    expect(html).toContain('after</div>')
  })

  it('drops <command-message> and emits <command-name> as a blue command chip', () => {
    const html = renderText(
      '<command-message>init</command-message><command-name>/init</command-name>',
    )
    expect(html).not.toContain('command-message')
    expect(html).toContain('<code class="cmd-tag cmd-name">/init</code>')
  })

  it('emits <command-args> as a plain code chip (no cmd-name) and escapes its content', () => {
    const html = renderText('<command-args><x></command-args>')
    expect(html).toContain('<code class="cmd-tag">&lt;x&gt;</code>')
  })

  it('drops empty <command-args> so /clear etc. do not render a trailing empty chip', () => {
    const html = renderText(
      '<command-name>/clear</command-name><command-args></command-args>',
    )
    expect(html).toContain('<code class="cmd-tag cmd-name">/clear</code>')
    // No empty chip after the /clear pill
    expect(html).not.toMatch(/<code class="cmd-tag"><\/code>/)
  })

  it('drops whitespace-only <command-args>', () => {
    const html = renderText(
      '<command-name>/init</command-name><command-args>   </command-args>',
    )
    expect(html).not.toMatch(/<code class="cmd-tag">\s*<\/code>/)
  })

  it('renders Codex plugin markdown links as plugin mention tokens', () => {
    const html = renderText('[@visualize](plugin://visualize@openai-bundled) xxx')
    expect(html).toContain('class="codex-plugin-ref cmd-name"')
    expect(html).toContain('codex-plugin-ref-ic--visualize')
    expect(html).toContain('<span class="codex-plugin-ref-name">Visualize</span>')
    expect(html).toContain(' xxx</div>')
    expect(html).not.toContain('plugin://visualize')
  })

  it('uses friendly names for slug-labeled Codex plugin links', () => {
    const html = renderText('[@template-creator](plugin://template-creator@openai-primary-runtime) xxx')
    expect(html).toContain('<span class="codex-plugin-ref-name">Template Creator</span>')
  })

  it('returns an empty string for empty input', () => {
    expect(renderText('')).toBe('')
  })

  // 用户反馈：`---` 被渲染成字面量 "---"，而不是分隔线。
  it('renders --- as a horizontal rule, not literal dashes', () => {
    const html = renderText('above\n\n---\n\nbelow')
    expect(html).toContain('<hr class="md-hr">')
    expect(html).toContain('<div class="text-run">above</div>')
    expect(html).toContain('<div class="text-run">below</div>')
    expect(html).not.toContain('---')
  })

  it('treats *** and ___ as horizontal rules too', () => {
    expect(renderText('***')).toContain('<hr class="md-hr">')
    expect(renderText('___')).toContain('<hr class="md-hr">')
  })

  it('does not mistake a GFM table separator row for a horizontal rule', () => {
    const html = renderText('| A | B |\n|---|---|\n| 1 | 2 |')
    expect(html).toContain('<table class="md-table">')
    expect(html).not.toContain('<hr')
  })

  // 用户反馈：标题/代码块前后空行叠成大段空白。空行应被压扁，标题间距交给 CSS。
  it('collapses stacked blank lines around a heading', () => {
    const html = renderText('## Heading\n\n\n\nbody')
    expect(html).toContain('<h3>Heading</h3>')
    expect(html).not.toContain('<br><h3>')
    expect(html).not.toContain('</h3><br>')
  })

  it('trims blank lines between prose and a fenced code block', () => {
    const html = renderText('intro\n\n\n```js\nx\n```')
    // the prose run must not carry a trailing run of <br> before the code block
    expect(html).not.toContain('<br></div>')
  })
})

describe('isCaveatOnlyMsg', () => {
  it('is true when every block is a local-command-caveat', () => {
    expect(
      isCaveatOnlyMsg(
        userMsg(block('text', '<local-command-caveat>x</local-command-caveat>')),
      ),
    ).toBe(true)
  })

  it('tolerates surrounding whitespace', () => {
    expect(
      isCaveatOnlyMsg(
        userMsg(block('text', '  \n<local-command-caveat>x</local-command-caveat>\n ')),
      ),
    ).toBe(true)
  })

  it('is false for non-user roles', () => {
    expect(
      isCaveatOnlyMsg({
        role: 'assistant',
        blocks: [block('text', '<local-command-caveat>x</local-command-caveat>')],
      }),
    ).toBe(false)
  })

  it('is false when the message has no blocks', () => {
    expect(isCaveatOnlyMsg(userMsg())).toBe(false)
  })

  it('is false when prose accompanies the caveat', () => {
    expect(
      isCaveatOnlyMsg(
        userMsg(block('text', 'hi <local-command-caveat>x</local-command-caveat>')),
      ),
    ).toBe(false)
  })

  it('is false when a non-text block is present', () => {
    expect(
      isCaveatOnlyMsg(
        userMsg(
          block('text', '<local-command-caveat>x</local-command-caveat>'),
          block('image'),
        ),
      ),
    ).toBe(false)
  })
})

describe('parseSystemEvent', () => {
  it('parses a /rename system reminder', () => {
    const ev = parseSystemEvent(
      userMsg(
        block(
          'text',
          '<system-reminder>\nThe user named this session "批量导入". More.\n</system-reminder>',
        ),
      ),
    )
    expect(ev).toEqual({ kind: 'rename', name: '批量导入' })
  })

  it('returns null for non-user roles', () => {
    expect(
      parseSystemEvent({
        role: 'assistant',
        blocks: [block('text', '<system-reminder>The user named this session "x"</system-reminder>')],
      }),
    ).toBeNull()
  })

  it('returns null when there is more than one block', () => {
    expect(
      parseSystemEvent(
        userMsg(
          block('text', '<system-reminder>The user named this session "x"</system-reminder>'),
          block('text', 'extra'),
        ),
      ),
    ).toBeNull()
  })

  it('returns null when prose surrounds the reminder', () => {
    expect(
      parseSystemEvent(
        userMsg(block('text', 'hello <system-reminder>The user named this session "x"</system-reminder>')),
      ),
    ).toBeNull()
  })

  it('returns null for an unrecognized reminder', () => {
    expect(
      parseSystemEvent(userMsg(block('text', '<system-reminder>some other note</system-reminder>'))),
    ).toBeNull()
  })

  it('returns null when there is no reminder at all', () => {
    expect(parseSystemEvent(userMsg(block('text', 'plain message')))).toBeNull()
  })

  it('parses a standalone interrupt marker', () => {
    expect(parseSystemEvent(userMsg(block('text', '[Request interrupted by user]')))).toEqual({
      kind: 'interrupt',
    })
  })

  it('parses the "for tool use" interrupt variant', () => {
    expect(
      parseSystemEvent(userMsg(block('text', '[Request interrupted by user for tool use]'))),
    ).toEqual({ kind: 'interrupt' })
  })

  it('does not treat prose mentioning interruption as an event', () => {
    expect(
      parseSystemEvent(userMsg(block('text', 'the [Request interrupted by user] earlier was odd'))),
    ).toBeNull()
  })
})

describe('metaKindIsPre', () => {
  it('treats command output / notifications / system as <pre>', () => {
    expect(metaKindIsPre('command-output')).toBe(true)
    expect(metaKindIsPre('task-notification')).toBe(true)
    expect(metaKindIsPre('system')).toBe(true)
  })

  it('treats compact / meta as markdown (not <pre>)', () => {
    expect(metaKindIsPre('compact')).toBe(false)
    expect(metaKindIsPre('meta')).toBe(false)
  })
})

describe('cleanMetaText', () => {
  it('strips the outer local-command-stdout wrapper', () => {
    expect(cleanMetaText('<local-command-stdout>hello\nworld</local-command-stdout>')).toBe(
      'hello\nworld',
    )
  })

  it('strips bash-stdout/stderr wrappers', () => {
    expect(cleanMetaText('<bash-stdout>done</bash-stdout>')).toBe('done')
    expect(cleanMetaText('<bash-stderr>oops</bash-stderr>')).toBe('oops')
  })

  it('strips ANSI escape sequences', () => {
    const esc = String.fromCharCode(27)
    const raw = `<local-command-stdout>${esc}[2mNote:${esc}[22m keep this</local-command-stdout>`
    expect(cleanMetaText(raw)).toBe('Note: keep this')
  })

  it('keeps inner pseudo-XML tags for task notifications (only outer shell removed)', () => {
    const raw =
      '<task-notification>\n<task-id>ba1tuv7k4</task-id>\n<event>ready</event>\n</task-notification>'
    expect(cleanMetaText(raw)).toBe('<task-id>ba1tuv7k4</task-id>\n<event>ready</event>')
  })

  it('leaves unwrapped text untouched (minus surrounding whitespace)', () => {
    expect(cleanMetaText('  plain text  ')).toBe('plain text')
  })
})

describe('parseMetaFields', () => {
  it('parses a task-notification into ordered key/value fields', () => {
    const raw =
      '<task-notification>\n<task-id>bpqfsy6zo</task-id>\n<summary>Monitor event: "DM-Watch"</summary>\n<event>DM-NEW name=王爱鑫 count=1</event>\n</task-notification>'
    expect(parseMetaFields(raw)).toEqual([
      { key: 'task-id', value: 'bpqfsy6zo' },
      { key: 'summary', value: 'Monitor event: "DM-Watch"' },
      { key: 'event', value: 'DM-NEW name=王爱鑫 count=1' },
    ])
  })

  it('parses the background-command notification shape', () => {
    const raw =
      '<task-notification>\n<task-id>bz2lxppsz</task-id>\n<status>completed</status>\n<summary>Background command "x" completed (exit code 0)</summary>\n</task-notification>'
    expect(parseMetaFields(raw)).toEqual([
      { key: 'task-id', value: 'bz2lxppsz' },
      { key: 'status', value: 'completed' },
      { key: 'summary', value: 'Background command "x" completed (exit code 0)' },
    ])
  })

  it('returns null for plain command output (no field tags)', () => {
    expect(parseMetaFields('<local-command-stdout>Terminal setup...</local-command-stdout>')).toBeNull()
  })

  it('returns null when prose is mixed in between tags', () => {
    expect(
      parseMetaFields('<task-notification><task-id>x</task-id> some stray prose</task-notification>'),
    ).toBeNull()
  })
})

describe('parseTeammateMessage', () => {
  it('extracts each teammate block as id → payload, dropping boilerplate', () => {
    const raw = [
      'Another Claude session sent a message:',
      '<teammate-message teammate_id="flow_detail_reader" color="blue">',
      '{"type":"idle_notification","from":"flow_detail_reader"}',
      '</teammate-message>',
      '',
      '<teammate-message teammate_id="records_reader" color="yellow">',
      'hello there',
      '</teammate-message>',
      '',
      'This came from another Claude session — not typed by your user...',
    ].join('\n')
    expect(parseTeammateMessage(raw)).toEqual([
      { key: 'flow_detail_reader', value: '{"type":"idle_notification","from":"flow_detail_reader"}' },
      { key: 'records_reader', value: 'hello there' },
    ])
  })

  it('returns null when there is no teammate-message block', () => {
    expect(parseTeammateMessage('just a normal message')).toBeNull()
  })
})

describe('formatSize', () => {
  it('formats bytes below 1 KiB', () => {
    expect(formatSize(0)).toBe('0 B')
    expect(formatSize(1023)).toBe('1023 B')
  })

  it('formats kibibytes with one decimal', () => {
    expect(formatSize(1024)).toBe('1.0 KB')
    expect(formatSize(1536)).toBe('1.5 KB')
  })

  it('formats mebibytes with one decimal', () => {
    expect(formatSize(1024 * 1024)).toBe('1.0 MB')
    expect(formatSize(2.5 * 1024 * 1024)).toBe('2.5 MB')
  })
})

describe('formatTime', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date(2026, 4, 22, 15, 0, 0))
  })
  afterEach(() => vi.useRealTimers())

  it('returns an em dash for missing or empty input', () => {
    expect(formatTime(undefined)).toBe('—')
    expect(formatTime('')).toBe('—')
  })

  it('returns an em dash for an unparseable value', () => {
    expect(formatTime('not-a-date')).toBe('—')
    expect(formatTime(NaN)).toBe('—')
  })

  it('labels a same-day timestamp as Today', () => {
    expect(formatTime(new Date(2026, 4, 22, 9, 5).getTime())).toBe('Today 09:05')
  })

  it('labels the previous calendar day as Yesterday', () => {
    expect(formatTime(new Date(2026, 4, 21, 23, 59).getTime())).toBe('Yesterday 23:59')
  })

  it('formats older timestamps as YYYY-MM-DD HH:MM', () => {
    expect(formatTime(new Date(2026, 0, 3, 8, 7).getTime())).toBe('2026-01-03 08:07')
  })
})

describe('formatElapsedSeconds', () => {
  it('formats seconds, minutes, and hours compactly', () => {
    expect(formatElapsedSeconds(7)).toBe('7s')
    expect(formatElapsedSeconds(1000)).toBe('16m 40s')
    expect(formatElapsedSeconds(3725)).toBe('1h 02m')
  })
})

describe('shortName', () => {
  it('returns the last path segment', () => {
    expect(shortName('/Users/me/apps/viewer')).toBe('viewer')
  })

  it('returns the last Windows path segment', () => {
    expect(shortName('C:\\Users\\me\\apps\\viewer')).toBe('viewer')
  })

  it('ignores a trailing slash', () => {
    expect(shortName('/Users/me/apps/viewer/')).toBe('viewer')
  })

  it('returns the input unchanged when there is no separator', () => {
    expect(shortName('viewer')).toBe('viewer')
  })

  it('returns the input for an empty string', () => {
    expect(shortName('')).toBe('')
  })
})

describe('highlightSegments', () => {
  it('returns a single non-hit segment when the query is empty', () => {
    expect(highlightSegments('workflow with obsidian', '')).toEqual([
      { text: 'workflow with obsidian', hit: false },
    ])
  })

  it('splits a single match into before / hit / after', () => {
    expect(highlightSegments('workflow with obsidian', 'obsidian')).toEqual([
      { text: 'workflow with ', hit: false },
      { text: 'obsidian', hit: true },
    ])
  })

  it('matches case-insensitively but keeps the original casing in the hit', () => {
    expect(highlightSegments('Obsidian Notes', 'obsidian')).toEqual([
      { text: 'Obsidian', hit: true },
      { text: ' Notes', hit: false },
    ])
  })

  it('highlights every occurrence', () => {
    expect(highlightSegments('aXaXa', 'a').filter((s) => s.hit)).toHaveLength(3)
  })

  it('treats regex-special characters literally', () => {
    expect(highlightSegments('a.b.c', '.')).toEqual([
      { text: 'a', hit: false },
      { text: '.', hit: true },
      { text: 'b', hit: false },
      { text: '.', hit: true },
      { text: 'c', hit: false },
    ])
  })

  it('returns one non-hit segment when there is no match', () => {
    expect(highlightSegments('hello', 'zzz')).toEqual([{ text: 'hello', hit: false }])
  })

  it('reproduces the original text when the segments are joined', () => {
    const text = 'fix the obsidian sync bug in obsidian'
    const joined = highlightSegments(text, 'obsidian')
      .map((s) => s.text)
      .join('')
    expect(joined).toBe(text)
  })

  it('ignores a whitespace-only query', () => {
    expect(highlightSegments('hello', '   ')).toEqual([{ text: 'hello', hit: false }])
  })
})

describe('formatTokens', () => {
  it('renders sub-1k as plain integer', () => {
    expect(formatTokens(0)).toBe('0')
    expect(formatTokens(1)).toBe('1')
    expect(formatTokens(999)).toBe('999')
  })

  it('renders thousands with one decimal place', () => {
    expect(formatTokens(1000)).toBe('1K')
    expect(formatTokens(1234)).toBe('1.2K')
    expect(formatTokens(12_345)).toBe('12.3K')
  })

  it('keeps the 1-decimal place even past 100K (codeburn parity, no silent rounding)', () => {
    expect(formatTokens(100_000)).toBe('100K') // 100.0 → trailing .0 trimmed
    expect(formatTokens(240_500)).toBe('240.5K')
    expect(formatTokens(345_678)).toBe('345.7K')
  })

  it('switches to M at one million', () => {
    expect(formatTokens(1_000_000)).toBe('1M')
    expect(formatTokens(1_234_567)).toBe('1.2M')
    expect(formatTokens(123_456_789)).toBe('123.5M')
  })

  it('returns "0" for non-finite / negative input', () => {
    expect(formatTokens(NaN)).toBe('0')
    expect(formatTokens(-5)).toBe('0')
    expect(formatTokens(Infinity)).toBe('0')
  })

  it('rounds sub-1k values to the nearest integer', () => {
    expect(formatTokens(999.4)).toBe('999')
    expect(formatTokens(500.6)).toBe('501')
  })
})
