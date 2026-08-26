const KB = 1024
const MB = KB * 1024
const GB = MB * 1024

/**
 * Форматирует мгновенную скорость скачивания (Ф-2 — байты в секунду
 * приходят с ядра, форматирование — забота UI, тот же принцип, что
 * `formatApproxSize` в E2).
 */
export function formatSpeed(bytesPerSec: number): string {
  if (bytesPerSec >= GB) return `${roundTo(bytesPerSec / GB, 1)} ГБ/с`
  if (bytesPerSec >= MB) return `${roundTo(bytesPerSec / MB, 1)} МБ/с`
  if (bytesPerSec >= KB) return `${Math.round(bytesPerSec / KB)} КБ/с`
  return `${Math.round(bytesPerSec)} Б/с`
}

function roundTo(value: number, decimals: number): string {
  return value.toFixed(decimals)
}
