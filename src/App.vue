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
import { computed, onMounted } from 'vue'

import ExitConfirmDialog from '@/components/ExitConfirmDialog.vue'
import ProbeSection from '@/components/ProbeSection.vue'
import QueueSection from '@/components/QueueSection.vue'
import SidecarStatusRow from '@/components/SidecarStatusRow.vue'
import YtDlpPrepareError from '@/components/YtDlpPrepareError.vue'
import YtDlpPrepareScreen from '@/components/YtDlpPrepareScreen.vue'
import YtDlpUpdateBlock from '@/components/YtDlpUpdateBlock.vue'
import { useExitConfirmation } from '@/composables/useExitConfirmation'
import { useSidecarCheck } from '@/composables/useSidecarCheck'
import { useYtDlpPrepare } from '@/composables/useYtDlpPrepare'
import { useYtDlpUpdate } from '@/composables/useYtDlpUpdate'
import { useDownloadTaskStore } from '@/stores/downloadTask'
import type { QualitySize, QualityStreams } from '@/types/generated/probe'
import type { SelectedQuality } from '@/types/generated/queue'

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
const { tasks: queueTasks, awaitingContinue, pauseReason, softStallSeconds, commandError: downloadCommandError } =
  storeToRefs(downloadTaskStore)

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
      <h1>tube-leak</h1>
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
</style>
