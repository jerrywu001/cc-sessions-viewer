import { spawnSync } from 'node:child_process'
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { afterEach, describe, expect, it } from 'vitest'

// Execute the real release step, but replace extraction/download/repacking with
// local fixtures. No network, FUSE, Linux binary or signing key is needed.
const workflow = readFileSync(resolve('.github/workflows/release.yml'), 'utf8')
const step = workflow.match(/      - name: Strip bundled libwayland-client from AppImage\n[\s\S]*?        run: \|\n([\s\S]*?)(?=\n      - )/)?.[1]
if (!step) throw new Error('Missing AppImage repack step in release.yml')
const script = step.replace(/^ {10}/gm, '')
const fixtures: string[] = []
afterEach(() => {
  for (const dir of fixtures.splice(0)) rmSync(dir, { recursive: true, force: true })
})

function executable(path: string, source: string) {
  writeFileSync(path, source)
  chmodSync(path, 0o755)
}

function fixture(launcher: 'file' | 'symlink' | 'missing') {
  const dir = mkdtempSync(join(tmpdir(), 'release-appimage-'))
  fixtures.push(dir)
  mkdirSync(join(dir, 'bin'))
  const bundle = join(dir, 'src-tauri/target/release/bundle/appimage')
  mkdirSync(bundle, { recursive: true })
  const appImage = join(bundle, 'Sessions Viewer_0.6.2_amd64.AppImage')
  if (launcher !== 'missing') {
    executable(appImage, `#!/usr/bin/env node
const fs = require('node:fs')
if (process.argv[2] !== '--appimage-extract') process.exit(1)
for (const path of ['squashfs-root', 'squashfs-root/usr', 'squashfs-root/usr/lib', 'squashfs-root/apprun-hooks']) {
  fs.mkdirSync(path, { mode: 0o700 })
  fs.chmodSync(path, 0o700)
}
for (const [path, mode] of [
  ['AppRun.wrapped', 0o770], ['usr/lib/libwayland-client.so.0', 0o600],
  ['usr/lib/libkeep.so', 0o600], ['apprun-hooks/hook.sh', 0o700],
]) {
  fs.writeFileSync('squashfs-root/' + path, 'fixture')
  fs.chmodSync('squashfs-root/' + path, mode)
}
if (${JSON.stringify(launcher)} === 'symlink') {
  fs.symlinkSync('AppRun.wrapped', 'squashfs-root/AppRun')
} else {
  fs.writeFileSync('squashfs-root/AppRun', 'fixture')
  fs.chmodSync('squashfs-root/AppRun', 0o600)
}
`)
  }
  const tool = join(dir, 'mock-appimagetool')
  executable(tool, `#!/usr/bin/env node
const fs = require('node:fs')
const path = require('node:path')
const [appdir, output, ...extra] = process.argv.slice(2)
if (extra.length) throw new Error('Unexpected options (permission checks must not be bypassed)')
const modes = {}
function walk(file) {
  const stat = fs.lstatSync(file)
  if (stat.isSymbolicLink()) return
  const mode = stat.mode & 0o777
  const required = stat.isDirectory() || (mode & 0o100) ? 0o555 : 0o444
  if ((mode & required) !== required) throw new Error('Insufficient permissions: ' + file)
  modes[path.relative(appdir, file) || '.'] = mode
  if (stat.isDirectory()) for (const name of fs.readdirSync(file)) walk(path.join(file, name))
}
walk(appdir)
if ((fs.statSync(path.join(appdir, 'AppRun')).mode & 0o555) !== 0o555) {
  throw new Error('AppRun must be executable for everyone')
}
fs.writeFileSync('repack-result.json', JSON.stringify({ modes, symlink: fs.lstatSync(path.join(appdir, 'AppRun')).isSymbolicLink() }))
fs.writeFileSync(output, 'repacked fixture')
`)
  executable(join(dir, 'bin/curl'), `#!/bin/bash
set -eu
while [ "$1" != '-o' ]; do shift; done
cp "$MOCK_REPACK_TOOL" "$2"
`)
  const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', `umask 077\n${script}`], {
    cwd: dir,
    env: { ...process.env, PATH: `${join(dir, 'bin')}:${process.env.PATH}`, MOCK_REPACK_TOOL: tool },
    encoding: 'utf8',
    timeout: 10000,
  })
  return { dir, appImage, result }
}

describe.skipIf(process.platform === 'win32')('Linux AppImage release repack', () => {
  it.each(['file', 'symlink'] as const)('normalizes restrictive permissions with a %s AppRun', (launcher) => {
    const { dir, appImage, result } = fixture(launcher)
    expect(result.error).toBeUndefined()
    expect(result.status, result.stderr).toBe(0)
    const snapshot = JSON.parse(readFileSync(join(dir, 'repack-result.json'), 'utf8'))
    expect(snapshot.symlink).toBe(launcher === 'symlink')
    expect(snapshot.modes).toMatchObject({
      '.': 0o755,
      usr: 0o755,
      'usr/lib': 0o755,
      'usr/lib/libkeep.so': 0o644,
      'AppRun.wrapped': 0o775,
      'apprun-hooks': 0o755,
      'apprun-hooks/hook.sh': 0o755,
    })
    expect(snapshot.modes).not.toHaveProperty('usr/lib/libwayland-client.so.0')
    expect(readFileSync(appImage, 'utf8')).toBe('repacked fixture')
    expect(existsSync(join(dir, 'squashfs-root'))).toBe(false)
    expect(existsSync(join(dir, 'appimagetool'))).toBe(false)
  })

  it('still skips repacking when no AppImage was built', () => {
    const { dir, result } = fixture('missing')
    expect(result.error).toBeUndefined()
    expect(result.status, result.stderr).toBe(0)
    expect(result.stdout).toContain('No AppImage found, skipping')
    expect(existsSync(join(dir, 'repack-result.json'))).toBe(false)
    expect(existsSync(join(dir, 'appimagetool'))).toBe(false)
  })
})
