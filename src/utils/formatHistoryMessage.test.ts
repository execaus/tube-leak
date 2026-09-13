import { describe, expect, it } from 'vitest'

import { formatHistoryMessage } from './formatHistoryMessage'

describe('formatHistoryMessage', () => {
  it('joins title and explanation with a colon per the design form', () => {
    expect(formatHistoryMessage('Не удалось открыть проводник', 'Не удалось запустить файловый менеджер операционной системы.')).toBe(
      'Не удалось открыть проводник: Не удалось запустить файловый менеджер операционной системы.',
    )
  })

  it('omits the trailing colon when the explanation is empty (neutral one-line messages)', () => {
    expect(formatHistoryMessage('Этой записи больше нет в истории', '')).toBe('Этой записи больше нет в истории')
  })
})
