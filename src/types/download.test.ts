import { describe, expect, it } from 'vitest'

import type {
  DownloadCommandError,
  DownloadErrorKind,
  DownloadPhase,
  DownloadPlan,
  DownloadProgressEvent,
  DownloadStream,
  PartialData,
} from './generated/download'

/**
 * Нормативный тест TL-39 (критерий приёмки issue #41): полный набор
 * значений `DownloadErrorKind` (9) и `DownloadPhase` (7) должен совпадать
 * с контрактом `src-tauri/src/types.rs`.
 *
 * Строковые литеральные объединения не существуют в рантайме, поэтому
 * прямое сравнение массивов невозможно — вместо этого используется
 * `Record<Тип, true>`: если Rust-контракт (и вслед за ним это зеркало)
 * получит новое значение enum, а объект ниже не будет дополнен, TypeScript
 * откажется компилировать `satisfies Record<...>` (лишний или недостающий
 * ключ) — расхождение ловится на сборке (`npm run type-check`), а не в
 * рантайме, ровно как требует критерий приёмки. Явная проверка
 * `Object.keys(...).length` в тесте — вторая, независимая линия защиты:
 * страхует от случая, когда кто-то расширит тип, но не тронет объект
 * (тогда `Record` перестанет собираться) и наоборот, от случая, когда
 * лишний ключ был бы добавлен без соответствующего варианта типа (тогда
 * не собрался бы `satisfies`).
 */
const ALL_DOWNLOAD_PHASES = {
  queued: true,
  fetching: true,
  downloading: true,
  merging: true,
  done: true,
  failed: true,
  cancelled: true,
} satisfies Record<DownloadPhase, true>

const ALL_DOWNLOAD_ERROR_KINDS = {
  connectionLost: true,
  diskFull: true,
  staleFormat: true,
  mergeFailed: true,
  destinationUnavailable: true,
  videoUnavailable: true,
  signInRequired: true,
  regionBlocked: true,
  ytDlpFailure: true,
} satisfies Record<DownloadErrorKind, true>

// Те же две независимые линии защиты для остальных enum-ов контракта —
// не по букве критерия приёмки issue (там названы только фаза и класс
// ошибки), но тем же приёмом и с той же ценой, что и они.
const ALL_DOWNLOAD_PLANS = {
  videoAndAudio: true,
  singleStream: true,
} satisfies Record<DownloadPlan, true>

const ALL_DOWNLOAD_STREAMS = {
  video: true,
  audio: true,
} satisfies Record<DownloadStream, true>

const ALL_PARTIAL_DATA = {
  nothingCreated: true,
  removed: true,
  kept: true,
} satisfies Record<PartialData, true>

// `DownloadCommandErrorKind` (эпик E4, TL-70) стал размеченным
// объединением объектов (несёт `existing` у `duplicateTask`), поэтому
// полнота набора здесь проверяется по `DownloadCommandError['kind']` —
// извлечённому индексированным доступом строковому union тегов, а не по
// имени типа целиком (правило CLAUDE.md о `satisfies` при разметке
// объединений).
const ALL_DOWNLOAD_COMMAND_ERROR_KINDS = {
  unknownTask: true,
  notFailed: true,
  notRetryable: true,
  noStreamsSelected: true,
  invalidUrl: true,
  duplicateTask: true,
  taskNotFinished: true,
} satisfies Record<DownloadCommandError['kind'], true>

describe('DownloadPhase — ровно семь значений контракта (Ф-3)', () => {
  it('has exactly 7 phases', () => {
    expect(Object.keys(ALL_DOWNLOAD_PHASES)).toHaveLength(7)
  })
})

describe('DownloadErrorKind — ровно девять классов контракта (Ф-10)', () => {
  it('has exactly 9 error kinds', () => {
    expect(Object.keys(ALL_DOWNLOAD_ERROR_KINDS)).toHaveLength(9)
  })

  it('reuses the four probe error kinds verbatim (E2 texts stay valid without translation)', () => {
    const reused: DownloadErrorKind[] = ['videoUnavailable', 'signInRequired', 'regionBlocked', 'ytDlpFailure']
    for (const kind of reused) {
      expect(ALL_DOWNLOAD_ERROR_KINDS).toHaveProperty(kind)
    }
  })
})

