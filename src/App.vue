<script setup lang="ts">
/**
 * Служебный экран проверки sidecar (Ф-9, Н-6, TL-8), предварённый экраном
 * подготовки yt-dlp при первом запуске (TL-17).
 *
 * # Порядок вызовов — критично (см. ревью TL-12/TL-17, #18)
 *
 * `check_sidecar` не вызывается, пока не разрешился промис `prepare_ytdlp`
 * — ни при каком сценарии, включая повторную проверку по кнопке. Во время
 * подготовки резолв пути к yt-dlp честно возвращает `notFound`, и
 * `check_sidecar` показал бы «Не нашли файл yt-dlp по ожидаемому пути» —
 * ложное утверждение при совершенно нормальном первом запуске. Поэтому обе
 * команды идут строго последовательно в {@link runPrepareAndCheckSidecar}
 * — единственном месте, откуда вообще вызывается `check()`, — а не
 * параллельно с гонкой на отрисовку.
 *
 * # Поднятие экрана подготовки
 *
 * Экран поднимается по приходу первого события `ytdlp://prepare`
 * (`stage` становится `unpacking` или `warmingUp`), а не по факту вызова
 * `prepare_ytdlp` — на тёплом запуске (обычный случай) событий нет вовсе,
 * и промис резолвится за доли секунды. Пока подготовка идёт, но событий
 * ещё не было (тёплый запуск целиком, либо сверхкороткое окно до первого
 * события на холодном), экран не «Запускаем…», а сразу тот же служебный
 * экран, что и после готовности: шапка с версией и обе строки sidecar в
 * состоянии «Проверяем…» — это устраивает и критерий приёмки 2 (служебный
 * экран сразу), и исходный дизайн E1 (Ф-9, Н-6, обе строки к t ≤ 3 с), и
 * не требует четвёртой раскладки только ради доли секунды ожидания.
 */
import { storeToRefs } from 'pinia'
import { computed, nextTick, onMounted, ref, watch } from 'vue'

import ExitConfirmDialog from '@/components/ExitConfirmDialog.vue'
import HistoryScreen from '@/components/HistoryScreen.vue'
import ProbeSection from '@/components/ProbeSection.vue'
import QueueSection from '@/components/QueueSection.vue'
import SettingsScreen from '@/components/SettingsScreen.vue'
import SidecarStatusRow from '@/components/SidecarStatusRow.vue'
import YtDlpPrepareError from '@/components/YtDlpPrepareError.vue'
import YtDlpPrepareScreen from '@/components/YtDlpPrepareScreen.vue'
import YtDlpUpdateBlock from '@/components/YtDlpUpdateBlock.vue'
import { useExitConfirmation } from '@/composables/useExitConfirmation'
import { useSidecarCheck } from '@/composables/useSidecarCheck'
import { useYtDlpPrepare } from '@/composables/useYtDlpPrepare'
import { useYtDlpUpdate } from '@/composables/useYtDlpUpdate'
import { useDownloadTaskStore } from '@/stores/downloadTask'
import type { DownloadPhase } from '@/types/generated/download'
import type { QualitySize, QualityStreams } from '@/types/generated/probe'
import type { SelectedQuality } from '@/types/generated/queue'
import { assertNever } from '@/utils/assertNever'
import { isTerminalQueuePhase } from '@/utils/queueTaskPhase'
import { toDownloadProgress } from '@/utils/queueTaskProgress'
import { formatTaskDisplayTitle } from '@/utils/queueTaskTitle'
import {
  type ActiveQueueTaskPhase,
  getActiveQueueStatusText,
  getStatusRowWaitingText,
  STATUS_ROW_YT_DLP_UPDATE_PAUSE_TEXT,
} from '@/utils/queueTexts'

// Версия приложения известна локально и не зависит от sidecar (дизайн E1,
// «Компоновка»). Держим в синхроне с `package.json` вручную — единственное
// поле, дублировать которое через JSON-импорт ради одной строки избыточно.
const APP_VERSION = '0.1.0'

const {
  stage,
  percent,
  etaSecs,
  error: prepareError,
  isPending: isPreparing,
  prepare,
} = useYtDlpPrepare()

const { report, isLoading, check } = useSidecarCheck()

/**
 * Блок «Обновление yt-dlp» (Ф-10, TL-59, дизайн E6) — независимый
 * композабл, монтируется наравне с проверкой sidecar, а не внутри неё:
 * контур обновления живёт своей жизнью в ядре и не ждёт исхода
 * `check_sidecar` (дизайн, «Насколько тихо — конкретно» — событие
 * `ytdlp://update` эмитится независимо от того, открыт ли служебный
 * экран). Активная версия для текста блока берётся из уже выполненной
 * проверки sidecar (`report.ytDlp.version`), второй раз не запрашивается
 * (дизайн, «Данные для UI»).
 */
const {
  snapshot: ytDlpUpdateSnapshot,
  checkNow: checkYtDlpUpdateNow,
  rollback: rollBackYtDlpUpdate,
} = useYtDlpUpdate()

