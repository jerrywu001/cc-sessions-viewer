// 统一图标层：所有图标改用 Iconify（lucide 集合）按需打包，编译期内联 SVG，
// 运行时不联网（Tauri 离线友好）。如需替换图标，直接换 import 路径即可：
//   import IconFoo from '~icons/lucide/foo-name'
// 浏览所有可用图标：https://iconify.design/

import IconPinUpRaw from '~icons/lucide/arrow-up-to-line'
import IconPinDownRaw from '~icons/lucide/arrow-down-to-line'
import IconTrashRaw from '~icons/lucide/trash-2'
import IconTrashOpenRaw from '~icons/quill/folder-trash'
import IconRestoreRaw from '~icons/lucide/archive-restore'
import IconDeleteLineRaw from '~icons/mingcute/delete-line'
import IconSettingsRaw from '~icons/lucide/settings'
import IconPlayRaw from '~icons/lucide/play'
import IconChatRaw from '~icons/lucide/message-circle'
import IconReaderRaw from '~icons/lucide/book-open'
import IconFolderRaw from '~icons/lucide/folder'
import IconInboxRaw from '~icons/lucide/inbox'
import IconRefreshRaw from '~icons/lucide/rotate-cw'
import IconArrowLeftRaw from '~icons/lucide/arrow-left'
import IconArrowUpRaw from '~icons/lucide/arrow-up'
import IconArrowDownRaw from '~icons/lucide/arrow-down'
import IconChevronRightRaw from '~icons/lucide/chevron-right'
import IconAtomRaw from '~icons/lucide/atom'
import IconEmptyBoxRaw from '~icons/lucide/package'
import IconPointLeftRaw from '~icons/lucide/chevron-left'
import IconSidebarRaw from '~icons/lucide/panel-left'
import IconCloseRaw from '~icons/lucide/x'
import IconSunRaw from '~icons/lucide/sun'
import IconMoonRaw from '~icons/lucide/moon'
import IconMonitorRaw from '~icons/lucide/monitor'
import IconLanguagesRaw from '~icons/lucide/languages'
import IconDatabaseRaw from '~icons/lucide/database'
import IconInfoRaw from '~icons/lucide/info'
import IconBellRaw from '~icons/lucide/bell'
import IconWrenchRaw from '~icons/lucide/wrench'
import IconHelpCircleRaw from '~icons/lucide/circle-help'
import IconPaletteRaw from '~icons/lucide/palette'
import IconCheckRaw from '~icons/lucide/check'
import IconPencilRaw from '~icons/lucide/pencil'
import IconCopyRaw from '~icons/lucide/copy'
import IconSearchRaw from '~icons/lucide/search'
import IconChevronUpRaw from '~icons/lucide/chevron-up'
import IconChevronDownRaw from '~icons/lucide/chevron-down'
import IconFoldRaw from '~icons/lucide/chevrons-down-up'
import IconUnfoldRaw from '~icons/lucide/chevrons-up-down'
import IconMinimizeRaw from '~icons/lucide/minus'
import IconDownloadRaw from '~icons/lucide/download'
import IconUploadRaw from '~icons/lucide/upload'
import IconMarkdownRaw from '~icons/lucide/file-text'
import IconFileRaw from '~icons/lucide/file'
import IconFileDiffRaw from '~icons/lucide/file-diff'
import IconHtmlRaw from '~icons/lucide/file-code'
import IconJsonRaw from '~icons/lucide/braces'
// 文件附件 chip 的分类型图标（统一 lucide 单色，靠形状区分，不破坏中性配色）。
import IconFileSheetRaw from '~icons/lucide/file-spreadsheet'
import IconFileSlidesRaw from '~icons/lucide/presentation'
import IconFileImageRaw from '~icons/lucide/file-image'
import IconFileVideoRaw from '~icons/lucide/file-video'
import IconFileAudioRaw from '~icons/lucide/file-audio'
import IconFileArchiveRaw from '~icons/lucide/file-archive'
import IconSortRaw from '~icons/lucide/arrow-down-up'
import IconSelectRaw from '~icons/lucide/list-checks'
import IconPlusRaw from '~icons/lucide/plus'
import IconHistoryRaw from '~icons/lucide/history'
import IconStarRaw from '~icons/lucide/star'
import IconCalendarClockRaw from '~icons/lucide/calendar-clock'
import IconExportHistoryRaw from '~icons/lucide/clock-arrow-down'
import IconMoreRaw from '~icons/lucide/more-horizontal'
import IconPriceTagRaw from '~icons/lucide/circle-dollar-sign'
import IconGithubRaw from '~icons/lucide/github'
import IconCornerDownLeftRaw from '~icons/lucide/corner-down-left'
import IconStopRaw from '~icons/lucide/square'
import IconChartRaw from '~icons/lucide/bar-chart-3'
import IconListRaw from '~icons/lucide/list'
import IconWalletRaw from '~icons/lucide/wallet'
import IconActivityRaw from '~icons/lucide/activity'
import IconLayersRaw from '~icons/lucide/layers'
import IconZapRaw from '~icons/lucide/zap'
import IconExternalLinkRaw from '~icons/lucide/external-link'
import IconLocateRaw from '~icons/lucide/locate'
import IconArchiveRaw from '~icons/lucide/archive'
import IconShieldCheckRaw from '~icons/lucide/shield-check'
import IconTerminalRaw from '~icons/lucide/terminal'
import IconEyeOffRaw from '~icons/lucide/eye-off'
import IconEyeRaw from '~icons/lucide/eye'
import IconGitBranchRaw from '~icons/lucide/git-branch'
import IconClaudeRaw from '~icons/material-icon-theme/claude'
import IconKeyboardRaw from '~icons/lucide/keyboard'
import IconSlidersRaw from '~icons/lucide/sliders-horizontal'
import IconWebhookRaw from '~icons/lucide/webhook'
import IconPaperclipRaw from '~icons/lucide/paperclip'
import IconSlashSquareRaw from '~icons/lucide/square-slash'
import IconSkillRaw from '~icons/lucide/box'
import IconContextWindowRaw from '~icons/lucide/layout-grid'
import IconExitPaneRaw from '~icons/lucide/log-out'

