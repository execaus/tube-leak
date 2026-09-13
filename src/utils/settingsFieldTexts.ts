import type { DestinationFolder, FolderProblem, TemplateProblem } from '@/types/generated/settings'
import type { SettingsCommandFailure } from './settingsCommandFailure'

/** Путь папки назначения — тот же принцип, что {@link import('./downloadOutcomeTexts').getFolderDisplayText}, но с тегом `DestinationFolder` (`system`/`custom`), а не `FolderDisplay` (`systemDownloads`/`custom`) контракта `download.ts`: два разных перечисления с похожим смыслом (doc-комментарий `DestinationFolder` в `src/types/generated/settings.ts`), не опечатка. Путь показан целиком, без сокращений (дизайн E5, «Папка назначения»: «путь — не текст для чтения человеком вслух, а факт»). */
export function getDestinationFolderPathText(folder: DestinationFolder): string {
  return folder.kind === 'system' ? '«Загрузки»' : folder.path
}

/** «<причина>» в тексте дизайна «Эта папка недоступна: <причина> — выберите другую.» (Ф-11, `notADirectory`). */
function getFolderProblemClause(problem: FolderProblem): string {
  switch (problem) {
    case 'notAbsolute':
      return 'указан не абсолютный путь'
    case 'notFound':
      return 'такой папки не существует'
    case 'notADirectory':
      return 'это не папка'
    case 'noAccess':
      return 'нет доступа к папке'
  }
}

/** Текст под подсказкой диапазона попыток при отказе `invalidValue` (Ф-13). */
export function getAttemptsInvalidValueText(min: number, max: number): string {
  return `Число попыток должно быть от ${min} до ${max}.`
}

/** Текст ошибки шаблона по классу {@link TemplateProblem} (Ф-12, дизайн E5, «Шаблон имени», примеры формулировок). */
export function getTemplateProblemText(problem: TemplateProblem): string {
  switch (problem.kind) {
    case 'unknownVariable':
      return (
        `Ошибка в шаблоне (символ ${problem.position}): неизвестная переменная «{${problem.name}}» — ` +
        'допустимы только {title}, {id}, {quality}, {date}.'
      )
    case 'unclosedBrace':
      return `Ошибка в шаблоне (символ ${problem.position}): незакрытая скобка «{».`
    case 'strayClosingBrace':
      return `Ошибка в шаблоне (символ ${problem.position}): лишняя закрывающая скобка «}», не открытая до неё.`
    case 'noVariables':
      return 'В шаблоне нет ни одной переменной — все файлы получили бы одно имя. Добавьте, например, {title} или {id}.'
  }
}

/** Общий текст отказа сохранения, когда причина — не то, что умеет объяснить конкретное поле (`writeFailed`, неконтрактный сбой, либо класс отказа, который для этого поля не должен приходить). */
export const SETTINGS_SAVE_FAILED_TEXT = 'Не удалось сохранить настройки. Попробуйте ещё раз.'

/**
 * Текст ошибки сохранения папки назначения (`settings_set` с
 * `destinationFolder`) — единственный контрактный класс для этого поля —
 * `notADirectory`; остальное (в т.ч. `writeFailed` — заглушка TL-91 до
 * TL-91) сводится к общему тексту.
 */
export function getFolderSaveErrorText(failure: SettingsCommandFailure): string {
  if (failure.kind === undefined) return SETTINGS_SAVE_FAILED_TEXT
  switch (failure.kind) {
    case 'notADirectory':
      return `Эта папка недоступна: ${getFolderProblemClause(failure.problem)} — выберите другую.`
    case 'writeFailed':
    case 'invalidTemplate':
    case 'invalidValue':
      return SETTINGS_SAVE_FAILED_TEXT
  }
}

/** Текст ошибки сохранения шаблона (`settings_set` с `nameTemplate`) — контрактный класс `invalidTemplate`. */
export function getTemplateSaveErrorText(failure: SettingsCommandFailure): string {
  if (failure.kind === undefined) return SETTINGS_SAVE_FAILED_TEXT
  switch (failure.kind) {
    case 'invalidTemplate':
      return getTemplateProblemText(failure.problem)
    case 'writeFailed':
    case 'notADirectory':
    case 'invalidValue':
      return SETTINGS_SAVE_FAILED_TEXT
  }
}

/** Текст ошибки сохранения числа попыток (`settings_set` с `maxAttempts`) — контрактный класс `invalidValue`. */
export function getAttemptsSaveErrorText(failure: SettingsCommandFailure): string {
  if (failure.kind === undefined) return SETTINGS_SAVE_FAILED_TEXT
  switch (failure.kind) {
    case 'invalidValue':
      return getAttemptsInvalidValueText(failure.min, failure.max)
    case 'writeFailed':
    case 'notADirectory':
    case 'invalidTemplate':
      return SETTINGS_SAVE_FAILED_TEXT
  }
}

/**
 * Нейтральный текст живого примера, когда `preview_name_template` не смог
 * его посчитать (класс `writeFailed`, включая заглушку TL-91 до TL-91, или
 * неконтрактный сбой IPC) — **не** текст ошибки сохранения (поправка
 * ведущего по ревью TL-83, комментарий в issue #101, 2026-09-13):
 * предпросмотр ничего не пишет, и «Не удалось сохранить» здесь было бы
 * ложью о том, что вообще произошла попытка записи.
 */
export const PREVIEW_UNAVAILABLE_TEXT = 'Пример недоступен.'

/**
 * Разбор ответа `preview_name_template` на то, что можно показать в живом
 * примере: точная ошибка шаблона (тот же валидатор, что и у `settings_set`,
 * дизайн «Шаблон имени») либо нейтральная недоступность для всего
 * остального, включая заглушку `writeFailed` (см. {@link PREVIEW_UNAVAILABLE_TEXT}).
 */
export type PreviewFailureDisplay = { kind: 'problem'; problem: TemplateProblem } | { kind: 'unavailable' }

export function getPreviewFailureDisplay(failure: SettingsCommandFailure): PreviewFailureDisplay {
  if (failure.kind === 'invalidTemplate') return { kind: 'problem', problem: failure.problem }
  return { kind: 'unavailable' }
}
