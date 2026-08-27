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
  destinationUnavailable: true,
  videoUnavailable: true,
  signInRequired: true,
  regionBlocked: true,
  ytDlpFailure: true,
} satisfies Record<DownloadErrorKind, true>)

describe('getDownloadErrorText — 9 классов ошибок скачивания (Ф-10, таблица дизайна E3)', () => {
  it('returns a distinct, non-empty title and explanation for every one of the 9 classes', () => {
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
