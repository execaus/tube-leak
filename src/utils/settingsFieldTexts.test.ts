import { describe, expect, it } from 'vitest'

import type { FolderProblem, TemplateProblem } from '@/types/generated/settings'
import {
  getAttemptsInvalidValueText,
  getAttemptsSaveErrorText,
  getDestinationFolderPathText,
  getFolderSaveErrorText,
  getPreviewFailureDisplay,
  getTemplateProblemText,
  getTemplateSaveErrorText,
  PREVIEW_UNAVAILABLE_TEXT,
  SETTINGS_SAVE_FAILED_TEXT,
} from './settingsFieldTexts'

describe('getDestinationFolderPathText', () => {
  it('shows the system Downloads folder in quotes, and a custom path as-is (no quotes, no shortening)', () => {
    expect(getDestinationFolderPathText({ kind: 'system' })).toBe('«Загрузки»')
    expect(getDestinationFolderPathText({ kind: 'custom', path: '/Users/execaus/Movies/YouTube' })).toBe(
      '/Users/execaus/Movies/YouTube',
    )
  })
})

describe('getFolderSaveErrorText', () => {
  const ALL_FOLDER_PROBLEMS: FolderProblem[] = ['notAbsolute', 'notFound', 'notADirectory', 'noAccess']

  it('gives every FolderProblem its own, distinct text', () => {
    const texts = ALL_FOLDER_PROBLEMS.map((problem) =>
      getFolderSaveErrorText({ kind: 'notADirectory', problem, message: 'diag' }),
    )
    expect(new Set(texts).size).toBe(ALL_FOLDER_PROBLEMS.length)
    for (const text of texts) {
      expect(text).toMatch(/^Эта папка недоступна: .+ — выберите другую\.$/)
    }
  })

  it('falls back to the generic save-failed text for writeFailed and non-contractual failures', () => {
    expect(getFolderSaveErrorText({ kind: 'writeFailed', message: 'diag' })).toBe(SETTINGS_SAVE_FAILED_TEXT)
    expect(getFolderSaveErrorText({ message: 'boom' })).toBe(SETTINGS_SAVE_FAILED_TEXT)
  })
})

describe('getTemplateProblemText', () => {
  it('gives every TemplateProblem kind its own, distinct text', () => {
    const problems: TemplateProblem[] = [
      { kind: 'unknownVariable', position: 8, name: 'channel' },
      { kind: 'unclosedBrace', position: 12 },
      { kind: 'strayClosingBrace', position: 3 },
      { kind: 'noVariables' },
    ]
    const texts = problems.map(getTemplateProblemText)
    expect(new Set(texts).size).toBe(problems.length)
  })

  it('names the unknown variable and its position, and lists the allowed variables', () => {
    const text = getTemplateProblemText({ kind: 'unknownVariable', position: 8, name: 'channel' })
    expect(text).toContain('символ 8')
    expect(text).toContain('{channel}')
    expect(text).toContain('{title}')
    expect(text).toContain('{id}')
    expect(text).toContain('{quality}')
    expect(text).toContain('{date}')
  })

  it('names the noVariables reason without a position (there is nowhere to point)', () => {
    const text = getTemplateProblemText({ kind: 'noVariables' })
    expect(text).not.toMatch(/символ/)
    expect(text).toContain('{title}')
  })
})

describe('getTemplateSaveErrorText', () => {
  it('uses the exact TemplateProblem text for invalidTemplate', () => {
    const failure = { kind: 'invalidTemplate' as const, problem: { kind: 'noVariables' as const }, message: 'diag' }
    expect(getTemplateSaveErrorText(failure)).toBe(getTemplateProblemText(failure.problem))
  })

  it('falls back to the generic save-failed text for writeFailed and non-contractual failures — NOT "Пример недоступен" (that text is reserved for the preview, not for saving)', () => {
    expect(getTemplateSaveErrorText({ kind: 'writeFailed', message: 'diag' })).toBe(SETTINGS_SAVE_FAILED_TEXT)
    expect(getTemplateSaveErrorText({ message: 'boom' })).toBe(SETTINGS_SAVE_FAILED_TEXT)
  })
})

describe('getAttemptsInvalidValueText / getAttemptsSaveErrorText', () => {
  it('names the min/max bounds from the server response', () => {
    expect(getAttemptsInvalidValueText(1, 20)).toBe('Число попыток должно быть от 1 до 20.')
  })

  it('uses the bounds text for invalidValue', () => {
    expect(getAttemptsSaveErrorText({ kind: 'invalidValue', min: 1, max: 20, message: 'diag' })).toBe(
      'Число попыток должно быть от 1 до 20.',
    )
  })

  it('falls back to the generic save-failed text for writeFailed and non-contractual failures', () => {
    expect(getAttemptsSaveErrorText({ kind: 'writeFailed', message: 'diag' })).toBe(SETTINGS_SAVE_FAILED_TEXT)
    expect(getAttemptsSaveErrorText({ message: 'boom' })).toBe(SETTINGS_SAVE_FAILED_TEXT)
  })
})

describe('getPreviewFailureDisplay', () => {
  it('surfaces invalidTemplate as the exact problem', () => {
    const problem: TemplateProblem = { kind: 'noVariables' }
    expect(getPreviewFailureDisplay({ kind: 'invalidTemplate', problem, message: 'diag' })).toStrictEqual({
      kind: 'problem',
      problem,
    })
  })

  it('treats writeFailed (the TL-91 stub, among other things) and a non-contractual failure as "unavailable", not a save failure', () => {
    expect(getPreviewFailureDisplay({ kind: 'writeFailed', message: 'diag' })).toStrictEqual({ kind: 'unavailable' })
    expect(getPreviewFailureDisplay({ message: 'boom' })).toStrictEqual({ kind: 'unavailable' })
  })
})

it('PREVIEW_UNAVAILABLE_TEXT is not the same string as the generic save-failed text (mutation guard for the writeFailed/preview mix-up)', () => {
  expect(PREVIEW_UNAVAILABLE_TEXT).not.toBe(SETTINGS_SAVE_FAILED_TEXT)
})

describe('getTemplateProblemText — tooLong (TL-91)', () => {
  it('names the limit from the problem, not a hardcoded number', () => {
    expect(getTemplateProblemText({ kind: 'tooLong', max: 200 })).toBe('Шаблон слишком длинный: не больше 200 символов.')
    expect(getTemplateProblemText({ kind: 'tooLong', max: 7 })).toContain('не больше 7 символов')
  })
})
