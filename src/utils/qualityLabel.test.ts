import { describe, expect, it } from 'vitest'

import type { QualityItem } from '@/types/probe'

import { qualityLabel } from './qualityLabel'

function item(partial: Partial<QualityItem> & Pick<QualityItem, 'kind'>): QualityItem {
  return { size: { kind: 'unknown' }, streams: {}, ...partial }
}

describe('qualityLabel', () => {
  it('labels a standard step by its height', () => {
    expect(qualityLabel(item({ kind: 'standard', heightPx: 1080 }))).toBe('1080p')
    expect(qualityLabel(item({ kind: 'standard', heightPx: 2160 }))).toBe('2160p')
  })

  it('labels the maxAvailable step descriptively (Р-1)', () => {
    expect(qualityLabel(item({ kind: 'maxAvailable', heightPx: 480 }))).toBe('Максимальное доступное (480p)')
  })

  it('labels the audioOnly step with a fixed caption', () => {
    expect(qualityLabel(item({ kind: 'audioOnly' }))).toBe('Только аудио')
  })

  it('falls back to a safe caption instead of printing "undefinedp" when heightPx is missing (contract drift)', () => {
    expect(qualityLabel(item({ kind: 'standard', heightPx: undefined }))).toBe('Видео')
    expect(qualityLabel(item({ kind: 'maxAvailable', heightPx: undefined }))).toBe(
      'Максимальное доступное качество',
    )
    expect(qualityLabel(item({ kind: 'standard', heightPx: undefined }))).not.toContain('undefined')
    expect(qualityLabel(item({ kind: 'maxAvailable', heightPx: undefined }))).not.toContain('undefined')
  })
})
