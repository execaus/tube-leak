import type { DownloadProgress } from '@/types/generated/download'
import type { QueuePauseReason } from '@/types/generated/queue'
import { pluralizeRu } from './pluralizeRu'

/** Заголовок и текст диалога подтверждения выхода (дизайн E4, TL-76). */
export interface ExitDialogText {
  heading: string
  body: string
}

/**
 * С TL-76 (эпик E4) диалог говорит про очередь целиком, а не про одну
 * задачу (дизайн «Диалог выхода», С-6) — заголовок точнее отражает это:
 * речь не обязательно про одну скачивающуюся задачу.
 */
const HEADING = 'Очередь ещё не завершена'

/** Активная задача диалога — та же пара, что уже держит `DownloadTask` (`displayTitle`) плюс её прогресс. */
export interface ExitDialogActiveTask {
  displayTitle: string
  progress: DownloadProgress
}

/**
 * Вход текста диалога (дизайн E4, «Диалог выхода»):
 * - `activeTask` — задача, которая реально выполняется прямо сейчас
 *   (Fetching/Downloading/Merging, включая паузу перед повтором);
 *   отсутствует ровно тогда, когда `pauseReason` присутствует — в паузе
 *   между задачами (Р-7) активной задачи физически нет;
 * - `pauseReason` — планировщик держит паузу на обновление yt-dlp (Р-7);
 * - `waitingCount` — число нетерминальных задач сверх той, что названа
 *   первым предложением (при паузе — все нетерминальные задачи, ни одна
 *   из них не активна).
 */
export interface ExitDialogInput {
  activeTask?: ExitDialogActiveTask
  pauseReason?: QueuePauseReason
  waitingCount: number
}

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
    // Терминальные фазы сюда не доходят — диалог не показывается, когда
    // нетерминальных задач нет (`useExitConfirmation`, TL-76). Ветка —
    // оборона, а не ожидаемый путь.
    case 'done':
    case 'failed':
    case 'cancelled':
      return 'загрузка ещё готовится'
  }
}

/** Именительный падеж «N задача/задачи/задач» — счётчик ожидающих (дизайн, «строка ожидания»). */
function tasksNominative(n: number): string {
  return pluralizeRu(n, 'задача', 'задачи', 'задач')
}

/**
 * Первое предложение — про то, что выполняется прямо сейчас. Во время
 * паузы между задачами (Р-7) конкретной выполняющейся задачи физически
 * нет (предыдущая уже терминальна, следующая ещё не стартовала) — вместо
 * названия задачи и выдуманного процента честная фраза о самой паузе
 * (тот же приём, что и в строке паузы `QueueSection.vue`, но собственный
 * текст диалога, не импорт чужой константы — они описывают разные вещи:
 * там это состояние всей секции, здесь одно предложение диалога выхода).
 */
function activeSentence(activeTask: ExitDialogActiveTask | undefined, pauseReason: QueuePauseReason | undefined): string {
  if (pauseReason === 'ytDlpUpdate') {
    return 'Между загрузками устанавливается обновлённый yt-dlp.'
  }
  if (!activeTask) {
    // Недостижимо по контракту вызова (composable не показывает диалог
    // для пустой очереди), но не выдумывать данные, которых нет.
    return 'Очередь ещё не завершена.'
  }
  return `${activeTask.displayTitle} ${stateFragment(activeTask.progress)}.`
}

/**
 * Второе предложение — счётчик ожидающих сверх названной в первом
 * предложении (дизайн: «добавляется, только если таких задач больше
 * нуля»).
 */
function waitingSentence(waitingCount: number): string {
  if (waitingCount <= 0) return ''
  return ` Ещё в очереди: ${waitingCount} ${tasksNominative(waitingCount)}.`
}

/**
 * Текст диалога (дизайн E4, «Диалог выхода», С-6). Фраза «вставьте ту же
 * ссылку ещё раз» из E3 сюда не вернулась: она стала ложью с появлением
 * снимка очереди (Ф-9) — нетерминальные задачи переживают перезапуск и
 * восстанавливаются приостановленными (Р-3), их не нужно ставить заново,
 * достаточно «Продолжить очередь».
 */
export function getExitDialogText(input: ExitDialogInput): ExitDialogText {
  return {
    heading: HEADING,
    body:
      `${activeSentence(input.activeTask, input.pauseReason)}${waitingSentence(input.waitingCount)} ` +
      'Если выйти сейчас, всё остановится. Уже скачанное останется на диске, а очередь — тоже: ' +
      'при следующем запуске она будет ждать вас, нажмите «Продолжить очередь», чтобы возобновить.',
  }
}
