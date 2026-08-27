import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { SidecarCheckReport, SidecarCheckResult } from '@/types/generated/sidecar'

const invokeMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

// Импортируется после мока модуля, чтобы composable получил замоканный `invoke`.
const { checkSidecar, useSidecarCheck } = await import('./useSidecarCheck')

/**
 * Фикстуры результата проверки sidecar, поле в поле повторяющие данные
 * Rust-контракта TL-1 (`src-tauri/src/types.rs`, `#[cfg(test)] mod tests`).
 */
const okResult: SidecarCheckResult = {
  name: 'yt-dlp',
  path: '/opt/tube-leak/bin/yt-dlp',
  status: 'ok',
  version: '2026.08.01',
}

const notFoundResult: SidecarCheckResult = {
  name: 'ffmpeg',
  path: '/opt/tube-leak/bin/ffmpeg',
  status: 'notFound',
  osErrorCode: 'ENOENT',
}

const launchFailedPermissionDeniedResult: SidecarCheckResult = {
  name: 'yt-dlp',
  path: '/opt/tube-leak/bin/yt-dlp',
  status: 'launchFailed',
  reason: 'permissionDenied',
  osErrorCode: 'EACCES',
}

const launchFailedCorruptedResult: SidecarCheckResult = {
  name: 'ffmpeg',
  path: '/opt/tube-leak/bin/ffmpeg',
  status: 'launchFailed',
  reason: 'corrupted',
  osErrorCode: 'ENOEXEC',
}

const nonZeroExitResult: SidecarCheckResult = {
  name: 'yt-dlp',
  path: '/opt/tube-leak/bin/yt-dlp',
  status: 'nonZeroExit',
  exitCode: 1,
  stderrTail: 'error: unsupported URL',
}

const timeoutResult: SidecarCheckResult = {
  name: 'ffmpeg',
  path: '/opt/tube-leak/bin/ffmpeg',
  status: 'timeout',
  timeoutMs: 5000,
}

beforeEach(() => {
  invokeMock.mockReset()
})

describe('checkSidecar', () => {
  it('calls the check_sidecar Tauri command with no arguments', async () => {
    const report: SidecarCheckReport = { ytDlp: okResult, ffmpeg: okResult }
    invokeMock.mockResolvedValueOnce(report)

    await checkSidecar()

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('check_sidecar')
  })

  it.each([
    ['ok', okResult],
    ['notFound', notFoundResult],
    ['launchFailed (permissionDenied)', launchFailedPermissionDeniedResult],
    ['launchFailed (corrupted)', launchFailedCorruptedResult],
    ['nonZeroExit', nonZeroExitResult],
    ['timeout', timeoutResult],
  ] satisfies Array<[string, SidecarCheckResult]>)(
    'resolves a report matching the %s result field-by-field',
    async (_label, result) => {
      const report: SidecarCheckReport = { ytDlp: result, ffmpeg: okResult }
      invokeMock.mockResolvedValueOnce(report)

      const received = await checkSidecar()

      expect(received).toStrictEqual(report)
      expect(received.ytDlp).toStrictEqual(result)
    },
  )

  it('propagates a rejection from invoke', async () => {
    const failure = new Error('sidecar command unavailable')
    invokeMock.mockRejectedValueOnce(failure)

    await expect(checkSidecar()).rejects.toThrow(failure)
  })
})

describe('useSidecarCheck', () => {
  it('starts with no report, no error and not loading', () => {
    const { report, error, isLoading } = useSidecarCheck()

    expect(report.value).toBeUndefined()
    expect(error.value).toBeUndefined()
    expect(isLoading.value).toBe(false)
  })

  it('populates the report on a successful check and resets loading', async () => {
    const report: SidecarCheckReport = { ytDlp: okResult, ffmpeg: notFoundResult }
    invokeMock.mockResolvedValueOnce(report)

    const state = useSidecarCheck()
    const pending = state.check()
    expect(state.isLoading.value).toBe(true)

    await pending

    expect(state.isLoading.value).toBe(false)
    expect(state.report.value).toStrictEqual(report)
    expect(state.error.value).toBeUndefined()
  })

  it('captures the error and clears loading when invoke rejects', async () => {
    const failure = new Error('sidecar command unavailable')
    invokeMock.mockRejectedValueOnce(failure)

    const state = useSidecarCheck()
    await state.check()

    expect(state.isLoading.value).toBe(false)
    expect(state.error.value).toBe(failure)
    expect(state.report.value).toBeUndefined()
  })
})
