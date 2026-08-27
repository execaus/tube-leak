import { describe, expect, it } from 'vitest'

import type { DownloadCommandErrorKind } from '@/types/generated/download'
import { knownKindsOf } from '@/utils/knownKinds'

import { getDownloadCommandErrorText } from './downloadCommandErrorTexts'

/**
 * Выведено из типа (TL-52), а не рукописный массив — см. doc
 * `@/utils/knownKinds`.
 */
const ALL_KINDS = knownKindsOf<DownloadCommandErrorKind>({
  alreadyActive: true,
  unknownTask: true,
  notFailed: true,
  notRetryable: true,
  noStreamsSelected: true,
  invalidUrl: true,
})

describe('getDownloadCommandErrorText — 6 классов отказа команд управления загрузкой', () => {
  it('returns a non-empty title and explanation for every class', () => {
    for (const kind of ALL_KINDS) {
      const text = getDownloadCommandErrorText(kind)
      expect(text.title.length).toBeGreaterThan(0)
      expect(text.explanation.length).toBeGreaterThan(0)
    }
  })

  it('the function signature has no parameter for the diagnostic message field at all', () => {
    expect(getDownloadCommandErrorText.length).toBe(1)
  })
})
