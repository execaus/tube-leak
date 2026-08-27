import type { QualitySize } from '@/types/generated/probe'

const KB = 1024
const MB = KB * 1024
const GB = MB * 1024

/**
 * Оценка размера пункта лестницы (Ф-4, дизайн E2): «≈ N ГБ/МБ/КБ» либо
 * «размер неизвестен» — не прочерк (его можно принять за «ещё грузится»)
 * и не «0».
 */
export function formatApproxSize(size: QualitySize): string {
  if (size.kind === 'unknown') return 'размер неизвестен'

  const bytes = size.bytes
  if (bytes >= GB) return `≈ ${roundTo(bytes / GB, 1)} ГБ`
  if (bytes >= MB) return `≈ ${Math.round(bytes / MB)} МБ`
  return `≈ ${Math.round(bytes / KB)} КБ`
}

function roundTo(value: number, decimals: number): string {
  return value.toFixed(decimals)
}