/**
 * `stage` (composable) уже не бывает терминальным (`ready`/`failed`
 * игнорируются на уровне `useYtDlpPrepare`) — значит, «идёт подготовка» и
 * «есть что показать на экране подготовки» совпадают: не нужно отдельно
 * держать в уме, что означает `stage.value` после разрешения промиса.
 */
const showPrepareScreen = computed(() => isPreparing.value && stage.value !== undefined)
const showPrepareError = computed(() => prepareError.value !== undefined)

/**
 * Кнопка «Повторить проверку» — одна на весь экран (Ф-9 — одна команда на
 * оба бинарника сразу). Показывается тогда и только тогда, когда отчёт уже
 * пришёл и хотя бы одна из строк не в состоянии «в порядке». Отчёт
 * появляется только после того, как `prepare_ytdlp` уже разрешился (см.
 * {@link runPrepareAndCheckSidecar}), так что достижимость кнопки не нужно
 * охранять отдельно.
 */
const showRetry = computed(() => {
  const r = report.value
  if (!r) return false
  return r.ytDlp.status !== 'ok' || r.ffmpeg.status !== 'ok'
})

/**
 * Гейт поля ссылки (эпик E2, TL-33) — зависит только от статуса **yt-dlp**
 * (дизайн, «Где живёт поле ссылки»): статус ffmpeg его не блокирует,
 * разбор ролика ffmpeg не использует (он нужен только в E3, для склейки).
 */
const ytDlpState = computed<'checking' | 'blocked' | 'ready'>(() => {
  if (isLoading.value || !report.value) return 'checking'
  return report.value.ytDlp.status === 'ok' ? 'ready' : 'blocked'
})

/** Готовит yt-dlp и, только по успешному разрешению, проверяет оба sidecar. */
async function runPrepareAndCheckSidecar(): Promise<void> {
  await prepare()
  if (!prepareError.value) {
    await check()
  }
}

onMounted(() => {
  void runPrepareAndCheckSidecar()
})

/**
 * Секция «Очередь загрузок» (эпик E4, TL-75, сменила «Текущую загрузку»
 * эпика E3/TL-45). Стор — единственный хозяин состояния очереди; `App.vue`
 * лишь связывает клик «Скачать» на карточке с постановкой задачи и
 * прокидывает пропсы секции, ничего не решая сам.
 *
 * `initialize()` — разовый снимок `queue_state` плюс подписка на
 * `queue://changed` (С-9), тем же приёмом, что `runPrepareAndCheckSidecar`
 * выше и `check_sidecar` эпика E1: вызывается явно из `onMounted`, а не
 * автоматически при первом обращении к стору — иначе `queue_state`
 * потребовался бы каждому потребителю стора (например,
 * `useExitConfirmation.ts`), а не только этому экрану.
 */
const downloadTaskStore = useDownloadTaskStore()

onMounted(() => {
  void downloadTaskStore.initialize()
})

/**
 * Диалог подтверждения выхода (Р-2, эпик E3, TL-46) — подписывается на
 * попытку закрытия окна независимо от того, какой экран сейчас показан
 * (служебный экран E1, разбор E2, панель загрузки), поэтому вызывается
 * на верхнем уровне `App.vue`, а не внутри одной из веток `v-else`.
 * Оконное событие подключено к настоящему Tauri API (TL-47/#49,
 * `core:window:allow-destroy`) — реализация целиком в
 * `src/composables/windowExitPort.ts`, здесь используется только через
 * интерфейс `WindowExitPort`.
 *
 * Читает срез всей очереди (эпик E4, TL-76): `activeTask`/`pauseReason`/
 * `waitingCount` — не одну задачу, как было до TL-76 (doc
 * `useExitConfirmation.ts`, «Полный срез очереди»).
 */
const {
  visible: showExitConfirm,
  activeTask: exitConfirmActiveTask,
  pauseReason: exitConfirmPauseReason,
  waitingCount: exitConfirmWaitingCount,
  stay: onExitStay,
  exitAnyway: onExitAnyway,
} = useExitConfirmation()
const {
  tasks: queueTasks,
  awaitingContinue,
  pauseReason,
  softStallSeconds,
  commandError: downloadCommandError,
  outcomeAnnouncement,
} = storeToRefs(downloadTaskStore)

function onDownloadRequested(payload: {
  url: string
  title: string
  streams: QualityStreams
  size: QualitySize
  quality: SelectedQuality
}): void {
  void downloadTaskStore.start({
    url: payload.url,
    title: payload.title,
    streams: payload.streams,
    size: payload.size,
    quality: payload.quality,
  })
}

