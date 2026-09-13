/**
 * Конвертация Unix-секунд UTC (формат контракта, например
 * `HistoryEntry.finishedAtUnixSecs`) в RFC 3339, который принимает
 * {@link import('./formatRelativeTime').formatRelativeTime} (дизайн E5,
 * пункт 2: «конвертация в аргумент существующего форматтера — на стороне
 * вызова, не в самом форматтере», чтобы не заводить второй форматтер
 * относительного времени с другой сигнатурой аргумента).
 */
export function unixSecsToIso(unixSecs: number): string {
  return new Date(unixSecs * 1000).toISOString()
}
