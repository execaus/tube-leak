import type { DownloadCommandErrorKind } from '@/types/generated/download'
import type { QueueTaskRef } from '@/types/generated/queue'
import { formatTaskDisplayTitle } from './queueTaskTitle'

/**
 * Тексты для семи классов отказа команд управления загрузкой и очередью
 * (`start_download`/`cancel_download`/`retry_download`/`resume_queue`/
 * `dismiss_queue_task`, {@link DownloadCommandErrorKind}) — не путать с
 * девятью классами отказа самой задачи
 * ({@link import('@/types/generated/download').DownloadErrorKind}), для
 * которых есть `downloadErrorTexts.ts`.
 *
 * Дизайн E3 это состояние не описывает — оно считалось недостижимым
 * (кнопка «Скачать» на исправном фронтенде не должна быть достижима для
 * шести классов). Ревью TL-45 нашло достижимый путь (несовпадение
 * обрезки пробелов между разбором и стартом) и потребовало не глушить
 * отказ в консоль молча — тот же класс дефекта, что «не отвечает» в E1.
 *
 * С эпика E4 (TL-70) список пополнился и изменился дважды:
 * - класс `alreadyActive` («слот занят другой задачей») **убран** —
 *   прямое следствие Ф-2 E4: постановка при занятом слоте больше не
 *   мгновенный отказ, а нормальный путь (задача встаёт в хвост очереди);
 * - класс `duplicateTask` **добавлен** (Ф-8/Р-5 E4) — точный дубль
 *   (тот же ролик, тот же пункт качества) среди нетерминальных задач;
 * - класс `taskNotFinished` **добавлен** — «Скрыть» доступно только
 *   терминальной задаче (`dismiss_queue_task`, дизайн «Данные для API»,
 *   п.5).
 *
 * Показ решён так же, как и остальные ошибки: заголовок и пояснение по
 * классу, без кнопки «Повторить» — ни для одного из семи классов повтор
 * того же вызова не имеет смысла (см. doc {@link DownloadCommandErrorKind}
 * в `src/types/generated/download.ts`): либо гонка уже разрешилась сама
 * (`unknownTask`), либо нужен другой ввод или другое действие, а не тот же
 * вызов ещё раз.
 *
 * Как и {@link import('./downloadErrorTexts').getDownloadErrorText} —
 * сигнатура не принимает диагностическое `message`. Исключение —
 * `duplicateTask`: он обязан назвать существующую задачу (Р-5 E4), но
 * не через свободную строку, а через структурные поля
 * {@link QueueTaskRef} (`existing`), уже безопасно показанные на экране
 * собственной строкой/панелью этой же задачи (дизайн E4, «Отказ по
 * дублю», «Явное расхождение с уже задокументированным правилом»).
 */
export interface DownloadCommandErrorText {
  title: string
  explanation: string
}

const UNKNOWN_TASK_TEXT: DownloadCommandErrorText = {
  title: 'Задача не найдена',
  explanation:
    'Похоже, приложение перезапускалось, и ядро больше не знает эту задачу. Вставьте ссылку и запустите загрузку заново.',
}

const NOT_FAILED_TEXT: DownloadCommandErrorText = {
  title: 'Повтор недоступен',
  explanation: 'Задачу можно повторить только после отказа — сейчас она не в этом состоянии.',
}

const NOT_RETRYABLE_TEXT: DownloadCommandErrorText = {
  title: 'Повтор недоступен',
  explanation: 'Для этого отказа повтор не поможет — попробуйте новый разбор ролика.',
}

const NO_STREAMS_SELECTED_TEXT: DownloadCommandErrorText = {
  title: 'Не выбрано качество',
  explanation: 'В запросе не оказалось ни одного потока для скачивания — выберите качество и попробуйте ещё раз.',
}

const INVALID_URL_TEXT: DownloadCommandErrorText = {
  title: 'Ссылка не распознана',
  explanation:
    'Проверьте, что в поле — обычная ссылка на ролик YouTube, без лишних символов, и попробуйте ещё раз.',
}

