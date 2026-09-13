import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { defineStore } from 'pinia'
import { onScopeDispose, ref } from 'vue'

import type {
  HistoryCommandError,
  HistoryCommandErrorKind,
  HistoryCursor,
  HistoryEntry,
  HistoryNotice,
  HistoryPage,
  HistoryUnavailableError,
  ShowInFolderError,
  ShowInFolderErrorKind,
} from '@/types/generated/history'
import { knownKindsOf } from '@/utils/knownKinds'
import { formatTaskDisplayTitle } from '@/utils/queueTaskTitle'

const HISTORY_PAGE_COMMAND = 'history_page'
const DELETE_HISTORY_RECORD_COMMAND = 'delete_history_record'
const CLEAR_HISTORY_COMMAND = 'clear_history'
const SHOW_IN_FOLDER_COMMAND = 'show_in_folder'

/** Уже существующее событие состава очереди (Ф-4: в нём видно появление `Done`) — новое событие история не заводит (доктрина «Зафиксировано анализом»). */
const QUEUE_CHANGED_EVENT = 'queue://changed'

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

/**
 * Проверяет и `kind`, и `message` — не только `kind` (в отличие от
 * прежней версии): контрактный {@link HistoryCommandError} несёт оба поля
 * всегда, а редирект `unavailable`-отказов в блокирующее состояние экрана
 * (мелочи правок ревью TL-93, второй раунд) читает `message` из уже
 * типизированного значения, а не заново лезет в сырой `err`.
 */
function isHistoryCommandErrorKind(value: unknown): value is HistoryCommandError {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Record<string, unknown>
  return (
    typeof candidate.kind === 'string' &&
    (KNOWN_COMMAND_ERROR_KINDS as readonly string[]).includes(candidate.kind) &&
    typeof candidate.message === 'string'
  )
}

/** Отказ `delete_history_record`/`clear_history`: контрактный класс (с `message`) либо неконтрактный сбой самого IPC-вызова (тот же приём, что `DownloadCommandFailure` в `downloadTask.ts`). */
export type HistoryCommandFailure = HistoryCommandError | { kind?: undefined; message: string }

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

/** Проверяет и `kind`, и `message` — см. doc {@link isHistoryCommandErrorKind}, тот же приём. */
function isShowInFolderErrorKind(value: unknown): value is ShowInFolderError {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Record<string, unknown>
  return (
    typeof candidate.kind === 'string' &&
    (KNOWN_SHOW_IN_FOLDER_ERROR_KINDS as readonly string[]).includes(candidate.kind) &&
    typeof candidate.message === 'string'
  )
}

/** Отказ `show_in_folder` — тот же приём, что {@link HistoryCommandFailure}. */
export type ShowInFolderFailure = ShowInFolderError | { kind?: undefined; message: string }

function toShowInFolderFailure(err: unknown): ShowInFolderFailure {
  if (isShowInFolderErrorKind(err)) return err
  if (err instanceof Error) return { message: err.message }
  if (typeof err === 'string' && err.length > 0) return { message: err }
  return { message: 'Команда «Показать в папке» отклонена по нераспознанной причине.' }
}

/**
 * Состояние строки «Показать в папке» (Ф-8) — либо один из пяти
 * контрактных классов (или неконтрактный сбой IPC, {@link ShowInFolderFailure}),
 * либо синтетическое `goneFromHistory`: С-3 (правки ревью TL-93, второй
 * раунд) требует для `unknownRecord` не совет «обновите историю», а
 * нейтральный факт плюс автоматический перезапрос первой страницы (см.
 * {@link useHistoryStore.showInFolder}) — `goneFromHistory` держит на
 * экране этот факт, пока сама запись либо пропадёт из списка настоящим
 * обновлением, либо останется (перезапрос не обязан её коснуться, если
 * она в хвосте, уже подгруженном «Показать ещё» — см. doc
 * {@link refreshFirst}).
 */
export type ShowInFolderRowState = ShowInFolderFailure | { kind: 'goneFromHistory' }

/** Копия записи без ключа `key` — без деструктуризации с отброшенным биндингом, чтобы не заводить неиспользуемую переменную. */
function withoutKey<T>(record: Record<string, T>, key: string): Record<string, T> {
  const next = { ...record }
  delete next[key]
  return next
}

/** Добавляет к уже показанным пометкам только те виды из ответа, которых там ещё нет — см. doc «Пометки» у {@link useHistoryStore}. */
function mergeNotices(current: HistoryNotice[], incoming: HistoryNotice[]): HistoryNotice[] {
  const known = new Set(current.map((n) => n.kind))
  const toAdd = incoming.filter((n) => !known.has(n.kind))
  return toAdd.length > 0 ? [...current, ...toAdd] : current
}

