import type { HistoryNotice, HistoryWriteFailure } from '@/types/generated/history'
import { assertNever } from './assertNever'

/**
 * Причина, вшитая в текст пометки «последняя запись не сохранена» (Ф-3,
 * дизайн E5, пункт 2). Контракт сузил кандидатов дизайна до трёх: «база
 * сейчас недоступна» не входит в {@link HistoryWriteFailure} — при
 * недоступной базе `history_page` отвечает {@link
 * import('@/types/generated/history').HistoryUnavailableError} целиком, и
 * страница с этой пометкой не приходит вовсе (doc `HistoryWriteFailure` в
 * `src/types/generated/history.ts`). `storageFailed` — третий кандидат,
 * не названный дизайном по имени; текст для него написан здесь (расхождение
 * дизайна с контрактом, см. отчёт задачи TL-93).
 */
function getWriteFailureReasonText(cause: HistoryWriteFailure): string {
  switch (cause) {
    case 'diskFull':
      return 'недостаточно места на диске'
    case 'noAccess':
      return 'нет доступа на запись'
    case 'storageFailed':
      return 'хранилище истории вернуло отказ'
    default:
      return assertNever(cause)
  }
}

/** Текст однократного баннера (Ф-1г/С-10 «порча базы», Ф-3 «последняя запись не сохранена») — дословно по дизайну E5, пункт 2. */
export function getHistoryNoticeText(notice: HistoryNotice): string {
  switch (notice.kind) {
    case 'baseRecreated':
      return (
        'Файл истории был повреждён — он отложен в сторону с меткой времени, ' +
        'ничего не удалено. Начата новая, пустая история.'
      )
    case 'lastWriteFailed':
      return (
        `Последняя запись не сохранена: ${getWriteFailureReasonText(notice.cause)}. ` +
        'Загрузка завершена успешно, файл на месте — не сохранилась только строка ' +
        'в истории.'
      )
    default:
      return assertNever(notice)
  }
}
