import type { DownloadCommandErrorKind } from '@/types/download'

/**
 * Тексты для шести классов отказа команд управления загрузкой
 * (`start_download`/`cancel_download`/`retry_download`,
 * {@link DownloadCommandErrorKind}) — не путать с девятью классами отказа
 * самой задачи ({@link import('@/types/download').DownloadErrorKind}),
 * для которых есть `downloadErrorTexts.ts`.
 *
 * Дизайн E3 это состояние не описывает — оно считалось недостижимым
 * (кнопка «Скачать» на исправном фронтенде не должна быть достижима для
 * `alreadyActive` и т.п.). Ревью TL-45 нашло достижимый путь (несовпадение
 * обрезки пробелов между разбором и стартом) и потребовало не глушить
 * отказ в консоль молча — тот же класс дефекта, что «не отвечает» в E1.
 *
 * Показ решён так же, как и остальные ошибки: заголовок и пояснение по
 * классу, без кнопки «Повторить» — для всех шести классов повтор того же
 * вызова не имеет смысла (см. doc {@link DownloadCommandErrorKind}
 * в `src/types/download.ts`): либо гонка уже разрешилась сама
 * (`alreadyActive`/`unknownTask`), либо нужен другой ввод, а не тот же
 * вызов ещё раз (`noStreamsSelected`/`invalidUrl`/`notFailed`/`notRetryable`).
 *
 * Как и {@link import('./downloadErrorTexts').getDownloadErrorText} —
 * сигнатура не принимает диагностическое `message`, подмена не
 * скомпилируется.
 */
export interface DownloadCommandErrorText {
  title: string
  explanation: string
}

const ALREADY_ACTIVE_TEXT: DownloadCommandErrorText = {
  title: 'Уже идёт другая загрузка',
  explanation: 'Слот занят предыдущей задачей — дождитесь её завершения или отмените её, затем попробуйте снова.',
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

/** Текст по классу отказа команды. Сигнатура не принимает `message` — см. doc выше. */
export function getDownloadCommandErrorText(kind: DownloadCommandErrorKind): DownloadCommandErrorText {
  switch (kind) {
    case 'alreadyActive':
      return ALREADY_ACTIVE_TEXT
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
  }
}

/** Фолбэк для неконтрактного отказа (паника команды, отказ самого IPC-вызова) — тот же приём, что и в остальных экранах. */
export const NON_CONTRACTUAL_COMMAND_ERROR_TEXT: DownloadCommandErrorText = {
  title: 'Не удалось выполнить команду',
  explanation: 'Не удалось разобрать причину отказа. Попробуйте ещё раз.',
}
