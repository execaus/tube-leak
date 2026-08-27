import { describe, expect, it } from 'vitest'

import type { YtDlpFailureReason } from '@/types/generated/probe'

import { getProbeErrorText, NON_CONTRACTUAL_FAILURE_TEXT, type BlockProbeErrorKind } from './probeErrorTexts'

/**
 * Нормативный тест TL-33 (раздел «Решения по контракту, принятые на
 * TL-27», риск подмены `message`): для каждого из восьми классов,
 * которые рисуются блоком (`notAUrl` — инлайн, не блок, см.
 * `BlockProbeErrorKind`), заголовок и пояснение берутся из таблицы
 * дизайна, а не из `ProbeError.message`. Каждая фикстура ниже намеренно
 * кладёт в `message` текст, который не совпадает ни с одним текстом
 * таблицы — если бы реализация ошибочно рендерила `message`, эта
 * фикстура немедленно провалила бы сравнение.
 */
const DECOY_MESSAGE = 'DECOY: диагностика ядра, не текст для пользователя'

interface Fixture {
  kind: BlockProbeErrorKind
  message: string
  reason?: YtDlpFailureReason
  timeoutSecs?: number
  expectedTitle: string
  expectedExplanation: string
  expectedCanRetry: boolean
}

type FixtureVariant = Omit<Fixture, 'kind'>

/**
 * Один вариант на класс — кроме `ytDlpFailure`, у которого их два (`reason`
 * `generic`/`outdated`), поэтому значение — массив, а не голая фикстура
 * (ревью TL-52 просило `Record<BlockProbeErrorKind, Fixture>`, но у этого
 * единственного класса структурно два ожидаемых текста, а не один).
 * Полнота по-прежнему проверяется компилятором: `Record<BlockProbeErrorKind,
 * FixtureVariant[]>` требует ровно восемь ключей — забытый класс не
 * скомпилируется, а не тихо выпадет из перебора `it.each`, как было бы с
 * рукописным плоским массивом.
 */
const FIXTURES_BY_KIND: Record<BlockProbeErrorKind, FixtureVariant[]> = {
  videoUnavailable: [
    {
      message: DECOY_MESSAGE,
      expectedTitle: 'Ролик недоступен',
      expectedExplanation: 'Похоже, он удалён, скрыт или никогда не существовал.',
      expectedCanRetry: true,
    },
  ],
  signInRequired: [
    {
      message: DECOY_MESSAGE,
      expectedTitle: 'Требуется вход в аккаунт YouTube',
      expectedExplanation:
        'В этой версии такие ролики не поддерживаются — ни с возрастным ограничением, ни по подписке.',
      expectedCanRetry: false,
    },
  ],
  regionBlocked: [
    {
      message: DECOY_MESSAGE,
      expectedTitle: 'Недоступно в вашем регионе',
      expectedExplanation: 'Владелец ролика ограничил его показ для вашей страны.',
      expectedCanRetry: false,
    },
  ],
  networkUnavailable: [
    {
      message: DECOY_MESSAGE,
      expectedTitle: 'Нет соединения с интернетом',
      expectedExplanation: 'Проверьте подключение и попробуйте ещё раз.',
      expectedCanRetry: true,
    },
  ],
  playlistUnsupported: [
    {
      message: DECOY_MESSAGE,
      expectedTitle: 'Плейлисты и каналы пока не поддерживаются',
      expectedExplanation: 'Вставьте ссылку на отдельный ролик.',
      expectedCanRetry: false,
    },
  ],
  liveUnsupported: [
    {
      message: DECOY_MESSAGE,
      expectedTitle: 'Прямые трансляции не поддерживаются',
      expectedExplanation: 'Дождитесь окончания эфира — запись обычного ролика разбирается как всегда.',
      expectedCanRetry: false,
    },
  ],
  ytDlpFailure: [
    {
      reason: 'generic',
      message: DECOY_MESSAGE,
      expectedTitle: 'Не удалось получить данные о ролике',
      expectedExplanation: 'Попробуйте ещё раз.',
      expectedCanRetry: true,
    },
    {
      reason: 'outdated',
      message: DECOY_MESSAGE,
      expectedTitle: 'Не удалось получить данные о ролике',
      expectedExplanation:
        'Похоже, встроенный yt-dlp устарел и не понимает текущий ответ YouTube. Обновление появится позже (E6); попробуйте другой ролик.',
      expectedCanRetry: true,
    },
  ],
  timeout: [
    {
      timeoutSecs: 30,
      message: DECOY_MESSAGE,
      expectedTitle: 'Разбор не завершился',
      expectedExplanation:
        'Не удалось получить данные за отведённое время (30 с). Возможно, медленное соединение или временная проблема на стороне YouTube.',
      expectedCanRetry: true,
    },
  ],
}

const fixtures: Fixture[] = Object.entries(FIXTURES_BY_KIND).flatMap(([kind, variants]) =>
  variants.map((variant) => ({ kind: kind as BlockProbeErrorKind, ...variant })),
)

describe('getProbeErrorText — таблица, не message (восемь блочных классов Ф-6)', () => {
  it.each(fixtures)('kind=$kind: заголовок и пояснение из таблицы', (fixture) => {
    const text = getProbeErrorText(fixture.kind, fixture.reason, fixture.timeoutSecs)

    expect(text.title).toBe(fixture.expectedTitle)
    expect(text.explanation).toBe(fixture.expectedExplanation)
    expect(text.canRetry).toBe(fixture.expectedCanRetry)

    // Самое главное: ни заголовок, ни пояснение не совпадают с `message`
    // фикстуры и не содержат его — таблица используется независимо от
    // содержимого message.
    expect(text.title).not.toBe(fixture.message)
    expect(text.explanation).not.toBe(fixture.message)
    expect(text.title).not.toContain('DECOY')
    expect(text.explanation).not.toContain('DECOY')
  })

  it('signature does not accept `message` at all — a mix-up is not just discouraged, it is impossible', () => {
    expect(getProbeErrorText.length).toBe(3)
  })

  it('signature does not accept notAUrl at all (blocker fix) — TypeScript rejects it at compile time', () => {
    // @ts-expect-error notAUrl не относится к блочным классам — не должен собираться.
    getProbeErrorText('notAUrl')
  })
})

describe('NON_CONTRACTUAL_FAILURE_TEXT', () => {
  it('provides an honest fallback for a reject that is not one of the nine classes', () => {
    expect(NON_CONTRACTUAL_FAILURE_TEXT.title).toBeTruthy()
    expect(NON_CONTRACTUAL_FAILURE_TEXT.explanation).toBeTruthy()
    expect(NON_CONTRACTUAL_FAILURE_TEXT.canRetry).toBe(true)
  })
})
