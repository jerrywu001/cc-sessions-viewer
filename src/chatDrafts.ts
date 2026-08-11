import type { ChatHistoryEntry } from './chatInputHistory'
import type { ChatSession } from './chatSessions'

const drafts = new WeakMap<ChatSession, ChatHistoryEntry>()

export function setChatDraft(session: ChatSession, draft: ChatHistoryEntry | null) {
  if (draft) drafts.set(session, draft)
  else drafts.delete(session)
}

export function takeChatDraft(session: ChatSession): ChatHistoryEntry | null {
  const draft = drafts.get(session) ?? null
  drafts.delete(session)
  return draft
}
