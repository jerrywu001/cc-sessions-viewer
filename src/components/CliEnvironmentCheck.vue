<script setup lang="ts">
import { onMounted } from 'vue'
import { t } from '../i18n'
import { agentIcons, IconInfo } from './icons'
import {
  cliVersions,
  loading,
  installing,
  installMsg,
  upgrading,
  diagnosisResults,
  upgradeMsg,
  upgradableCount,
  anyUpgrading,
  anyDiagnosing,
  refresh,
  install,
  upgrade,
  upgradeAll,
  diagnoseAll,
} from '../cliEnvStore'

const cliLabels: Record<string, string> = {
  claude: 'Claude Code',
  codex: 'Codex',
  agy: 'Antigravity CLI',
  opencode: 'opencode',
  grok: 'Grok Build CLI',
  kimi: 'Kimi Code',
  pi: 'Pi',
}

const cliUrls: Record<string, string> = {
  claude: 'https://docs.anthropic.com/en/docs/claude-code/overview',
  codex: 'https://developers.openai.com/codex/cli',
  agy: 'https://antigravity.google/docs/cli/getting-started',
  opencode: 'https://opencode.ai/docs/',
  grok: 'https://docs.x.ai/docs/grok-cli',
  kimi: 'https://www.kimi.com/code/en',
  pi: 'https://github.com/badlogic/pi-mono',
}

const pmLabels: Record<string, string> = {
  'homebrew-cask': 'Homebrew Cask',
  homebrew: 'Homebrew',
  nvm: 'nvm',
  volta: 'Volta',
  fnm: 'fnm',
  bun: 'bun',
  npm: 'npm global',
  system: 'system',
  unknown: 'unknown',
}

function platformLabel() {
  const p = navigator.platform.toLowerCase()
  if (p.includes('mac')) return 'macOS'
  if (p.includes('win')) return 'Windows'
  return 'Linux'
}

function versionStatusHint(cli: string, installed: boolean, latestVersion?: string | null) {
  return cli === 'kimi' && installed && !latestVersion
    ? t('settings.cli.kimiVersionStatusHint')
    : ''
}

function agentIconForCli(cli: string) {
  return agentIcons[cli === 'kimi' ? 'kimicode' : cli as keyof typeof agentIcons]
}

function msgText(cli: string) {
  const m = upgradeMsg.value[cli]
  if (!m) return ''
  if (m.ok) return t('settings.cli.upgradeSuccess', { v: m.text })
  if (m.text === 'version_unchanged') return t('settings.cli.versionUnchanged')
  return t('settings.cli.upgradeFailed', { e: m.text })
}

function installMsgText(cli: string) {
  const m = installMsg.value[cli]
  if (!m) return ''
  if (m.ok) return t('settings.cli.installSuccess', { v: m.text })
  if (m.text === 'npm_not_found') return t('settings.cli.npmNotFound')
  if (m.text === 'no_install_method') return t('settings.cli.noInstallMethod')
  return t('settings.cli.installFailed', { e: m.text })
}

function showInstallFallbackLink(cli: string) {
  const m = installMsg.value[cli]
  return m && !m.ok && (m.text === 'npm_not_found' || m.text === 'no_install_method')
}

onMounted(() => {
  // CLI versions can change outside the app (for example after installing a
  // different Grok build), so never reuse the previous tab result on mount.
  refresh()
})
</script>

