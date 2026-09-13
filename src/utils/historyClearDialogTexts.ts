import { pluralizeRu } from './pluralizeRu'

/**
 * Тело диалога подтверждения очистки истории (дизайн E5, «Очистить —
 * подтверждение»). `knownCount` — число уже загруженных на экран записей;
 * `undefined`, если экран точно не знает, что это все записи целиком (есть
 * `nextCursor`, то есть пользователь не долистал «Показать ещё» до конца) —
 * дизайн предпочитает текст без числа настоящему, но не гарантированно
 * точному числу.
 */
export function getClearHistoryConfirmBody(knownCount: number | undefined): string {
  const tail =
    'Файлы на диске не тронет ничего — это очищает только список, не содержимое папок.'
  if (knownCount === undefined) {
    return `Будут удалены все записи истории. ${tail}`
  }
  const noun = pluralizeRu(knownCount, 'запись', 'записи', 'записей')
  return `Будут удалены все ${knownCount} ${noun} истории. ${tail}`
}