/**
 * Домен «история загрузок» (эпик E5, TL-93) — один Pinia-стор на домен
 * (CLAUDE.md), отдельный от `downloadTask.ts` (очередь — другой домен).
 * Ядро — источник истины (Ф-4/Ф-1): стор — проекция страниц `history_page`,
 * не накопитель, который сам решает порядок или дедуплицирует иначе, чем
 * сказал ответ команды.
 *
 * # Пометки — копятся, не перезаписываются (Б-1, правки ревью TL-93, второй раунд)
 *
 * Первая версия присваивала `notices.value = page.notices` на каждом
 * ответе без курсора — и однократная пометка ядра гасла сама, стоило
 * прийти следующему `queue://changed` с пустым `notices` (обычнейший
 * случай: любое следующее событие очереди, не только «Скрыть»). Контракт
 * обещает пометку **ровно один раз** (doc {@link HistoryNotice}), и это
 * «один раз» держит клиент: {@link mergeNotices} добавляет к уже
 * показанным только виды, которых там ещё нет, а убирает их исключительно
 * {@link dismissNotice}. Пустой ответ, соответственно, не трогает уже
 * показанные пометки вовсе.
 *
 * # Первая страница — сверяется со списком, а не только дописывает новое (Б-2/С-1)
 *
 * Дизайн Ф-4 требует обновлять список и по монтированию, и по
 * `queue://changed` (видно появление `Done`), и, начиная с этого раунда
 * правок, по активации вкладки «История» (см. `HistoryScreen.vue`,
 * `props.active`) — а критерий TL-93 требует не терять уже подгруженные
 * «Показать ещё» страницы при этом. Первая версия решала это, домешивая к
 * началу списка только записи с незнакомым `id` — и тем самым никогда не
 * замечала, что уже показанная запись **изменилась** (статус файла,
 * Б-2) или вовсе пропала на стороне ядра (пересозданная база, С-1).
 *
 * Текущий алгоритм {@link refreshFirst} всегда запрашивает страницу без
 * курсора (настоящую «голову» списка) и затем сверяет её с уже показанным:
 *
 * - Если это самая первая загрузка (`hasLoadedOnce` ещё `false`) — ответ
 *   заменяет список целиком, `nextCursor` берётся из него же.
 * - Иначе, если свежая страница **пуста** — список сбрасывается в пустой
 *   (ядру больше нечего показать «сверху»; хвост, который эта страница не
 *   видела, всё равно устарел бы, если бы база пересоздана целиком).
 * - Иначе ищется **последняя** запись свежей страницы в уже показанном
 *   списке (порядок `(finishedAt, id)` строгий и общий у списка и
 *   страницы, doc {@link HistoryCursor}):
 *   - Найдена на позиции `i` — список становится «свежая страница» +
 *     «хвост списка после позиции `i`»: всё, что было в уже показанном
 *     префиксе (статусы файлов, наличие записей), заменяется свежими
 *     данными, а хвост, подгруженный более ранними «Показать ещё», не
 *     трогается. `nextCursor` в этой ветке **не переписывается** — он и
 *     так указывает «что идёт после хвоста», а хвост от прихода записей
 *     сверху не меняется.
 *   - Не найдена — расхождение слишком велико, чтобы аккуратно склеить
 *     (новых записей больше, чем страница; либо база пересоздана и старых
 *     `id` в ней больше нет вовсе): список **сбрасывается** к свежей
 *     странице целиком, вместе с её `nextCursor`. Хвост в этом случае
 *     теряется намеренно — источник истины ядро, а не клиентская догадка,
 *     что от хвоста ещё актуально.
 *
 * `loadMore()` (курсорный запрос) при этом никогда не трогает `notices` —
 * контракт и так обещает пустой список пометок в ответе с курсором (doc
 * `HistoryPage.notices`), но стор не полагается на это молча: `notices`
 * трогает только {@link refreshFirst}.
 *
 * # Гонка двух `refreshFirst` (правки ревью TL-93, второй раунд)
 *
 * Монтирование, `queue://changed` и активация вкладки могут запустить
 * {@link refreshFirst} почти одновременно; поскольку это два независимых
 * IPC-вызова, ответ на **более старый** запрос иногда доходит **позже**
 * ответа на новый. Сторож — монотонный счётчик `refreshCallId`, тот же
 * приём, что `generation` в `useProbe.ts`: каждый вызов запоминает своё
 * значение при старте и сверяет его перед тем, как что-либо записать в
 * состояние; несовпадение — работа отброшена целиком, и успех, и отказ.
 */
