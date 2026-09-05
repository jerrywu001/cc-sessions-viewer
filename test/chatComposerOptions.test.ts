import { describe, expect, it } from 'vitest'
import {
  CLAUDE_PERMISSION_MODES,
  CODEX_PERMISSION_MODES,
  permissionModesFor,
  defaultPermissionMode,
  CHAT_MODEL_MENU,
  CLAUDE_ALIAS_MODEL_MENU,
  CHAT_EFFORT_LEVELS,
  hasModelChoice,
  autoPickModel,
  requiresCredits,
  hasEffortChoice,
  effortLevelsFor,
  modelSupportsEffort,
  effectiveEffort,
  fallbackEffort,
  allModels,
  modelLabel,
  modelMenuFor,
  effortLabel,
  defaultModel,
  defaultEffort,
  permissionLabelKey,
  permissionModeDisabled,
  fallbackPermissionMode,
  chatSupported,
  sanitizeModel,
} from '../src/chatComposerOptions'

describe('chatComposerOptions', () => {
  it('Claude 权限五档，顺序对齐 Claude Code「Mode」菜单', () => {
    expect(CLAUDE_PERMISSION_MODES.map((m) => m.value)).toEqual([
      'default',
      'acceptEdits',
      'plan',
      'auto',
      'bypassPermissions',
    ])
  })

  it('Codex 权限五档，独立于 Claude', () => {
    expect(CODEX_PERMISSION_MODES.map((m) => m.value)).toEqual([
      'ask',
      'plan',
      'approve',
      'fullAccess',
      'custom',
    ])
  })

  it('permissionModesFor 按 agent 返回独立列表', () => {
    expect(permissionModesFor('claude')).toBe(CLAUDE_PERMISSION_MODES)
    expect(permissionModesFor('codex')).toBe(CODEX_PERMISSION_MODES)
  })

  it('defaultPermissionMode：Claude → bypassPermissions，Codex → fullAccess', () => {
    expect(defaultPermissionMode('claude')).toBe('bypassPermissions')
    expect(defaultPermissionMode('codex')).toBe('fullAccess')
  })

  it('permissionLabelKey：按 agent 从对应列表查找，未知回退首项', () => {
    expect(permissionLabelKey('claude', 'plan')).toBe('chat.composer.permission.plan')
    expect(permissionLabelKey('claude', 'nope')).toBe('chat.composer.permission.ask')
    expect(permissionLabelKey('codex', 'plan')).toBe('chat.composer.permission.plan')
    expect(permissionLabelKey('codex', 'fullAccess')).toBe('chat.composer.permission.codex.fullAccess')
    expect(permissionLabelKey('codex', 'nope')).toBe('chat.composer.permission.codex.ask')
  })

  it('Claude / Codex 有模型与 effort 候选', () => {
    expect(hasModelChoice('claude')).toBe(true)
    expect(hasModelChoice('codex')).toBe(true)
    expect(hasEffortChoice('claude')).toBe(true)
    expect(hasEffortChoice('codex')).toBe(true)
  })

  it('Claude 模型用完整标准 id（主列表 + More），且一律不带 [1m]', () => {
    expect(CHAT_MODEL_MENU.claude.primary.map((m) => m.value)).toEqual([
      'claude-fable-5-1',
      'claude-opus-5',
      'claude-sonnet-5',
      'claude-haiku-4-5-20251001',
    ])
    expect(CHAT_MODEL_MENU.claude.more.map((m) => m.value)).toEqual([
      'claude-fable-5',
      'claude-opus-4-8',
      'claude-sonnet-4-6',
      'claude-opus-4-7',
      'claude-opus-4-6',
    ])
  })

  it('Claude API-key 菜单改走 alias，让 Claude CLI 自己按 settings.json 做模型映射', () => {
    expect(CLAUDE_ALIAS_MODEL_MENU.primary.map((m) => m.value)).toEqual([
      'opus',
      'sonnet',
      'haiku',
      'fable',
    ])
    expect(modelMenuFor('claude', { claudeAliasMode: true }).primary.map((m) => m.value)).toEqual([
      'opus',
      'sonnet',
      'haiku',
      'fable',
    ])
  })

  it('Claude alias 菜单会把本地映射模型名拼到展示标签上', () => {
    expect(
      modelMenuFor('claude', {
        claudeAliasMode: true,
        claudeAliasTargets: { opus: 'mimo-v2.5-pro' },
      }).primary[0].label,
    ).toBe('Opus (mimo-v2.5-pro)')
  })

  it('autoPickModel：Fable 5.1 需 credits 不作新会话默认，订阅落到 Opus 5，alias 照常取 opus', () => {
    expect(requiresCredits('claude-fable-5-1')).toBe(true)
    expect(requiresCredits('claude-fable-5')).toBe(true)
    expect(requiresCredits('claude-opus-5')).toBe(false)
    // 订阅：primary[0] 是烧额度的 Fable 5.1 → 跳过 → 第一个不烧额度的 Opus 5
    expect(autoPickModel('claude')).toBe('claude-opus-5')
    // alias 模式：primary[0] 是 opus 别名（不烧额度）→ 照常返回
    expect(autoPickModel('claude', { claudeAliasMode: true })).toBe('opus')
  })

  it('关键回归：任何下发模型 id 都不含 [1m]（否则会触发 1M-context credits 报错）', () => {
    for (const agent of ['claude', 'codex', 'agy'] as const) {
      for (const m of allModels(agent)) {
        expect(m.value).not.toContain('[1m]')
      }
    }
    for (const m of allModels('claude', { claudeAliasMode: true })) {
      expect(m.value).not.toContain('[1m]')
    }
  })

  it('Claude effort 五档，Codex reasoning effort 四档', () => {
    expect(CHAT_EFFORT_LEVELS.claude).toEqual(['low', 'medium', 'high', 'xhigh', 'max'])
    expect(CHAT_EFFORT_LEVELS.codex).toEqual(['low', 'medium', 'high', 'xhigh'])
  })

  it('候选 value 仅 [A-Za-z0-9._-]（与后端 valid_flag_token 对齐，可被 posix_quote 安全转义）', () => {
    const c = CHAT_MODEL_MENU.claude
    const vals = [
      ...c.unavailable,
      ...c.primary,
      ...c.more,
      ...CHAT_MODEL_MENU.codex.primary,
      ...CHAT_MODEL_MENU.codex.more,
    ].map((o) => o.value)
    for (const v of [...vals, ...CHAT_EFFORT_LEVELS.claude, ...CHAT_EFFORT_LEVELS.codex]) {
      expect(v).toMatch(/^[A-Za-z0-9._-]+$/)
    }
  })

  it('modelLabel / effortLabel：命中返回展示名，未知回退原值', () => {
    expect(modelLabel('claude', 'claude-opus-5')).toBe('Opus 5')
    expect(modelLabel('claude', 'claude-opus-4-8')).toBe('Opus 4.8')
    expect(modelLabel('claude', 'claude-opus-4-7')).toBe('Opus 4.7')
    expect(modelLabel('claude', 'opus')).toBe('Opus')
    expect(modelLabel('claude', 'haiku')).toBe('Haiku')
    expect(modelLabel('claude', 'sonnet', { claudeAliasMode: true })).toBe('Sonnet')
    expect(modelLabel('claude', undefined)).toBe('')
    expect(modelLabel('claude', 'weird-id')).toBe('weird-id')
    expect(effortLabel('high')).toBe('High')
    expect(effortLabel(undefined)).toBe('')
  })

  it('effortLabel：特殊值走映射（xhigh → Extra High），其余首字母大写', () => {
    expect(effortLabel('xhigh')).toBe('Extra High')
    expect(effortLabel('ultracode')).toBe('Ultracode')
    expect(effortLabel('max')).toBe('Max')
    expect(effortLabel('low')).toBe('Low')
  })

  it('effortLevelsFor：Fable 5.1 / Fable 5 / Opus 5 / Opus 4.7 / 4.8 在 max 后多一档 ultracode，其余模型只有基础五档', () => {
    const base = ['low', 'medium', 'high', 'xhigh', 'max']
    expect(effortLevelsFor('claude', 'claude-fable-5-1')).toEqual([...base, 'ultracode'])
    expect(effortLevelsFor('claude', 'claude-fable-5')).toEqual([...base, 'ultracode'])
    expect(effortLevelsFor('claude', 'claude-opus-5')).toEqual([...base, 'ultracode'])
    expect(effortLevelsFor('claude', 'claude-opus-4-8')).toEqual([...base, 'ultracode'])
    expect(effortLevelsFor('claude', 'claude-opus-4-7')).toEqual([...base, 'ultracode'])
    expect(effortLevelsFor('claude', 'claude-opus-4-6')).toEqual(base)
    expect(effortLevelsFor('claude', 'claude-sonnet-5')).toEqual(base)
    expect(effortLevelsFor('claude', undefined)).toEqual(base)
    expect(effortLevelsFor('codex', 'gpt-5.4')).toEqual(['low', 'medium', 'high', 'xhigh'])
    expect(effortLevelsFor('codex', 'gpt-5.6-luna')).toEqual(['low', 'medium', 'high', 'xhigh', 'max'])
    expect(effortLevelsFor('codex', 'gpt-5.6-terra')).toEqual(['low', 'medium', 'high', 'xhigh', 'max', 'ultra'])
    expect(effortLevelsFor('codex', 'gpt-5.6-sol')).toEqual(['low', 'medium', 'high', 'xhigh', 'max', 'ultra'])
    expect(effortLevelsFor('codex', 'gpt-6-astra')).toEqual(['low', 'medium', 'high', 'xhigh', 'max', 'ultra'])
  })

  it('modelSupportsEffort：Haiku 无 effort；Opus/Sonnet 有', () => {
    expect(modelSupportsEffort('claude', 'claude-opus-5')).toBe(true)
    expect(modelSupportsEffort('claude', 'claude-opus-4-8')).toBe(true)
    expect(modelSupportsEffort('claude', 'claude-sonnet-5')).toBe(true)
    expect(modelSupportsEffort('claude', 'claude-haiku-4-5-20251001')).toBe(false)
    // 未指定模型时按「支持」处理（滑杆默认展示）。
    expect(modelSupportsEffort('claude', undefined)).toBe(true)
  })

  it('effectiveEffort：Haiku 抹掉 effort；ultracode 落到 max（headless 天花板）；其余透传', () => {
    expect(effectiveEffort('claude', 'claude-opus-5', 'high')).toBe('high')
    expect(effectiveEffort('claude', 'claude-opus-5', 'ultracode')).toBe('max')
    expect(effectiveEffort('claude', 'claude-opus-4-8', 'high')).toBe('high')
    expect(effectiveEffort('claude', 'claude-opus-4-8', 'ultracode')).toBe('max')
    expect(effectiveEffort('claude', 'claude-haiku-4-5-20251001', 'high')).toBeUndefined()
  })

  it('fallbackEffort：切到不支持当前档的模型 → 退最高可用档；否则原样', () => {
    // Opus 的 ultracode 切到 Sonnet（无 ultracode）→ 退到 max。
    expect(fallbackEffort('ultracode', 'claude', 'claude-sonnet-5')).toBe('max')
    // 档位在新模型下仍存在 → 原样。
    expect(fallbackEffort('high', 'claude', 'claude-opus-5')).toBe('high')
    expect(fallbackEffort('high', 'claude', 'claude-opus-4-8')).toBe('high')
    expect(fallbackEffort('ultracode', 'claude', 'claude-opus-4-7')).toBe('ultracode')
    expect(fallbackEffort(undefined, 'claude', 'claude-sonnet-5')).toBeUndefined()
  })

  it('Claude: Haiku 不支持 auto 权限模式；其它模型不受限', () => {
    expect(permissionModeDisabled('claude', 'auto', 'claude-haiku-4-5-20251001')).toBe(true)
    expect(permissionModeDisabled('claude', 'auto', 'claude-opus-5')).toBe(false)
    expect(permissionModeDisabled('claude', 'auto', 'claude-opus-4-8')).toBe(false)
    expect(permissionModeDisabled('claude', 'auto', 'claude-sonnet-5')).toBe(false)
    expect(permissionModeDisabled('claude', 'acceptEdits', 'claude-haiku-4-5-20251001')).toBe(false)
    expect(permissionModeDisabled('claude', 'auto', undefined)).toBe(false)
  })

  it('Codex: 权限模式无禁用限制', () => {
    expect(permissionModeDisabled('codex', 'fullAccess', 'gpt-5.5')).toBe(false)
    expect(permissionModeDisabled('codex', 'ask', 'gpt-5.4')).toBe(false)
  })

  it('fallbackPermissionMode：Claude Haiku+auto → bypassPermissions，其余原样返回', () => {
    expect(fallbackPermissionMode('claude', 'auto', 'claude-haiku-4-5-20251001')).toBe('bypassPermissions')
    expect(fallbackPermissionMode('claude', 'auto', 'claude-opus-5')).toBe('auto')
    expect(fallbackPermissionMode('claude', 'auto', 'claude-opus-4-8')).toBe('auto')
    expect(fallbackPermissionMode('claude', 'plan', 'claude-haiku-4-5-20251001')).toBe('plan')
    expect(fallbackPermissionMode('codex', 'fullAccess', 'gpt-5.5')).toBe('fullAccess')
  })

  it('defaultModel / defaultEffort：明确起步值（无 "default" 概念）', () => {
    expect(defaultModel('claude')).toBeUndefined()
    expect(defaultModel('codex')).toBe('gpt-5.5')
    expect(defaultEffort('claude')).toBeUndefined()
    expect(defaultEffort('codex')).toBe('high')
  })

  it('Codex 模型列表：6-astra 为首、旧模型在 More', () => {
    expect(CHAT_MODEL_MENU.codex.primary.map((m) => m.value)).toEqual([
      'gpt-6-astra',
      'gpt-5.6-sol',
      'gpt-5.6-terra',
      'gpt-5.6-luna',
      'gpt-5.5',
    ])
    expect(CHAT_MODEL_MENU.codex.more.map((m) => m.value)).toEqual([
      'gpt-5.4',
      'gpt-5.4-mini',
      'gpt-5.3-codex-spark',
    ])
    expect(CHAT_MODEL_MENU.codex.showFastMode).toBe(false)
  })

  it('Codex modelLabel 返回展示名', () => {
    expect(modelLabel('codex', 'gpt-6-astra')).toBe('GPT-6-Astra')
    expect(modelLabel('codex', 'gpt-5.5')).toBe('GPT-5.5')
    expect(modelLabel('codex', 'gpt-5.4-mini')).toBe('GPT-5.4-Mini')
    expect(modelLabel('codex', 'unknown-model')).toBe('unknown-model')
  })

  it('chatSupported：Claude 和 Codex 支持 chat，Grok Build/agy/opencode 不支持', () => {
    expect(chatSupported('claude')).toBe(true)
    expect(chatSupported('codex')).toBe(true)
    expect(chatSupported('grok')).toBe(false)
    expect(chatSupported('agy')).toBe(false)
    expect(chatSupported('opencode')).toBe(false)
    expect(CHAT_MODEL_MENU.grok.primary).toEqual([])
    expect(CHAT_EFFORT_LEVELS.grok).toEqual([])
  })

  it('Codex 权限模式 value 仅含安全字符', () => {
    for (const m of CODEX_PERMISSION_MODES) {
      expect(m.value).toMatch(/^[A-Za-z0-9._-]+$/)
    }
  })

  describe('sanitizeModel — 旧数据的幽灵模型回退', () => {
    it('codex 记忆的已下架模型 gpt-5.3-codex → 回退 gpt-5.5', () => {
      expect(sanitizeModel('codex', 'gpt-5.3-codex')).toBe('gpt-5.5')
    })

    it('codex 任意不在菜单的模型 → 回退 gpt-5.5(= defaultModel)', () => {
      expect(sanitizeModel('codex', 'gpt-4o')).toBe(defaultModel('codex'))
      expect(sanitizeModel('codex', 'totally-unknown')).toBe('gpt-5.5')
    })

    it('codex 在菜单内的模型原样保留(primary 与 more 都算)', () => {
      expect(sanitizeModel('codex', 'gpt-6-astra')).toBe('gpt-6-astra')
      expect(sanitizeModel('codex', 'gpt-5.6-sol')).toBe('gpt-5.6-sol')
      expect(sanitizeModel('codex', 'gpt-5.5')).toBe('gpt-5.5')
      expect(sanitizeModel('codex', 'gpt-5.4-mini')).toBe('gpt-5.4-mini')
      expect(sanitizeModel('codex', 'gpt-5.3-codex-spark')).toBe('gpt-5.3-codex-spark')
    })

    it('claude 不在菜单的模型 → 回退 opus-5', () => {
      expect(sanitizeModel('claude', 'claude-opus-4-5')).toBe('claude-opus-5')
      expect(sanitizeModel('claude', 'gpt-5.3-codex')).toBe('claude-opus-5')
    })

    it('claude 在菜单内(含 alias 档)的模型原样保留', () => {
      expect(sanitizeModel('claude', 'claude-opus-5')).toBe('claude-opus-5')
      expect(sanitizeModel('claude', 'claude-opus-4-8')).toBe('claude-opus-4-8')
      expect(sanitizeModel('claude', 'claude-sonnet-5')).toBe('claude-sonnet-5')
      expect(sanitizeModel('claude', 'opus')).toBe('opus')
    })

    it('空/undefined 原样返回(交给上层 ?? defaultModel 处理)', () => {
      expect(sanitizeModel('codex', undefined)).toBeUndefined()
      expect(sanitizeModel('claude', undefined)).toBeUndefined()
      expect(sanitizeModel('codex', '')).toBe('')
    })

    it('composer 的 effectiveModel 复现:codex 记忆幽灵模型时不会停在幽灵上', () => {
      // startChat: session.model = sanitizeModel(agent, opts.model) ?? defaultModel(agent)
      const sessionModel = sanitizeModel('codex', 'gpt-5.3-codex') ?? defaultModel('codex')
      // ChatComposer: effectiveModel = session.model ?? session.lastModel
      const effectiveModel = sessionModel ?? undefined
      expect(effectiveModel).toBe('gpt-5.5')
      // 确认结果确实在当前菜单里(能被选择器高亮、能发出去)
      const codexValues = [...CHAT_MODEL_MENU.codex.primary, ...CHAT_MODEL_MENU.codex.more].map((m) => m.value)
      expect(codexValues).toContain(effectiveModel)
    })
  })
})
