import { describe, expect, it } from 'vitest'

import type { DownloadCommandError, DownloadCommandErrorKind } from '@/types/generated/download'
import type { QueueTaskRef } from '@/types/generated/queue'
import { knownKindsOf } from '@/utils/knownKinds'

import { getDownloadCommandErrorText } from './downloadCommandErrorTexts'

/**
 * Выведено из типа (TL-52), а не рукописный массив — см. doc
 * `@/utils/knownKinds`. С TL-70/TL-75: `DownloadCommandErrorKind` стал
 * размеченным объединением объектов (не строк), поэтому белый список
 * строится по `DownloadCommandError['kind']` (правило CLAUDE.md о
 * `satisfies` при разметке объединений) — это тот же самый строковый
 * union тегов, только извлечённый индексированным доступом, а не
 * названный по имени.
 */
const ALL_KINDS = knownKindsOf({
  unknownTask: true,
  notFailed: true,
  notRetryable: true,
  noStreamsSelected: true,
  invalidUrl: true,
  duplicateTask: true,
  taskNotFinished: true,
} satisfies Record<DownloadCommandError['kind'], true>)

const SAMPLE_EXISTING: QueueTaskRef = {
  taskId: 'task-existing',
  title: 'Летний влог',
  quality: { kind: 'standard', heightPx: 720 },
}

/** `duplicateTask` несёт структурные данные — конструируется отдельно от остальных «пустых» тегов. */
function sampleFor(kind: (typeof ALL_KINDS)[number]): DownloadCommandErrorKind {
  if (kind === 'duplicateTask') {
    return { kind, existing: SAMPLE_EXISTING }
  }
  return { kind } as DownloadCommandErrorKind
}

describe('getDownloadCommandErrorText — 7 классов отказа команд управления загрузкой/очередью', () => {
  it('returns a non-empty title and explanation for every class', () => {
    for (const kind of ALL_KINDS) {
      const text = getDownloadCommandErrorText(sampleFor(kind))
      expect(text.title.length).toBeGreaterThan(0)
      expect(text.explanation.length).toBeGreaterThan(0)
    }
  })

  it('the function signature has no separate parameter for the diagnostic message field', () => {
    expect(getDownloadCommandErrorText.length).toBe(1)
  })

  it('duplicateTask cites the existing task by its exact title and quality (Р-5) — not a generic message', () => {
    const text = getDownloadCommandErrorText({ kind: 'duplicateTask', existing: SAMPLE_EXISTING })
    expect(text.explanation).toContain('«Летний влог» — 720p')
  })
})
