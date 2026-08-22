import type { DiffHunk, DiffLine } from './types'

type PatchOp = 'update' | 'add' | 'delete'
type PatchLineKind = 'ctx' | 'add' | 'del' | 'hunk'

export interface CodexPatchLine {
  kind: PatchLineKind
  text: string
  oldNo?: number
  newNo?: number
}

export interface CodexPatchSection {
  op: PatchOp
  path: string
  movedTo?: string
  lines: CodexPatchLine[]
  addCount: number
  delCount: number
}

const FILE_HEADER_RE = /^\*\*\* (Update|Add|Delete) File: (.+)$/
const MOVE_TO_RE = /^\*\*\* Move to: (.+)$/
const HUNK_HEADER_RE = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

function opFromHeader(op: string): PatchOp {
  if (op === 'Add') return 'add'
  if (op === 'Delete') return 'delete'
  return 'update'
}

function displayPath(path: string, cwd?: string): string {
  const normalizedPath = path.replace(/\\/g, '/')
  const normalizedCwd = cwd?.replace(/\\/g, '/').replace(/\/$/, '')
  if (normalizedCwd && normalizedPath.startsWith(`${normalizedCwd}/`)) {
    return normalizedPath.slice(normalizedCwd.length + 1)
  }
  return normalizedPath
}

function opLabel(op: PatchOp): string {
  if (op === 'add') return 'Added'
  if (op === 'delete') return 'Deleted'
  return 'Updated'
}

function renderPatchLine(line: CodexPatchLine): string {
  if (line.kind === 'hunk') {
    if (line.text.trim() === '@@') return ''
    return `<div class="codex-patch-line hunk"><span class="codex-patch-text">${escapeHtml(line.text)}</span></div>`
  }
  const sign = line.kind === 'add' ? '+' : line.kind === 'del' ? '-' : ''
  const lineNo = line.kind === 'add' ? line.newNo : line.oldNo
  const text = line.text.length ? escapeHtml(line.text) : '&nbsp;'
  return `<div class="codex-patch-line ${line.kind}"><span class="codex-patch-no">${lineNo ?? ''}</span><span class="codex-patch-sign">${sign}</span><span class="codex-patch-text">${text}</span></div>`
}

export function parseCodexApplyPatch(input: string): CodexPatchSection[] {
  const lines = (input ?? '').split('\n')
  const sections: CodexPatchSection[] = []
  let current: CodexPatchSection | null = null
  let oldNo: number | undefined
  let newNo: number | undefined

  const flush = () => {
    if (!current) return
    sections.push(current)
    current = null
  }

  for (const line of lines) {
    if (!line) continue
    if (line === '*** Begin Patch') continue
    if (line === '*** End Patch') break
    if (line === '*** End of File') continue

    const fileHeader = FILE_HEADER_RE.exec(line)
    if (fileHeader) {
      flush()
      const op = opFromHeader(fileHeader[1])
      current = {
        op,
        path: fileHeader[2],
        lines: [],
        addCount: 0,
        delCount: 0,
      }
      oldNo = op === 'delete' ? 1 : undefined
      newNo = op === 'add' ? 1 : undefined
      continue
    }

    if (!current) continue

    const moveTo = MOVE_TO_RE.exec(line)
    if (moveTo) {
      current.movedTo = moveTo[1]
      continue
    }

    if (line.startsWith('@@')) {
      current.lines.push({ kind: 'hunk', text: line })
      const hunk = HUNK_HEADER_RE.exec(line)
      if (hunk) {
        oldNo = Number(hunk[1])
        newNo = Number(hunk[2])
      }
      continue
    }
    if (line.startsWith('+')) {
      current.lines.push({ kind: 'add', text: line.slice(1), newNo })
      current.addCount += 1
      if (newNo !== undefined) newNo += 1
      continue
    }
    if (line.startsWith('-')) {
      current.lines.push({ kind: 'del', text: line.slice(1), oldNo })
      current.delCount += 1
      if (oldNo !== undefined) oldNo += 1
      continue
    }
    if (line.startsWith(' ')) {
      current.lines.push({ kind: 'ctx', text: line.slice(1), oldNo, newNo })
      if (oldNo !== undefined) oldNo += 1
      if (newNo !== undefined) newNo += 1
      continue
    }
  }

  flush()
  return sections.filter((section, index) => {
    const next = sections[index + 1]
    return !(
      section.op === 'delete' &&
      section.lines.length === 0 &&
      next?.op === 'add' &&
      next.path === section.path
    )
  })
}

export function renderCodexApplyPatchHtml(input: string, cwd?: string): string | null {
  const sections = parseCodexApplyPatch(input)
  if (!sections.length) return null

  return renderCodexPatchSectionsHtml(sections, cwd)
}

export function renderCodexFileChangeHtml(
  hunks: DiffHunk[] | undefined,
  path: string,
  changeType?: string,
  cwd?: string,
): string {
  const op = patchOpFromChangeType(changeType) ?? patchOpFromHunks(hunks)
  const lines: CodexPatchLine[] = []
  let addCount = 0
  let delCount = 0
  for (const hunk of hunks ?? []) {
    const oldCount = hunk.lines.filter((line) => line.kind !== 'add').length
    const newCount = hunk.lines.filter((line) => line.kind !== 'del').length
    lines.push({
      kind: 'hunk',
      text: `@@ -${hunk.oldStart},${oldCount} +${hunk.newStart},${newCount} @@`,
    })
    for (const line of hunk.lines) {
      if (line.kind === 'add') addCount += 1
      if (line.kind === 'del') delCount += 1
      lines.push({
        kind: line.kind,
        text: line.text,
        oldNo: line.oldNo ?? undefined,
        newNo: line.newNo ?? undefined,
      })
    }
  }
  return renderCodexPatchSectionsHtml([{
    op,
    path,
    lines,
    addCount,
    delCount,
  }], cwd)
}

