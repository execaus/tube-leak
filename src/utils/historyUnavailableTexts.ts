import type { HistoryUnavailableReason } from '@/types/generated/history'
import { assertNever } from './assertNever'

/**
 * Единственный абзац, которым экран истории целиком заменяется, когда базы
 * нет в этом сеансе (Ф-1 б/в/д, дизайн E5, «База новее приложения / нет
 * доступа / отказ миграции — экран блокируется целиком, кроме заголовка»).
 * Тексты — дословно по дизайну; `corrupted` сюда не входит по контракту
 * ({@link HistoryUnavailableReason} не несёт этого варианта — порча не
 * лишает сеанс истории, см. doc `HistoryUnavailableReason` в
 * `src/types/generated/history.ts`).
 */
export function getHistoryUnavailableText(reason: HistoryUnavailableReason): string {
  switch (reason) {
    case 'newerVersion':
      return (
        'История недоступна в этом сеансе: файл базы данных создан более новой ' +
        'версией tube-leak — эта версия прочитать его не может. Файл не тронут: ' +
        'откроется снова, когда вы обновите приложение до этой или более новой ' +
        'версии.'
      )
    case 'noAccess':
      return (
        'История недоступна в этом сеансе: нет доступа на запись в папку данных ' +
        'приложения. Проверьте права доступа к ней и запустите tube-leak снова.'
      )
    case 'migrationFailed':
      return (
        'История недоступна в этом сеансе: не удалось обновить формат базы данных ' +
        'до текущей версии приложения. Файл остался на прежней версии и не ' +
        'повреждён — сообщите об этом, если увидите снова.'
      )
    default:
      return assertNever(reason)
  }
}
