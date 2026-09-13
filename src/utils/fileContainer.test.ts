import { describe, expect, it } from 'vitest'

import { fileContainer } from './fileContainer'

describe('fileContainer', () => {
  it('extracts the lowercase extension after the last dot', () => {
    expect(fileContainer('Как приручить дракона.mp4')).toBe('mp4')
    expect(fileContainer('track.M4A')).toBe('m4a')
    expect(fileContainer('archive.tar.gz')).toBe('gz')
  })

  it('defends against a missing/trailing dot instead of throwing (unreachable by contract, E3 always appends an extension)', () => {
    expect(fileContainer('no-extension')).toBe('')
    expect(fileContainer('trailing.')).toBe('')
  })
})
