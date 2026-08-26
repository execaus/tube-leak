/**
 * Форматирует оценку оставшегося времени скачивания («осталось ≈ 1 мин 40
 * с») — секунды приходят с ядра (Ф-2), компоновка строки — забота UI.
 */
export function formatEtaSecs(totalSeconds: number): string {
  const safeSeconds = Math.max(0, Math.round(totalSeconds))
  const hours = Math.floor(safeSeconds / 3600)
  const minutes = Math.floor((safeSeconds % 3600) / 60)
  const seconds = safeSeconds % 60

  const parts: string[] = []
  if (hours > 0) parts.push(`${hours} ч`)
  if (hours > 0 || minutes > 0) parts.push(`${minutes} мин`)
  parts.push(`${seconds} с`)

  return `≈ ${parts.join(' ')}`
}