export const IconPinUp = IconPinUpRaw
export const IconPinDown = IconPinDownRaw
export const IconTrash = IconTrashRaw
export const IconTrashOpen = IconTrashOpenRaw
export const IconRestore = IconRestoreRaw
export const IconDeleteLine = IconDeleteLineRaw
export const IconSettings = IconSettingsRaw
export const IconPlay = IconPlayRaw
export const IconChat = IconChatRaw
export const IconReader = IconReaderRaw
export const IconFolder = IconFolderRaw
export const IconInbox = IconInboxRaw
export const IconRefresh = IconRefreshRaw
export const IconArrowLeft = IconArrowLeftRaw
export const IconArrowUp = IconArrowUpRaw
export const IconArrowDown = IconArrowDownRaw
export const IconChevronRight = IconChevronRightRaw
export const IconAtom = IconAtomRaw
export const IconEmptyBox = IconEmptyBoxRaw
export const IconPointLeft = IconPointLeftRaw
export const IconSidebar = IconSidebarRaw
export const IconClose = IconCloseRaw
export const IconExitPane = IconExitPaneRaw
export const IconSun = IconSunRaw
export const IconMoon = IconMoonRaw
export const IconMonitor = IconMonitorRaw
export const IconLanguages = IconLanguagesRaw
export const IconDatabase = IconDatabaseRaw
export const IconInfo = IconInfoRaw
export const IconBell = IconBellRaw
export const IconWrench = IconWrenchRaw
export const IconHelpCircle = IconHelpCircleRaw
export const IconPalette = IconPaletteRaw
export const IconCheck = IconCheckRaw
export const IconPencil = IconPencilRaw
export const IconCopy = IconCopyRaw
export const IconSearch = IconSearchRaw
export const IconChevronUp = IconChevronUpRaw
export const IconChevronDown = IconChevronDownRaw
export const IconFold = IconFoldRaw
export const IconUnfold = IconUnfoldRaw
export const IconMinimize = IconMinimizeRaw
export const IconDownload = IconDownloadRaw
export const IconUpload = IconUploadRaw
export const IconMarkdown = IconMarkdownRaw
export const IconFile = IconFileRaw
export const IconFileDiff = IconFileDiffRaw
export const IconHtml = IconHtmlRaw
export const IconJson = IconJsonRaw
export const IconSort = IconSortRaw
export const IconSelect = IconSelectRaw
export const IconPlus = IconPlusRaw
export const IconHistory = IconHistoryRaw
export const IconStar = IconStarRaw
export const IconCalendarClock = IconCalendarClockRaw
export const IconExportHistory = IconExportHistoryRaw
export const IconMore = IconMoreRaw
export const IconPriceTag = IconPriceTagRaw
export const IconGithub = IconGithubRaw
export const IconCornerDownLeft = IconCornerDownLeftRaw
export const IconStop = IconStopRaw
/** 发送按钮复用「回车」图标（↵），与 Claude 客户端一致。 */
export const IconSend = IconCornerDownLeftRaw
export const IconChart = IconChartRaw
export const IconList = IconListRaw
export const IconWallet = IconWalletRaw
export const IconActivity = IconActivityRaw
export const IconLayers = IconLayersRaw
export const IconZap = IconZapRaw
export const IconExternalLink = IconExternalLinkRaw
export const IconLocate = IconLocateRaw
export const IconArchive = IconArchiveRaw
export const IconShieldCheck = IconShieldCheckRaw
export const IconTerminal = IconTerminalRaw
export const IconEyeOff = IconEyeOffRaw
export const IconEye = IconEyeRaw
export const IconGitBranch = IconGitBranchRaw
export const IconKeyboard = IconKeyboardRaw
export const IconSliders = IconSlidersRaw
export const IconWebhook = IconWebhookRaw
export const IconPaperclip = IconPaperclipRaw
export const IconSlashSquare = IconSlashSquareRaw
export const IconSkill = IconSkillRaw
export const IconContextWindow = IconContextWindowRaw
// 「已 pin」状态的小圆点指示器；6×6 实心圆，自己拼比拉一整个集合便宜。
import { defineComponent, h, type Component } from 'vue'

