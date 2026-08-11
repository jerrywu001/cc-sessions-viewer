// 后台版本检查 —— 启动时跑一次，结果缓存到 localStorage 24h；侧边栏 Settings
// 入口据此显示一个小红点提示有新版本。失败完全静默 —— 这是后台检查，不该
// 拿网络问题打扰用户；点 Settings 里「检查更新」会拿到真实的报错信息。
//
// 与 src/api.ts 的 checkUpdate 的关系：api 层负责调 GitHub、解析响应；本模块
// 负责"什么时候检查、结果存哪、UI 据此渲染什么"。SettingsModal 手动检查
// 完成后也会回调 syncFromManualCheck，让红点与手动结果保持一致。

import { markRaw, ref, shallowRef } from 'vue'
import { relaunch } from '@tauri-apps/plugin-process'
import { check as checkTauriUpdate, type Update } from '@tauri-apps/plugin-updater'
import { appVersion, checkUpdate, cleanupRuntimeChildren, openUrl, type UpdateInfo } from './api'

// 没拿到具体 release 的 html_url 时的兜底地址。和 App.vue 的 REPO_URL 同源；
// /releases/latest 永远会重定向到当前最新 release 页面，等价于"先点 Latest"。
const RELEASES_LATEST_PAGE =
  'https://github.com/jerrywu001/cc-sessions-viewer/releases/latest'

const CACHE_KEY = 'updateCheck:v1'
const TTL_MS = 24 * 60 * 60 * 1000 // 一天 —— GitHub 未授权 API 是 60 次/小时/IP，足够安全

interface Cached {
  checkedAt: number
  latest: string
  htmlUrl?: string
}

/** 有新版本时为 true；驱动侧边栏 Settings 按钮的小红点。 */
export const updateAvailable = ref(false)
/** 远端最新版本号（不带 v 前缀），用于 tooltip / Settings 里展示。 */
export const latestVersion = ref<string | null>(null)
/** 对应 GitHub release 页 URL，后续可以做"点击直达"。 */
export const releaseUrl = ref<string | null>(null)
export const updaterUpdate = shallowRef<Update | null>(null)
export const updateDownloaded = ref(false)
export const updateProgress = ref<number | null>(null)
/** 下载/安装是否进行中。**模块级**状态 —— 关掉再打开 SettingsModal 不会丢，
 *  UI 据此把按钮置灰显示「下载中」，避免重复点出第二个下载进程。 */
export const updateDownloading = ref(false)
/** 最近一次下载/安装的错误。与 updateAvailable 无关 —— updateAvailable 为 true
 *  时 updateMsg 的 v-if 会把错误藏掉，所以需要独立的 ref。 */
export const updateInstallError = ref<string | null>(null)

