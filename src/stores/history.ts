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
  storageFailed: true,
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

/** `true`, когда среди пометок ответа есть «база пересоздана» — см. doc «Пересоздание базы» у {@link useHistoryStore}. */
function hasBaseRecreatedNotice(notices: HistoryNotice[]): boolean {
  return notices.some((n) => n.kind === 'baseRecreated')
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
 * состояние (список/курсор/доступность); несовпадение — эта часть работы
 * отброшена.
 *
 * # Пометки сливаются из ЛЮБОГО ответа (Б-1, правки ревью TL-93, третий раунд)
 *
 * Второй раунд отбрасывал устаревший по `refreshCallId` ответ **целиком** —
 * вместе с однократной пометкой ядра, которую этот ответ несёт. Ядро шлёт
 * `queue://changed` дважды подряд на каждый `Done` при непустой очереди
 * (`commit`, затем `pump`), так что почти всегда первый из двух запущенных
 * {@link refreshFirst} успевает устареть раньше, чем пользователь видит его
 * ответ, — и пометка терялась систематически, а не изредка. Исправление:
 * {@link mergeNotices} вызывается на **каждом** успешном ответе, до всех
 * проверок `callId`/`listGeneration` ниже и независимо от их исхода. Список,
 * курсор и `availability` по-прежнему берутся только из «выигравшего» по
 * актуальности ответа — сливаются только пометки.
 *
 * # Поколения списка — loadMore/удаление/очистка не пересекаются с устаревшим ответом
 * (мелочи Б-1, правки ревью TL-93, третий раунд)
 *
 * `refreshCallId` упорядочивает только вызовы {@link refreshFirst} между
 * собой. Он не защищает от другой гонки: {@link refreshFirst} или
 * {@link loadMore} в полёте, пока пользователь успешно нажал «Удалить» или
 * «Очистить» — тогда пришедший позже ответ несёт данные **до** этого
 * изменения, и его наивное применение воскресит то, что уже убрано. Второй
 * счётчик, `listGeneration`, отслеживает именно структурные изменения
 * списка:
 *
 * - продвигается на единицу при успешном {@link clearHistory}, при успешном
 *   {@link deleteRecord} (включая тихое удаление по `unknownRecord`) и при
 *   каждом случае, когда {@link refreshFirst} **заменяет** уже показанные
 *   записи — сброс целиком, сброс к пустому списку, безусловный сброс по
 *   «база пересоздана» и замена префикса (даже когда после замены список
 *   выглядит так же, как выглядел бы без неё — дешевле отбросить лишний раз
 *   параллельный запрос, чем пропустить случай, где замена префикса и
 *   вправду отличается);
 * - не продвигается на самой первой загрузке (`!hasLoadedOnce`) — до неё
 *   `nextCursor` ещё не выдан, и {@link loadMore} физически не может быть в
 *   полёте.
 *
 * {@link refreshFirst} и {@link loadMore} запоминают текущее значение
 * `listGeneration` при старте и сверяют его перед тем, как записать
 * список/курсор в состояние; несовпадение отбрасывает именно эту часть
 * ответа (пометки из {@link refreshFirst}, как сказано выше, сливаются
 * всё равно).
 *
 * # Пересоздание базы — сброс безусловный (мелочь №3, правки ревью TL-93, третий раунд)
 *
 * `AUTOINCREMENT` новой базы начинает счёт заново, так что id страницы,
 * пришедшей после пересоздания, может случайно совпасть с id, уже видимым
 * в списке. Проверка «последний id свежей страницы есть в списке» в этом
 * случае нашла бы совпадение не по смыслу, а по совпадению — и тогда
 * список смешался бы с записями из уже несуществующей базы, а старый
 * `nextCursor` пережил бы пересоздание, которое обязано было его стереть.
 * Пометка `baseRecreated` в ответе без курсора обходит эту сверку целиком:
 * список и курсор берутся из свежей страницы безусловно, даже если
 * совпадение id формально нашлось бы.
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
  /** Сторож гонки {@link refreshFirst} против {@link refreshFirst} — см. doc-комментарий класса, «Гонка двух refreshFirst». */
  let refreshCallId = 0
  /** Сторож структурных изменений списка — см. doc-комментарий класса, «Поколения списка». */
  let listGeneration = 0

  function bumpListGeneration(): void {
    listGeneration += 1
  }

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
    const generationAtStart = listGeneration
    try {
      const page = await invoke<HistoryPage>(HISTORY_PAGE_COMMAND, { cursor: undefined })

      // Б-1 (правки ревью TL-93, третий раунд): пометки сливаются из ЛЮБОГО
      // ответа — раньше проверок ниже и независимо от их исхода. См. doc
      // класса, «Пометки сливаются из ЛЮБОГО ответа».
      notices.value = mergeNotices(notices.value, page.notices)

      if (callId !== refreshCallId) return // отброшен более новым вызовом refreshFirst, пока этот был в полёте
      availability.value = undefined
      ipcFailure.value = false

      if (listGeneration !== generationAtStart) {
        // Список успел структурно измениться («Удалить»/«Очистить») пока
        // этот ответ был в полёте — он несёт данные до этого изменения, и
        // применить их значило бы воскресить то, что пользователь только
        // что убрал (G3/G4, правки ревью TL-93, третий раунд). Пометки уже
        // слиты выше; список и курсор не трогаем.
        return
      }

      const knownIdsBefore = new Set(entries.value.map((e) => e.id))
      const isLiveUpdate = hasLoadedOnce
      const isBaseRecreated = hasBaseRecreatedNotice(page.notices)

      if (!hasLoadedOnce) {
        entries.value = page.entries
        nextCursor.value = page.nextCursor
        hasLoadedOnce = true
      } else if (isBaseRecreated) {
        // Пересоздание базы (мелочь №3, правки ревью TL-93, третий раунд):
        // сброс безусловный, без сверки со списком — см. doc класса.
        entries.value = page.entries
        nextCursor.value = page.nextCursor
        bumpListGeneration()
      } else if (page.entries.length === 0) {
        entries.value = []
        nextCursor.value = page.nextCursor
        bumpListGeneration()
      } else {
        const lastFreshId = page.entries[page.entries.length - 1]!.id
        const splitIndex = entries.value.findIndex((e) => e.id === lastFreshId)
        if (splitIndex === -1) {
          entries.value = page.entries
          nextCursor.value = page.nextCursor
          bumpListGeneration()
        } else {
          const tail = entries.value.slice(splitIndex + 1)
          entries.value = [...page.entries, ...tail]
          // `nextCursor` умышленно не трогается здесь — см. doc класса.
          bumpListGeneration()
        }
      }

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

  /**
   * «Показать ещё» (Ф-4) — курсор идёт в запрос как получен от предыдущего
   * ответа `history_page`, не собирается заново из полей записи (см.
   * doc-комментарий теста `history.test.ts`, «Курсор как есть»).
   *
   * `generationAtStart` (G2/U2, правки ревью TL-93, третий раунд) — этот
   * запрос продолжает список таким, каким он был на момент вызова; если
   * пока он был в полёте список успел структурно замениться
   * ({@link refreshFirst} со сбросом/заменой префикса, успешные
   * «Удалить»/«Очистить», см. doc класса «Поколения списка»), его хвост
   * либо задублирует уже показанные id, либо продолжит список, которого
   * больше нет, — ответ отбрасывается целиком, без записи в состояние.
   */
  async function loadMore(): Promise<void> {
    if (!nextCursor.value || isLoadingMore.value) return
    const generationAtStart = listGeneration
    isLoadingMore.value = true
    try {
      const page = await invoke<HistoryPage>(HISTORY_PAGE_COMMAND, { cursor: nextCursor.value })
      if (listGeneration !== generationAtStart) return
      entries.value = [...entries.value, ...page.entries]
      nextCursor.value = page.nextCursor
    } catch (err) {
      if (listGeneration !== generationAtStart) return
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
      bumpListGeneration()
      showInFolderErrors.value = withoutKey(showInFolderErrors.value, id)
      commandError.value = undefined
    } catch (err) {
      const failure = toHistoryCommandFailure(err)
      if (failure.kind === 'unknownRecord') {
        entries.value = entries.value.filter((e) => e.id !== id)
        bumpListGeneration()
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
      bumpListGeneration()
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
