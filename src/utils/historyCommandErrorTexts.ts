import type { HistoryCommandErrorKind } from '@/types/generated/history'
import { assertNever } from './assertNever'
import { getHistoryUnavailableText } from './historyUnavailableTexts'

/**
 * Тексты отказа `delete_history_record`/`clear_history` (Ф-6, Ф-17) — не
 * описаны дословно дизайном (мокап рисует только успешный путь: удаление
 * без подтверждения, очистка с подтверждением), поэтому написаны здесь по
 * образцу остальных экранов (заголовок + пояснение, `downloadCommandErrorTexts.ts`).
 * Решение и разрыв с дизайном названы в отчёте задачи TL-93.
 */
export interface HistoryCommandErrorText {
  title: string
  explanation: string
}

export function getHistoryCommandErrorText(kind: HistoryCommandErrorKind): HistoryCommandErrorText {
  switch (kind.kind) {
    case 'unknownRecord':
      return {
        title: 'Запись не найдена',
        explanation: 'Похоже, список устарел.',
      }
    case 'writeFailed':
      return {
        title: 'Не удалось сохранить изменение',
        explanation: 'Запись на диск отклонена. Попробуйте ещё раз через некоторое время.',
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

/** Фолбэк для неконтрактного отказа команды (тот же приём, что у остальных экранов). */
export const NON_CONTRACTUAL_HISTORY_COMMAND_ERROR_TEXT: HistoryCommandErrorText = {
  title: 'Не удалось выполнить команду',
  explanation: 'Не удалось разобрать причину отказа. Попробуйте ещё раз.',
}
