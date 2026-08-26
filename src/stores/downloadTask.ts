import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { defineStore } from 'pinia'
import { computed, onScopeDispose, ref } from 'vue'

import type {
  DownloadCommandError,
  DownloadCommandErrorKind,
  DownloadPhase,
  DownloadPlan,
  DownloadProgress,
  DownloadProgressEvent,
  DownloadStarted,
  StartDownloadRequest,
} from '@/types/download'

const START_DOWNLOAD_COMMAND = 'start_download'
const CANCEL_DOWNLOAD_COMMAND = 'cancel_download'
const RETRY_DOWNLOAD_COMMAND = 'retry_download'

/** Имя события прогресса, эмитится Rust-стороной (Ф-2, TL-38/TL-44). */
const PROGRESS_EVENT_NAME = 'download://progress'

/** Порог локального косметического индикатора зависания (дизайн E3, «Числа»). */
const SOFT_STALL_THRESHOLD_MS = 5_000
const SOFT_STALL_TICK_MS = 1_000

/** То, что панель держит про текущую задачу помимо серверного `DownloadProgress`. */
export interface DownloadTask {
  taskId: string
  plan: DownloadPlan
  /**
   * Заголовок панели, снятый с карточки ролика **в момент клика** «Скачать»
   * (требование С-13/TL-45, п.6) — не читается из карточки повторно, поэтому
   * переживает замену карточки на новую ссылку.
   */
  displayTitle: string
}

const KNOWN_COMMAND_ERROR_KINDS: readonly DownloadCommandErrorKind[] = [
  'alreadyActive',
  'unknownTask',
  'notFailed',
  'notRetryable',
  'noStreamsSelected',
  'invalidUrl',
]

function isDownloadCommandError(value: unknown): value is DownloadCommandError {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Record<string, unknown>
  return (
    typeof candidate.kind === 'string' &&
    (KNOWN_COMMAND_ERROR_KINDS as readonly string[]).includes(candidate.kind) &&
    typeof candidate.message === 'string'
  )
}

/**
 * То, что реально может оказаться отказом одной из трёх команд
 * (`start_download`/`cancel_download`/`retry_download`). Контрактный путь
 * честный ({@link DownloadCommandError}), но неконтрактный отказ существует
 * (паника команды, отказ самого IPC-вызова) и должен быть учтён без каста
 * вслепую — тот же приём, что `ProbeFailure` в `useProbe.ts` (эпик E2) и
 * `PrepareFailure` в `useYtDlpPrepare.ts` (эпик E1).
 */
export type DownloadCommandFailure = DownloadCommandError | { kind?: undefined; message: string }

function toDownloadCommandFailure(err: unknown): DownloadCommandFailure {
  if (isDownloadCommandError(err)) return err
  if (err instanceof Error) return { message: err.message }
  if (typeof err === 'string' && err.length > 0) return { message: err }
  return { message: 'Команда управления загрузкой отклонена по нераспознанной причине.' }
}

function isTerminalPhase(progress: DownloadProgress): boolean {
  return progress.phase === 'done' || progress.phase === 'failed' || progress.phase === 'cancelled'
}

/**
 * Начальное представление задачи сразу после `start_download` — строится
 * по фазе из ответа команды, а не константой (ревью TL-45): doc-комментарий
 * `DownloadStarted.phase` в `src/types/download.ts` прямо требует рисовать
 * по присланному полю, потому что в эпике очереди (E4) задача может
 * реально задержаться в `queued`, ожидая слот.
 *
 * Ветки для терминальных фаз и `merging` — оборона на случай будущего
 * расширения контракта, а не ожидаемый путь (задача только что создана):
 * ближайшая честная трактовка — «ещё не начиналась»/«идёт скачивание без
 * данных потока».
 */
function initialProgressForPhase(phase: DownloadPhase): DownloadProgress {
  switch (phase) {
    case 'queued':
      return { phase: 'queued' }
    case 'fetching':
      return { phase: 'fetching' }
    case 'downloading':
      return { phase: 'downloading', state: 'running' }
    case 'merging':
      return { phase: 'merging' }
    case 'done':
    case 'failed':
    case 'cancelled':
      return { phase: 'queued' }
  }
}

