import { invoke } from '@tauri-apps/api/core'
import { ref, type Ref } from 'vue'

import type { SidecarCheckReport } from '@/types/generated/sidecar'

const CHECK_SIDECAR_COMMAND = 'check_sidecar'

/**
 * Единственная точка входа фронтенда к Tauri-команде `check_sidecar`
 * (Ф-9 эпика E1). Фронтенд не запускает процессы и не трогает файловую
 * систему напрямую (CLAUDE.md, Ф-3) — только через `invoke`.
 */
export async function checkSidecar(): Promise<SidecarCheckReport> {
  return invoke<SidecarCheckReport>(CHECK_SIDECAR_COMMAND)
}

export interface UseSidecarCheckReturn {
  report: Ref<SidecarCheckReport | undefined>
  error: Ref<unknown>
  isLoading: Ref<boolean>
  check: () => Promise<void>
}

/**
 * Composable-обёртка над {@link checkSidecar} для использования в компонентах:
 * хранит последний результат, статус загрузки и ошибку.
 */
export function useSidecarCheck(): UseSidecarCheckReturn {
  const report = ref<SidecarCheckReport>()
  const error = ref<unknown>()
  const isLoading = ref(false)

  async function check(): Promise<void> {
    isLoading.value = true
    error.value = undefined
    try {
      report.value = await checkSidecar()
    } catch (err) {
      error.value = err
    } finally {
      isLoading.value = false
    }
  }

  return { report, error, isLoading, check }
}