<template>
  <div class="ce">
    <!-- header -->
    <div class="ce-head">
      <div>
        <h3 class="ce-title">{{ t('settings.cli.title') }}</h3>
      </div>
      <div class="ce-head-actions">
        <button class="ce-btn" :disabled="loading" @click="refresh">
          <svg :class="{ 'ce-spin': loading }" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M1 8a7 7 0 0 1 12.45-4.35M15 1v4h-4"/>
            <path d="M15 8a7 7 0 0 1-12.45 4.35M1 15v-4h4"/>
          </svg>
          {{ t('settings.cli.refresh') }}
        </button>
        <button
          v-if="cliVersions.length > 0"
          class="ce-btn"
          :disabled="anyDiagnosing"
          @click="diagnoseAll"
        >
          <svg v-if="anyDiagnosing" class="ce-spin" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2"><circle cx="8" cy="8" r="6" opacity=".25"/><path d="M14 8a6 6 0 0 0-6-6"/></svg>
          {{ anyDiagnosing ? t('settings.cli.diagnosing') : t('settings.cli.diagnose') }}
        </button>
        <button
          v-if="upgradableCount > 0"
          class="ce-btn ce-btn-primary"
          :disabled="anyUpgrading"
          @click="upgradeAll"
        >
          {{ t('settings.cli.upgradeAllCount', { n: upgradableCount }) }}
        </button>
      </div>
    </div>

    <!-- skeleton loading -->
    <div v-if="loading && cliVersions.length === 0" class="ce-list">
      <div v-for="i in 3" :key="i" class="ce-card ce-skel">
        <div class="ce-skel-row">
          <div class="ce-skel-circle" />
          <div class="ce-skel-bar" style="width:90px" />
          <div class="ce-skel-bar ce-skel-sm" style="width:42px" />
        </div>
        <div class="ce-skel-row">
          <div class="ce-skel-bar" style="width:60px;height:18px" />
          <div class="ce-skel-bar" style="width:14px;height:14px" />
          <div class="ce-skel-bar" style="width:60px;height:18px" />
        </div>
      </div>
    </div>

    <!-- cards -->
    <div v-else class="ce-list">
      <div v-for="info in cliVersions" :key="info.cli" class="ce-card">
        <!-- row 1: icon + name + badge -->
        <div class="ce-row-top">
          <div class="ce-id">
            <component :is="agentIconForCli(info.cli)" class="ce-icon" />
            <span class="ce-name">{{ cliLabels[info.cli] || info.cli }}</span>
            <span class="ce-tag">{{ platformLabel() }}</span>
          </div>
          <span v-if="info.upgradable" class="ce-badge ce-badge-up">{{ t('settings.cli.upgradable') }}</span>
          <span v-else-if="info.installed && info.latestVersion" class="ce-badge ce-badge-ok">{{ t('settings.cli.upToDate') }}</span>
          <span v-else-if="!info.installed" class="ce-badge ce-badge-na">{{ t('settings.cli.notInstalled') }}</span>
          <span
            v-if="info.health"
            class="ce-badge"
            :class="info.health.healthy ? 'ce-badge-health-ok' : 'ce-badge-health-error'"
            :title="info.health.summary || undefined"
          >
            {{ info.health.healthy ? t('settings.cli.healthOk') : t('settings.cli.healthFailed') }}
          </span>
          <span
            v-if="versionStatusHint(info.cli, info.installed, info.latestVersion)"
            class="ce-version-hint"
            v-tooltip="versionStatusHint(info.cli, info.installed, info.latestVersion)"
            :aria-label="versionStatusHint(info.cli, info.installed, info.latestVersion)"
            tabindex="0"
          >
            <IconInfo aria-hidden="true" />
          </span>
        </div>

        <!-- row 2: versions + actions -->
        <div class="ce-row-bot">
          <div class="ce-ver">
            <template v-if="info.installed">
              <span class="ce-ver-cur" :class="{ stale: info.upgradable }">{{ info.currentVersion }}</span>
              <template v-if="info.upgradable && info.latestVersion">
                <span class="ce-ver-arrow">→</span>
                <span class="ce-ver-lat">{{ info.latestVersion }}</span>
              </template>
            </template>
          </div>
          <div class="ce-actions">
            <button
              v-if="info.upgradable"
              class="ce-btn ce-btn-sm ce-btn-primary"
              :disabled="upgrading[info.cli]"
              @click="upgrade(info.cli)"
            >
              <svg v-if="upgrading[info.cli]" class="ce-spin" width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2"><circle cx="8" cy="8" r="6" opacity=".25"/><path d="M14 8a6 6 0 0 0-6-6"/></svg>
              {{ upgrading[info.cli] ? t('settings.cli.upgrading') : t('settings.cli.upgrade') }}
            </button>
            <button
              v-else-if="!info.installed"
              class="ce-btn ce-btn-sm ce-btn-primary"
              :disabled="installing[info.cli]"
              @click="install(info.cli)"
            >
              <svg v-if="installing[info.cli]" class="ce-spin" width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2"><circle cx="8" cy="8" r="6" opacity=".25"/><path d="M14 8a6 6 0 0 0-6-6"/></svg>
              {{ installing[info.cli] ? t('settings.cli.installing') : t('settings.cli.install') }}
            </button>
          </div>
        </div>

        <!-- upgrade message -->
        <div v-if="upgradeMsg[info.cli]" class="ce-msg" :class="upgradeMsg[info.cli]?.ok ? 'ce-msg-ok' : 'ce-msg-err'">
          {{ msgText(info.cli) }}
        </div>

        <!-- install message -->
        <div v-if="installMsg[info.cli]" class="ce-msg" :class="installMsg[info.cli]?.ok ? 'ce-msg-ok' : 'ce-msg-err'">
          {{ installMsgText(info.cli) }}
          <a v-if="showInstallFallbackLink(info.cli)" class="ce-install-link" :href="cliUrls[info.cli]" target="_blank">
            {{ t('settings.cli.goInstall') }}
            <svg width="11" height="11" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M5 3h8v8"/><path d="M13 3L3 13"/></svg>
          </a>
        </div>

        <!-- diagnosis panel -->
        <div v-if="diagnosisResults[info.cli]" class="ce-diag">
          <div v-if="diagnosisResults[info.cli]!.hasConflict" class="ce-diag-warn">
            {{ t('settings.cli.conflictWarning') }}
          </div>
          <div v-else class="ce-diag-ok">{{ t('settings.cli.noConflict') }}</div>
          <div
            v-for="(inst, idx) in diagnosisResults[info.cli]!.installations"
            :key="idx"
            class="ce-inst"
          >
            <code class="ce-inst-path">{{ inst.path }}</code>
            <!-- The per-install version only matters when comparing multiple
                 conflicting installs; with a single install it just duplicates
                 the version already shown in the card header. -->
            <span v-if="diagnosisResults[info.cli]!.hasConflict" class="ce-inst-ver">{{ inst.version || '?' }}</span>
            <span v-if="inst.isDefault" class="ce-tag ce-tag-default">{{ t('settings.cli.default') }}</span>
            <span class="ce-tag">{{ pmLabels[inst.packageManager] || inst.packageManager }}</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.ce { padding: 0; }

