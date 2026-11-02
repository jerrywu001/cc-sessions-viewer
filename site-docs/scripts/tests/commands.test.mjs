// Execute the actual documented jq filters / SQL on synthetic data only.
// No real agent roots, tokens, provider requests or resume commands are used.
import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync, mkdtempSync, rmSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'

const src = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const locales = ['', 'zh/', 'ja/']
function markdown(locale, agent) {
  return readFileSync(join(src, locale, 'agents', agent + '.md'), 'utf8')
}
function programs(text) {
  return [...text.matchAll(/\bjq(?:\s+-[a-zA-Z]+)*\s+'([^']+)'/g)].map((match) => match[1])
}
function jq(program, records) {
  const input = records.map((record) => JSON.stringify(record)).join('\n') + '\n'
  const result = spawnSync('jq', ['-r', program], { input, encoding: 'utf8' })
  assert.equal(result.error, undefined, 'Install jq to run documentation command tests')
  assert.equal(result.status, 0, result.stderr)
  return result.stdout
}
// Public downloads are the test inputs: keep examples and regressions in sync.
const sampleDir = join(src, 'public/examples')
const expected = JSON.parse(readFileSync(join(sampleDir, 'expected.json'), 'utf8'))
const primaryFilters = { 'claude-code': 0, codex: 0, 'grok-build': 1, 'kimi-code': 1, pi: 0, 'antigravity-cli': 2 }
const fixtures = Object.fromEntries(Object.keys(primaryFilters).map((agent) => [
  agent,
  readFileSync(join(sampleDir, agent + '.jsonl'), 'utf8').trim().split('\n').map((line) => JSON.parse(line)),
]))

for (const [agent, index] of Object.entries(primaryFilters)) {
  test(`${agent}: public expected output matches the documented filter`, () => {
    assert.equal(jq(programs(markdown('', agent))[index], fixtures[agent]), expected[agent])
  })
}
for (const locale of locales) {
  test(`${locale || 'en/'}task guides: Claude sample extraction and Codex metadata`, () => {
    const compatibility = readFileSync(join(src, locale, 'guide/compatibility.md'), 'utf8')
    const codexGuide = readFileSync(join(src, locale, 'guide/codex-session-viewer.md'), 'utf8')
    assert.equal(jq(programs(compatibility)[0], fixtures['claude-code']), expected['claude-code'])
    assert.equal(jq(programs(codexGuide)[0], fixtures.codex), '/synthetic/project\n')
  })
}

for (const agent of Object.keys(fixtures)) {
  test(`${agent}: translated jq filters are identical`, () => {
    const english = programs(markdown('', agent))
    assert.ok(english.length > 0)
    for (const locale of locales.slice(1)) assert.deepEqual(programs(markdown(locale, agent)), english)
  })
}
for (const locale of locales) {
  test(`${locale || 'en/'}Claude Code: string and block text`, () => {
    const filters = programs(markdown(locale, 'claude-code'))
    assert.equal(jq(filters[0], fixtures['claude-code']), 'Question\nAnswer\nLegacy string\n')
    assert.equal(jq(filters[1], fixtures['claude-code']), 'Read\n')
  })
  test(`${locale || 'en/'}Codex: nested cwd and no duplicate response items`, () => {
    const filters = programs(markdown(locale, 'codex'))
    assert.equal(jq(filters[0], fixtures.codex), 'Question\nAnswer\n')
    assert.equal(jq(filters[1], fixtures.codex), '/synthetic/project\n')
    assert.equal(jq(filters[2], fixtures.codex), '/synthetic/project\n')
  })
  test(`${locale || 'en/'}Grok: nested ACP text and metadata`, () => {
    const filters = programs(markdown(locale, 'grok-build'))
    assert.equal(jq(filters[1], fixtures['grok-build']), 'Question\nAnswer\n')
    const summary = [{ title: 'Synthetic title', info: { cwd: '/synthetic/project' } }]
    assert.equal(jq(filters[0], summary), '/synthetic/project\n')
    assert.deepEqual(JSON.parse(jq(filters[2], summary)), summary[0])
  })
  test(`${locale || 'en/'}Kimi: primary events, origins and index IDs`, () => {
    const filters = programs(markdown(locale, 'kimi-code'))
    assert.equal(jq(filters[1], fixtures['kimi-code']), 'Question\nAnswer\nLegacy string\n')
    assert.equal(jq(filters[0], [{ sessionId: 'synthetic-session', title: 'Synthetic title' }]), 'synthetic-session\tSynthetic title\n')
  })
  test(`${locale || 'en/'}Pi: nested strings, blocks and documented all-branch behavior`, () => {
    assert.equal(jq(programs(markdown(locale, 'pi'))[0], fixtures.pi), 'Question\nAnswer\nOther branch\n')
  })
  test(`${locale || 'en/'}Antigravity: multiline USER_REQUEST extraction`, () => {
    const filters = programs(markdown(locale, 'antigravity-cli'))
    assert.equal(jq(filters[2], fixtures['antigravity-cli']), 'Question\non two lines\n')
    assert.equal(jq(filters[1], fixtures['antigravity-cli']), '1\tUSER_EXPLICIT\tUSER_INPUT\n2\tMODEL\tMODEL_RESPONSE\n3\tUSER_EXPLICIT\tUSER_INPUT\n')
    assert.equal(jq(filters[0], [{ timestamp: '2026-10-05', workspace: '/synthetic/project', display: 'Synthetic title' }]), '2026-10-05\t/synthetic/project\tSynthetic title\n')
  })
}

