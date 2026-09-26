import { getProbeErrorText } from '@/utils/probeErrorTexts'

import type { DownloadErrorKind } from '@/types/generated/download'
import type { YtDlpFailureReason } from '@/types/generated/probe'

/**
 * Тексты для десяти классов отказа скачивания (Ф-10, TL-130 добавил
 * `streamsMissing`), строго по таблице раздела «Ошибки» дизайна E3.
 *
 * # Нормативно: не `DownloadError.message`
 *
 * `message` — формулировка ядра для свёрнутого «Подробнее» и лога, не
 * основной текст экрана (см. doc-комментарий `DownloadError.message` в
 * `src/types/generated/download.ts`). Тот же приём, что `getProbeErrorText` в E2
 * (см. `src/utils/probeErrorTexts.ts`): {@link getDownloadErrorText}
 * принимает только `kind` и `reason` — примитивы контракта, не объект
 * ошибки целиком, — так что подать сюда `error.message` вместо `kind` не
 * получится даже по ошибке: типы не совпадают структурно (`message` —
 * произвольная строка, `kind` — сужённый литеральный union).
 *
 * # Кнопка «Повторить» — не отсюда
 *
 * В отличие от `ProbeErrorText`, здесь нет поля `canRetry`: панель решает,
 * показывать ли «Повторить», по `DownloadError.retryable` — полю,
 * присланному ядром (проекция `DownloadErrorKind.is_retryable`), а не по
 * собственной копии таблицы классов (doc-комментарий `retryable` в
 * `src/types/generated/download.ts`). Дублировать здесь ту же таблицу второй раз —
 * значит завести два источника истины, которые разойдутся при первом же
 * уточнении дизайна.
 */
export interface DownloadErrorText {
  title: string
  explanation: string
}

const CONNECTION_LOST_TEXT: DownloadErrorText = {
  title: 'Соединение потеряно',
  explanation:
    'Не получилось докачать ролик — попытки закончились, а сеть за это время не восстановилась.',
}

const DISK_FULL_TEXT: DownloadErrorText = {
  title: 'Не хватает места на диске',
  explanation: 'Освободите место и попробуйте ещё раз — докачка продолжится с того же места.',
}

const STALE_FORMAT_TEXT: DownloadErrorText = {
  title: 'Данные о ролике устарели',
  explanation:
    'Формат, который вы выбрали, больше не доступен — YouTube мог обновить список дорожек. Обновите карточку ролика выше и выберите качество заново.',
}

const MERGE_FAILED_TEXT: DownloadErrorText = {
  title: 'Не удалось склеить видео и звук',
  explanation: 'Оба потока скачались, но объединить их в один файл не получилось.',
}

/**
 * `streamsMissing` (TL-130, живой дефект #137 на Windows): yt-dlp
 * завершился, но приложение не нашло файлы скачанных потоков — склеивать
 * (и вообще запускать ffmpeg) было нечего. Раньше этот исход попадал под
 * {@link MERGE_FAILED_TEXT}, и все три её утверждения — «оба потока
 * скачались», «склейка», «уже скачанное осталось на диске» (примечание
 * `getFailedPartialDataNote`) — были неправдой: владелец прочитал текст
 * про сбой склейки и пошёл искать несуществующую поломку ffmpeg. Текст
 * ниже не называет причину пропажи файлов (её не знает и ядро) и не
 * утверждает, что на диске что-то осталось или не осталось — это решает
 * отдельно `partialData` (см. doc-комментарий {@link getFailedPartialDataNote}
 * в `src/utils/downloadOutcomeTexts.ts`).
 */
const STREAMS_MISSING_TEXT: DownloadErrorText = {
  title: 'Файлы потоков не найдены',
  explanation: 'Приложение не получило файлы скачанных потоков. Попробуйте ещё раз — потоки скачаются заново.',
}

const DESTINATION_UNAVAILABLE_TEXT: DownloadErrorText = {
  title: 'Папка «Загрузки» недоступна',
  explanation: 'Нет прав на запись или папка была удалена. Проверьте папку и попробуйте ещё раз.',
}

const YTDLP_FAILURE_GENERIC_TEXT: DownloadErrorText = {
  title: 'Не удалось скачать ролик',
  explanation: 'Попробуйте ещё раз.',
}

/**
 * Под-причина «yt-dlp устарел» (CLAUDE.md: пользователь обязан отличать её
 * от прочих сбоев).
 *
 * Заголовок — из таблицы **E3** (`YTDLP_FAILURE_GENERIC_TEXT.title`), не
 * заголовок экрана разбора E2 (ревью TL-45): там заголовок про получение
 * данных о ролике («Не удалось получить данные о ролике») — неправда для
 * этой панели, где данные уже получены и качало именно скачивание.
 * Пояснение, наоборот, берётся дословно из E2
 * (`getProbeErrorText('ytDlpFailure', 'outdated').explanation`), чтобы
 * различающая формулировка про устаревший yt-dlp была одной и той же на
 * обоих экранах — инвариант CLAUDE.md выполняется без противоречия с
 * «заголовок должен соответствовать тому, что не удалось».
 */
const YTDLP_FAILURE_OUTDATED_TEXT: DownloadErrorText = {
  title: YTDLP_FAILURE_GENERIC_TEXT.title,
  explanation: getProbeErrorText('ytDlpFailure', 'outdated').explanation,
}

/**
 * Текст по классу отказа скачивания, строго из таблицы дизайна. Три
 * класса (`videoUnavailable`, `signInRequired`, `regionBlocked`)
 * переиспользуют формулировки E2 один в один (дизайн E3, таблица
 * «Ошибки»: «как в E2») — вместо второй копии тех же строк.
 */
export function getDownloadErrorText(
  kind: DownloadErrorKind,
  reason?: YtDlpFailureReason,
): DownloadErrorText {
  switch (kind) {
    case 'connectionLost':
      return CONNECTION_LOST_TEXT
    case 'diskFull':
      return DISK_FULL_TEXT
    case 'staleFormat':
      return STALE_FORMAT_TEXT
    case 'mergeFailed':
      return MERGE_FAILED_TEXT
    case 'streamsMissing':
      return STREAMS_MISSING_TEXT
    case 'destinationUnavailable':
      return DESTINATION_UNAVAILABLE_TEXT
    case 'videoUnavailable':
    case 'signInRequired':
    case 'regionBlocked':
      return getProbeErrorText(kind)
    case 'ytDlpFailure':
      return reason === 'outdated' ? YTDLP_FAILURE_OUTDATED_TEXT : YTDLP_FAILURE_GENERIC_TEXT
  }
}
