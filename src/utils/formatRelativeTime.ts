/**
 * Форматирует момент из контракта (`YtDlpUpdateSnapshot`'s `at`, RFC 3339
 * UTC — см. doc `YtDlpUpdateStatus` в `src/types/generated/update.ts`) в
 * относительную русскую строку («3 часа назад»). Это забота UI, не
 * фактическая величина (doc-комментарий контракта: «"3 часа назад"
 * считает UI, потому что это его формат, а не факт») — ядро отдаёт только
 * абсолютный штамп времени.
 *
 * `now` — необязательный второй параметр ради тестируемости без мока
 * системных часов на каждый вызов; по умолчанию — текущее время.
 */

const MINUTE_MS = 60_000
const HOUR_MS = 60 * MINUTE_MS
const DAY_MS = 24 * HOUR_MS

/**
 * Русское склонение по числу (1 → one, 2–4 → few, 0/5–20 → many), с учётом
 * исключения 11–14 (эти всегда «many», а не «few» вопреки последней
 * цифре).
 */
function pluralizeRu(n: number, one: string, few: string, many: string): string {
  const mod10 = n % 10
  const mod100 = n % 100
  if (mod10 === 1 && mod100 !== 11) return one
  if (mod10 >= 2 && mod10 <= 4 && (mod100 < 10 || mod100 >= 20)) return few
  return many
}

export function formatRelativeTime(iso: string, now: Date = new Date()): string {
  const thenMs = new Date(iso).getTime()
  const diffMs = Math.max(0, now.getTime() - thenMs)

  if (diffMs < MINUTE_MS) {
    return 'только что'
  }
  if (diffMs < HOUR_MS) {
    const minutes = Math.floor(diffMs / MINUTE_MS)
    return `${minutes} ${pluralizeRu(minutes, 'минуту', 'минуты', 'минут')} назад`
  }
  if (diffMs < DAY_MS) {
    const hours = Math.floor(diffMs / HOUR_MS)
    return `${hours} ${pluralizeRu(hours, 'час', 'часа', 'часов')} назад`
  }
  const days = Math.floor(diffMs / DAY_MS)
  return `${days} ${pluralizeRu(days, 'день', 'дня', 'дней')} назад`
}
