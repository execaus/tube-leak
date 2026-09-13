import type { SettingsCommandError, SettingsCommandErrorKind } from '@/types/generated/settings'
import { knownKindsOf } from './knownKinds'

const KNOWN_SETTINGS_COMMAND_ERROR_KINDS = knownKindsOf({
  notADirectory: true,
  invalidTemplate: true,
  invalidValue: true,
  writeFailed: true,
} satisfies Record<SettingsCommandErrorKind['kind'], true>)

/**
 * Проверяет и `kind`, и `message` — тот же приём, что
 * `isHistoryCommandErrorKind`/`isShowInFolderErrorKind` в `stores/history.ts`
 * (TL-93): контрактный {@link SettingsCommandError} несёт оба поля всегда.
 */
function isSettingsCommandError(value: unknown): value is SettingsCommandError {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Record<string, unknown>
  return (
    typeof candidate.kind === 'string' &&
    (KNOWN_SETTINGS_COMMAND_ERROR_KINDS as readonly string[]).includes(candidate.kind) &&
    typeof candidate.message === 'string'
  )
}

/** Отказ `settings_set`/`preview_name_template`: контрактный класс либо неконтрактный сбой самого IPC-вызова — тот же приём, что `HistoryCommandFailure`. */
export type SettingsCommandFailure = SettingsCommandError | { kind?: undefined; message: string }

export function toSettingsCommandFailure(err: unknown): SettingsCommandFailure {
  if (isSettingsCommandError(err)) return err
  if (err instanceof Error) return { message: err.message }
  if (typeof err === 'string' && err.length > 0) return { message: err }
  return { message: 'Команда настроек отклонена по нераспознанной причине.' }
}