/**
 * Домен «текущая загрузка» (эпик E3, TL-45): один Pinia-стор на домен
 * (CLAUDE.md), переживающий замену `VideoCard` и используемый и панелью, и
 * (с TL-46) диалогом подтверждения выхода — общее состояние, а не удобство
 * изложения.
 *
 * # Почему это стор, а не composable, локальный компоненту
 *
 * Задача должна пережить размонтирование карточки, которая её запустила
 * (С-13), и её должен видеть код вне панели (кнопка «Скачать» на
 * возможно уже другой карточке, будущий диалог выхода TL-46) — состояние
 * не принадлежит времени жизни одного компонента.
 *
 * # Поток событий после отказа (требование TL-45, п.3)
 *
 * Подписка на `download://progress` живёт всё время жизни стора (эффективно
 * — всё приложение) и никогда не отписывается и не фильтруется по фазе:
 * `failed` не глушит подписку, `retry()` лишь просит ядро продолжить ту же
 * задачу, а новые события того же `taskId` продолжают приходить в тот же
 * `progress` без каких-либо дополнительных действий на этой стороне (doc
 * `DownloadProgressEvent` в `src/types/download.ts`).
 *
 * # Отказ команды — виден на экране, не только в консоли (ревью TL-45)
 *
 * Кнопка «Скачать» на исправном фронтенде не должна быть достижима для
 * `alreadyActive`, но путь к молчаливому отказу реален (несовпадение
 * обрезки пробелов между разбором и стартом — почин в `ProbeSection.vue`)
 * и для остальных пяти классов тоже: приложение не может полагаться
 * только на то, что кнопка была неактивна (дизайн E3, «Кнопка Скачать»).
 * Поэтому отказ любой из трёх команд не глушится молча — он оседает в
 * `commandError` и рисуется тем же приёмом, что и прочие ошибки: заголовок
 * и пояснение по классу, без кнопки «Повторить» (все шесть классов
 * означают, что повторять нечего — либо гонка уже разрешилась сама,
 * либо нужен другой ввод, а не тот же вызов ещё раз).
 */
