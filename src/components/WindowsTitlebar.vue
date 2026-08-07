<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { MenuHandlers } from '../menu'

export type WindowMenuEntry =
  | { type: 'separator' }
  | {
      type: 'item'
      id: string
      label: string
      shortcut?: string
      checked?: boolean
      disabled?: boolean
    }
  | {
      type: 'submenu'
      label: string
      items: WindowMenuEntry[]
    }

export type WindowMenuGroup = {
  label: string
  items: WindowMenuEntry[]
}

defineProps<{
  menus: WindowMenuGroup[]
  handlers: MenuHandlers
}>()

const activeMenu = ref<string | null>(null)
const titlebarEl = ref<HTMLElement | null>(null)

function minimizeWindow() {
  getCurrentWindow().minimize().catch((e) => console.warn('[window] minimize failed:', e))
}

function toggleMaximizeWindow() {
  getCurrentWindow().toggleMaximize().catch((e) => console.warn('[window] toggle maximize failed:', e))
}

function closeWindow() {
  getCurrentWindow().close().catch((e) => console.warn('[window] close failed:', e))
}

function toggleFullscreenWindow() {
  const win = getCurrentWindow()
  win.isFullscreen()
    .then((fullscreen) => win.setFullscreen(!fullscreen))
    .catch((e) => console.warn('[window] toggle fullscreen failed:', e))
}

function toggleMenu(label: string) {
  activeMenu.value = activeMenu.value === label ? null : label
}

function switchMenu(label: string) {
  if (activeMenu.value) activeMenu.value = label
}

function closeMenu() {
  activeMenu.value = null
}

function onDocumentPointerDown(e: PointerEvent) {
  if (!activeMenu.value) return
  const target = e.target as Node | null
  if (target && titlebarEl.value?.contains(target)) return
  closeMenu()
}

function onDocumentKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') closeMenu()
}

function runMenuEntry(entry: WindowMenuEntry, handlers: MenuHandlers) {
  if (entry.type !== 'item' || entry.disabled) return
  if (entry.id === 'window:minimize') {
    minimizeWindow()
    closeMenu()
    return
  }
  if (entry.id === 'window:maximize') {
    toggleMaximizeWindow()
    closeMenu()
    return
  }
  if (entry.id === 'window:fullscreen') {
    toggleFullscreenWindow()
    closeMenu()
    return
  }
  const handler = handlers[entry.id]
  if (!handler) {
    console.warn('[window-menu] unknown menu id:', entry.id)
    return
  }
  handler()
  closeMenu()
}

onMounted(() => {
  document.addEventListener('pointerdown', onDocumentPointerDown)
  document.addEventListener('keydown', onDocumentKeydown)
})

onUnmounted(() => {
  document.removeEventListener('pointerdown', onDocumentPointerDown)
  document.removeEventListener('keydown', onDocumentKeydown)
})
</script>

