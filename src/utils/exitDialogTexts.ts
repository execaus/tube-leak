import type { DownloadProgress } from '@/types/generated/download'

/** Заголовок и текст диалога подтверждения выхода (дизайн E3, TL-46). */
export interface ExitDialogText {
  heading: string
  body: string
}

const HEADING = 'Загрузка ещё не завершена'

/**
 * Фрагмент «что сейчас происходит с задачей», подставляемый сразу после
 * названия ролика — единственное, что различается между текстовыми
 * вариантами (дизайн: «Текст зависит от того, есть ли уже процент... или
 * ещё нет»). Остальной текст диалога — общий на оба варианта.
 *
 * Источник процента и его округление — тот же, что уже использует панель
 * (`DownloadPanel.vue`, `Math.round(progress.percent)`), чтобы число в
 * диалоге не разошлось с тем, что пользователь видит на панели секундами
 * раньше.
 *
 * `merging` не сведён в общий «загрузка ещё готовится» с `queued`/
 * `fetching`: к моменту склейки оба потока уже скачаны целиком, и назвать
 * это «ещё готовится» значило бы соврать про стадию; числового процента
 * же у склейки нет и не может быть (remux, `DownloadProgress` не несёт
 * поля для `merging` — см. `src/types/generated/download.ts`), поэтому для неё —
 * отдельная честная формулировка, а не выдуманное число.
 */
function stateFragment(progress: DownloadProgress): string {
  switch (progress.phase) {
    case 'queued':
    case 'fetching':
      return 'загрузка ещё готовится'
    case 'downloading':
      if (progress.percent === undefined) {
        // Процент ещё не известен (ни оценки, ни данных от yt-dlp) — как и
        // для Fetching/Queued, честно нечего показать, кроме «готовится».
        return 'загрузка ещё готовится'
      }
      return progress.state === 'waitingRetry'
        ? `скачивается (${Math.round(progress.percent)} %, сохранено)`
        : `скачивается (${Math.round(progress.percent)} %)`
    case 'merging':
      return 'идёт склейка видео и звука'
    // Терминальные фазы сюда не доходят — диалог не показывается для
    // terminal-задачи (`useDownloadTaskStore.isActive`, requirement TL-46).
    // Ветка — оборона, а не ожидаемый путь.
    case 'done':
    case 'failed':
    case 'cancelled':
      return 'загрузка ещё готовится'
  }
}

/**
 * Текст диалога (Р-2, дизайн «Диалог подтверждения выхода»). `displayTitle`
 * — тот же снимок «название + качество», что уже держит панель
 * (`DownloadTask.displayTitle`), а не второй источник заголовка.
 */
export function getExitDialogText(displayTitle: string, progress: DownloadProgress): ExitDialogText {
  const fragment = stateFragment(progress)
  return {
    heading: HEADING,
    body:
      `${displayTitle} ${fragment}. Если выйти сейчас, загрузка остановится. ` +
      'Уже скачанное останется на диске в папке «Загрузки» — чтобы продолжить, ' +
      'после следующего запуска вставьте ту же ссылку ещё раз.',
  }
}