/**
 * Навигация «Главный/История/Настройки» (Ф-16, TL-92, дизайн E5
 * «Навигация») — локальный `ref`, без `vue-router` (запрещён требованием
 * буквально). Три ветки одного `v-show` в шаблоне ниже, а не `v-if`:
 * `ProbeSection` держит собственное состояние поля ссылки и разбора
 * (`useLinkProbe`, эпик E2) в своём `<script setup>`, и `v-if` снёс бы
 * его при каждом уходе на «Историю»/«Настройки» и создал заново с нуля —
 * ровно то разрушение состояния, которого нет права быть по критерию
 * К-14 «без потери состояния». Сам стор очереди (`downloadTaskStore`) и
 * его подписки на события живут на верхнем уровне этого `<script setup>`
 * независимо от того, что сейчас показано — переключение вкладки их не
 * касается вовсе, `v-show`/`v-if` здесь ничего не меняет.
 */
type TabId = 'main' | 'history' | 'settings'

const TABS: { id: TabId; label: string }[] = [
  { id: 'main', label: 'Главный' },
  { id: 'history', label: 'История' },
  { id: 'settings', label: 'Настройки' },
]

const activeTab = ref<TabId>('main')

const mainHeadingEl = ref<HTMLHeadingElement | null>(null)
const historyHeadingEl = ref<HTMLHeadingElement | null>(null)
const settingsHeadingEl = ref<HTMLHeadingElement | null>(null)
// Контейнер `tablist` (правки ревью TL-92, Б-1) — нужен, чтобы найти DOM-узел
// только что выбранной кнопки-вкладки после клавиатурной активации (см.
// {@link activateTabFromKeyboard}); заголовки панелей уже держат
// собственные ref-ы выше, кнопкам вкладок отдельные ref-ы заводить незачем
// — один запрос по `id` внутри уже известного контейнера дешевле пяти
// новых переменных.
const tablistEl = ref<HTMLDivElement | null>(null)
// Кнопка «На главный» строки состояния (правки ревью TL-92, Н-3) — нужна
// только чтобы определить, что фокус в момент исчезновения строки стоял
// именно на ней (см. watcher `showQueueStatusRow` ниже).
const backToMainButtonEl = ref<HTMLButtonElement | null>(null)

function headingElFor(tab: TabId): HTMLHeadingElement | null {
  switch (tab) {
    case 'main':
      return mainHeadingEl.value
    case 'history':
      return historyHeadingEl.value
    case 'settings':
      return settingsHeadingEl.value
    default:
      return assertNever(tab)
  }
}

function tabButtonElFor(tab: TabId): HTMLButtonElement | null {
  return tablistEl.value?.querySelector<HTMLButtonElement>(`#tab-${tab}`) ?? null
}

/**
 * Переключение вкладки кликом мыши по кнопке и кнопкой «На главный» строки
 * состояния (дизайн «Фокус при переключении экрана»): `aria-selected`/
 * видимость панелей меняются синхронно с `activeTab`, а сразу после —
 * фокус программно переводится на `<h2 tabindex="-1">` в начале новой
 * панели (у «Главного» — уже существующий `<h1>`).
 *
 * Повторный клик по уже активной вкладке фокус не трогает вовсе (Н-4,
 * правки ревью TL-92, второй раунд) — без этой защиты клик по вкладке, на
 * которой пользователь и так стоит, отбирал бы фокус у элемента, который
 * он только что нажал, и передавал его заголовку без единого настоящего
 * переключения.
 *
 * Клавиатурная активация стрелками/Home/End идёт **не** через эту
 * функцию — см. {@link activateTabFromKeyboard} и его doc-комментарий о
 * том, почему автоматическая активация обязана переводить фокус на новую
 * кнопку-вкладку, а не на заголовок панели.
 */
async function selectTab(tab: TabId): Promise<void> {
  if (activeTab.value === tab) return
  activeTab.value = tab
  await nextTick()
  headingElFor(tab)?.focus()
}

/**
 * Клавиатурная активация `tablist` (Б-1, правки ревью TL-92, второй
 * раунд) — стандартный паттерн ARIA tabs «автоматическая активация»:
 * стрелка/Home/End сразу меняют `activeTab` **и** переводят фокус на саму
 * новую кнопку-вкладку, а не на заголовок панели, как раньше.
 *
 * Первая версия (см. историю файла) переводила фокус на заголовок панели
 * при любом переключении, включая клавиатурное, — и следующее нажатие
 * стрелки било мимо: `keydown` висит на самом `tablist`
 * ({@link handleTablistKeydown}), а заголовок панели вне `tablist`, и
 * событие с него до контейнера не всплывает. Держать фокус внутри
 * `tablist` (на кнопке) — единственный способ, чтобы второе, третье и
 * последующие нажатия стрелки подряд продолжали доходить до обработчика.
 */
function activateTabFromKeyboard(tab: TabId): void {
  activeTab.value = tab
  void nextTick().then(() => {
    tabButtonElFor(tab)?.focus()
  })
}

