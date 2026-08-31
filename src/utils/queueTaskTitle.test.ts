import { describe, expect, it } from 'vitest'

import type { SelectedQuality } from '@/types/generated/queue'

import { formatTaskDisplayTitle } from './queueTaskTitle'

describe('formatTaskDisplayTitle', () => {
  it('formats «title» — quality-label, the same shape App.vue built for DownloadTask.displayTitle in E3', () => {
    const quality: SelectedQuality = { kind: 'standard', heightPx: 1080 }
    expect(formatTaskDisplayTitle('Как приручить дракона', quality)).toBe('«Как приручить дракона» — 1080p')
  })

  it('works for audioOnly, which has no heightPx', () => {
    const quality: SelectedQuality = { kind: 'audioOnly' }
    expect(formatTaskDisplayTitle('Ролик A', quality)).toBe('«Ролик A» — Только аудио')
  })
})
