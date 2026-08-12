/** Chat 正文内联图片占位符的纯函数辅助工具。 */

const IMAGE_TOKEN_RE = /\[Image #(\d+)\]/g

export function inlineImagePlaceholders(text: string): string[] {
  const out: string[] = []
  const seen = new Set<string>()
  for (const match of text.matchAll(IMAGE_TOKEN_RE)) {
    const token = match[0]
    if (!seen.has(token)) {
      seen.add(token)
      out.push(token)
    }
  }
  return out
}

export function nextInlineImageNumber(text: string, start = 1): number {
  let next = Math.max(1, start)
  for (const match of text.matchAll(IMAGE_TOKEN_RE)) {
    const n = Number(match[1])
    if (Number.isSafeInteger(n)) next = Math.max(next, n + 1)
  }
  return next
}

/** 删除一个正文 token，并返回删除后的文本与原 token 起点。 */
export function removeInlineImagePlaceholder(text: string, placeholder: string): { text: string; start: number } | null {
  const start = text.indexOf(placeholder)
  if (start < 0) return null
  return { text: text.slice(0, start) + text.slice(start + placeholder.length), start }
}

/** 将历史消息中的图片按正文 token 出现顺序建立绑定。 */
export function bindInlineImagePlaceholders<T extends { inlinePlaceholder?: string }>(text: string, images: T[]): T[] {
  const placeholders = inlineImagePlaceholders(text)
  // A live/draft message may contain both ordinary top attachments and inline
  // attachments. Explicit bindings are authoritative; do not accidentally
  // assign a legacy placeholder to an ordinary attachment in that mixed case.
  if (images.some((image) => image.inlinePlaceholder)) return images
  const used = new Set(images.map((image) => image.inlinePlaceholder).filter(Boolean))
  let next = 0
  return images.map((image) => {
    if (image.inlinePlaceholder) return image
    while (next < placeholders.length && used.has(placeholders[next])) next += 1
    const placeholder = placeholders[next++]
    if (!placeholder) return image
    used.add(placeholder)
    return { ...image, inlinePlaceholder: placeholder }
  })
}

/**
 * 将旧版混合附件消息中的图片绑定到正文 token。
 *
 * Codex 的旧 rollout 用全部附件的序号命名 `[Image #N]`，而不是只数图片；
 * 因此图片需要带着它在 file/image 附件列表中的 1-based 位置参与绑定。
 */
export function bindInlineImagePlaceholdersAtAttachmentPositions<T extends { inlinePlaceholder?: string }>(
  text: string,
  images: T[],
  positions: number[],
): T[] {
  const placeholderList = inlineImagePlaceholders(text)
  // Explicit source bindings are authoritative. This is important for Claude
  // transcripts where [Image #N] follows an interleaved image block and N is not
  // the overall file/image attachment position.
  if (images.some((image) => image.inlinePlaceholder)) return images
  const placeholders = new Set(placeholderList)
  const used = new Set(images.map((image) => image.inlinePlaceholder).filter(Boolean))
  let boundByPosition = false
  const bound = images.map((image, index) => {
    if (image.inlinePlaceholder) return image
    const position = positions[index]
    if (!Number.isSafeInteger(position) || position < 1) return image
    const placeholder = `[Image #${position}]`
    if (!placeholders.has(placeholder) || used.has(placeholder)) return image
    used.add(placeholder)
    boundByPosition = true
    return { ...image, inlinePlaceholder: placeholder }
  })
  // Claude 的旧 transcript 使用跨消息的图片编号，无法用本消息的附件位置还原。
  // 只有完全没有位置匹配时才按 token 顺序回退；Codex 混合附件一旦有精确匹配，
  // 普通图片就不会被误绑定。
  if (boundByPosition) return bound
  let next = 0
  return bound.map((image) => {
    if (image.inlinePlaceholder) return image
    while (next < placeholderList.length && used.has(placeholderList[next])) next += 1
    const placeholder = placeholderList[next++]
    if (!placeholder) return image
    used.add(placeholder)
    return { ...image, inlinePlaceholder: placeholder }
  })
}
