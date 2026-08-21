// 模型向用户提的结构化选择题（Claude `AskUserQuestion`，走 `--permission-prompt-tool stdio`
// 的 `can_use_tool` 控制协议）的纯逻辑：把用户在卡片上的选择构造成 CLI 控制协议的 decision，
// 以及一些判定（是否多选 / 是否并排预览 / 是否答完）。不依赖 Vue / Tauri，便于单测；
// ChatView / ChatQuestionPrompt 与 chatSessions 共用。
//
// 答案编码（经实测确认，见会话记录）：
//   answers[问题文本] = "选项label"        —— 单选
//   answers[问题文本] = "labelA, labelB"   —— 多选（逗号+空格拼接）
//   Other 自填文本也并入 answers[问题文本]（而非顶层 `response`）—— 顶层 `response` 会在
//     tool_result 文案里「覆盖」掉结构化答案，多问题场景会吞掉别的答案，故统一走 answers。

import type { ChatQuestionItem, ChatQuestionRequest } from './types'

const MAX_QUESTION_INPUT_LENGTH = 64 * 1024
const MAX_QUESTION_TEXT_LENGTH = 8 * 1024
const MAX_OPTION_LABEL_LENGTH = 1024
const MAX_OPTION_DESCRIPTION_LENGTH = 8 * 1024

function shortString(value: unknown, maximum: number): string | null {
  return typeof value === 'string' && value.trim() && value.length <= maximum ? value.trim() : null
}

/**
 * 从 Claude transcript 里的 AskUserQuestion tool_use input 提取一个安全的
 * 只读请求。历史记录中的 input 是 JSON 字符串，不能直接把它当成完整的
 * ChatQuestionRequest 透传给组件：requestId 不在 input 里，且异常记录也可能
 * 缺少 questions。
 */
export function parseQuestionRequest(
  input: string,
  requestId = 'history-question',
): ChatQuestionRequest | null {
  let raw: unknown
  try {
    raw = JSON.parse(input)
  } catch {
    return null
  }
  if (
    input.length > MAX_QUESTION_INPUT_LENGTH ||
    !raw ||
    typeof raw !== 'object' ||
    Array.isArray(raw) ||
    !Array.isArray((raw as { questions?: unknown }).questions)
  ) {
    return null
  }

  const questions = (raw as { questions: unknown[] }).questions
  if (questions.length < 1 || questions.length > 4) return null
  const questionTexts = new Set<string>()
  let invalid = false
  const normalizedQuestions = questions
    .map((item): ChatQuestionItem | null => {
      if (!item || typeof item !== 'object') return null
      const q = item as Record<string, unknown>
      const question = shortString(q.question, MAX_QUESTION_TEXT_LENGTH)
      if (!question || questionTexts.has(question) || !Array.isArray(q.options)) {
        invalid = true
        return null
      }
      questionTexts.add(question)
      if (q.options.length < 2 || q.options.length > 4) {
        invalid = true
        return null
      }
      const labels = new Set<string>()
      const options = q.options
        .map((option): ChatQuestionItem['options'][number] | null => {
          if (!option || typeof option !== 'object') return null
          const o = option as Record<string, unknown>
          const label = shortString(o.label, MAX_OPTION_LABEL_LENGTH)
          if (!label || labels.has(label)) {
            invalid = true
            return null
          }
          labels.add(label)
          return {
            label,
            ...(shortString(o.description, MAX_OPTION_DESCRIPTION_LENGTH)
              ? { description: shortString(o.description, MAX_OPTION_DESCRIPTION_LENGTH)! }
              : {}),
            ...(shortString(o.preview, MAX_OPTION_DESCRIPTION_LENGTH)
              ? { preview: shortString(o.preview, MAX_OPTION_DESCRIPTION_LENGTH)! }
              : {}),
          }
        })
        .filter((option): option is ChatQuestionItem['options'][number] => option !== null)
      if (invalid || options.length !== q.options.length) return null
      return {
        question,
        ...(shortString(q.header, MAX_OPTION_LABEL_LENGTH)
          ? { header: shortString(q.header, MAX_OPTION_LABEL_LENGTH)! }
          : {}),
        ...(q.multiSelect === true || q.multi_select === true ? { multiSelect: true } : {}),
        ...(q.allowOther === false ? { allowOther: false } : {}),
        options,
      }
    })
    .filter((question): question is ChatQuestionItem => question !== null)

  if (invalid || normalizedQuestions.length !== questions.length) return null
  return {
    requestId,
    questions: normalizedQuestions,
    ...((raw as { background?: unknown }).background === true ? { background: true } : {}),
  }
}