function loadCache(): Cached | null {
  try {
    const raw = localStorage.getItem(CACHE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as Cached
    if (typeof parsed?.checkedAt !== 'number' || typeof parsed?.latest !== 'string') {
      return null
    }
    return parsed
  } catch {
    return null
  }
}

function saveCache(c: Cached) {
  try {
    localStorage.setItem(CACHE_KEY, JSON.stringify(c))
  } catch {
    /* 配额耗尽等场景静默忽略，不阻塞后续逻辑 */
  }
}

// 复制一份小 semver 比较 —— 与 api.ts 内部那份逻辑一致，不跨模块借用以保持
// api.ts 作为纯 invoke 包装层的边界。
function compareVer(a: string, b: string): number {
  const pa = a.replace(/^v/i, '').split(/[.-]/).map((x) => parseInt(x, 10) || 0)
  const pb = b.replace(/^v/i, '').split(/[.-]/).map((x) => parseInt(x, 10) || 0)
  const n = Math.max(pa.length, pb.length)
  for (let i = 0; i < n; i++) {
    const da = pa[i] ?? 0
    const db = pb[i] ?? 0
    if (da !== db) return da - db
  }
  return 0
}

function applyInfo(info: UpdateInfo) {
  updateAvailable.value = info.hasUpdate
  latestVersion.value = info.latest
  releaseUrl.value = info.htmlUrl ?? null
}

/**
 * 应用启动时调用一次。
 *   1. 先用 localStorage 缓存即时把红点/版本号点亮（同步显示）；
 *      跟 fresh appVersion 比对，万一用户已经升级过了缓存还说"有更新"，立刻清掉。
 *   2. 如果缓存超过 24h（或没缓存）再去发一次真实请求。失败完全静默。
 */
export async function runBackgroundCheck(): Promise<void> {
  const cached = loadCache()
  const current = await appVersion().catch(() => null)

  // 优先用缓存即刻刷新 UI；hasUpdate 用现在的 current 和缓存里的 latest 现算 ——
  // 这样用户升级后第一次启动就能正确清掉红点，不需要等下一次 24h 后的新请求。
  if (cached && current) {
    updateAvailable.value = compareVer(cached.latest, current) > 0
    latestVersion.value = cached.latest
    releaseUrl.value = cached.htmlUrl ?? null
  }

  const fresh = cached && Date.now() - cached.checkedAt < TTL_MS
  if (fresh) return

  try {
    const info = await checkUpdate()
    applyInfo(info)
    saveCache({ checkedAt: Date.now(), latest: info.latest, htmlUrl: info.htmlUrl })
  } catch {
    /* 后台检查的网络/HTTP 错误静默吞掉 —— 手动检查会把真实错误展示给用户 */
  }
}

export async function checkAppUpdate(): Promise<UpdateInfo> {
  const current = await appVersion()
  updaterUpdate.value?.close().catch(() => {})
  updaterUpdate.value = null
  updateDownloaded.value = false
  updateProgress.value = null
  updateInstallError.value = null

  try {
    const update = await checkTauriUpdate()
    if (update) {
      updaterUpdate.value = markRaw(update)
      const info = {
        current: update.currentVersion || current,
        latest: update.version,
        hasUpdate: true,
        htmlUrl: releaseUrl.value ?? undefined,
      }
      syncFromManualCheck(info)
      return info
    }
    const info = { current, latest: current, hasUpdate: false }
    syncFromManualCheck(info)
    return info
  } catch (e) {
    // 兼容旧 release：如果远端还没有 latest.json，至少保留原来的 GitHub
    // Releases 版本检查和“查看 release”降级路径。
    console.warn('[updateCheck] tauri updater check failed, falling back to GitHub release check', e)
    const info = await checkUpdate()
    syncFromManualCheck(info)
    return info
  }
}

// 同一次下载只跑一个 promise —— 期间任何重复调用都复用它，绝不会并发开第二个下载。
let inFlightDownload: Promise<void> | null = null

export function downloadAndInstallUpdate(): Promise<void> {
  if (inFlightDownload) return inFlightDownload
  const upd = updaterUpdate.value
  if (!upd) return Promise.reject(new Error('No update available'))

  updateDownloading.value = true
  updateDownloaded.value = false
  updateProgress.value = 0
  updateInstallError.value = null
  let downloaded = 0
  let total: number | undefined

  inFlightDownload = (async () => {
    try {
      await upd.downloadAndInstall((event) => {
        if (event.event === 'Started') {
          downloaded = 0
          total = event.data.contentLength
          updateProgress.value = total ? 0 : null
        } else if (event.event === 'Progress') {
          downloaded += event.data.chunkLength
          updateProgress.value = total
            ? Math.min(100, Math.round((downloaded / total) * 100))
            : null
        } else if (event.event === 'Finished') {
          updateProgress.value = 100
        }
      })
      updateDownloaded.value = true
    } finally {
      updateDownloading.value = false
      inFlightDownload = null
    }
  })()

  return inFlightDownload
}

export async function relaunchApp(): Promise<void> {
  await cleanupRuntimeChildren()
  await relaunch()
}

/**
 * SettingsModal 手动「检查更新」完成后调一下，把红点状态与最新一次手动检查
 * 对齐（顺便刷新 TTL —— 用户刚手动看过，没必要 24h 内再背着他打一次）。
 */
export function syncFromManualCheck(info: UpdateInfo): void {
  applyInfo(info)
  saveCache({ checkedAt: Date.now(), latest: info.latest, htmlUrl: info.htmlUrl })
}

/**
 * 在系统浏览器中打开当前已知最新版本的 release 页。优先用 GitHub API 返回
 * 的 html_url（精确到那一条 release）；拿不到就退到 /releases/latest（GitHub
 * 会自动 302 到最新一条）。出错只在 console 留个痕，不抛 —— 调用方一般是
 * 装饰性按钮，失败也不该阻塞主流程。
 */
export async function openReleasePage(): Promise<void> {
  const url = releaseUrl.value ?? RELEASES_LATEST_PAGE
  try {
    await openUrl(url)
  } catch (e) {
    console.warn('[updateCheck] openUrl failed', e)
  }
}
