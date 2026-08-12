import type { ChatTextElement } from './types'

/**
 * 行内 @ 文件引用的轻量解析器。
 *
 * @ 文件属于正文语义，不应被当成普通附件从正文中移走。这里保留原始
 * token 的字符范围，供输入框高亮、Codex textElements 和消息气泡渲染共用。
 */
export interface InlineFileMention {
  start: number
  end: number
  token: string
  path: string
  name: string
  isQuoted: boolean
}

function isTokenBoundary(char: string | undefined): boolean {
  return !char || /\s/.test(char)
}

function baseName(path: string): string {
  return path.replace(/[/\\]+$/, '').split(/[/\\]/).pop() || path
}

function looksLikeFilePath(path: string): boolean {
  if (!path || path.includes('://')) return false
  return (
    path.startsWith('/') ||
    path.startsWith('~/') ||
    path.startsWith('./') ||
    path.startsWith('../') ||
    path.includes('/') ||
    /^[A-Za-z]:[\\/]/.test(path) ||
    /\.[A-Za-z][\w-]*$/.test(path)
  )
}

/**
 * Parse @"path with spaces" and unquoted @path tokens.
 * Unquoted tokens intentionally require a path-like shape so ordinary @user
 * mentions and Codex plugin mentions remain ordinary text.
 */
export function inlineFileMentions(text: string): InlineFileMention[] {
  const out: InlineFileMention[] = []
  let i = 0
  while (i < text.length) {
    if (text[i] !== '@' || (i > 0 && !/\s/.test(text[i - 1]))) {
      i += 1
      continue
    }

    const next = text[i + 1]
    if (next === '"') {
      const close = text.indexOf('"', i + 2)
      if (close > i + 2) {
        const path = text.slice(i + 2, close)
        if (path && isTokenBoundary(text[close + 1])) {
          out.push({
            start: i,
            end: close + 1,
            token: text.slice(i, close + 1),
            path,
            name: baseName(path),
            isQuoted: true,
          })
          i = close + 1
          continue
        }
      }
    } else {
      let end = i + 1
      while (end < text.length && !/\s/.test(text[end])) end += 1
      const raw = text.slice(i + 1, end)
      // Keep the boundary rule aligned with the mention popup. A path can be
      // glued to CJK punctuation/text, but a plain @word must not be styled.
      let path = raw.replace(/[，。！？；：、）】》'”]+$/u, '')
      // For the common `@src/file.ts的实现` form, stop at the ASCII file
      // extension before the CJK prose. Existing paths with CJK names remain
      // supported when they are separated by whitespace or quoted.
      const extension = path.match(/^(.+\.[A-Za-z][\w-]*)(?=[^\w-]|$)/)
      if (extension) path = extension[1]
      if (path && looksLikeFilePath(path)) {
        const tokenEnd = i + 1 + path.length
        out.push({
          start: i,
          end: tokenEnd,
          token: text.slice(i, tokenEnd),
          path,
          name: baseName(path),
          isQuoted: false,
        })
        i = tokenEnd
        continue
      }
    }
    i += 1
  }
  return out
}

export function formatInlineFileMention(path: string): string {
  const bareAmbiguous = !/[\\/]/.test(path) && !/\.[A-Za-z][\w-]*$/.test(path)
  return /\s/.test(path) || bareAmbiguous ? `@"${path}"` : `@${path}`
}

export function utf8ByteLength(text: string): number {
  return new TextEncoder().encode(text).length
}

export function inlineFileMentionTextElements(text: string, byteOffset = 0): ChatTextElement[] {
  return inlineFileMentions(text).map((mention) => ({
    type: 'mention',
    byteRange: {
      start: byteOffset + utf8ByteLength(text.slice(0, mention.start)),
      end: byteOffset + utf8ByteLength(text.slice(0, mention.end)),
    },
    path: mention.path,
    name: mention.name,
    placeholder: mention.token,
  }))
}
