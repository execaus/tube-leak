import { describe, expect, it } from 'vitest'

import type { DownloadErrorKind } from '@/types/generated/download'
import { knownKindsOf } from '@/utils/knownKinds'

import { getDownloadErrorText } from './downloadErrorTexts'

/**
 * Выведено из типа (TL-52), а не рукописный массив: пропущенный класс
 * ронял бы `npm run type-check`, а не тихо выпадал бы из перебора ниже.
 */
const ALL_KINDS = knownKindsOf({
  connectionLost: true,
  diskFull: true,
  staleFormat: true,
  mergeFailed: true,
  streamsMissing: true,
  destinationUnavailable: true,
  videoUnavailable: true,
  signInRequired: true,
  regionBlocked: true,
  ytDlpFailure: true,
} satisfies Record<DownloadErrorKind, true>)

describe('getDownloadErrorText — 10 классов ошибок скачивания (Ф-10, таблица дизайна E3 + TL-130)', () => {
  it('returns a distinct, non-empty title and explanation for every one of the 10 classes', () => {
    const texts = ALL_KINDS.map((kind) => getDownloadErrorText(kind))
    for (const text of texts) {
      expect(text.title.length).toBeGreaterThan(0)
      expect(text.explanation.length).toBeGreaterThan(0)
    }
    const titles = texts.map((t) => t.title)
    expect(new Set(titles).size).toBeGreaterThan(1)
  })

  it('matches the exact wording of the design table for each class', () => {
    expect(getDownloadErrorText('connectionLost')).toStrictEqual({
      title: 'Соединение потеряно',
      explanation:
        'Не получилось докачать ролик — попытки закончились, а сеть за это время не восстановилась.',
    })
    expect(getDownloadErrorText('diskFull')).toStrictEqual({
      title: 'Не хватает места на диске',
      explanation: 'Освободите место и попробуйте ещё раз — докачка продолжится с того же места.',
    })
    expect(getDownloadErrorText('staleFormat')).toStrictEqual({
      title: 'Данные о ролике устарели',
      explanation:
        'Формат, который вы выбрали, больше не доступен — YouTube мог обновить список дорожек. Обновите карточку ролика выше и выберите качество заново.',
    })
    expect(getDownloadErrorText('mergeFailed')).toStrictEqual({
      title: 'Не удалось склеить видео и звук',
      explanation: 'Оба потока скачались, но объединить их в один файл не получилось.',
    })
    expect(getDownloadErrorText('destinationUnavailable')).toStrictEqual({
      title: 'Папка «Загрузки» недоступна',
      explanation: 'Нет прав на запись или папка была удалена. Проверьте папку и попробуйте ещё раз.',
    })
  })

  it('reuses the E2 wording verbatim for the three carried-over classes', () => {
    expect(getDownloadErrorText('videoUnavailable').title).toBe('Ролик недоступен')
    expect(getDownloadErrorText('signInRequired').title).toBe('Требуется вход в аккаунт YouTube')
    expect(getDownloadErrorText('regionBlocked').title).toBe('Недоступно в вашем регионе')
  })
})

describe('getDownloadErrorText — под-причина «yt-dlp устарел» (CLAUDE.md, требование п.2)', () => {
  it('shows a different explanation for reason "outdated" than for the generic ytDlpFailure', () => {
    const generic = getDownloadErrorText('ytDlpFailure')
    const outdated = getDownloadErrorText('ytDlpFailure', 'outdated')

    expect(outdated.explanation).not.toBe(generic.explanation)
    expect(outdated.explanation).toContain('устарел')
  })

  it('treats reason "generic" the same as no reason at all', () => {
    expect(getDownloadErrorText('ytDlpFailure', 'generic')).toStrictEqual(getDownloadErrorText('ytDlpFailure'))
  })

  it('keeps the E3 title (about failing to download) for both cases — outdated must not borrow the E2 probe-screen title (ревью TL-45)', () => {
    // Заголовок E2 про этот же класс — «Не удалось получить данные о
    // ролике» (получение данных, не скачивание). На панели загрузки это
    // неправда: данные уже получены, карточка построена, качало именно
    // скачивание — поэтому заголовок обязан быть панельным в обоих
    // случаях (с под-причиной и без неё), а меняться должно только
    // пояснение.
    expect(getDownloadErrorText('ytDlpFailure').title).toBe('Не удалось скачать ролик')
    expect(getDownloadErrorText('ytDlpFailure', 'outdated').title).toBe('Не удалось скачать ролик')
    expect(getDownloadErrorText('ytDlpFailure', 'outdated').title).not.toContain('данные о ролике')
  })
})

describe('getDownloadErrorText — «потоков нет» (TL-131, живой дефект #137 на Windows)', () => {
  it('matches the exact wording — replacing it with the old merge-failure text must fail this test', () => {
    expect(getDownloadErrorText('streamsMissing')).toStrictEqual({
      title: 'Файлы потоков не найдены',
      explanation: 'Приложение не получило файлы скачанных потоков. Попробуйте ещё раз — потоки скачаются заново.',
    })
  })

  it('never blames ffmpeg or the merge step — yt-dlp finished, the merge never even started', () => {
    const text = getDownloadErrorText('streamsMissing')
    const combined = `${text.title} ${text.explanation}`.toLowerCase()
    expect(combined).not.toContain('ffmpeg')
    expect(combined).not.toContain('склеи')
    expect(combined).not.toContain('склад')
    expect(combined).not.toContain('объединит')
  })

  it('does not promise that partially downloaded stream files survived — that is `partialData`, not this text', () => {
    const text = getDownloadErrorText('streamsMissing')
    const combined = `${text.title} ${text.explanation}`.toLowerCase()
    expect(combined).not.toContain('осталось на диске')
    expect(combined).not.toContain('оба потока скачались')
  })
})

describe('getDownloadErrorText — нормативный запрет на подмену message (требование п.1)', () => {
  it('the function signature has no parameter for the diagnostic message field at all', () => {
    // Проверка на уровне типов, а не только соглашением: `getDownloadErrorText`
    // принимает `kind` (сужённый литеральный union) и `reason` — но не
    // `message`. Подставить сюда `error.message: string` вместо `kind`
    // не скомпилируется, потому что `string` не сужается до
    // `DownloadErrorKind` автоматически.
    expect(getDownloadErrorText.length).toBe(2)
  })
})
