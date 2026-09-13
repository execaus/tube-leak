import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { defineStore } from 'pinia'
import { onScopeDispose, ref } from 'vue'

import type {
  HistoryCommandErrorKind,
  HistoryCursor,
  HistoryEntry,
  HistoryNotice,
  HistoryPage,
  HistoryUnavailableError,
  ShowInFolderErrorKind,
} from '@/types/generated/history'
import { knownKindsOf } from '@/utils/knownKinds'

const HISTORY_PAGE_COMMAND = 'history_page'
const DELETE_HISTORY_RECORD_COMMAND = 'delete_history_record'
const CLEAR_HISTORY_COMMAND = 'clear_history'
const SHOW_IN_FOLDER_COMMAND = 'show_in_folder'

/** Уже существующее событие состава очереди (Ф-4: в нём видно появление `Done`) — новое событие история не заводит (доктрина «Зафиксировано анализом»). */
const QUEUE_CHANGED_EVENT = 'queue://changed'

/** Стартовая точка калибровки размера страницы (дизайн E5, «Команды», Н-7). */
const HISTORY_PAGE_LIMIT = 30

const KNOWN_UNAVAILABLE_REASONS = knownKindsOf({
  newerVersion: true,
  noAccess: true,
  migrationFailed: true,
} satisfies Record<HistoryUnavailableError['reason'], true>)

function isHistoryUnavailableError(value: unknown): value is HistoryUnavailableError {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Record<string, unknown>
  return (
    typeof candidate.reason === 'string' &&
    (KNOWN_UNAVAILABLE_REASONS as readonly string[]).includes(candidate.reason) &&
    typeof candidate.message === 'string'
  )
}

const KNOWN_COMMAND_ERROR_KINDS = knownKindsOf({
  unknownRecord: true,
  writeFailed: true,
  unavailable: true,
} satisfies Record<HistoryCommandErrorKind['kind'], true>)

function isHistoryCommandErrorKind(value: unknown): value is HistoryCommandErrorKind {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Record<string, unknown>
  return typeof candidate.kind === 'string' && (KNOWN_COMMAND_ERROR_KINDS as readonly string[]).includes(candidate.kind)
}

/** Отказ `delete_history_record`/`clear_history`: контрактный класс либо неконтрактный сбой самого IPC-вызова (тот же приём, что `DownloadCommandFailure` в `downloadTask.ts`). */
export type HistoryCommandFailure = HistoryCommandErrorKind | { kind?: undefined; message: string }

function toHistoryCommandFailure(err: unknown): HistoryCommandFailure {
  if (isHistoryCommandErrorKind(err)) return err
  if (err instanceof Error) return { message: err.message }
  if (typeof err === 'string' && err.length > 0) return { message: err }
  return { message: 'Команда истории отклонена по нераспознанной причине.' }
}

const KNOWN_SHOW_IN_FOLDER_ERROR_KINDS = knownKindsOf({
  fileMissing: true,
  folderMissing: true,
  launcherFailed: true,
  unknownRecord: true,
  unavailable: true,
} satisfies Record<ShowInFolderErrorKind['kind'], true>)

function isShowInFolderErrorKind(value: unknown): value is ShowInFolderErrorKind {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Record<string, unknown>
  return typeof candidate.kind === 'string' && (KNOWN_SHOW_IN_FOLDER_ERROR_KINDS as readonly string[]).includes(candidate.kind)
}

/** Отказ `show_in_folder` — тот же приём, что {@link HistoryCommandFailure}. */
export type ShowInFolderFailure = ShowInFolderErrorKind | { kind?: undefined; message: string }

function toShowInFolderFailure(err: unknown): ShowInFolderFailure {
  if (isShowInFolderErrorKind(err)) return err
  if (err instanceof Error) return { message: err.message }
  if (typeof err === 'string' && err.length > 0) return { message: err }
  return { message: 'Команда «Показать в папке» отклонена по нераспознанной причине.' }
}

/** Копия записи без ключа `key` — без деструктуризации с отброшенным биндингом, чтобы не заводить неиспользуемую переменную. */
function withoutKey<T>(record: Record<string, T>, key: string): Record<string, T> {
  const next = { ...record }
  delete next[key]
  return next
}