/**
 * Клавиатура `tablist` (дизайн «Навигация», практика ARIA tabs):
 * ArrowLeft/ArrowRight циклически двигают выбор, Home/End — к первой/
 * последней вкладке; активация — сразу по нажатию, без отдельного шага
 * подтверждения (Enter/Space), см. doc {@link activateTabFromKeyboard}
 * про то, куда при этом уходит фокус.
 *
 * ArrowUp/ArrowDown осознанно не обрабатываются и не глушатся (Н-4,
 * правки ревью TL-92, второй раунд): панель вкладок горизонтальная
 * (`aria-orientation` по умолчанию, отдельно не выставлен), а
 * ArrowUp/ArrowDown в браузере — это прокрутка страницы; отбирать её
 * компоненту, для которого эти клавиши не значат ничего по паттерну ARIA
 * tabs, не за чем.
 */
function handleTablistKeydown(event: KeyboardEvent): void {
  const ids = TABS.map((t) => t.id)
  const currentIndex = ids.indexOf(activeTab.value)
  let nextIndex: number
  switch (event.key) {
    case 'ArrowRight':
      nextIndex = (currentIndex + 1) % ids.length
      break
    case 'ArrowLeft':
      nextIndex = (currentIndex - 1 + ids.length) % ids.length
      break
    case 'Home':
      nextIndex = 0
      break
    case 'End':
      nextIndex = ids.length - 1
      break
    default:
      return
  }
  event.preventDefault()
  activateTabFromKeyboard(ids[nextIndex]!)
}

/**
 * Три фазы, которые могут принадлежать активной задаче строки состояния
 * (не `queued` — она ещё ждёт, не терминальная — она уже не «происходит»,
 * см. doc {@link ActiveQueueTaskPhase} в `queueTexts.ts`). `assertNever`
 * в `default` держит границу с контрактом (`DownloadPhase`, TL-51/TL-70):
 * восьмая фаза не даёт этой функции тихо остаться в `undefined`, а роняет
 * `npm run type-check` на этой строке.
 */
function toActiveQueueTaskPhase(phase: DownloadPhase): ActiveQueueTaskPhase | undefined {
  switch (phase) {
    case 'fetching':
      return 'fetching'
    case 'downloading':
      return 'downloading'
    case 'merging':
      return 'merging'
    case 'queued':
    case 'done':
    case 'failed':
    case 'cancelled':
      return undefined
    default:
      return assertNever(phase)
  }
}

/**
 * Текст строки состояния (дизайн «Навигация»): активная задача — по
 * приоритету первая, пауза на обновление yt-dlp — вторая, восстановленная
 * после перезапуска очередь — третья; иначе строки нет вовсе. Читает
 * только уже существующий полный срез очереди (`tasks`/`awaitingContinue`/
 * `pauseReason` — не временные геттеры `task`/`progress`/`isActive`,
 * помеченные к удалению в #86/#89, doc-комментарий `useDownloadTaskStore`,
 * «Обратная совместимость»), никакого нового опроса не заводит.
 */
const activeQueueStatusText = computed<string | undefined>(() => {
  for (const t of queueTasks.value) {
    const phase = toActiveQueueTaskPhase(t.phase)
    if (phase === undefined) continue
    const progress = toDownloadProgress(t)
    const percent = progress.phase === 'downloading' ? progress.percent : undefined
    return getActiveQueueStatusText(formatTaskDisplayTitle(t.title, t.quality), phase, percent)
  }
  return undefined
})

/**
 * Текст паузы на обновление yt-dlp — **свой** для строки состояния, не
 * `YT_DLP_UPDATE_PAUSE_TEXT` (правки ревью TL-92, С-3): та константа несёт
 * хвост «— обычно занимает меньше минуты», нужный полной секции «Очередь
 * загрузок» (`QueueSection.vue`) и диалогу выхода, но лишний в компактной
 * однострочной сводке дизайна E5 «Навигация» — макет называет ровно
 * «Между загрузками устанавливается обновлённый yt-dlp», без второго
 * предложения.
 */
const queueStatusText = computed<string | undefined>(() => {
  if (activeQueueStatusText.value !== undefined) return activeQueueStatusText.value
  if (pauseReason.value === 'ytDlpUpdate') return STATUS_ROW_YT_DLP_UPDATE_PAUSE_TEXT
  if (awaitingContinue.value) {
    const waitingCount = queueTasks.value.filter((t) => !isTerminalQueuePhase(t.phase)).length
    if (waitingCount > 0) return getStatusRowWaitingText(waitingCount)
  }
  return undefined
})

/**
 * Видна только на «Истории»/«Настройках» (дизайн: на «Главном» её роль и
 * так играет полная секция «Очередь загрузок» — своя строка там была бы
 * вторым источником того же самого).
 */
const showQueueStatusRow = computed(() => activeTab.value !== 'main' && queueStatusText.value !== undefined)

