import { describe, expect, it } from 'vitest'

import { looksLikeUrl } from './looksLikeUrl'

describe('looksLikeUrl', () => {
  it.each([
    'https://www.youtube.com/watch?v=dQw4w9WgXcQ',
    'http://youtu.be/dQw4w9WgXcQ',
    'HTTPS://WWW.YOUTUBE.COM/watch?v=x',
    '  https://youtu.be/x  ',
  ])('accepts %s', (value) => {
    expect(looksLikeUrl(value)).toBe(true)
  })

  it.each([
    '',
    '   ',
    'просто текст',
    '-о--',
    '/etc/passwd',
    'www.youtube.com/watch?v=x',
    'ftp://example.com/video',
    'javascript:alert(1)',
  ])('rejects %s', (value) => {
    expect(looksLikeUrl(value)).toBe(false)
  })
})
