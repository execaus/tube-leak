import { describe, expect, it } from 'vitest'

import type { ProbeError, ProbeErrorKind } from '@/types/probe'

import { getProbeErrorText, NON_CONTRACTUAL_FAILURE_TEXT } from './probeErrorTexts'

/**
 * Нормативный тест TL-33 (раздел «Решения по контракту, принятые на
 * TL-27», риск подмены `message`): для каждого из девяти классов
 * заголовок и пояснение берутся из таблицы дизайна, а не из
 * `ProbeError.message`. Каждая фикстура ниже намеренно кладёт в `message`
 * текст, который не совпадает ни с одним текстом таблицы — если бы
 * реализация ошибочно рендерила `message`, эта фикстура немедленно
 * провалила бы сравнение.
 */
const DECOY_MESSAGE = 'DECOY: диагностика ядра, не текст для пользователя'

const fixtures: Array<{
  error: ProbeError
  expectedTitle: string
  expectedExplanation: string
  expectedCanRetry: boolean
}> = [
  {
    error: { kind: 'notAUrl', message: DECOY_MESSAGE },
    expectedTitle: 'Это не ссылка',
    expectedExplanation:
      'Это не похоже на ссылку на ролик YouTube. Проверьте, что скопировали именно адрес страницы (https://…).',
    expectedCanRetry: false,
  },
  {
    error: { kind: 'videoUnavailable', message: DECOY_MESSAGE },
    expectedTitle: 'Ролик недоступен',
    expectedExplanation: 'Похоже, он удалён, скрыт или никогда не существовал.',
    expectedCanRetry: true,
  },
  {
    error: { kind: 'signInRequired', message: DECOY_MESSAGE },
    expectedTitle: 'Требуется вход в аккаунт YouTube',
    expectedExplanation:
      'В этой версии такие ролики не поддерживаются — ни с возрастным ограничением, ни по подписке.',
    expectedCanRetry: false,
  },
  {
    error: { kind: 'regionBlocked', message: DECOY_MESSAGE },
    expectedTitle: 'Недоступно в вашем регионе',
    expectedExplanation: 'Владелец ролика ограничил его показ для вашей страны.',
    expectedCanRetry: false,
  },
  {
    error: { kind: 'networkUnavailable', message: DECOY_MESSAGE },
    expectedTitle: 'Нет соединения с интернетом',
    expectedExplanation: 'Проверьте подключение и попробуйте ещё раз.',
    expectedCanRetry: true,
  },
  {
    error: { kind: 'playlistUnsupported', message: DECOY_MESSAGE },
    expectedTitle: 'Плейлисты и каналы пока не поддерживаются',
    expectedExplanation: 'Вставьте ссылку на отдельный ролик.',
    expectedCanRetry: false,
  },
  {
    error: { kind: 'liveUnsupported', message: DECOY_MESSAGE },
    expectedTitle: 'Прямые трансляции не поддерживаются',
    expectedExplanation: 'Дождитесь окончания эфира — запись обычного ролика разбирается как всегда.',
    expectedCanRetry: false,
  },
  {
    error: { kind: 'ytDlpFailure', reason: 'generic', message: DECOY_MESSAGE },
    expectedTitle: 'Не удалось получить данные о ролике',
    expectedExplanation: 'Попробуйте ещё раз.',
    expectedCanRetry: true,
  },
  {
    error: { kind: 'ytDlpFailure', reason: 'outdated', message: DECOY_MESSAGE },
    expectedTitle: 'Не удалось получить данные о ролике',
    expectedExplanation:
      'Похоже, встроенный yt-dlp устарел и не понимает текущий ответ YouTube. Обновление появится позже (E6); попробуйте другой ролик.',
    expectedCanRetry: true,
  },
  {
    error: { kind: 'timeout', timeoutSecs: 30, message: DECOY_MESSAGE },
    expectedTitle: 'Разбор не завершился',
    expectedExplanation:
      'Не удалось получить данные за отведённое время (30 с). Возможно, медленное соединение или временная проблема на стороне YouTube.',
    expectedCanRetry: true,
  },
]

describe('getProbeErrorText — таблица, не message (все девять классов Ф-6)', () => {
  it.each(fixtures)('kind=$error.kind: заголовок и пояснение из таблицы', (fixture) => {
    const text = getProbeErrorText(fixture.error.kind, fixture.error.reason, fixture.error.timeoutSecs)

    expect(text.title).toBe(fixture.expectedTitle)
    expect(text.explanation).toBe(fixture.expectedExplanation)
    expect(text.canRetry).toBe(fixture.expectedCanRetry)

    // Самое главное: ни заголовок, ни пояснение не совпадают с `message`
    // фикстуры и не содержат его — таблица используется независимо от
    // содержимого message.
    expect(text.title).not.toBe(fixture.error.message)
    expect(text.explanation).not.toBe(fixture.error.message)
    expect(text.title).not.toContain('DECOY')
    expect(text.explanation).not.toContain('DECOY')
  })

  it('covers exactly the nine classes declared in the contract', () => {
    const kinds: ProbeErrorKind[] = fixtures.map((f) => f.error.kind)
    const uniqueKinds = new Set(kinds)
    // ytDlpFailure появляется дважды (generic/outdated) — те же 9 классов Ф-6.
    expect(uniqueKinds.size).toBe(9)
  })

  it('signature does not accept `message` at all — a mix-up is not just discouraged, it is impossible', () => {
    expect(getProbeErrorText.length).toBe(3)
  })
})

describe('NON_CONTRACTUAL_FAILURE_TEXT', () => {
  it('provides an honest fallback for a reject that is not one of the nine classes', () => {
    expect(NON_CONTRACTUAL_FAILURE_TEXT.title).toBeTruthy()
    expect(NON_CONTRACTUAL_FAILURE_TEXT.explanation).toBeTruthy()
    expect(NON_CONTRACTUAL_FAILURE_TEXT.canRetry).toBe(true)
  })
})