/**
 * Н-3 (правки ревью TL-92, второй раунд): если фокус стоял на кнопке «На
 * главный» в момент, когда строка состояния пропадает (задача завершилась,
 * пауза кончилась), `v-if` убирает саму кнопку вместе со строкой — без
 * этого наблюдателя фокус молча падает на `<body>`. `watch` по умолчанию
 * выполняется до патча DOM (`flush: 'pre'`), поэтому в колбэке
 * `backToMainButtonEl.value` — это ещё старый, ещё не удалённый узел, и
 * сравнение с `document.activeElement` застаёт фокус на месте; после
 * `nextTick()` (узел уже удалён) фокус переводится на заголовок текущей
 * панели — ту же цель, что и у обычного переключения вкладки.
 */
watch(showQueueStatusRow, (visible, wasVisible) => {
  if (visible || !wasVisible) return
  if (document.activeElement !== backToMainButtonEl.value) return
  void nextTick().then(() => {
    headingElFor(activeTab.value)?.focus()
  })
})

/**
 * Живая зона исходов на «Истории»/«Настройках» (TL-98, issue #105) —
 * терминальный исход задачи, которая была активной, и отказ команды
 * постановки: строка статуса (`.queue-status-announcer` выше) молчит о
 * них, потому что зеркалит только *текущее* состояние очереди и уже
 * поменялась/пропала к тому моменту, когда пользователь мог бы это
 * услышать (doc `showQueueStatusRow`, «на Истории/Настройках»).
 *
 * # Почему не читает `outcomeAnnouncement` напрямую в шаблоне
 *
 * **Молчание на «Главном» — решается здесь, не в сторе, и на записи, не
 * на чтении.** Стор ничего не знает про вкладки (домен) и порождает факт
 * при любом исходе независимо от того, что сейчас видно; здесь этот факт
 * просто отбрасывается сразу, если `activeTab === 'main'` в момент
 * прихода события — тот же приём, что Б-3 у `queueStatusText`/
 * `showQueueStatusRow` (правки ревью TL-92, третий раунд), но с гейтом на
 * **записи**, а не на каждом чтении/рендере: гейт на чтении (например,
 * `computed(() => activeTab.value === 'main' ? '' : outcomeAnnouncement.value?.text)`)
 * пересчитывался бы при каждом переключении вкладки и показал бы задним
 * числом исход, случившийся, пока пользователь был на «Главном» и уже
 * видел его через `DownloadPanel`/`DownloadCommandErrorBlock`, — при
 * возврате с «Главного» на «Историю» текст просто появился бы снова, хотя
 * никакого нового события не было.
 *
 * # Переозвучка одинакового текста подряд
 *
 * Скринридер не обязан заново озвучить `aria-live`-зону, если её
 * текстовое содержимое не изменилось буквально (тот же исход у двух
 * разных задач с одинаковым названием подряд — обычный случай, не край).
 * `:key="outcomeAnnouncementKey"` на самом узле — при каждом новом
 * объявлении ключ меняется, и Vue пересоздаёт `<p>` целиком (удаляет
 * старый узел, вставляет новый с уже готовым текстом), а не переиспользует
 * прежний элемент с обновлённым `textContent`. Для дерева доступности это
 * не правка текста уже известного узла, а появление нового — не зависит
 * от того, успевает ли конкретный AT заметить промежуточное пустое
 * состояние при обычной мутации текста (приём «пересоздать узел», а не
 * «очистить и переписать текст», не измерялся живым скринридером в этом
 * проекте — предпочтён как не зависящий от гонки между двумя правками
 * одного узла, doc-класс `App.tabs.test.ts`, «живая зона исходов»,
 * проверяет ровно замену DOM-узла, а не предположение о поведении AT).
 */
const outcomeAnnouncementText = ref('')
const outcomeAnnouncementKey = ref(0)

watch(outcomeAnnouncement, (announcement) => {
  if (!announcement) return
  if (activeTab.value === 'main') return
  outcomeAnnouncementText.value = announcement.text
  outcomeAnnouncementKey.value += 1
})
</script>

