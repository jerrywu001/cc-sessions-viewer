import { describe, expect, it } from 'vitest'
import {
  allQuestionsAnswered,
  buildQuestionCancelDecision,
  buildQuestionDecision,
  parseQuestionAnswers,
  parseQuestionRequest,
  questionAnswered,
  questionHasPreview,
  type QuestionSelection,
} from '../src/chatQuestion'
import type { ChatQuestionItem, ChatQuestionRequest } from '../src/types'

const req = (questions: ChatQuestionItem[]): ChatQuestionRequest => ({ requestId: 'r1', questions })

const single: ChatQuestionItem = {
  question: 'Pick a language',
  header: 'Language',
  options: [
    { label: 'TypeScript', description: 'typed JS' },
    { label: 'Rust', description: 'systems' },
  ],
}
const multi: ChatQuestionItem = {
  question: 'Which have you used?',
  multiSelect: true,
  options: [{ label: 'Python' }, { label: 'Go' }, { label: 'Rust' }],
}

describe('buildQuestionDecision — answer encoding', () => {
  it('maps a single-select pick to answers[question] = label', () => {
    const sel: QuestionSelection[] = [{ labels: ['Rust'] }]
    const d = buildQuestionDecision(req([single]), sel)
    expect(d).toEqual({
      behavior: 'allow',
      updatedInput: { questions: [single], answers: { 'Pick a language': 'Rust' } },
    })
  })

  it('comma-joins multi-select picks (label order preserved)', () => {
    const sel: QuestionSelection[] = [{ labels: ['Python', 'Go'] }]
    const d = buildQuestionDecision(req([multi]), sel) as any
    expect(d.updatedInput.answers).toEqual({ 'Which have you used?': 'Python, Go' })
  })

  it('encodes Other free-text into answers[question]', () => {
    const sel: QuestionSelection[] = [{ labels: [], otherText: "I'd rather use Go" }]
    const d = buildQuestionDecision(req([single]), sel) as any
    expect(d.updatedInput.answers).toEqual({ 'Pick a language': "I'd rather use Go" })
  })

  it('joins structured labels with Other text for multi-select', () => {
    const sel: QuestionSelection[] = [{ labels: ['Python'], otherText: 'Zig' }]
    const d = buildQuestionDecision(req([multi]), sel) as any
    expect(d.updatedInput.answers).toEqual({ 'Which have you used?': 'Python, Zig' })
  })

  it('omits unanswered questions and trims blank Other text', () => {
    const two = req([single, multi])
    const sel: QuestionSelection[] = [{ labels: ['TypeScript'] }, { labels: [], otherText: '   ' }]
    const d = buildQuestionDecision(two, sel) as any
    expect(d.updatedInput.answers).toEqual({ 'Pick a language': 'TypeScript' })
  })

  it('echoes the original questions back in updatedInput', () => {
    const r = req([single])
    const d = buildQuestionDecision(r, [{ labels: ['Rust'] }]) as any
    expect(d.updatedInput.questions).toBe(r.questions)
  })
})

describe('buildQuestionCancelDecision', () => {
  it('denies without interrupting the turn', () => {
    expect(buildQuestionCancelDecision()).toEqual({
      behavior: 'deny',
      message: 'The user declined to answer the question.',
      interrupt: false,
    })
  })
})

describe('questionAnswered / allQuestionsAnswered', () => {
  it('treats a question with no labels and no Other as unanswered', () => {
    expect(questionAnswered({ labels: [] })).toBe(false)
    expect(questionAnswered({ labels: [], otherText: '' })).toBe(false)
    expect(questionAnswered(undefined)).toBe(false)
  })

  it('treats any label or non-blank Other as answered', () => {
    expect(questionAnswered({ labels: ['Rust'] })).toBe(true)
    expect(questionAnswered({ labels: [], otherText: 'x' })).toBe(true)
  })

  it('requires every question answered', () => {
    const r = req([single, multi])
    expect(allQuestionsAnswered(r, [{ labels: ['Rust'] }, { labels: [] }])).toBe(false)
    expect(allQuestionsAnswered(r, [{ labels: ['Rust'] }, { labels: ['Go'] }])).toBe(true)
  })
})