/* ---- header ---- */
.ce-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 16px;
}
.ce-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--text);
  margin: 0;
}
.ce-head-actions {
  display: flex;
  gap: 8px;
}

/* ---- buttons ---- */
.ce-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 5px 12px;
  font-size: 12px;
  font-weight: 500;
  border-radius: 6px;
  border: 1px solid var(--border);
  background: var(--surface);
  color: var(--text);
  cursor: pointer;
  transition: background 0.15s;
  white-space: nowrap;
}
.ce-btn:hover:not(:disabled) { background: var(--surface-hover); }
.ce-btn:disabled { opacity: 0.45; cursor: default; }
.ce-btn-primary {
  background: var(--text);
  color: var(--bg);
  border-color: var(--text);
}
.ce-btn-primary:hover:not(:disabled) {
  background: var(--text-dim);
  border-color: var(--text-dim);
}
.ce-btn-sm {
  padding: 3px 10px;
  font-size: 11px;
}

.ce-badge-health-ok {
  color: var(--green, #2d8a5b);
  border-color: color-mix(in srgb, var(--green, #2d8a5b) 35%, var(--border));
}
.ce-badge-health-error {
  color: var(--red, #c84b4b);
  border-color: color-mix(in srgb, var(--red, #c84b4b) 35%, var(--border));
}
.ce-version-hint {
  display: inline-flex;
  align-items: center;
  color: var(--text-mute);
  cursor: help;
  flex-shrink: 0;
}
.ce-version-hint :deep(svg) {
  width: 14px;
  height: 14px;
}
.ce-version-hint:focus-visible {
  outline: 1px solid var(--accent, var(--text));
  outline-offset: 2px;
  border-radius: 2px;
}

/* ---- cards ---- */
.ce-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.ce-card {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 12px 14px;
  background: var(--surface);
  transition: border-color 0.15s;
}
.ce-card:hover {
  border-color: var(--border-strong);
}

/* row top: icon + name + badge */
.ce-row-top {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.ce-id {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 0;
}
.ce-icon {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
}
.ce-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

/* tags / badges */
.ce-tag {
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 4px;
  background: var(--surface-hover);
  color: var(--text-mute);
  font-weight: 500;
  white-space: nowrap;
}
.ce-tag-default {
  background: rgba(239, 108, 0, 0.10);
  color: #ef6c00;
  font-weight: 600;
}
.ce-badge {
  font-size: 10px;
  padding: 2px 7px;
  border-radius: 4px;
  font-weight: 600;
  white-space: nowrap;
  flex-shrink: 0;
}
.ce-badge-up {
  background: rgba(239, 108, 0, 0.10);
  color: #ef6c00;
}
.ce-badge-ok {
  background: rgba(16, 185, 129, 0.10);
  color: #059669;
}
.ce-badge-na {
  background: var(--surface-hover);
  color: var(--text-mute);
}

/* row bottom: versions + actions */
.ce-row-bot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.ce-ver {
  display: flex;
  align-items: baseline;
  gap: 6px;
  font-variant-numeric: tabular-nums;
}
.ce-ver-cur {
  font-size: 15px;
  font-weight: 600;
  color: var(--text);
  letter-spacing: -0.01em;
}
.ce-ver-cur.stale {
  color: var(--text-mute);
}
.ce-ver-arrow {
  font-size: 12px;
  color: var(--text-mute);
}
.ce-ver-lat {
  font-size: 15px;
  font-weight: 600;
  color: #ef6c00;
  letter-spacing: -0.01em;
}
.ce-install-link {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  margin-left: 6px;
  font-size: 11px;
  color: var(--accent, var(--text-mute));
  text-decoration: underline;
  text-underline-offset: 2px;
  transition: color 0.15s;
}
.ce-install-link:hover {
  color: var(--text);
}
.ce-actions {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}

/* ---- upgrade message ---- */
.ce-msg {
  margin-top: 8px;
  padding: 5px 8px;
  border-radius: 5px;
  font-size: 11px;
  font-weight: 500;
}
.ce-msg-ok {
  background: rgba(16, 185, 129, 0.08);
  color: #059669;
}
.ce-msg-err {
  background: rgba(239, 68, 68, 0.08);
  color: #dc2626;
}

/* ---- diagnosis panel ---- */
.ce-diag {
  margin-top: 8px;
  padding: 8px 10px;
  border-radius: 6px;
  background: var(--surface-2);
  font-size: 11px;
}
.ce-diag-warn {
  color: #ef6c00;
  font-weight: 600;
  margin-bottom: 6px;
}
.ce-diag-ok {
  color: var(--text-mute);
  margin-bottom: 4px;
}
.ce-inst {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 0;
}
.ce-inst + .ce-inst {
  border-top: 1px solid var(--border);
}
.ce-inst-path {
  font-size: 11px;
  color: var(--text);
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ce-inst-ver {
  font-size: 11px;
  color: var(--text);
  font-weight: 500;
  font-variant-numeric: tabular-nums;
}

/* ---- skeleton loading ---- */
.ce-skel {
  pointer-events: none;
}
.ce-skel-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.ce-skel-row + .ce-skel-row { margin-top: 10px; }
.ce-skel-circle {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: var(--surface-hover);
  animation: ce-pulse 1.4s ease-in-out infinite;
}
.ce-skel-bar {
  height: 12px;
  border-radius: 4px;
  background: var(--surface-hover);
  animation: ce-pulse 1.4s ease-in-out infinite;
}
.ce-skel-sm { opacity: 0.6; }
.ce-skel-row:nth-child(1) .ce-skel-bar { animation-delay: 0.1s; }
.ce-skel-row:nth-child(2) .ce-skel-bar:nth-child(1) { animation-delay: 0.2s; }
.ce-skel-row:nth-child(2) .ce-skel-bar:nth-child(3) { animation-delay: 0.3s; }

@keyframes ce-pulse {
  0%, 100% { opacity: 0.4; }
  50%      { opacity: 1; }
}

/* ---- spinner ---- */
.ce-spin {
  animation: ce-spin 0.8s linear infinite;
}
@keyframes ce-spin {
  to { transform: rotate(360deg); }
}

@media (prefers-reduced-motion: reduce) {
  .ce-skel-circle, .ce-skel-bar, .ce-spin { animation: none; }
}
</style>