describe('прочие enum-ы контракта — полнота набора', () => {
  it('DownloadPlan has exactly 2 values', () => {
    expect(Object.keys(ALL_DOWNLOAD_PLANS)).toHaveLength(2)
  })

  it('DownloadStream has exactly 2 values', () => {
    expect(Object.keys(ALL_DOWNLOAD_STREAMS)).toHaveLength(2)
  })

  it('PartialData has exactly 3 values', () => {
    expect(Object.keys(ALL_PARTIAL_DATA)).toHaveLength(3)
  })

  it('DownloadCommandErrorKind has exactly 7 values (TL-70/TL-75: alreadyActive убран, duplicateTask/taskNotFinished добавлены)', () => {
    expect(Object.keys(ALL_DOWNLOAD_COMMAND_ERROR_KINDS)).toHaveLength(7)
  })
})

/**
 * Дословная форма событий `download://progress` из issue #41 — присвоение
 * литералов типу `DownloadProgressEvent` ловит структурную ошибку зеркала
 * (не то поле, не в том месте, забытый optional) на сборке: если форма
 * зеркала разойдётся с этими примерами, `npm run type-check` откажет.
 */
describe('DownloadProgressEvent — дословная форма из issue #41 принимается типом', () => {
  it('queued / fetching / merging — без данных', () => {
    const events: DownloadProgressEvent[] = [
      { taskId: 'task-1', phase: 'queued' },
      { taskId: 'task-1', phase: 'fetching' },
      { taskId: 'task-1', phase: 'merging' },
    ]
    expect(events).toHaveLength(3)
  })

  it('downloading/running — первое событие прогресса, размер ещё не известен', () => {
    const event: DownloadProgressEvent = { taskId: 'task-1', phase: 'downloading', state: 'running' }
    expect(event.phase).toBe('downloading')
  })

  it('downloading/running — полный набор опциональных полей', () => {
    const event: DownloadProgressEvent = {
      taskId: 'task-1',
      phase: 'downloading',
      state: 'running',
      stream: 'video',
      percent: 62,
      speedBytesPerSec: 4404019,
      etaSecs: 100,
      attempt: { number: 2, total: 6 },
    }
    expect(event.phase).toBe('downloading')
  })

  it('downloading/waitingRetry — без скорости и оценки времени, с обратным отсчётом', () => {
    const event: DownloadProgressEvent = {
      taskId: 'task-1',
      phase: 'downloading',
      state: 'waitingRetry',
      percent: 62,
      attempt: { number: 2, total: 6 },
      delaySecs: 10,
      remainingSecs: 8,
    }
    expect(event.phase).toBe('downloading')
  })

  it('done — имя готового файла', () => {
    const event: DownloadProgressEvent = {
      taskId: 'task-1',
      phase: 'done',
      fileName: 'Как приручить дракона.mp4',
      folderDisplay: { kind: 'systemDownloads' },
    }
    expect(event.phase === 'done' && event.fileName).toBe('Как приручить дракона.mp4')
  })

  it('failed — класс ytDlpFailure с обязательным reason', () => {
    const event: DownloadProgressEvent = {
      taskId: 'task-1',
      phase: 'failed',
      error: {
        kind: 'ytDlpFailure',
        message: '…',
        retryable: true,
        reason: 'outdated',
        partialData: 'kept',
        details: { stderrTail: '…', exitCode: 1 },
      },
    }
    expect(event.phase).toBe('failed')
  })

  it('failed — mergeFailed без reason (поле только для ytDlpFailure)', () => {
    const event: DownloadProgressEvent = {
      taskId: 'task-1',
      phase: 'failed',
      error: {
        kind: 'mergeFailed',
        message: '…',
        retryable: true,
        partialData: 'kept',
        details: { stderrTail: '…', exitCode: 1 },
      },
    }
    expect(event.phase).toBe('failed')
  })

  it('cancelled — подчистка выполнена', () => {
    const event: DownloadProgressEvent = { taskId: 'task-1', phase: 'cancelled', partialData: 'removed' }
    expect(event.partialData).toBe('removed')
  })
})