describe('questionHasPreview — two-pane gating', () => {
  it('is true for a single-select question with a non-empty preview', () => {
    expect(
      questionHasPreview({
        question: 'q',
        options: [{ label: 'A', preview: 'code' }, { label: 'B' }],
      }),
    ).toBe(true)
  })

  it('is false for multi-select even with previews (preview is single-select only)', () => {
    expect(
      questionHasPreview({
        question: 'q',
        multiSelect: true,
        options: [{ label: 'A', preview: 'code' }],
      }),
    ).toBe(false)
  })

  it('is false when no option carries a preview', () => {
    expect(questionHasPreview(single)).toBe(false)
  })
})

describe('history transcript parsing', () => {
  it('normalizes an AskUserQuestion tool input for the read-only card', () => {
    const parsed = parseQuestionRequest(
      JSON.stringify({
        questions: [
          {
            header: 'Frameworks',
            question: 'Which have you used?',
            multiSelect: true,
            options: [{ label: 'Vue', description: 'UI' }, { label: 'React' }],
          },
        ],
      }),
      'tool-1',
    )
    expect(parsed).toEqual({
      requestId: 'tool-1',
      questions: [
        {
          header: 'Frameworks',
          question: 'Which have you used?',
          multiSelect: true,
          options: [{ label: 'Vue', description: 'UI' }, { label: 'React' }],
        },
      ],
    })
  })

  it('rejects malformed tool input without throwing', () => {
    expect(parseQuestionRequest('{bad json}')).toBeNull()
    expect(parseQuestionRequest(JSON.stringify({ questions: [{ question: 'empty', options: [] }] }))).toBeNull()
  })

  it('extracts answers from Claude tool-result text', () => {
    expect(
      parseQuestionAnswers(
        'Your questions have been answered: "Frameworks"="Vue,React", "Build tool"="Vite".',
      ),
    ).toEqual({ Frameworks: 'Vue,React', 'Build tool': 'Vite' })
  })
})

describe('Kimi AskUserQuestion history parsing', () => {
  it('normalizes Kimi snake_case multi-select questions', () => {
    const request = parseQuestionRequest(
      JSON.stringify({
        background: true,
        questions: [
          {
            question: 'Which modes?',
            header: 'Mode',
            multi_select: true,
            options: [
              { label: 'Safe', description: 'No writes' },
              { label: 'Fast' },
            ],
          },
        ],
      }),
      'kimi-call',
    )

    expect(request).toMatchObject({
      requestId: 'kimi-call',
      background: true,
      questions: [{ question: 'Which modes?', multiSelect: true }],
    })
    expect(request?.questions[0].options).toHaveLength(2)
  })

  it('rejects malformed or ambiguous historical question payloads', () => {
    expect(
      parseQuestionRequest(JSON.stringify({ questions: [{ question: 'Only one', options: [{ label: 'A' }] }] })),
    ).toBeNull()
    expect(
      parseQuestionRequest(
        JSON.stringify({ questions: [{ question: 'Duplicate', options: [{ label: 'A' }, { label: 'A' }] }] }),
      ),
    ).toBeNull()
  })

  it('accepts Kimi JSON answers before Claude legacy answers', () => {
    expect(parseQuestionAnswers('{"answers":{"Which modes?":"Safe, Fast"},"note":"done"}')).toEqual({
      'Which modes?': 'Safe, Fast',
    })
    expect(parseQuestionAnswers('"Framework"="Vue,React"')).toEqual({ Framework: 'Vue,React' })
    expect(parseQuestionAnswers('{"answers":{"Which modes?":true}}')).toEqual({})
  })
})