export const useHistoryStore = defineStore('history', () => {
  const entries = ref<HistoryEntry[]>([])
  const nextCursor = ref<HistoryCursor>()
  const notices = ref<HistoryNotice[]>([])
  const availability = ref<HistoryUnavailableError>()
  /**
   * Отказ `history_page`, который не разобрался в типизированный
   * {@link HistoryUnavailableError} (С-6) — исключение самого IPC-вызова.
   * Отдельный флаг, не переиспользование `availability`: у него нет
   * контрактной `reason`, а придумывать её было бы нечестной догадкой
   * (см. `getHistoryUnavailableText`/`HISTORY_IPC_FAILURE_TEXT`).
   */
  const ipcFailure = ref(false)
  /** `true` после самого первого ответа (успешного или отказа) — гейт «пусто» против «ещё не спрашивали» на экране. */
  const loaded = ref(false)
  const isLoadingMore = ref(false)
  const commandError = ref<HistoryCommandFailure>()
  /** Отказ «Показать в папке» по каждой затронутой записи (Ф-8) — не общий баннер: у каждой строки свой файл и своя причина. */
  const showInFolderErrors = ref<Record<string, ShowInFolderRowState>>({})
  /** Текст скрытой живой зоны экрана (дизайн E5, «Доступность») — см. doc {@link refreshFirst}. */
  const liveAnnouncement = ref('')

  /** `false` сразу после `clearHistory()` (полный сброс, Б-2/С-1) — следующий {@link refreshFirst} должен снова вести себя как самая первая загрузка. */
  let hasLoadedOnce = false
  let unlistenQueue: UnlistenFn | undefined
  let listeningQueue: Promise<void> | undefined
  /** Сторож гонки {@link refreshFirst} — см. doc-комментарий класса, «Гонка двух refreshFirst». */
  let refreshCallId = 0

  function applyPageError(err: unknown): void {
    if (isHistoryUnavailableError(err)) {
      availability.value = err
      ipcFailure.value = false
    } else {
      console.error('history_page rejected', err)
      ipcFailure.value = true
    }
  }

  /** Первая страница (без курсора) — сверяет её с уже показанным списком. См. doc класса выше, «Первая страница». */
  async function refreshFirst(): Promise<void> {
    const callId = ++refreshCallId
    try {
      const page = await invoke<HistoryPage>(HISTORY_PAGE_COMMAND, { cursor: undefined })
      if (callId !== refreshCallId) return // отброшен более новым вызовом, пока этот был в полёте
      availability.value = undefined
      ipcFailure.value = false

      const knownIdsBefore = new Set(entries.value.map((e) => e.id))
      const isLiveUpdate = hasLoadedOnce

      if (!hasLoadedOnce) {
        entries.value = page.entries
        nextCursor.value = page.nextCursor
        hasLoadedOnce = true
      } else if (page.entries.length === 0) {
        entries.value = []
        nextCursor.value = page.nextCursor
      } else {
        const lastFreshId = page.entries[page.entries.length - 1]!.id
        const splitIndex = entries.value.findIndex((e) => e.id === lastFreshId)
        if (splitIndex === -1) {
          entries.value = page.entries
          nextCursor.value = page.nextCursor
        } else {
          const tail = entries.value.slice(splitIndex + 1)
          entries.value = [...page.entries, ...tail]
          // `nextCursor` умышленно не трогается здесь — см. doc класса.
        }
      }

      notices.value = mergeNotices(notices.value, page.notices)

      // Живая зона (дизайн E5, «Доступность»): «структурные изменения... —
      // отдельной скрытой aria-live="polite" строкой», а не пересказом
      // содержимого списка целиком. Только для настоящего фонового
      // обновления (после первой загрузки) и только когда реально
      // добавилась хотя бы одна новая запись. Текст включает название —
      // не «мелочи»: одна и та же строка "Добавлена новая запись" дважды
      // подряд не была бы замечена скринридером (aria-live озвучивает
      // изменение текста, а не факт присваивания одного и того же
      // значения), см. doc «мелочи» задачи TL-93.
      if (isLiveUpdate) {
        const freshOnes = page.entries.filter((e) => !knownIdsBefore.has(e.id))
        if (freshOnes.length === 1) {
          liveAnnouncement.value = `Добавлена новая запись: ${formatTaskDisplayTitle(freshOnes[0]!.title, freshOnes[0]!.quality)}`
        } else if (freshOnes.length > 1) {
          liveAnnouncement.value = `Добавлено новых записей: ${freshOnes.length}`
        }
      }
    } catch (err) {
      if (callId !== refreshCallId) return
      applyPageError(err)
    } finally {
      if (callId === refreshCallId) loaded.value = true
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

  /** «Показать ещё» (Ф-4) — курсор идёт в запрос как получен от предыдущего ответа `history_page`, не собирается заново из полей записи (см. doc-комментарий теста `history.test.ts`, «Курсор как есть»). */
  async function loadMore(): Promise<void> {
    if (!nextCursor.value || isLoadingMore.value) return
    isLoadingMore.value = true
    try {
      const page = await invoke<HistoryPage>(HISTORY_PAGE_COMMAND, { cursor: nextCursor.value })
      entries.value = [...entries.value, ...page.entries]
      nextCursor.value = page.nextCursor
    } catch (err) {
      applyPageError(err)
    } finally {
      isLoadingMore.value = false
    }
  }

  /** «Скрыть» баннер-пометку (Ф-3/С-10, Б-1) — единственный способ убрать уже показанную пометку; следующий `refreshFirst`, пустой или нет, её не возвращает (doc класса, «Пометки»). */
  function dismissNotice(kind: HistoryNotice['kind']): void {
    notices.value = notices.value.filter((n) => n.kind !== kind)
  }

  /**
   * Удаление одной записи (Ф-6) — без подтверждения (С-4), убирает строку
   * сразу по ответу команды. `unknownRecord` (С-3, правки ревью TL-93,
   * второй раунд) убирает строку так же тихо, без баннера: ядро уже не
   * знает об этой записи, а именно это и было целью нажатия «Удалить» —
   * цель достигнута, сообщать не о чем. `unavailable` переводит экран в
   * то же блокирующее состояние, что и `history_page` (доктрина «мелочи»
   * задачи TL-93: у истории одно, а не два представления недоступности).
   */
  async function deleteRecord(id: string): Promise<void> {
    try {
      await invoke<void>(DELETE_HISTORY_RECORD_COMMAND, { id })
      entries.value = entries.value.filter((e) => e.id !== id)
      showInFolderErrors.value = withoutKey(showInFolderErrors.value, id)
      commandError.value = undefined
    } catch (err) {
      const failure = toHistoryCommandFailure(err)
      if (failure.kind === 'unknownRecord') {
        entries.value = entries.value.filter((e) => e.id !== id)
        return
      }
      if (failure.kind === 'unavailable') {
        availability.value = { reason: failure.reason, message: failure.message }
        return
      }
      console.error('delete_history_record rejected', err)
      commandError.value = failure
    }
  }

  /**
   * Очистка всей истории (Ф-6) — вызывается после подтверждения диалогом
   * (С-4), одной транзакцией на стороне ядра. Успех сбрасывает не только
   * список, но и `hasLoadedOnce` (Б-2/С-1): следующий {@link refreshFirst}
   * обязан вести себя как самая первая загрузка (взять `nextCursor` из
   * ответа), а не как сверка с пустым списком, которая иначе потеряла бы
   * курсор полной первой страницы (тест-сценарий R4 ревью).
   */
  async function clearHistory(): Promise<void> {
    try {
      await invoke<void>(CLEAR_HISTORY_COMMAND)
      entries.value = []
      nextCursor.value = undefined
      showInFolderErrors.value = {}
      commandError.value = undefined
      hasLoadedOnce = false
    } catch (err) {
      const failure = toHistoryCommandFailure(err)
      if (failure.kind === 'unavailable') {
        availability.value = { reason: failure.reason, message: failure.message }
        return
      }
      console.error('clear_history rejected', err)
      commandError.value = failure
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
   * `unknownRecord` (С-3) не советует «обновить историю» руками — сам
   * перезапрашивает первую страницу и оставляет на строке нейтральный факт
   * `goneFromHistory` (текст — `HISTORY_ROW_GONE_TEXT` в
   * `historyShowInFolderTexts.ts`); `unavailable` — та же блокировка
   * экрана, что у `delete_history_record`/`clear_history` выше.
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
      if (failure.kind === 'unavailable') {
        availability.value = { reason: failure.reason, message: failure.message }
        return
      }
      if (failure.kind === 'unknownRecord') {
        showInFolderErrors.value = { ...showInFolderErrors.value, [entry.id]: { kind: 'goneFromHistory' } }
        void refreshFirst()
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
    ipcFailure,
    loaded,
    isLoadingMore,
    commandError,
    showInFolderErrors,
    liveAnnouncement,
    initialize,
    refreshFirst,
    loadMore,
    dismissNotice,
    deleteRecord,
    clearHistory,
    dismissCommandError,
    showInFolder,
  }
})