export const IconSplitH = defineComponent({
  name: 'IconSplitH',
  render: () => h('svg', { xmlns: 'http://www.w3.org/2000/svg', width: '16', height: '16', viewBox: '0 0 16 16', fill: 'none', stroke: 'currentColor', 'stroke-width': '1.3', 'stroke-linecap': 'round' }, [
    h('rect', { x: '1.5', y: '1.5', width: '13', height: '13', rx: '2' }),
    h('line', { x1: '8', y1: '1.5', x2: '8', y2: '14.5' }),
  ]),
})
export const IconSplitV = defineComponent({
  name: 'IconSplitV',
  render: () => h('svg', { xmlns: 'http://www.w3.org/2000/svg', width: '16', height: '16', viewBox: '0 0 16 16', fill: 'none', stroke: 'currentColor', 'stroke-width': '1.3', 'stroke-linecap': 'round' }, [
    h('rect', { x: '1.5', y: '1.5', width: '13', height: '13', rx: '2' }),
    h('line', { x1: '1.5', y1: '8', x2: '14.5', y2: '8' }),
  ]),
})
import type { Agent } from '../types'
export const IconPinFilled = defineComponent({
  name: 'IconPinFilled',
  setup() {
    return () =>
      h(
        'svg',
        {
          viewBox: '0 0 24 24',
          fill: 'currentColor',
          'aria-hidden': 'true',
        },
        [h('circle', { cx: 12, cy: 12, r: 6 })],
      )
  },
})

