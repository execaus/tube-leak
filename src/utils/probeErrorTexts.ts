import type { ProbeErrorKind, YtDlpFailureReason } from '@/types/probe'

/**
 * Тексты и правило показа «Повторить» для восьми классов ошибок разбора,
 * которые рисуются блоком (`role="alert"`), строго по таблице раздела
 * «Состояния» дизайна E2. Девятый класс, `notAUrl`, здесь нет вовсе —
 * даже пришедший из ядра (Ф-2: полная валидация — на стороне Rust,
 * фронтовая проверка лишь подсказка), он сводится к тому же
 * состоянию, что и мгновенная фронтовая проверка (см. doc-комментарий
 * `useLinkProbe` в `src/composables/useProbe.ts`), и рисуется инлайн под
 * полем, а не блоком: у него нет заголовка блочного представления в
 * таблице дизайна (там прочерк) и нет технических деталей («Подробнее») —
 * процесс не запускался. Поэтому `kind` здесь сужен через `Exclude` —
 * подать сюда `notAUrl` не получится даже по ошибке (блокер ревью TL-33).
 *
 * # Нормативно: не `ProbeError.message`
 *
 * `message` — формулировка ядра для свёрнутого «Подробнее» и лога, не
 * основной текст экрана (см. doc-комментарий `ProbeError.message` в
 * `src/types/probe.ts` и раздел «Решения по контракту, принятые на TL-27»
 * эпика E2 — риск подмены реален именно потому, что `message` заполнено
 * читаемым русским текстом и тестами эта подмена сама по себе не ловится).
 *
 * Поэтому {@link getProbeErrorText} **сознательно не принимает `message`
 * вовсе** — воспользоваться им по ошибке здесь структурно невозможно, а не
 * только «не рекомендуется».
 */
export interface ProbeErrorText {
  title: string
  explanation: string
  /** Показывать ли кнопку «Повторить» — не для всех классов повтор осмыслен. */
  canRetry: boolean
}

/** Классы, которые рисуются блоком ошибки — все, кроме `notAUrl` (см. doc выше). */
export type BlockProbeErrorKind = Exclude<ProbeErrorKind, 'notAUrl'>

const VIDEO_UNAVAILABLE_TEXT: ProbeErrorText = {
  title: 'Ролик недоступен',
  explanation: 'Похоже, он удалён, скрыт или никогда не существовал.',
  canRetry: true,
}

const SIGN_IN_REQUIRED_TEXT: ProbeErrorText = {
  title: 'Требуется вход в аккаунт YouTube',
  explanation:
    'В этой версии такие ролики не поддерживаются — ни с возрастным ограничением, ни по подписке.',
  canRetry: false,
}

const REGION_BLOCKED_TEXT: ProbeErrorText = {
  title: 'Недоступно в вашем регионе',
  explanation: 'Владелец ролика ограничил его показ для вашей страны.',
  canRetry: false,
}

const NETWORK_UNAVAILABLE_TEXT: ProbeErrorText = {
  title: 'Нет соединения с интернетом',
  explanation: 'Проверьте подключение и попробуйте ещё раз.',
  canRetry: true,
}

const PLAYLIST_UNSUPPORTED_TEXT: ProbeErrorText = {
  title: 'Плейлисты и каналы пока не поддерживаются',
  explanation: 'Вставьте ссылку на отдельный ролик.',
  canRetry: false,
}

const LIVE_UNSUPPORTED_TEXT: ProbeErrorText = {
  title: 'Прямые трансляции не поддерживаются',
  explanation: 'Дождитесь окончания эфира — запись обычного ролика разбирается как всегда.',
  canRetry: false,
}

const YTDLP_FAILURE_GENERIC_TEXT: ProbeErrorText = {
  title: 'Не удалось получить данные о ролике',
  explanation: 'Попробуйте ещё раз.',
  canRetry: true,
}

const YTDLP_FAILURE_OUTDATED_TEXT: ProbeErrorText = {
  title: 'Не удалось получить данные о ролике',
  explanation:
    'Похоже, встроенный yt-dlp устарел и не понимает текущий ответ YouTube. Обновление появится позже (E6); попробуйте другой ролик.',
  canRetry: true,
}

function timeoutText(timeoutSecs: number | undefined): ProbeErrorText {
  const seconds = timeoutSecs ?? '—'
  return {
    title: 'Разбор не завершился',
    explanation: `Не удалось получить данные за отведённое время (${String(seconds)} с). Возможно, медленное соединение или временная проблема на стороне YouTube.`,
    canRetry: true,
  }
}

/**
 * Текст по классу ошибки, строго из таблицы дизайна. Сигнатура не
 * принимает `message` — подмена структурно невозможна (см. doc выше); не
 * принимает и `notAUrl` — по той же причине структурно, не только по
 * соглашению.
 */
export function getProbeErrorText(
  kind: BlockProbeErrorKind,
  reason?: YtDlpFailureReason,
  timeoutSecs?: number,
): ProbeErrorText {
  switch (kind) {
    case 'videoUnavailable':
      return VIDEO_UNAVAILABLE_TEXT
    case 'signInRequired':
      return SIGN_IN_REQUIRED_TEXT
    case 'regionBlocked':
      return REGION_BLOCKED_TEXT
    case 'networkUnavailable':
      return NETWORK_UNAVAILABLE_TEXT
    case 'playlistUnsupported':
      return PLAYLIST_UNSUPPORTED_TEXT
    case 'liveUnsupported':
      return LIVE_UNSUPPORTED_TEXT
    case 'ytDlpFailure':
      return reason === 'outdated' ? YTDLP_FAILURE_OUTDATED_TEXT : YTDLP_FAILURE_GENERIC_TEXT
    case 'timeout':
      return timeoutText(timeoutSecs)
  }
}

/**
 * Текст для отказа, не подпадающего под контракт (не один из девяти
 * классов) — например, паника команды или отказ самого IPC-вызова. Тот же
 * приём, что `YtDlpPrepareError`'s FALLBACK_EXPLANATION в E1: пустой текст
 * хуже честного «не удалось разобрать причину».
 */
export const NON_CONTRACTUAL_FAILURE_TEXT: ProbeErrorText = {
  title: 'Не удалось получить данные о ролике',
  explanation: 'Не удалось разобрать причину ошибки. Попробуйте ещё раз.',
  canRetry: true,
}
