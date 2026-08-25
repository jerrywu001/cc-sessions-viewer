export type PasteIntent = 'image' | 'text' | 'unified'

export type PasteKeyEvent = Pick<
  KeyboardEvent,
  'type' | 'key' | 'ctrlKey' | 'shiftKey' | 'altKey' | 'metaKey'
>

/** Keyboard intent for an embedded CLI only. Shell tabs must not call this. */
export function pasteIntentForEvent(
  event: PasteKeyEvent,
  platform = typeof navigator === 'undefined' ? '' : navigator.platform,
): PasteIntent | null {
  if (event.type !== 'keydown' || event.key.toLowerCase() !== 'v' || event.shiftKey || event.altKey) {
    return null
  }
  if (/Mac/i.test(platform)) {
    if (event.ctrlKey && !event.metaKey) return 'image'
    if (event.metaKey && !event.ctrlKey) return 'text'
    return null
  }
  return event.ctrlKey && !event.metaKey ? 'unified' : null
}

export interface ClipboardImage {
  data: string
  mediaType: string
}

export interface ClipboardReader {
  readImages?: () => Promise<ClipboardImage[]>
  readText: () => Promise<string>
}

export interface PasteTarget {
  paste: (value: string) => void
}

export interface PasteActions {
  saveImage: (data: string, mediaType: string) => Promise<string>
}

export type PasteResult = 'handled-image' | 'handled-text' | 'fallback' | 'failed'

function bytesToBase64(bytes: Uint8Array): string {
  let binary = ''
  const chunkSize = 0x8000
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize))
  }
  return btoa(binary)
}

/** Read image MIME entries without guessing from file names or text prefixes. */
export async function readClipboardImages(): Promise<ClipboardImage[]> {
  if (!navigator.clipboard?.read) return []
  const items = await navigator.clipboard.read()
  const images: ClipboardImage[] = []
  for (const item of items) {
    for (const type of item.types) {
      if (!type.startsWith('image/')) continue
      const blob = await item.getType(type)
      images.push({ data: bytesToBase64(new Uint8Array(await blob.arrayBuffer())), mediaType: type })
    }
  }
  return images
}

export async function runEmbeddedCliPaste(
  intent: PasteIntent,
  target: PasteTarget,
  reader: ClipboardReader,
  actions: PasteActions,
): Promise<PasteResult> {
  if (intent === 'image' || intent === 'unified') {
    try {
      const images = (await reader.readImages?.()) ?? []
      if (images.length > 0) {
        const image = images[0]
        const path = await actions.saveImage(image.data, image.mediaType)
        target.paste(path)
        return 'handled-image'
      }
    } catch {
      return 'failed'
    }
    if (intent === 'image') return 'fallback'
  }

  if (intent === 'text' || intent === 'unified') {
    try {
      const text = await reader.readText()
      if (text.length > 0) {
        target.paste(text)
        return 'handled-text'
      }
      return 'fallback'
    } catch {
      return 'failed'
    }
  }
  return 'fallback'
}