// Codex brand mark — single path with a built-in linear gradient (purple → blue).
// Inlined as a render function so we don't pull in vite-svg-loader for one icon;
// the gradient id is namespaced (`codexLogoGrad`) to avoid clashing if multiple
// instances mount in the same page.
const CODEX_PATH =
  'm84.3 5.1q3.7-1.5 7.7-2.6 3.9-1 7.9-1.6 4-0.5 8.1-0.6 4 0 8 0.5 20.7 2.4 37.1 17.7 0.1 0.1 0.4 0.3 0.1 0 0.2 0 0 0 0.2 0 0 0 0.1 0 0 0 0.1 0 5.2-1.4 10.7-1.9 5.4-0.4 10.7 0.1 5.5 0.4 10.7 1.9 5.2 1.3 10.1 3.6l0.6 0.4 1.6 0.8q5.2 2.5 9.7 6.1 4.7 3.4 8.6 7.7 3.8 4.3 6.9 9.2 3 4.8 5.2 10.2 4.3 10.5 4.3 22.1 0.2 2.1 0 4.2-0.1 2.2-0.2 4.3-0.3 2.1-0.7 4.3-0.4 2.1-0.9 4.1 0 0.2 0 0.4 0 0.2 0 0.5 0 0.1 0.1 0.4 0.1 0.1 0.3 0.3 12.3 12.6 16.3 30 6 29.7-12.2 53.5l-1.9 2.2q-3 3.5-6.5 6.4-3.4 3.1-7.3 5.5-3.8 2.4-8.1 4.2-4.1 1.9-8.5 3.2-0.3 0-0.4 0.2-0.3 0-0.4 0.1-0.1 0.1-0.3 0.4 0 0.1-0.1 0.3c-2.7 7.7-5.3 14.2-10.2 20.7-12.5 16.5-30.8 25.5-51.5 25.5q-24.6-0.1-43.6-18.1-0.2-0.1-0.4-0.2-0.2-0.1-0.4-0.1-0.2 0-0.3 0-0.3 0-0.4 0c-5.4 1.7-10.9 1.9-16.7 1.9q-3.5 0-7-0.5-3.4-0.4-6.9-1.2-3.3-0.8-6.6-2-3.3-1.2-6.4-2.8-3.3-1.6-6.4-3.6-3-2-5.8-4.3-3-2.3-5.5-5-2.5-2.6-4.6-5.6c-2.2-2.7-4.3-5.4-5.8-8.5q-0.8-1.6-1.6-3.2-0.6-1.7-1.3-3.3-0.7-1.7-1.2-3.4-0.5-1.6-1-3.4-1.1-4-1.6-7.9-0.6-4-0.6-8 0-4 0.6-8 0.4-4 1.4-8 0 0 0-0.1 0-0.1 0-0.1 0.2-0.2 0.2-0.3 0-0.1-0.2-0.1 0-0.2 0-0.3 0-0.1-0.1-0.1 0-0.2 0-0.2-0.1-0.1-0.1-0.1-2.4-2.5-4.6-5.2-2.1-2.7-4-5.4-1.7-3-3.2-6-1.5-3.1-2.6-6.3-0.8-2-1.3-4.1-0.7-2-1.1-4-0.4-2.1-0.7-4.2-0.2-2.2-0.4-4.3-0.2-2.8-0.1-5.6 0-2.8 0.3-5.4 0.1-2.8 0.6-5.6 0.4-2.8 1.1-5.5 7-23.1 26.9-36.3 4.3-2.9 8.2-4.5 4.5-1.9 9-3.2 0.2 0 0.3-0.1 0.1-0.2 0.3-0.3 0.1 0 0.1-0.3 0.1-0.1 0.1-0.2 1-3.1 2.2-6 1-2.9 2.5-5.7 1.5-3 3.2-5.6 1.7-2.7 3.7-5.1 2.5-3.2 5.3-5.9 3-2.8 6.1-5.4 3.2-2.4 6.8-4.4 3.5-2 7.2-3.5zm48.3 146.4c-2.3 0.1-4.4 1-6 2.8-1.5 1.6-2.4 3.7-2.4 5.9 0 2.3 0.9 4.4 2.4 6.2 1.6 1.6 3.7 2.5 6 2.6h50.4c2.4 0.1 4.8-0.6 6.5-2.4 1.7-1.6 2.8-4 2.8-6.4 0-2.4-1.1-4.7-2.8-6.3-1.7-1.8-4.1-2.6-6.5-2.4zm-56.7-64.9c-1.2-1.9-3-3.4-5.3-3.9-2.2-0.5-4.5-0.3-6.5 0.9-2 1.1-3.5 3-4.1 5.2-0.7 2.2-0.4 4.6 0.6 6.5l17.7 30.9-17.5 29.5c-1.2 2-1.6 4.5-1.1 6.8 0.7 2.3 2.1 4.1 4.1 5.3 2 1.2 4.4 1.6 6.7 0.9 2.2-0.5 4.2-1.9 5.4-3.9l20.1-34.1q0.7-0.9 0.9-2.1 0.3-1.1 0.3-2.3 0-1.2-0.3-2.2-0.2-1.2-0.8-2.2z'