<template>
  <div ref="titlebarEl" class="window-titlebar">
    <div class="window-app-id" data-tauri-drag-region>
      <span class="window-app-icon" aria-hidden="true">sv</span>
      <span>Sessions Viewer</span>
    </div>
    <nav
      class="window-menu"
      aria-label="Application menu"
      @pointerdown.stop
      @click.stop
    >
      <div
        v-for="menu in menus"
        :key="menu.label"
        class="window-menu-group"
      >
        <button
          type="button"
          class="window-menu-trigger"
          :class="{ active: activeMenu === menu.label }"
          :aria-expanded="activeMenu === menu.label"
          @pointerdown.stop
          @click.stop="toggleMenu(menu.label)"
          @mouseenter="switchMenu(menu.label)"
        >
          {{ menu.label }}
        </button>
        <div
          v-if="activeMenu === menu.label"
          class="window-menu-panel"
          role="menu"
        >
          <template v-for="(entry, index) in menu.items" :key="`${menu.label}-${index}`">
            <div v-if="entry.type === 'separator'" class="window-menu-separator" role="separator" />
            <div v-else-if="entry.type === 'submenu'" class="window-menu-submenu">
              <button
                type="button"
                class="window-menu-item window-menu-item-parent"
                role="menuitem"
                @pointerdown.stop
                @click.stop
              >
                <span class="window-menu-check" aria-hidden="true" />
                <span class="window-menu-label">{{ entry.label }}</span>
                <span class="window-menu-arrow" aria-hidden="true">›</span>
              </button>
              <div class="window-menu-subpanel" role="menu">
                <template v-for="(child, childIndex) in entry.items" :key="`${entry.label}-${childIndex}`">
                  <div v-if="child.type === 'separator'" class="window-menu-separator" role="separator" />
                  <button
                    v-else-if="child.type === 'item'"
                    type="button"
                    class="window-menu-item"
                    :class="{ checked: child.checked, disabled: child.disabled }"
                    :disabled="child.disabled"
                    role="menuitem"
                    @pointerdown.stop
                    @click.stop="runMenuEntry(child, handlers)"
                  >
                    <span class="window-menu-check" aria-hidden="true">{{ child.checked ? '✓' : '' }}</span>
                    <span class="window-menu-label">{{ child.label }}</span>
                    <span v-if="child.shortcut" class="window-menu-shortcut">{{ child.shortcut }}</span>
                  </button>
                </template>
              </div>
            </div>
            <button
              v-else
              type="button"
              class="window-menu-item"
              :class="{ checked: entry.checked, disabled: entry.disabled }"
              :disabled="entry.disabled"
              role="menuitem"
              @pointerdown.stop
              @click.stop="runMenuEntry(entry, handlers)"
            >
              <span class="window-menu-check" aria-hidden="true">{{ entry.checked ? '✓' : '' }}</span>
              <span class="window-menu-label">{{ entry.label }}</span>
              <span v-if="entry.shortcut" class="window-menu-shortcut">{{ entry.shortcut }}</span>
            </button>
          </template>
        </div>
      </div>
    </nav>
    <div class="window-drag-spacer" data-tauri-drag-region />
    <div
      class="window-controls"
      aria-label="Window controls"
      @pointerdown.stop
      @click.stop
    >
      <button type="button" class="window-control window-control-minimize" aria-label="Minimize" @pointerdown.stop @click.stop="minimizeWindow">
        <span aria-hidden="true" />
      </button>
      <button type="button" class="window-control window-control-maximize" aria-label="Maximize" @pointerdown.stop @click.stop="toggleMaximizeWindow">
        <span aria-hidden="true" />
      </button>
      <button type="button" class="window-control window-control-close" aria-label="Close" @pointerdown.stop @click.stop="closeWindow">
        <span aria-hidden="true" />
      </button>
    </div>
  </div>
</template>