<template>
  <ExitConfirmDialog
    v-if="showExitConfirm"
    :active-task="exitConfirmActiveTask"
    :pause-reason="exitConfirmPauseReason"
    :waiting-count="exitConfirmWaitingCount"
    @stay="onExitStay"
    @exit-anyway="onExitAnyway"
  />

  <main class="screen">
    <header>
      <!--
        `tabindex="-1"` (TL-92, дизайн E5 «Фокус при переключении экрана»):
        цель программного фокуса при возврате на «Главный» с «Истории»/
        «Настроек» — заголовок не входит в обычный порядок обхода Tab,
        доступен только через `selectTab('main')`.
      -->
      <h1
        ref="mainHeadingEl"
        tabindex="-1"
      >
        tube-leak
      </h1>
      <!--
        Версия — известна локально, sidecar не нужен, дизайн E1 держит её
        видимой к t ≤ 3 с независимо от того, идёт ли ещё 40-секундная
        подготовка (ревью TL-17, #18): раньше пряталась на время экрана
        подготовки — больше не прячется.
      -->
      <p class="version">
        версия {{ APP_VERSION }}
      </p>
    </header>

    <!--
      Панель вкладок (Ф-16, TL-92, дизайн E5 «Навигация») — самый верхний
      уровень разметки, рендерится безусловно: «Главный» ниже показывает
      ровно то же самое, что показывал бы без вкладок вообще (экран
      подготовки yt-dlp, его ошибку либо обычное содержимое E1–E4, без
      изменений внутри), а «История»/«Настройки» не зависят от готовности
      sidecar вовсе.
    -->
    <!--
      Одна линия под панелью вкладок, не две (Н-1, правки ревью TL-92,
      второй раунд): раньше нижняя граница `.tabs` (акцент под выбранной
      вкладкой плюс серая полоса на всю ширину контейнера) шла вместе с
      безусловным `<hr class="screen__divider">` сразу следом — макет
      рисует ровно одну черту здесь. Граница у `.tabs` и остаётся
      единственной линией; второй `<hr>` (ниже, перед содержимым панели)
      условный и по-прежнему появляется только вместе со строкой
      состояния — это отдельная, вторая черта макета, а не дубль первой.
    -->
    <div
      ref="tablistEl"
      class="tabs"
      role="tablist"
      aria-label="Разделы приложения"
      @keydown="handleTablistKeydown"
    >
      <button
        v-for="tab in TABS"
        :id="`tab-${tab.id}`"
        :key="tab.id"
        type="button"
        role="tab"
        class="tabs__tab tap-target"
        :aria-selected="activeTab === tab.id"
        :aria-controls="`tabpanel-${tab.id}`"
        :tabindex="activeTab === tab.id ? 0 : -1"
        @click="selectTab(tab.id)"
      >
        {{ tab.label }}
      </button>
    </div>

    <!--
      Живая зона строки состояния — постоянный контейнер, не `v-if` (Н-3,
      правки ревью TL-92, второй раунд, тот же приём, что
      `probe-section__inline-error` в `ProbeSection.vue`): если создавать
      элемент с `aria-live` только в момент появления текста, скринридер
      не видит самого узла заранее и первое объявление теряется — нет
      наблюдаемого изменения внутри уже зарегистрированной живой зоны,
      есть только появление нового узла с текстом сразу внутри. Визуально
      скрыт (`.visually-hidden`, не `display: none` — иначе AT его тоже
      не видит), декоративный «●» сюда не входит: это чисто текстовая
      копия для скринридера, видимая строка ниже несёт тот же текст для
      зрячих пользователей.

      Текст зоны зависит от `showQueueStatusRow`, не только от
      `queueStatusText` (Б-3, правки ревью TL-92, третий раунд):
      `queueStatusText` вычисляется и на «Главном» (там прогресс несёт
      `DownloadPanel`/`QueueSection` своими средствами, см. комментарий в
      `DownloadPanel.vue`), и раньше эта зона озвучивала там каждый процент
      второй раз — регрессия к тому, что ревью TL-45 сознательно убрало.
      На «Истории»/«Настройках» `showQueueStatusRow` совпадает с наличием
      текста, так что там поведение не меняется.
    -->
    <p
      class="visually-hidden queue-status-announcer"
      aria-live="polite"
    >
      {{ showQueueStatusRow ? queueStatusText : '' }}
    </p>

    <!--
      Живая зона исходов (TL-98, issue #105, doc-комментарий
      `outcomeAnnouncementText` в `<script setup>`) — постоянный узел,
      как и `.queue-status-announcer` выше (не `v-if`, по той же причине:
      скринридер должен знать о зоне заранее, до первого текста в ней).
      Собственный узел, не переиспользование `.queue-status-announcer`:
      та зона зеркалит текущее состояние и вправе перезаписываться сколь
      угодно часто, а это — разовое событие, которое обязано пережить
      следующую же смену текста статусной строки (issue #105, «commit,
      затем pump»).
    -->
    <p
      :key="outcomeAnnouncementKey"
      class="visually-hidden queue-outcome-announcer"
      aria-live="polite"
    >
      {{ outcomeAnnouncementText }}
    </p>

    <!--
      Компактная строка состояния очереди (дизайн «Навигация») — видна
      только на «Истории»/«Настройках», пока где-то реально идёт или ждёт
      загрузка: на «Главном» её роль и так играет полная секция «Очередь
      загрузок» ниже. Кнопка «На главный» просто переключает вкладку, не
      эмитит никаких команд. Без собственного `aria-live` (правки ревью
      TL-92, Н-3) — живая зона теперь только у постоянного узла выше,
      второй `aria-live` на том же тексте озвучил бы его дважды.
    -->
    <p
      v-if="showQueueStatusRow"
      class="queue-status-row"
    >
      <span class="queue-status-row__text">
        <span
          v-if="activeQueueStatusText !== undefined"
          class="queue-status-row__bullet"
          aria-hidden="true"
        >●</span>
        {{ queueStatusText }}
      </span>
      <button
        ref="backToMainButtonEl"
        type="button"
        class="tap-target"
        @click="selectTab('main')"
      >
        На главный
      </button>
    </p>

    <hr
      v-if="showQueueStatusRow"
      class="screen__divider"
    >

    <!--
      `v-show`, не `v-if`, на всех трёх панелях (doc-комментарий `activeTab`
      в `<script setup>`, К-14): переключение вкладки не должно
      размонтировать `ProbeSection`/`QueueSection` и терять их состояние.
    -->
    <section
      v-show="activeTab === 'main'"
      id="tabpanel-main"
      role="tabpanel"
      aria-labelledby="tab-main"
    >
      <YtDlpPrepareScreen
        v-if="showPrepareScreen"
        :stage="stage!"
        :percent="percent"
        :eta-secs="etaSecs"
      />

      <YtDlpPrepareError
        v-else-if="showPrepareError"
        :error="prepareError!"
        @retry="runPrepareAndCheckSidecar"
      />

      <template v-else>
        <section
          aria-live="polite"
          :aria-busy="isLoading"
        >
          <SidecarStatusRow
            fallback-name="yt-dlp"
            :result="report?.ytDlp"
          />
          <SidecarStatusRow
            fallback-name="ffmpeg"
            :result="report?.ffmpeg"
          />
        </section>

        <footer
          v-if="showRetry"
          class="screen__footer"
        >
          <button
            type="button"
            class="tap-target"
            :disabled="isLoading"
            @click="check"
          >
            {{ isLoading ? 'Проверяем…' : 'Повторить проверку' }}
          </button>
        </footer>

        <!--
          Блок «Обновление yt-dlp» (Ф-10, TL-59/TL-60, дизайн E6) — между
          строками SidecarStatusRow и разделителем перед полем ссылки
          (дизайн, «Где живёт блок»). Кнопка «Вернуться к …» и инлайн-
          подтверждение рисуются самим блоком по снимку контура; вызов
          самой команды отката (Р-3) — здесь, тем же приёмом, что «Проверить
          сейчас»/`checkYtDlpUpdateNow`.
        -->
        <YtDlpUpdateBlock
          :snapshot="ytDlpUpdateSnapshot"
          :active-version="report?.ytDlp.version"
          @check="checkYtDlpUpdateNow"
          @rollback="rollBackYtDlpUpdate"
        />

        <!--
          Разделитель — единственное, что явно отделяет «служебную» часть
          экрана (E1, про инструменты) от «рабочей» (про конкретный ролик,
          E2), чтобы ошибка ffmpeg выше не путалась с состоянием разбора
          ниже (дизайн E2, «Где живёт поле ссылки»).
        -->
        <hr class="screen__divider">

        <ProbeSection
          :yt-dlp-state="ytDlpState"
          @download="onDownloadRequested"
        />

        <!--
          Секция «Очередь загрузок» (дизайн E4) — рендерится тогда и только
          тогда, когда есть хоть одна задача **или** есть отказ команды
          управления загрузкой/очередью, который ещё не скрыт (ревью TL-45,
          «Достижимый путь к молчаливому отказу», унаследовано TL-75): отказ
          `start_download`/`dismiss_queue_task` и т.п. возможен и без
          существующей задачи в списке. Пока ни того, ни другого нет, макет
          не резервирует под секцию пустое место (дизайн E3, «Где живёт
          задача экрана», унаследовано дизайном E4). Условие продублировано
          здесь (а не только внутри `QueueSection`) ради разделителя —
          `<hr>` не должен появляться перед пустой секцией.

          Без собственного aria-live на разделителе секции (ревью TL-45,
          «Заметки»): `DownloadCommandErrorBlock` несёт role="alert",
          `DownloadPanel` — свою единственную живую зону для нетерминальных
          фаз и role="status" для терминальных, `QueueWaitingRow` — свою.
        -->
        <template v-if="queueTasks.length > 0 || downloadCommandError">
          <hr class="screen__divider">

          <QueueSection
            :tasks="queueTasks"
            :awaiting-continue="awaitingContinue"
            :pause-reason="pauseReason"
            :command-error="downloadCommandError"
            :soft-stall-seconds="softStallSeconds"
            @cancel="downloadTaskStore.cancel"
            @retry="downloadTaskStore.retry"
            @hide="downloadTaskStore.hide"
            @hide-all-terminal="downloadTaskStore.hideAllTerminal"
            @resume="downloadTaskStore.resume"
            @dismiss-command-error="downloadTaskStore.dismissCommandError"
          />
        </template>
      </template>
    </section>

    <!--
      Экран истории (TL-93, Ф-4…Ф-6, Ф-8, С-1…С-4, С-10) — заголовок и его
      фокус-цель остаются здесь (К-14), содержимое ниже целиком несёт
      `HistoryScreen`; не зависит от готовности sidecar (дизайн «Навигация»).

      Правки ревью TL-93 (второй раунд): `:active` сообщает `HistoryScreen`
      о собственной видимости (решение ведущего — перезапрашивать первую
      страницу и при активации вкладки, не только при монтировании и
      `queue://changed`; doc-класс `HistoryScreen.vue`), а
      `@request-heading-focus` — обратная связь для С-7 («Удалить»
      последней строки/подтверждённая «Очистить» переводят фокус на этот
      же `<h2>`, которым `HistoryScreen` не владеет).
    -->
    <section
      v-show="activeTab === 'history'"
      id="tabpanel-history"
      role="tabpanel"
      aria-labelledby="tab-history"
    >
      <h2
        ref="historyHeadingEl"
        tabindex="-1"
      >
        История
      </h2>
      <HistoryScreen
        :active="activeTab === 'history'"
        @request-heading-focus="historyHeadingEl?.focus()"
      />
    </section>

    <!--
      Экран настроек (TL-94, Ф-9…Ф-13, С-5…С-9) — заголовок и его
      фокус-цель остаются здесь (К-14), содержимое ниже целиком несёт
      `SettingsScreen`; не зависит от готовности sidecar (дизайн «Навигация»).
    -->
    <section
      v-show="activeTab === 'settings'"
      id="tabpanel-settings"
      role="tabpanel"
      aria-labelledby="tab-settings"
    >
      <h2
        ref="settingsHeadingEl"
        tabindex="-1"
      >
        Настройки
      </h2>
      <SettingsScreen :active="activeTab === 'settings'" />
    </section>
  </main>
</template>

<style scoped>
/*
 * Пересмотр ограничения E1 «без прокрутки» (дизайн E2): контентная область
 * (всё, что ниже шапки) может не поместиться по высоте с карточкой,
 * превью и лестницей до пяти строк. Шапка не закреплена принудительно —
 * никакого `position: sticky`/фиксированной высоты здесь нет, страница
 * прокручивается штатно средствами браузера/webview без дополнительной
 * разметки.
 */
.screen {
  max-width: 40rem;
  margin: 0 auto;
  padding: 1.5rem;
  font-family: system-ui, -apple-system, sans-serif;
}

.screen__divider {
  margin: 1.5rem 0;
  border: none;
  border-top: 1px solid var(--color-border);
}

.version {
  margin: 0 0 1rem;
  color: var(--color-text-muted);
}

.screen__footer {
  margin-top: 1rem;
}

.tap-target {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 40px;
  min-height: 40px;
  padding: 0.5rem 1rem;
  box-sizing: border-box;
}

.tap-target:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}