export const useDownloadTaskStore = defineStore('downloadTask', () => {
  const task = ref<DownloadTask>()
  const progress = ref<DownloadProgress>()
  const commandError = ref<DownloadCommandFailure>()

  /**
   * Секунды без события — только пока идёт `downloading`/`running`
   * (единственное состояние, где есть скорость, которую нечестно было бы
   * продолжать показывать замороженной, дизайн E3 «Три разных нет
   * движения», пункт 1). `undefined` — индикатор не показан.
   */
  const softStallSeconds = ref<number>()

  let unlisten: UnlistenFn | undefined
  let listening: Promise<void> | undefined
  let lastEventAt = 0
  let stallTimer: ReturnType<typeof setInterval> | undefined
  // Окно двойного клика (ревью TL-45, «Заметки»): одна из трёх команд в
  // любой момент, кнопки не блокируют себя сами — блокирует стор.
  let commandInFlight = false

  function clearStallTimer(): void {
    if (stallTimer !== undefined) {
      clearInterval(stallTimer)
      stallTimer = undefined
    }
    softStallSeconds.value = undefined
  }

  function noteActivity(): void {
    lastEventAt = Date.now()
    softStallSeconds.value = undefined
  }

  function ensureStallTimer(): void {
    if (stallTimer !== undefined) return
    stallTimer = setInterval(() => {
      const current = progress.value
      if (!current || current.phase !== 'downloading' || current.state !== 'running') {
        clearStallTimer()
        return
      }
      const elapsedMs = Date.now() - lastEventAt
      softStallSeconds.value =
        elapsedMs >= SOFT_STALL_THRESHOLD_MS ? Math.floor(elapsedMs / 1000) : undefined
    }, SOFT_STALL_TICK_MS)
  }

  function handleProgressEvent(event: { payload: DownloadProgressEvent }): void {
    const current = task.value
    // Ключ задачи — `taskId`; сверяем его, а не считаем любое пришедшее
    // событие «своим» (doc `DownloadProgressEvent`).
    if (!current || event.payload.taskId !== current.taskId) return

    // `taskId` — только ключ маршрутизации события, в состояние панели не
    // входит (панель уже знает id из своей задачи); явно отделяем его от
    // остальных полей, а не оставляем висеть лишним свойством на объекте.
    const { taskId, ...rest } = event.payload
    void taskId
    progress.value = rest

    if (rest.phase === 'downloading' && rest.state === 'running') {
      noteActivity()
      ensureStallTimer()
    } else {
      clearStallTimer()
    }
  }

  function ensureListening(): Promise<void> {
    if (!listening) {
      listening = listen<DownloadProgressEvent>(PROGRESS_EVENT_NAME, handleProgressEvent)
        .then((fn) => {
          unlisten = fn
        })
        .catch((err: unknown) => {
          // Сбрасываем memoization — следующий `start()` должен снова
          // попытаться подписаться, а не унаследовать отклонённый промис
          // навсегда (тот же приём, что `useYtDlpPrepare`, эпик E1).
          listening = undefined
          throw err
        })
    }
    return listening
  }

  /**
   * Слот занят, пока задача существует и не в терминальной фазе — по этому
   * полю кнопка «Скачать» на карточке решает, показывать ли подсказку С-13.
   * Мгновенно после `start()` `progress` уже заполнен, поэтому «задача
   * создана, но фаза ещё не пришла» здесь не бывает.
   */
  const isActive = computed(() => {
    const current = progress.value
    return task.value !== undefined && current !== undefined && !isTerminalPhase(current)
  })

  /** Скрывает баннер отказа команды («Скрыть» на нём же). */
  function dismissCommandError(): void {
    commandError.value = undefined
  }

  /**
   * Запускает новую задачу. Слот уже должен быть свободен (кнопка на
   * карточке это гарантирует) — ядро всё равно проверяет это само
   * (`alreadyActive`) и является единственным источником истины (дизайн
   * E3, «Кнопка Скачать»). Отказ не глушится: он оседает в `commandError`
   * и виден на экране (см. doc стора).
   */
  async function start(request: StartDownloadRequest, displayTitle: string): Promise<void> {
    if (commandInFlight) return
    commandInFlight = true
    commandError.value = undefined
    try {
      await ensureListening()
      const started = await invoke<DownloadStarted>(START_DOWNLOAD_COMMAND, { request })
      task.value = { taskId: started.taskId, plan: started.plan, displayTitle }
      progress.value = initialProgressForPhase(started.phase)
      clearStallTimer()
    } catch (err) {
      console.error('start_download rejected', err)
      commandError.value = toDownloadCommandFailure(err)
    } finally {
      commandInFlight = false
    }
  }

  /** Отмена — доступна в любой нетерминальной фазе (Ф-4). */
  async function cancel(): Promise<void> {
    if (commandInFlight) return
    const current = task.value
    if (!current) return
    commandInFlight = true
    try {
      await invoke<void>(CANCEL_DOWNLOAD_COMMAND, { taskId: current.taskId })
      commandError.value = undefined
    } catch (err) {
      console.error('cancel_download rejected', err)
      commandError.value = toDownloadCommandFailure(err)
    } finally {
      commandInFlight = false
    }
  }

  /**
   * Повтор — продолжение той же задачи (тот же `taskId`), не новая
   * сущность (дизайн E3, «Управляющие вызовы», п.3). Поток событий
   * возобновляется сам, без какого-либо сброса состояния здесь.
   */
  async function retry(): Promise<void> {
    if (commandInFlight) return
    const current = task.value
    if (!current) return
    commandInFlight = true
    try {
      await invoke<void>(RETRY_DOWNLOAD_COMMAND, { taskId: current.taskId })
      commandError.value = undefined
    } catch (err) {
      console.error('retry_download rejected', err)
      commandError.value = toDownloadCommandFailure(err)
    } finally {
      commandInFlight = false
    }
  }

  /** «Скрыть» — освобождает слот панели; допустимо только для терминальной задачи. */
  function hide(): void {
    const current = progress.value
    if (current && isTerminalPhase(current)) {
      task.value = undefined
      progress.value = undefined
      clearStallTimer()
    }
  }

  onScopeDispose(() => {
    clearStallTimer()
    if (unlisten) unlisten()
  })

  return {
    task,
    progress,
    softStallSeconds,
    commandError,
    isActive,
    start,
    cancel,
    retry,
    hide,
    dismissCommandError,
  }
})