<style scoped>
.window-titlebar {
  height: 38px;
  flex-shrink: 0;
  display: grid;
  grid-template-columns: auto auto minmax(24px, 1fr) auto;
  align-items: center;
  background:
    linear-gradient(to bottom, color-mix(in srgb, var(--surface) 92%, white), var(--surface-2));
  border-bottom: 1px solid var(--border);
  -webkit-app-region: no-drag;
}
:global(:root.has-custom-background .window-titlebar) {
  background: transparent;
}
.window-app-id {
  min-width: 0;
  display: inline-flex;
  align-items: center;
  gap: 7px;
  height: 100%;
  padding-left: 10px;
  padding-right: 14px;
  color: var(--text-dim);
  font-size: 12.5px;
  font-weight: 600;
  white-space: nowrap;
  -webkit-app-region: drag;
}
.window-app-icon {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid color-mix(in srgb, var(--brand) 28%, var(--border));
  border-radius: 4px;
  background: var(--brand-soft);
  color: var(--brand);
  font-size: 8px;
  font-weight: 700;
  line-height: 1;
  text-transform: uppercase;
}
.window-menu {
  min-width: 0;
  width: max-content;
  height: 100%;
  display: flex;
  align-items: center;
  gap: 4px;
  overflow: visible;
  -webkit-app-region: no-drag;
}
.window-menu-group {
  position: relative;
  height: 100%;
  display: flex;
  align-items: center;
  -webkit-app-region: no-drag;
}
.window-menu button,
.window-menu-panel,
.window-controls,
.window-control {
  -webkit-app-region: no-drag;
}
.window-menu-trigger {
  height: 26px;
  padding: 0 9px;
  border-radius: 4px;
  color: var(--text-dim);
  font-size: 12px;
  line-height: 26px;
  white-space: nowrap;
}
.window-menu-trigger:hover,
.window-menu-trigger.active {
  background: var(--surface-hover);
  color: var(--text);
}
.window-menu-panel,
.window-menu-subpanel {
  position: absolute;
  z-index: 80;
  min-width: 236px;
  padding: 5px;
  border: 1px solid var(--border);
  border-radius: 7px;
  background: var(--surface);
  box-shadow: var(--shadow-lg);
}
.window-menu-panel {
  top: 30px;
  left: 0;
}
.window-menu-submenu {
  position: relative;
}
.window-menu-subpanel {
  top: -5px;
  left: calc(100% + 4px);
  display: none;
}
.window-menu-submenu::after {
  content: "";
  position: absolute;
  top: -5px;
  left: 100%;
  width: 8px;
  height: calc(100% + 10px);
}
.window-menu-submenu:hover > .window-menu-subpanel {
  display: block;
}
.window-menu-item {
  width: 100%;
  height: 27px;
  display: grid;
  grid-template-columns: 18px minmax(128px, 1fr) auto;
  align-items: center;
  gap: 8px;
  padding: 0 8px 0 4px;
  border-radius: 5px;
  color: var(--text);
  font-size: 12px;
  line-height: 27px;
  text-align: left;
  white-space: nowrap;
}
.window-menu-item:hover:not(:disabled),
.window-menu-submenu:hover > .window-menu-item {
  background: var(--surface-hover);
}
.window-menu-item.disabled,
.window-menu-item:disabled {
  color: var(--text-mute);
  cursor: default;
  opacity: 0.48;
}
.window-menu-check {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--brand);
  font-size: 11px;
  font-weight: 700;
}
.window-menu-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}
.window-menu-shortcut,
.window-menu-arrow {
  color: var(--text-mute);
  font-size: 11px;
}
.window-menu-arrow {
  font-size: 15px;
  line-height: 1;
}
.window-menu-separator {
  height: 1px;
  margin: 5px 4px;
  background: var(--border);
}
.window-drag-spacer {
  width: 100%;
  height: 100%;
  -webkit-app-region: drag;
}
.window-controls {
  height: 100%;
  display: grid;
  grid-template-columns: repeat(3, 45px);
  -webkit-app-region: no-drag;
}
.window-control {
  width: 45px;
  height: 38px;
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 0;
  color: var(--text-dim);
  line-height: 1;
}
.window-control span {
  position: relative;
  width: 12px;
  height: 12px;
  opacity: 1;
}
.window-control-minimize span::before {
  content: "";
  position: absolute;
  left: 1px;
  right: 1px;
  top: 8px;
  height: 1px;
  background: currentColor;
}
.window-control-maximize span::before {
  content: "";
  position: absolute;
  inset: 2px;
  border: 1px solid currentColor;
}
.window-control-close span::before,
.window-control-close span::after {
  content: "";
  position: absolute;
  left: 1px;
  right: 1px;
  top: 6px;
  height: 1px;
  background: currentColor;
}
.window-control-close span::before {
  transform: rotate(45deg);
}
.window-control-close span::after {
  transform: rotate(-45deg);
}
.window-control:hover {
  background: var(--surface-hover);
  color: var(--text);
}
.window-control-close:hover {
  background: #e81123;
  color: #fff;
}
</style>