/**
 * 解析 Claude 写入 tool_result 的简短答案回显，例如：
 * `"Frameworks"="Vue,React", "Build tool"="Vite"`。
 * 仅用于历史卡片标记已选项，解析失败时返回空对象，不影响消息展示。
 */
export function parseQuestionAnswers(text: string): Record<string, string> {
  if (text.length <= MAX_QUESTION_INPUT_LENGTH) {
    try {
      const value: unknown = JSON.parse(text)
      if (value && typeof value === 'object' && !Array.isArray(value)) {
        const answers = (value as { answers?: unknown }).answers
        if (answers && typeof answers === 'object' && !Array.isArray(answers)) {
          const result: Record<string, string> = {}
          for (const [question, answer] of Object.entries(answers)) {
            if (typeof answer !== 'string') return {}
            result[question] = answer
          }
          return result
        }
      }
    } catch {
      // Claude's legacy response is not JSON. Fall through to its quoted pairs.
    }
  }
  const answers: Record<string, string> = {}
  const pairRe = /"((?:\\.|[^"\\])*)"\s*=\s*"((?:\\.|[^"\\])*)"/g
  let match: RegExpExecArray | null
  while ((match = pairRe.exec(text)) !== null) {
    const unescape = (value: string) => value.replace(/\\"/g, '"').replace(/\\\\/g, '\\')
    answers[unescape(match[1])] = unescape(match[2])
  }
  return answers
}

/** 单条提问的用户选择：选中的结构化选项 label（单选 0–1 个、多选任意个）+ 可选的 Other 自填。 */
export interface QuestionSelection {
  /** 选中的结构化选项 `label`。 */
  labels: string[]
  /** 「Other」自填文本（选了 Other 才有；与 `labels` 一起逗号拼接成最终答案）。 */
  otherText?: string
}

/** 把一条选择折叠成最终答案串：结构化 label + Other 文本，去空后以 `, ` 拼接。 */
function answerText(sel: QuestionSelection): string {
  const parts = sel.labels.map((s) => s.trim()).filter(Boolean)
  const other = sel.otherText?.trim()
  if (other) parts.push(other)
  return parts.join(', ')
}

/** 该问题是否已作答（有任何结构化选项或非空 Other 文本）。submit 据此逐题门控。 */
export function questionAnswered(sel: QuestionSelection | undefined): boolean {
  return !!sel && answerText(sel).length > 0
}

/** 是否每条提问都已作答 —— 全部答完才允许提交。 */
export function allQuestionsAnswered(
  req: ChatQuestionRequest,
  selections: QuestionSelection[],
): boolean {
  return req.questions.every((_, i) => questionAnswered(selections[i]))
}

/** 该题是否走「并排预览」布局 —— 仅单选题、且至少一个选项带非空 `preview`。 */
export function questionHasPreview(q: ChatQuestionItem): boolean {
  return (
    !q.multiSelect &&
    q.options.some((o) => typeof o.preview === 'string' && o.preview.trim().length > 0)
  )
}

/**
 * 把用户的选择构造成 CLI 控制协议的 `decision`（作答）：
 *   `{behavior:'allow', updatedInput:{questions:<原样带回>, answers:{<问题文本>:<答案串>}}}`
 * 没作答的问题不进 `answers`（CLI 会当作「未回答该题」）。
 */
export function buildQuestionDecision(
  req: ChatQuestionRequest,
  selections: QuestionSelection[],
): Record<string, unknown> {
  const answers: Record<string, string> = {}
  req.questions.forEach((q, i) => {
    const text = answerText(selections[i] ?? { labels: [] })
    if (text) answers[q.question] = text
  })
  return {
    behavior: 'allow',
    updatedInput: { questions: req.questions, answers },
  }
}

/**
 * 取消作答的 `decision`：`{behavior:'deny', message, interrupt:false}` —— 把「用户没回答」
 * 反馈给模型，但不打断本轮（模型可换个方式继续）。
 */
export function buildQuestionCancelDecision(): Record<string, unknown> {
  return {
    behavior: 'deny',
    message: 'The user declined to answer the question.',
    interrupt: false,
  }
}
