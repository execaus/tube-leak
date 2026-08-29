import type { YtDlpUpdateFailure, YtDlpUpdateStatus } from '@/types/generated/update'
import { assertNever } from '@/utils/assertNever'

/**
 * Тексты статус-строки блока «Обновление yt-dlp», строго по таблице «Все
 * состояния» дизайна E6 (десять вариантов `YtDlpUpdateStatus` покрывают
 * все 14 строк — `failed` разворачивается в пять по классу
 * {@link YtDlpUpdateFailure}, doc-комментарий контракта).
 *
 * # Откуда берётся `X` (активная версия)
 *
 * По дизайну (раздел «Данные для UI») активная версия **не** входит в
 * `YtDlpUpdateSnapshot` — она уже приходит из `check_sidecar`
 * (`SidecarCheckResult.version`), второй раз не запрашивается. Поэтому
 * здесь она параметр функции, а не поле статуса; `upToDate` и оба
 * «безверсийных» класса `failed` (`networkUnavailable`/`sourceUnavailable`)
 * не несут версии вовсе — без параметра эти строки нечем было бы
 * заполнить.
 *
 * `rollbackWaiting`, `updated` и `rolledBack`, наоборот, называют нужную
 * версию прямо в своём поле контракта (`version`/`active`/`abandoned`) —
 * туда параметр `activeVersion` не подставляется, чтобы не разойтись с
 * тем, что реально утверждает снимок (doc `YtDlpUpdateStatus.version` —
 * «куда смотрит `version`» — в сгенерированном файле).
 *
 * # Откуда берётся отформатированное «когда»
 *
 * `formatWhen` — внешняя функция форматирования (`formatRelativeTime` в
 * реальном использовании), а не вызов её отсюда напрямую: строковая
 * сборка тестируется без мока системных часов, форматирование времени —
 * отдельно и один раз (`formatRelativeTime.test.ts`).
 */

const NO_ACTIVE_VERSION_PLACEHOLDER = '—'

function versionOrPlaceholder(version: string | undefined): string {
  return version ?? NO_ACTIVE_VERSION_PLACEHOLDER
}

function getFailureText(failure: YtDlpUpdateFailure, activeVersionText: string, when: string): string {
  switch (failure.kind) {
    case 'networkUnavailable':
      return `Проверено ${when} (нет соединения с интернетом) — работаем на ${activeVersionText}.`
    case 'sourceUnavailable':
      return `Проверено ${when} (GitHub не отвечает) — работаем на ${activeVersionText}.`
    case 'archiveCorrupted':
      return (
        `Обновление ${failure.version} скачалось повреждённым и было отброшено — ` +
        `работаем на ${activeVersionText}. Попробуем ещё раз позже.`
      )
    case 'notEnoughSpace':
      return (
        `Не удалось подготовить обновление ${failure.version} — не хватает места на диске. ` +
        `Работаем на ${activeVersionText}.`
      )
    case 'smokeCheckFailed':
      return (
        `Обновление ${failure.version} не прошло проверку запуска и не было установлено — ` +
        `работаем на ${activeVersionText}.`
      )
    default:
      return assertNever(failure)
  }
}

export function getYtDlpUpdateStatusText(
  status: YtDlpUpdateStatus,
  activeVersion: string | undefined,
  formatWhen: (iso: string) => string,
): string {
  const activeVersionText = versionOrPlaceholder(activeVersion)

  switch (status.status) {
    case 'neverChecked':
      return 'Ещё не проверяли обновления.'
    case 'checking':
      return 'Проверяем обновления…'
    case 'upToDate':
      return `Проверено ${formatWhen(status.at)} — установлена последняя версия (${activeVersionText}).`
    case 'downloading':
      return `Скачиваем обновление ${status.version}… ${status.percent} %.`
    case 'preparing':
      return `Готовим обновление ${status.version}…`
    case 'readyWaiting':
      return `Обновление ${status.version} готово — применится, когда закончится текущая загрузка.`
    case 'rollbackWaiting':
      // `status.version` — версия, к которой возвращаются и которая
      // станет активной (doc контракта, «Куда смотрит version»), не
      // параметр `activeVersion` (тот всё ещё называет старую активную).
      return `Возврат к ${status.version} принят — применится, когда закончится текущая загрузка.`
    case 'updated':
      return `Обновлено до ${status.version}, ${formatWhen(status.at)}.`
    case 'rolledBack':
      return (
        `Возврат выполнен: активна версия ${status.active}. Автообновление до ${status.abandoned} ` +
        `не предложится, пока апстрим не выпустит более новую версию.`
      )
    case 'failed':
      return getFailureText(status.failure, activeVersionText, formatWhen(status.at))
    default:
      return assertNever(status)
  }
}

/**
 * `true` для всех пяти классов `failed` (строки 8–12 таблицы) — единственный
 * визуальный сигнал ошибки в блоке (дизайн E6: приглушённый цвет вместо
 * значка «✕», С-4/С-5/С-11).
 */
export function isYtDlpUpdateStatusFailure(status: YtDlpUpdateStatus): boolean {
  return status.status === 'failed'
}
