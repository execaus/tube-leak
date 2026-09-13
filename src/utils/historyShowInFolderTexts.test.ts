import { describe, expect, it } from 'vitest'

import type { ShowInFolderErrorKind } from '@/types/generated/history'
import { getLauncherFailureDetails, getShowInFolderErrorText } from './historyShowInFolderTexts'

describe('getShowInFolderErrorText', () => {
  it('returns a distinct title per kind, exhaustively over all five contract classes', () => {
    const kinds: ShowInFolderErrorKind[] = [
      { kind: 'fileMissing' },
      { kind: 'folderMissing' },
      { kind: 'launcherFailed', details: {} },
      { kind: 'unknownRecord' },
      { kind: 'unavailable', reason: 'noAccess' },
    ]
    const titles = kinds.map((k) => getShowInFolderErrorText(k).title)
    expect(new Set(titles).size).toBe(kinds.length)
  })

  it('mutation guard: an unhandled kind throws via assertNever', () => {
    expect(() =>
      getShowInFolderErrorText({ kind: 'bogus' } as unknown as ShowInFolderErrorKind),
    ).toThrow()
  })
})

describe('getLauncherFailureDetails', () => {
  it('extracts details only for launcherFailed, undefined for every other kind', () => {
    const details = { exitCode: 1, stderrTail: 'no such file' }
    expect(getLauncherFailureDetails({ kind: 'launcherFailed', details })).toBe(details)
    expect(getLauncherFailureDetails({ kind: 'fileMissing' })).toBeUndefined()
    expect(getLauncherFailureDetails({ kind: 'unknownRecord' })).toBeUndefined()
  })
})