/*
 * Панель вкладок (TL-92) — выбранная вкладка отмечена нижней границей
 * акцентом: не текст, а декоративная граница элемента управления (тот же
 * разрешённый случай использования `--color-accent`, что и обводка
 * фокуса/левая граница активной задачи, см. doc-комментарий
 * `src/style.css`, «Акцент — один и тот же оттенок в обеих темах»).
 */
.tabs {
  display: flex;
  gap: 0.25rem;
  margin-top: 1rem;
  /*
   * Раньше нижний отступ давал соседний безусловный `<hr>` (его верхнее
   * поле 1.5rem) — теперь эта же линия и есть единственная черта под
   * панелью (Н-1, правки ревью TL-92, второй раунд), поэтому поле
   * переезжает сюда, чтобы раскладка ниже не сдвинулась.
   */
  margin-bottom: 1.5rem;
  border-bottom: 1px solid var(--color-border);
}

.tabs__tab {
  background: none;
  border: none;
  border-bottom: 2px solid transparent;
  border-radius: 0;
  margin-bottom: -1px;
  color: var(--color-text-secondary);
}

.tabs__tab[aria-selected='true'] {
  border-bottom-color: var(--color-accent);
  color: var(--color-text-strong);
  font-weight: 600;
}

.queue-status-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  margin: 0;
  color: var(--color-text-secondary);
}

.queue-status-row__text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* «●» перед названием активной задачи (С-3, правки ревью TL-92, второй раунд, макет дизайна «Навигация») — декоративный отступ до текста, сам знак aria-hidden в разметке. */
.queue-status-row__bullet {
  margin-right: 0.35rem;
}

/*
 * Заголовки-цели программного фокуса при переключении вкладки (дизайн
 * «Фокус при переключении экрана») — та же рамка, что у `.tap-target`,
 * чтобы фокус был заметен независимо от того, что именно его получило.
 */
h1:focus-visible,
h2:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}

/*
 * Постоянная живая зона строки состояния (Н-3, правки ревью TL-92, второй
 * раунд) — видна только скринридеру: `position: absolute` + `clip`, не
 * `display: none`/`visibility: hidden` (те убрали бы узел из дерева
 * доступности вместе с текстом). Тот же приём, что `.visually-hidden` в
 * `SidecarStatusRow.vue` — не общий класс между файлами (`<style scoped>`
 * в каждом компоненте), а не повторно используемый через дублирование.
 */
.visually-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}
</style>
