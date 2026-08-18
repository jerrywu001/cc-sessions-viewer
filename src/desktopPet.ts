import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { emitTo } from '@tauri-apps/api/event'
import type { Agent, SessionMeta } from './types'

export type DesktopPetCharacter = string

export interface DesktopPetDefinition {
  key: string
  id: string
  displayName: string
  description: string | null
  spriteVersionNumber: 1 | 2
  spritesheetPath: string
  source: 'codex' | 'custom'
}

export interface DesktopPetCatalog {
  pets: DesktopPetDefinition[]
  customDirectory: string
  codexInstalled: boolean
}

export interface DesktopPetPosition {
  x: number
  y: number
}

export type DesktopTaskState = 'started' | 'blocked' | 'completed' | 'failed'

export interface DesktopTask {
  agent: Agent
  path: string
  state: DesktopTaskState
  title: string
  updatedAt: number
}

export interface DesktopPetResolvedSession {
  projectKey: string
  session: SessionMeta
}

const DESKTOP_TASK_PRIORITY: Record<DesktopTaskState, number> = {
  blocked: 0,
  failed: 1,
  completed: 2,
  started: 3,
}

export function sortDesktopTasks(tasks: DesktopTask[]) {
  return tasks.slice().sort((left, right) =>
    DESKTOP_TASK_PRIORITY[left.state] - DESKTOP_TASK_PRIORITY[right.state]
      || right.updatedAt - left.updatedAt,
  )
}

export function dominantDesktopTaskState(tasks: DesktopTask[]): DesktopTaskState | null {
  return sortDesktopTasks(tasks)[0]?.state ?? null
}

export const DESKTOP_PET_MIN_SIZE = 80
export const DESKTOP_PET_MAX_SIZE = 224
export const DESKTOP_PET_DEFAULT_SIZE = 112
export const DESKTOP_PET_DEFAULT_CHARACTER = 'codex:codex'

const ENABLED_KEY = 'desktopPetEnabled:v1'
const CHARACTER_KEY = 'desktopPetCharacter:v1'
const SIZE_KEY = 'desktopPetSize:v1'
const POSITION_KEY = 'desktopPetPosition:v1'
const DEFAULT_CHARACTER = DESKTOP_PET_DEFAULT_CHARACTER

function readCharacter(): DesktopPetCharacter {
  const value = localStorage.getItem(CHARACTER_KEY)
  return value?.includes(':') ? value : DEFAULT_CHARACTER
}

export function clampDesktopPetSize(value: number) {
  if (!Number.isFinite(value)) return DESKTOP_PET_DEFAULT_SIZE
  return Math.round(Math.min(DESKTOP_PET_MAX_SIZE, Math.max(DESKTOP_PET_MIN_SIZE, value)))
}

function readSize() {
  return clampDesktopPetSize(Number(localStorage.getItem(SIZE_KEY) ?? DESKTOP_PET_DEFAULT_SIZE))
}

function readPosition(): DesktopPetPosition | null {
  try {
    const value = JSON.parse(localStorage.getItem(POSITION_KEY) ?? 'null')
    return Number.isFinite(value?.x) && Number.isFinite(value?.y)
      ? { x: Math.round(value.x), y: Math.round(value.y) }
      : null
  } catch {
    return null
  }
}

export const desktopPetEnabled = ref(localStorage.getItem(ENABLED_KEY) === '1')
export const desktopPetCharacter = ref<DesktopPetCharacter>(readCharacter())
export const desktopPetSize = ref(readSize())
export const desktopPetPosition = ref<DesktopPetPosition | null>(readPosition())
export const desktopPetCatalog = ref<DesktopPetCatalog | null>(null)
export const desktopPetCatalogLoading = ref(false)
export const desktopPetCatalogError = ref('')
export const activeDesktopPet = computed(() =>
  desktopPetCatalog.value?.pets.find((pet) => pet.key === desktopPetCharacter.value) ?? null,
)

export function setDesktopPetEnabled(enabled: boolean) {
  desktopPetEnabled.value = enabled
  localStorage.setItem(ENABLED_KEY, enabled ? '1' : '0')
}

export function setDesktopPetCharacter(character: DesktopPetCharacter) {
  desktopPetCharacter.value = character
  localStorage.setItem(CHARACTER_KEY, character)
}

export function setDesktopPetSize(value: number) {
  const size = clampDesktopPetSize(value)
  desktopPetSize.value = size
  localStorage.setItem(SIZE_KEY, String(size))
  return size
}

export function setDesktopPetPosition(position: DesktopPetPosition | null) {
  desktopPetPosition.value = position
  if (position) localStorage.setItem(POSITION_KEY, JSON.stringify(position))
  else localStorage.removeItem(POSITION_KEY)
}

export function resetDesktopPetSettings() {
  setDesktopPetEnabled(false)
  setDesktopPetCharacter(DESKTOP_PET_DEFAULT_CHARACTER)
  setDesktopPetSize(DESKTOP_PET_DEFAULT_SIZE)
  setDesktopPetPosition(null)
}

export async function loadDesktopPetCatalog() {
  desktopPetCatalogLoading.value = true
  desktopPetCatalogError.value = ''
  try {
    const catalog = await invoke<DesktopPetCatalog>('desktop_pet_catalog')
    desktopPetCatalog.value = catalog
    if (!catalog.pets.some((pet) => pet.key === desktopPetCharacter.value)) {
      const fallback = catalog.pets.find((pet) => pet.key === DEFAULT_CHARACTER) ?? catalog.pets[0]
      if (fallback) setDesktopPetCharacter(fallback.key)
    }
    return catalog
  } catch (error) {
    desktopPetCatalogError.value = String(error)
    throw error
  } finally {
    desktopPetCatalogLoading.value = false
  }
}

export const deleteCustomDesktopPet = (petId: string) =>
  invoke<void>('delete_custom_desktop_pet', { petId })

export const updateDesktopPetWindow = (enabled: boolean) =>
  invoke<void>('set_desktop_pet_enabled', { enabled })

export const focusDesktopPetMain = () => invoke<void>('focus_desktop_pet_main')

export const fetchDesktopPetTasks = () => invoke<DesktopTask[]>('desktop_pet_tasks')

export const openDesktopPetSession = (task: Pick<DesktopTask, 'agent' | 'path'>) =>
  invoke<void>('open_desktop_pet_session', { agent: task.agent, path: task.path })

export const acknowledgeDesktopPetTask = (task: Pick<DesktopTask, 'agent' | 'path'>) =>
  invoke<void>('acknowledge_desktop_pet_task', { agent: task.agent, path: task.path })

export const resolveDesktopPetSession = (task: Pick<DesktopTask, 'agent' | 'path'>) =>
  invoke<DesktopPetResolvedSession | null>('resolve_desktop_pet_session', {
    agent: task.agent,
    path: task.path,
  })

export async function restoreDesktopPet() {
  if (desktopPetEnabled.value) await updateDesktopPetWindow(true)
}

async function emitDesktopPetPreferences() {
  try {
    await emitTo('desktop-pet', 'desktop-pet://preferences', {
      character: desktopPetCharacter.value,
      size: desktopPetSize.value,
    })
  } catch {
    // The avatar window is optional and has no listener while tucked away.
  }
}

export async function notifyDesktopPetCharacter(character: DesktopPetCharacter) {
  setDesktopPetCharacter(character)
  await emitDesktopPetPreferences()
}

export async function notifyDesktopPetSize(value: number) {
  setDesktopPetSize(value)
  await emitDesktopPetPreferences()
}