function sqlPrograms(text) {
  return [...text.matchAll(/```sql\n([\s\S]*?)\n```/g)].map((match) => match[1]).filter((sql) => sql.startsWith('SELECT'))
}
test('opencode: translated SQL is identical', () => {
  const english = sqlPrograms(markdown('', 'opencode'))
  assert.equal(english.length, 4)
  for (const locale of locales.slice(1)) assert.deepEqual(sqlPrograms(markdown(locale, 'opencode')), english)
})
test('opencode: legacy and v2 SQL read synthetic DBs without writes', () => {
  const dir = mkdtempSync(join(tmpdir(), 'sessions-viewer-docs-'))
  const path = join(dir, 'synthetic.db')
  function sqlite(database, sql) {
    const result = spawnSync('sqlite3', [database], { input: sql, encoding: 'utf8' })
    assert.equal(result.error, undefined, 'Install sqlite3 to run documentation command tests')
    assert.equal(result.status, 0, result.stderr)
    return result.stdout
  }
  try {
    sqlite(path, readFileSync(join(sampleDir, 'opencode.sql'), 'utf8'))
    const before = readFileSync(path)
    const readonly = `file:${path}?mode=ro`
    const v2path = join(dir, 'synthetic-v2.db')
    sqlite(v2path, readFileSync(join(sampleDir, 'opencode-v2.sql'), 'utf8'))
    const v2before = readFileSync(v2path)
    for (const locale of locales) {
      const [v2query, ...filters] = sqlPrograms(markdown(locale, 'opencode'))
      assert.match(v2query, /FROM session_message/)
      const rows = sqlite(`file:${v2path}?mode=ro`, v2query).trim().split('\n').map((line) => line.split('|'))
      assert.deepEqual(rows.map((row) => row.slice(0, 3)), [['msg_z', 'user', '1'], ['msg_a', 'assistant', '2']])
      assert.equal(JSON.parse(rows[0][3]).text, 'Question')
      assert.equal(JSON.parse(rows[1][3]).model.id, 'synthetic-model')
      assert.equal(sqlite(readonly, filters[0]), 'ses_...|/synthetic/project|Synthetic session|1.0|1970-01-01 00:00:00\n')
      assert.equal(sqlite(readonly, filters[1]), expected.opencode)
      assert.equal(sqlite(readonly, filters[2]), '/synthetic/project|3.0|2\n')
      const guide = readFileSync(join(src, locale, 'guide/compatibility.md'), 'utf8')
      const exampleQuery = guide.match(/"(SELECT json_extract\(data[^\"]+)"/)
      assert.ok(exampleQuery, 'compatibility guide must include the synthetic read query')
      assert.equal(sqlite(readonly, exampleQuery[1]), 'Question\n')
    }
    assert.deepEqual(readFileSync(path), before)
    assert.deepEqual(readFileSync(v2path), v2before)
  } finally { rmSync(dir, { recursive: true, force: true }) }
})
