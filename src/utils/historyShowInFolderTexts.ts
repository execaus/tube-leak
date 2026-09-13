import type { LauncherFailureDetails, ShowInFolderErrorKind } from '@/types/generated/history'
import { assertNever } from './assertNever'
import { getHistoryUnavailableText } from './historyUnavailableTexts'

/**
 * Тексты отказа `show_in_folder` (Ф-8, Ф-17) для строки записи, где клик
 * пришёлся не на ожидаемый по дизайну случай.
 *
 * Ровно один из пяти классов контракта — `fileMissing` **вместе** с
 * `fileStatus: missing, folderExists: true` — не заводит здесь текст
 * вовсе: дизайн E5 («таблица трёх случаев», строка 2) называет это
 * ожидаемым исходом «Показать в папке» для такой записи («папка
 * открывается... а пометка не меняется») — сама третья строка записи уже
 * объясняет, что файла нет, и вызывающая сторона (`useHistoryStore.showInFolder`)
 * не кладёт этот случай в `showInFolderErrors` вовсе, эта функция сюда не
 * вызывается. Здесь — тексты для всех пяти классов на случай остальных
 * четырёх путей и на случай гонки (`present`-запись, у которой файл исчез
 * между загрузкой страницы и кликом, тоже получает `fileMissing`, но это
 * уже не ожидаемый дизайном случай, а гонка, требующая текста).
 */
export interface ShowInFolderErrorText {
  title: string
  explanation: string
}

function getLauncherFailedExplanation(): string {
  return 'Не удалось запустить файловый менеджер операционной системы.'
}

export function getShowInFolderErrorText(kind: ShowInFolderErrorKind): ShowInFolderErrorText {
  switch (kind.kind) {
    case 'fileMissing':
      return {
        title: 'Файл не найден',
        explanation: 'Файл сейчас не на месте — возможно, его удалили, переместили или переименовали.',
      }
    case 'folderMissing':
      return {
        title: 'Папка не найдена',
        explanation: 'Папка, где должен быть файл, сейчас не существует.',
      }
    case 'launcherFailed':
      return {
        title: 'Не удалось открыть проводник',
        explanation: getLauncherFailedExplanation(),
      }
    case 'unknownRecord':
      return {
        title: 'Запись не найдена',
        explanation: 'Похоже, список устарел.',
      }
    case 'unavailable':
      return {
        title: 'История недоступна',
        explanation: getHistoryUnavailableText(kind.reason),
      }
    default:
      return assertNever(kind)
  }
}

/** Подробности для `<details>` под текстом `launcherFailed` (Ф-17: «системные подробности — в свёрнутых деталях»), тот же паттерн, что у `Failed` в `DownloadPanel.vue`. */
export function getLauncherFailureDetails(kind: ShowInFolderErrorKind): LauncherFailureDetails | undefined {
  return kind.kind === 'launcherFailed' ? kind.details : undefined
}

/** Фолбэк для неконтрактного отказа команды. */
export const NON_CONTRACTUAL_SHOW_IN_FOLDER_ERROR_TEXT: ShowInFolderErrorText = {
  title: 'Не удалось выполнить команду',
  explanation: 'Не удалось разобрать причину отказа. Попробуйте ещё раз.',
}

/**
 * Нейтральное сообщение строки при `unknownRecord` у «Показать в папке»
 * (С-3, правки ревью TL-93, второй раунд): не совет переключить вкладку
 * или обновить историю вручную (`useHistoryStore.showInFolder` уже сам
 * перезапрашивает первую страницу в этот момент, doc-комментарий там же) —
 * просто факт, без действия, которое пользователю и так не нужно
 * выполнять руками. Одна строка, без заголовка (нет `explanation` —
 * {@link import('./formatHistoryMessage').formatHistoryMessage} отдаёт её
 * как есть, без двоеточия).
 */
export const HISTORY_ROW_GONE_TEXT = 'Этой записи больше нет в истории.'