/**
 * `taskNotFinished` — «Скрыть» доступно только терминальной задаче
 * (Done/Failed/Cancelled): текст по образцу остальных пяти классов, без
 * `message`, но этот случай не должен быть достижим с исправного
 * фронтенда (кнопка «Скрыть» рисуется только для терминальной ветки
 * `DownloadPanel`) — оборона на случай гонки, тот же класс дефекта, что и
 * у остальных шести.
 */
const TASK_NOT_FINISHED_TEXT: DownloadCommandErrorText = {
  title: 'Скрыть нельзя',
  explanation:
    'Скрыть можно только завершённую задачу — дождитесь её исхода (Готово, Отменена или Ошибка) и попробуйте снова.',
}

/**
 * `duplicateTask` — единственный класс, чей текст зависит от данных
 * (Р-5: «отказ называет существующую задачу»). `existing` несёт ровно те
 * же два поля, что уже безопасно показаны в списке очереди собственной
 * строкой/панелью этой задачи ({@link QueueTaskRef}), поэтому это не
 * утечка диагностики, а цитирование уже видимого экрана.
 */
function getDuplicateTaskText(existing: QueueTaskRef): DownloadCommandErrorText {
  const displayTitle = formatTaskDisplayTitle(existing.title, existing.quality)
  return {
    title: 'Такая задача уже в очереди',
    explanation:
      `${displayTitle} уже стоит в очереди с этим же качеством. Дождитесь её исхода или отмените ` +
      'её в списке ниже, если хотите начать заново.',
  }
}

/** Текст по классу отказа команды. Сигнатура не принимает `message` — см. doc выше. */
export function getDownloadCommandErrorText(kind: DownloadCommandErrorKind): DownloadCommandErrorText {
  switch (kind.kind) {
    case 'unknownTask':
      return UNKNOWN_TASK_TEXT
    case 'notFailed':
      return NOT_FAILED_TEXT
    case 'notRetryable':
      return NOT_RETRYABLE_TEXT
    case 'noStreamsSelected':
      return NO_STREAMS_SELECTED_TEXT
    case 'invalidUrl':
      return INVALID_URL_TEXT
    case 'duplicateTask':
      return getDuplicateTaskText(kind.existing)
    case 'taskNotFinished':
      return TASK_NOT_FINISHED_TEXT
  }
}

/** Фолбэк для неконтрактного отказа (паника команды, отказ самого IPC-вызова) — тот же приём, что и в остальных экранах. */
export const NON_CONTRACTUAL_COMMAND_ERROR_TEXT: DownloadCommandErrorText = {
  title: 'Не удалось выполнить команду',
  explanation: 'Не удалось разобрать причину отказа. Попробуйте ещё раз.',
}

/**
 * Форма отказа, достаточная, чтобы выбрать текст (правки ревью TL-98,
 * Н-5) — не полный {@link import('@/stores/downloadTask').DownloadCommandFailure}:
 * импорт оттуда сюда завёл бы цикл (`downloadTask.ts` уже импортирует из
 * этого файла). Обе стороны (семь контрактных классов и неконтрактный
 * отказ) совпадают ровно по полю `kind`, которое здесь и используется —
 * `message`/`existing` конкретного варианта резолверу не нужны.
 */
export type CommandFailureTextInput = DownloadCommandErrorKind | { kind?: undefined }

/**
 * Одна точка выбора текста отказа команды постановки — до этой правки
 * один и тот же тернарник (`kind === undefined ? NON_CONTRACTUAL_… :
 * getDownloadCommandErrorText(…)`) был скопирован дословно в
 * `DownloadCommandErrorBlock.vue` (для баннера на экране) и в
 * `downloadTask.ts` (для текста живой зоны исходов, TL-98) — оба места
 * теперь зовут эту функцию, а не держат свою копию условия.
 */
export function resolveDownloadCommandErrorText(failure: CommandFailureTextInput): DownloadCommandErrorText {
  return failure.kind === undefined ? NON_CONTRACTUAL_COMMAND_ERROR_TEXT : getDownloadCommandErrorText(failure)
}
