import { t } from './i18n'

/**
 * 把单会话锁和 Codex app-server 的底层错误翻译成用户能理解的提示。
 * 未知错误原样返回，避免丢失排障信息。
 */
export function humanizeSessionError(error: unknown): string {
  const raw = String(error)
  const message = raw.replace(/^Error:\s*/i, '')

  if (/already open in GUI chat/i.test(message)) {
    return t('session.error.inUseChat')
  }
  if (/already open in (?:the )?in-app terminal/i.test(message)) {
    return t('session.error.inUseTerminal')
  }
  if (/already has an active writer|active writer/i.test(message)) {
    return t('session.error.activeWriter')
  }
  return raw
}

/**
 * 内嵌终端没有 Chat 的“重试”按钮，因此占用错误要明确告诉用户：先释放占用，
 * 再关闭当前终端并重新打开会话。普通 Chat 仍使用 humanizeSessionError 的短文案。
 */
export function humanizeTerminalSessionError(error: unknown): string {
  const raw = String(error)
  const message = raw.replace(/^Error:\s*/i, '')

  if (/already open in GUI chat/i.test(message)) {
    return t('tui.sessionInUseChat')
  }
  if (/already open in (?:the )?in-app terminal/i.test(message)) {
    return t('tui.sessionInUseTerminal')
  }
  if (/already has an active writer|active writer/i.test(message)) {
    return t('tui.codexActiveWriter')
  }
  return humanizeSessionError(error)
}

/** 恢复失败时保留原有上下文前缀；已知锁错误使用完整的人话文案。 */
export function humanizeRestoreError(error: unknown): string {
  const raw = String(error)
  const friendly = humanizeSessionError(raw)
  return friendly === raw ? `Unable to restore this chat: ${raw}` : friendly
}

/**
 * 只有单会话锁冲突可以通过“重试”解决：用户关闭占用它的 Chat、终端或外部
 * Codex 客户端后，重新发起一次 resume 即可。其它启动错误仍应保留原错误提示，
 * 避免按钮给用户一个实际上无效的操作。
 */
export function isRetryableSessionError(error?: string): boolean {
  if (!error) return false

  // 正常情况下 error 是当前语言的完整翻译；直接比较 key 可以避免依赖文案细节。
  if (
    error === t('session.error.inUseChat') ||
    error === t('session.error.inUseTerminal') ||
    error === t('session.error.activeWriter')
  ) {
    return true
  }

  // 错误发生后用户可能才切换语言，或旧状态里保存的是英文/其它语言文案。
  // 这些模式只覆盖单会话锁错误，不把普通“启动失败”误判成可重试。
  return /(?:already open in .*?(?:chat|terminal)|active writer|应用内\s*Chat|应用内终端|其他 Codex 进程|應用程式\s*Chat|應用程式終端機|其他 Codex 程序|アプリ内\s*Chat|アプリ内ターミナル|別の Codex プロセス|使用中です)/i.test(error)
}
