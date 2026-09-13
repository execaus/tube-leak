const KB = 1024
const MB = KB * 1024
const GB = MB * 1024

/**
 * Точный размер файла записи истории (Ф-2, С-5, правки ревью TL-93,
 * второй раунд) — без «≈»: `HistoryEntry.sizeBytes` снят с диска в момент
 * записи (doc-комментарий `HistoryEntry.sizeBytes` в
 * `src/types/generated/history.ts`), это не оценка лестницы качеств
 * (та несёт свою собственную неопределённость и форматируется отдельным
 * {@link import('./formatApproxSize').formatApproxSize}, который эта
 * функция не заменяет и не трогает — два разных факта с разной точностью,
 * не должны делить один форматтер).
 */
export function formatExactSize(bytes: number): string {
  if (bytes >= GB) return `${roundTo(bytes / GB, 1)} ГБ`
  if (bytes >= MB) return `${Math.round(bytes / MB)} МБ`
  return `${Math.round(bytes / KB)} КБ`
}

function roundTo(value: number, decimals: number): string {
  return value.toFixed(decimals)
}
