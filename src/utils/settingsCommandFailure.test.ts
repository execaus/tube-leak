import { describe, expect, it } from 'vitest'

import { toSettingsCommandFailure } from './settingsCommandFailure'

describe('toSettingsCommandFailure', () => {
  it('passes through a contractual SettingsCommandError as-is', () => {
    const err = { kind: 'invalidValue', min: 1, max: 20, message: 'diag' }
    expect(toSettingsCommandFailure(err)).toStrictEqual(err)
  })

  it('rejects an unknown kind — not every object with a "kind" string is contractual — and falls back to the neutral message, same as history.ts', () => {
    const err = { kind: 'somethingElse', message: 'diag' }
    expect(toSettingsCommandFailure(err)).toStrictEqual({
      message: 'Команда настроек отклонена по нераспознанной причине.',
    })
  })

  it('turns a plain Error into a message-only failure', () => {
    expect(toSettingsCommandFailure(new Error('boom'))).toStrictEqual({ message: 'boom' })
  })

  it('turns a plain string into a message-only failure', () => {
    expect(toSettingsCommandFailure('boom')).toStrictEqual({ message: 'boom' })
  })

  it('falls back to a neutral message for anything else', () => {
    expect(toSettingsCommandFailure(undefined)).toStrictEqual({
      message: 'Команда настроек отклонена по нераспознанной причине.',
    })
  })
})