const IconCodexRaw = defineComponent({
  name: 'IconCodex',
  setup() {
    return () =>
      h(
        'svg',
        {
          viewBox: '0 0 250 250',
          xmlns: 'http://www.w3.org/2000/svg',
          'aria-hidden': 'true',
          class: 'iconify',
        },
        [
          h('defs', null, [
            h(
              'linearGradient',
              {
                id: 'codexLogoGrad',
                gradientUnits: 'userSpaceOnUse',
                x1: 125,
                y1: 0.332,
                x2: 125,
                y2: 249.667,
              },
              [
                h('stop', { 'stop-color': '#b1a7ff' }),
                h('stop', { offset: '.5', 'stop-color': '#7a9dff' }),
                h('stop', { offset: '1', 'stop-color': '#3941ff' }),
              ],
            ),
          ]),
          h('path', { fill: 'url(#codexLogoGrad)', d: CODEX_PATH }),
        ],
      )
  },
})

// Terminal app brand icons for the external terminal picker.
const IconTerminalAppRaw = defineComponent({
  name: 'IconTerminalApp',
  setup() {
    return () =>
      h('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '2', 'stroke-linecap': 'round', 'stroke-linejoin': 'round', 'aria-hidden': 'true', class: 'iconify' }, [
        h('rect', { x: 2, y: 4, width: 20, height: 16, rx: 2 }),
        h('path', { d: 'M7 9l3 3-3 3' }),
        h('path', { d: 'M13 15h4' }),
      ])
  },
})
const IconITerm2Raw = defineComponent({
  name: 'IconITerm2',
  setup() {
    return () =>
      h('svg', { viewBox: '0 0 24 24', fill: 'none', 'aria-hidden': 'true', class: 'iconify' }, [
        h('rect', { x: 2, y: 3, width: 20, height: 18, rx: 3, fill: '#1d1d1f' }),
        h('text', { x: 12, y: 16.5, 'text-anchor': 'middle', 'font-size': '13', 'font-weight': 'bold', 'font-family': 'monospace', fill: '#34c759' }, '$_'),
      ])
  },
})
const IconGhosttyRaw = defineComponent({
  name: 'IconGhostty',
  setup() {
    return () =>
      h('svg', { viewBox: '0 0 26.73 32', fill: 'none', 'aria-hidden': 'true', class: 'iconify' }, [
        h('path', { d: 'M20.4 32c-1.25 0-2.48-.38-3.52-1.07a6.73 6.73 0 01-3.52 1.07 6.73 6.73 0 01-3.52-1.07A6.73 6.73 0 016.37 32h-.04a6.36 6.36 0 01-4.5-1.91A6.37 6.37 0 010 25.61V13.36C0 5.99 5.99 0 13.36 0s13.36 5.99 13.36 13.36v12.25a6.35 6.35 0 01-5.98 6.38 6 6 0 01-.35 0z', fill: '#3551F3' }),
        h('path', { d: 'M23.91 13.36v12.25c0 1.88-1.45 3.46-3.32 3.57a3.78 3.78 0 01-2.4-.77 2.82 2.82 0 00-2.66.02c-.6.47-1.35.75-2.18.75s-1.58-.28-2.17-.75a2.82 2.82 0 00-2.68 0c-.59.47-1.34.75-2.15.75-1.95.01-3.54-1.63-3.54-3.57V13.36c0-5.83 4.72-10.55 10.55-10.55s10.55 4.72 10.55 10.55z', fill: 'var(--surface, #fff)' }),
        h('path', { d: 'M11.28 12.44l-3.93-2.27a.82.82 0 00-1.12.39.82.82 0 00.39 1.12l2.33 1.34-2.33 1.34a.82.82 0 00-.39 1.12.82.82 0 001.12.39l3.93-2.27c.71-.41.71-1.44 0-1.85z', fill: 'var(--text, #000)' }),
        h('path', { d: 'M20.18 12.29h-5.16a.72.72 0 00-.72.72c0 .39.32.72.72.72h5.16c.4 0 .72-.32.72-.72s-.32-.72-.72-.72z', fill: 'var(--text, #000)' }),
      ])
  },
})
import cmuxIconLight from '../assets/cmux-icon-light.png'
import cmuxIconDark from '../assets/cmux-icon-dark.png'
const IconCmuxRaw = defineComponent({
  name: 'IconCmux',
  setup() {
    return () => {
      const isDark = document.documentElement.classList.contains('theme-dark')
        || document.documentElement.classList.contains('theme-dracula')
      return h('img', {
        src: isDark ? cmuxIconLight : cmuxIconDark,
        'aria-hidden': 'true',
        class: 'iconify',
        style: 'width:1em;height:1em;border-radius:3px',
      })
    }
  },
})
const IconWarpRaw = defineComponent({
  name: 'IconWarp',
  setup() {
    return () =>
      h('svg', { viewBox: '0 0 24 24', fill: 'none', 'aria-hidden': 'true', class: 'iconify' }, [
        h('defs', null, [
          h('linearGradient', { id: 'warpGrad', x1: '0', y1: '0', x2: '1', y2: '1' }, [
            h('stop', { offset: '0', 'stop-color': '#01C1E4' }),
            h('stop', { offset: '1', 'stop-color': '#7F5AF0' }),
          ]),
        ]),
        h('rect', { x: 2, y: 2, width: 20, height: 20, rx: 5, fill: 'url(#warpGrad)' }),
        h('path', { d: 'M8 7l5 5-5 5', stroke: '#fff', 'stroke-width': '2.2', 'stroke-linecap': 'round', 'stroke-linejoin': 'round', fill: 'none' }),
        h('path', { d: 'M13 15h4', stroke: '#fff', 'stroke-width': '2.2', 'stroke-linecap': 'round' }),
      ])
  },
})
export const IconTerminalApp = IconTerminalAppRaw
export const IconITerm2 = IconITerm2Raw
export const IconGhostty = IconGhosttyRaw
export const IconCmux = IconCmuxRaw
export const IconWarp = IconWarpRaw