/**
 * Домен «история загрузок» (эпик E5, TL-93) — один Pinia-стор на домен
 * (CLAUDE.md), отдельный от `downloadTask.ts` (очередь — другой домен).
 * Ядро — источник истины (Ф-4/Ф-1): стор — проекция страниц `history_page`,
 * не накопитель, который сам решает порядок или дедуплицирует иначе, чем
 * сказал ответ команды.
 *
 * # Как список переживает `queue://changed`, не теряя страниц «Показать ещё»
 *
 * Дизайн Ф-4 требует обновлять список и по монтированию, и по
 * `queue://changed` (видно появление `Done`), а критерий TL-93 требует не
 * терять уже подгруженные «Показать ещё» страницы при этом. И то, и другое
 * решает один и тот же путь: {@link refreshFirst} всегда запрашивает
 * страницу **без курсора** (то есть настоящую «голову» списка — самые новые
 * записи), но не заменяет `entries` целиком, а **примешивает к началу**
 * только те записи ответа, чьего `id` ещё нет в уже показанном списке;
 * записи, уже подгруженные более ранними «Показать ещё» (весь «хвост»
 * списка), не трогает вовсе. Курсор для продолжения (`nextCursor`)
 * обновляется из ответа `refreshFirst` только при самом первом вызове
 * (`hasLoadedOnce` ещё `false`) — после этого он указывает «что идёт после
 * уже показанного хвоста», а хвост от прихода новых записей сверху не
 * меняется, так что перезаписывать курсор повторным `refreshFirst` было бы
 * ошибкой (последующий `loadMore()` продолжил бы не с того места).
 *
 * `loadMore()` (курсорный запрос) при этом никогда не трогает `notices` —
 * контракт и так обещает пустой список пометок в ответе с курсором (doc
 * `HistoryPage.notices` в `src/types/generated/history.ts`), но стор не
 * полагается на это молча: `notices` присваивается только в
 * {@link refreshFirst}.
 */