/** Render Pi's native `edit` / `write` arguments using the same file-diff card
 * as Codex. Pi persists replacement text rather than a unified patch. */
export function renderPiFileChangeHtml(input: string, cwd?: string): string | null {
  let value: Record<string, unknown>
  try {
    const parsed: unknown = JSON.parse(input)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null
    value = parsed as Record<string, unknown>
  } catch {
    return null
  }

  const path = typeof value.path === 'string' ? value.path : ''
  if (!path) return null
  const edits = Array.isArray(value.edits) ? value.edits : []
  const hunks: DiffHunk[] = []

  if (edits.length > 0) {
    for (const raw of edits) {
      if (!raw || typeof raw !== 'object' || Array.isArray(raw)) continue
      const edit = raw as Record<string, unknown>
      if (typeof edit.oldText !== 'string' || typeof edit.newText !== 'string') continue
      hunks.push(piTextDiffHunk(edit.oldText, edit.newText))
    }
  } else if (typeof value.content === 'string') {
    hunks.push(piTextDiffHunk('', value.content))
  }

  if (!hunks.length) return null
  return renderCodexFileChangeHtml(hunks, path, edits.length ? 'update' : 'add', cwd)
}

function piTextDiffHunk(oldText: string, newText: string): DiffHunk {
  const oldLines = splitDiffLines(oldText)
  const newLines = splitDiffLines(newText)
  const useLcs = oldLines.length * newLines.length <= 1_000_000
  const lcs = useLcs
    ? Array.from({ length: oldLines.length + 1 }, () => new Uint32Array(newLines.length + 1))
    : []
  if (useLcs) {
    for (let oi = oldLines.length - 1; oi >= 0; oi--) {
      for (let ni = newLines.length - 1; ni >= 0; ni--) {
        lcs[oi][ni] = oldLines[oi] === newLines[ni]
          ? lcs[oi + 1][ni + 1] + 1
          : Math.max(lcs[oi + 1][ni], lcs[oi][ni + 1])
      }
    }
  }

  const lines: DiffLine[] = []
  let oi = 0
  let ni = 0
  let oldNo = 1
  let newNo = 1
  while (oi < oldLines.length || ni < newLines.length) {
    if (oi < oldLines.length && ni < newLines.length && oldLines[oi] === newLines[ni]) {
      lines.push({ kind: 'ctx', text: oldLines[oi], oldNo, newNo })
      oi++; ni++; oldNo++; newNo++
    } else if (oi < oldLines.length && (!useLcs || ni === newLines.length || lcs[oi + 1][ni] >= lcs[oi][ni + 1])) {
      lines.push({ kind: 'del', text: oldLines[oi], oldNo, newNo: null })
      oi++; oldNo++
    } else if (ni < newLines.length) {
      lines.push({ kind: 'add', text: newLines[ni], oldNo: null, newNo })
      ni++; newNo++
    }
  }
  return { oldStart: oldLines.length ? 1 : 0, newStart: newLines.length ? 1 : 0, lines }
}

function splitDiffLines(text: string): string[] {
  if (!text) return []
  const lines = text.split('\n')
  // A trailing newline terminates the final line; it is not an extra blank line.
  if (lines[lines.length - 1] === '') lines.pop()
  return lines
}

function patchOpFromChangeType(changeType?: string): PatchOp | null {
  const op = changeType?.toLowerCase()
  if (!op) return null
  if (['add', 'added', 'create', 'created', 'new'].includes(op)) return 'add'
  if (['delete', 'deleted', 'remove', 'removed', 'del'].includes(op)) return 'delete'
  if (['update', 'updated', 'modify', 'modified', 'change', 'changed'].includes(op)) return 'update'
  return null
}

function patchOpFromHunks(hunks: DiffHunk[] | undefined): PatchOp {
  const lines = (hunks ?? []).flatMap((hunk) => hunk.lines)
  const addCount = lines.filter((line) => line.kind === 'add').length
  const delCount = lines.filter((line) => line.kind === 'del').length
  if (delCount === 0 && (hunks ?? []).some((hunk) => hunk.oldStart === 0)) return 'add'
  if (addCount === 0 && (hunks ?? []).some((hunk) => hunk.newStart === 0)) return 'delete'
  return 'update'
}

function renderCodexPatchSectionsHtml(sections: CodexPatchSection[], cwd?: string): string {
  return sections
    .map((section) => {
      const target = section.movedTo ?? section.path
      const visiblePath = displayPath(target, cwd)
      const stat = `+${section.addCount} -${section.delCount}`
      const body = section.lines.map(renderPatchLine).filter(Boolean).join('')
      return [
        '<div class="codex-patch-file">',
        '<div class="codex-patch-head">',
        `<a href="${escapeHtml(target)}" class="local-file-link codex-patch-path" data-local-file-link="1" data-local-target="${escapeHtml(target)}" title="${escapeHtml(target)}">${escapeHtml(visiblePath)}</a>`,
        `<span class="codex-patch-op">${opLabel(section.op)}</span>`,
        `<span class="codex-patch-stat"><span class="add">+${section.addCount}</span><span class="del">-${section.delCount}</span></span>`,
        '</div>',
        body
          ? `<div class="codex-patch-diff">${body}</div>`
          : `<div class="codex-patch-empty">${escapeHtml(stat)}</div>`,
        '</div>',
      ].join('')
    })
    .join('')
}