import type { TerminalApp } from '../settings'
export const terminalIcons: Record<TerminalApp, Component> = {
  terminal: IconTerminalAppRaw,
  iterm2: IconITerm2Raw,
  ghostty: IconGhosttyRaw,
  cmux: IconCmuxRaw,
  warp: IconWarpRaw,
}

// Antigravity CLI (agy) brand mark — official rainbow arch "A" icon.
// PNG in public/antigravity-icon.png, referenced as absolute URL (Vite serves public/ at root).
const IconAgyRaw = defineComponent({
  name: 'IconAgy',
  setup() {
    return () =>
      h('img', {
        src: '/antigravity-icon.png',
        width: 16,
        height: 16,
        alt: 'agy',
        'aria-hidden': 'true',
        class: 'iconify',
        style: 'vertical-align: middle',
      })
  },
})

// opencode brand mark — official icon PNG in public/opencode-icon.png,
// referenced as absolute URL (Vite serves public/ at root)，与 agy 同款 <img> 方案。
const IconOpencodeRaw = defineComponent({
  name: 'IconOpencode',
  setup() {
    return () =>
      h('img', {
        src: '/opencode-icon.png',
        width: 16,
        height: 16,
        alt: 'opencode',
        'aria-hidden': 'true',
        class: 'iconify',
        style: 'vertical-align: middle',
      })
  },
})

// Official Grok favicon, rasterized from grok.com/images/favicon.svg at one
// third of its original 512 px dimensions and optimized for the app bundle.
const IconGrokRaw = defineComponent({
  name: 'IconGrok',
  setup() {
    return () =>
      h('img', {
        src: '/grok-icon.png',
        width: 16,
        height: 16,
        alt: 'Grok Build',
        'aria-hidden': 'true',
        class: 'iconify',
        style: 'vertical-align: middle',
      })
  },
})