export const useHistoryStore = defineStore('history', () => {
  const entries = ref<HistoryEntry[]>([])
  const nextCursor = ref<HistoryCursor>()
  const notices = ref<HistoryNotice[]>([])
  const availability = ref<HistoryUnavailableError>()
  /** `true` после самого первого ответа (успешного или отказа) — гейт «пусто» против «ещё не спрашивали» на экране. */
  const loaded = ref(false)
  const isLoadingMore = ref(false)
  const commandError = ref<HistoryCommandFailure>()
  /** Отказ «Показать в папке» по каждой затронутой записи (Ф-8) — не общий баннер: у каждой строки свой файл и своя причина. */
  const showInFolderErrors = ref<Record<string, ShowInFolderFailure>>({})
  /** Текст скрытой живой зоны экрана (дизайн E5, «Доступность») — см. doc {@link refreshFirst}. */
  const liveAnnouncement = ref('')

  let hasLoadedOnce = false
  let unlistenQueue: UnlistenFn | undefined
  let listeningQueue: Promise<void> | undefined

  function applyPageError(err: unknown): void {
    if (isHistoryUnavailableError(err)) {
      availability.value = err
    } else {
      console.error('history_page rejected', err)
    }
  }

  /** Первая страница (без курсора) — примешивает новые записи к началу, не трогая уже загруженный хвост. См. doc класса выше. */
  async function refreshFirst(): Promise<void> {
    try {
      const page = await invoke<HistoryPage>(HISTORY_PAGE_COMMAND, { cursor: undefined, limit: HISTORY_PAGE_LIMIT })
      availability.value = undefined
      const known = new Set(entries.value.map((e) => e.id))
      const fresh = page.entries.filter((e) => !known.has(e.id))
      const isLiveUpdate = hasLoadedOnce
      entries.value = [...fresh, ...entries.value]
      notices.value = page.notices
      if (!hasLoadedOnce) {
        nextCursor.value = page.nextCursor
        hasLoadedOnce = true
      }
      // Живая зона (дизайн E5, «Доступность»): «структурные изменения... —
      // отдельной скрытой aria-live="polite" строкой», а не пересказом
      // содержимого списка целиком. Только для настоящего фонового
      // обновления (после первой загрузки, то есть по `queue://changed`) и
      // только когда реально добавилась хотя бы одна новая запись — ни
      // самая первая загрузка, ни «Показать ещё» (пользователь и так видит
      // результат своего клика) сюда не попадают.
      if (isLiveUpdate && fresh.length > 0) {
        liveAnnouncement.value = 'Добавлена новая запись'
      }
    } catch (err) {
      applyPageError(err)
    } finally {
      loaded.value = true
    }
  }

  function ensureQueueListening(): Promise<void> {
    if (!listeningQueue) {
      listeningQueue = listen(QUEUE_CHANGED_EVENT, () => {
        void refreshFirst()
      })
        .then((fn) => {
          unlistenQueue = fn
        })
        .catch((err: unknown) => {
          listeningQueue = undefined
          throw err
        })
    }
    return listeningQueue
  }

  /** Разовая загрузка первой страницы плюс подписка на `queue://changed` — вызывается явно из `HistoryScreen.vue` (`onMounted`), тем же приёмом, что `downloadTaskStore.initialize()`. */
  async function initialize(): Promise<void> {
    await ensureQueueListening()
    await refreshFirst()
  }

  /** «Показать ещё» (Ф-4) — курсор идёт в запрос как получен, без пересборки (мутация «пересобрать курсор» обязана покраснить тест). */
  async function loadMore(): Promise<void> {
    if (!nextCursor.value || isLoadingMore.value) return
    isLoadingMore.value = true
    try {
      const page = await invoke<HistoryPage>(HISTORY_PAGE_COMMAND, {
        cursor: nextCursor.value,
        limit: HISTORY_PAGE_LIMIT,
      })
      entries.value = [...entries.value, ...page.entries]
      nextCursor.value = page.nextCursor
    } catch (err) {
      applyPageError(err)
    } finally {
      isLoadingMore.value = false
    }
  }

  /** «Скрыть» баннер-пометку (Ф-3/С-10) — локально, до конца сеанса; следующий `refreshFirst` заменит `notices` целиком собственным ответом (doc `HistoryNotice` в `src/types/generated/history.ts`: сервер не переприносит однажды выданную пометку). */
  function dismissNotice(kind: HistoryNotice['kind']): void {
    notices.value = notices.value.filter((n) => n.kind !== kind)
  }

  /** Удаление одной записи (Ф-6) — без подтверждения (С-4), убирает строку сразу по ответу команды. */
  async function deleteRecord(id: string): Promise<void> {
    try {
      await invoke<void>(DELETE_HISTORY_RECORD_COMMAND, { id })
      entries.value = entries.value.filter((e) => e.id !== id)
      showInFolderErrors.value = withoutKey(showInFolderErrors.value, id)
      commandError.value = undefined
    } catch (err) {
      console.error('delete_history_record rejected', err)
      commandError.value = toHistoryCommandFailure(err)
    }
  }

  /** Очистка всей истории (Ф-6) — вызывается после подтверждения диалогом (С-4), одной транзакцией на стороне ядра. */
  async function clearHistory(): Promise<void> {
    try {
      await invoke<void>(CLEAR_HISTORY_COMMAND)
      entries.value = []
      nextCursor.value = undefined
      showInFolderErrors.value = {}
      commandError.value = undefined
    } catch (err) {
      console.error('clear_history rejected', err)
      commandError.value = toHistoryCommandFailure(err)
    }
  }

  function dismissCommandError(): void {
    commandError.value = undefined
  }

  /**
   * «Показать в папке» (Ф-8) — по `id` записи. Случай
   * `fileMissing` вместе с уже известным `missing+folderExists` не заводит
   * ошибку строки (дизайн E5, «таблица трёх случаев», строка 2: побочный
   * эффект уже произошёл — папка открылась, пометка строки не меняется).
   */
  async function showInFolder(entry: HistoryEntry): Promise<void> {
    try {
      await invoke<void>(SHOW_IN_FOLDER_COMMAND, { id: entry.id })
      showInFolderErrors.value = withoutKey(showInFolderErrors.value, entry.id)
    } catch (err) {
      const failure = toShowInFolderFailure(err)
      const isExpectedFileMissing =
        failure.kind === 'fileMissing' && entry.fileStatus.kind === 'missing' && entry.fileStatus.folderExists
      if (isExpectedFileMissing) {
        showInFolderErrors.value = withoutKey(showInFolderErrors.value, entry.id)
        return
      }
      console.error('show_in_folder rejected', err)
      showInFolderErrors.value = { ...showInFolderErrors.value, [entry.id]: failure }
    }
  }

  onScopeDispose(() => {
    if (unlistenQueue) unlistenQueue()
  })

  return {
    entries,
    nextCursor,
    notices,
    availability,
    loaded,
    isLoadingMore,
    commandError,
    showInFolderErrors,
    liveAnnouncement,
    initialize,
    loadMore,
    dismissNotice,
    deleteRecord,
    clearHistory,
    dismissCommandError,
    showInFolder,
  }
})