// Brand marks for the agents, pulled from iconify at build time so
// runtime stays offline-friendly. Sources: `material-icon-theme:claude`,
// our own `assets/codex.svg`, and inline render for agy / opencode.
// Re-exported individually for direct use and aggregated into `agentIcons`
// for dispatch-by-agent.
export const IconClaude = IconClaudeRaw
export const IconCodex = IconCodexRaw
export const IconAgy = IconAgyRaw
export const IconOpencode = IconOpencodeRaw
export const IconGrok = IconGrokRaw

/**
 * Global dictionary of agent → brand-mark icon component. Use as
 * `<component :is="agentIcons[agent]" />` so consumers don't have to
 * branch on the agent name themselves. Keep additions to this map in
 * sync with `Agent` in `src/types.ts`.
 */
export const agentIcons: Record<Agent, Component> = {
  claude: IconClaudeRaw,
  codex: IconCodexRaw,
  agy: IconAgyRaw,
  opencode: IconOpencodeRaw,
  grok: IconGrokRaw,
}

// ---- 文件附件按扩展名分型的图标 ----
export const IconFileDoc = IconMarkdownRaw
export const IconFileSheet = IconFileSheetRaw
export const IconFileSlides = IconFileSlidesRaw
export const IconFileImage = IconFileImageRaw
export const IconFileVideo = IconFileVideoRaw
export const IconFileAudio = IconFileAudioRaw
export const IconFileArchive = IconFileArchiveRaw
export const IconFileCode = IconHtmlRaw

// 扩展名 → 图标。同类多扩展共用一个图标，未命中回落到通用 file 图标。
const FILE_ICON_BY_EXT: Record<string, Component> = {}
const registerFileIcon = (icon: Component, exts: string[]) => {
  for (const e of exts) FILE_ICON_BY_EXT[e] = icon
}
registerFileIcon(IconMarkdownRaw, [
  'txt', 'text', 'log', 'md', 'markdown', 'mdx', 'rtf', 'doc', 'docx', 'odt', 'pages', 'pdf',
])
registerFileIcon(IconFileSheetRaw, ['xls', 'xlsx', 'csv', 'tsv', 'ods', 'numbers'])
registerFileIcon(IconFileSlidesRaw, ['ppt', 'pptx', 'odp', 'key'])
registerFileIcon(IconFileImageRaw, [
  'png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'heic', 'heif', 'avif', 'tiff', 'tif', 'ico', 'svg',
])
registerFileIcon(IconFileVideoRaw, ['mp4', 'mov', 'avi', 'mkv', 'webm', 'm4v', 'flv', 'wmv', 'mpeg', 'mpg'])
registerFileIcon(IconFileAudioRaw, ['mp3', 'wav', 'flac', 'aac', 'm4a', 'ogg', 'opus', 'wma', 'aiff'])
registerFileIcon(IconFileArchiveRaw, ['zip', 'rar', '7z', 'tar', 'gz', 'tgz', 'bz2', 'xz', 'zst', 'zstd'])
registerFileIcon(IconJsonRaw, ['json', 'jsonc', 'json5'])
registerFileIcon(IconHtmlRaw, [
  'js', 'mjs', 'cjs', 'jsx', 'ts', 'tsx', 'vue', 'svelte', 'py', 'rb', 'php', 'go', 'rs',
  'java', 'kt', 'kts', 'c', 'h', 'cpp', 'cc', 'cxx', 'hpp', 'cs', 'swift', 'dart', 'scala',
  'sh', 'bash', 'zsh', 'sql', 'html', 'htm', 'css', 'scss', 'sass', 'less', 'xml',
  'yaml', 'yml', 'toml', 'ini',
])

/** 取文件名末段扩展名对应的图标；无扩展名（含 `.gitignore` 这类无后缀点文件）回落到通用图标。 */
export function fileIconFor(path: string): Component {
  const name = path.replace(/[/\\]+$/, '')
  const slash = Math.max(name.lastIndexOf('/'), name.lastIndexOf('\\'))
  const dot = name.lastIndexOf('.')
  if (dot <= slash + 1) return IconFileRaw
  return FILE_ICON_BY_EXT[name.slice(dot + 1).toLowerCase()] ?? IconFileRaw
}
