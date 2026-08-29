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

import DownloadCommandErrorBlock from '@/components/DownloadCommandErrorBlock.vue'
import DownloadPanel from '@/components/DownloadPanel.vue'
import ExitConfirmDialog from '@/components/ExitConfirmDialog.vue'
import ProbeSection from '@/components/ProbeSection.vue'
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
 * Секция «Текущая загрузка» (эпик E3, TL-45). Стор — единственный хозяин
 * состояния задачи; `App.vue` лишь связывает клик «Скачать» на карточке со
 * стартом задачи и прокидывает пропсы панели, ничего не решая сам.
 */
const downloadTaskStore = useDownloadTaskStore()

/**
 * Диалог подтверждения выхода (Р-2, эпик E3, TL-46) — подписывается на
 * попытку закрытия окна независимо от того, какой экран сейчас показан
 * (служебный экран E1, разбор E2, панель загрузки), поэтому вызывается
 * на верхнем уровне `App.vue`, а не внутри одной из веток `v-else`.
 * Оконное событие подключено к настоящему Tauri API (TL-47/#49,
 * `core:window:allow-destroy`) — реализация целиком в
 * `src/composables/windowExitPort.ts`, здесь используется только через
 * интерфейс `WindowExitPort`.
 */
const {
  visible: showExitConfirm,
  task: exitConfirmTask,
  progress: exitConfirmProgress,
  stay: onExitStay,
  exitAnyway: onExitAnyway,
} = useExitConfirmation()
const {
  task: downloadTask,
  progress: downloadProgress,
  softStallSeconds,
  commandError: downloadCommandError,
  isActive: isDownloadActive,
} = storeToRefs(downloadTaskStore)

/**
 * Заголовок панели — снимок «название + качество», собранный **здесь**, в
 * момент клика, а не прочитанный из карточки позже (требование С-13/TL-45
 * п.6): карточка может смениться или уже смениться содержимым к моменту,
 * когда панель решит перерисоваться, а `displayTitle` в сторе уже не
 * зависит от неё.
 */
function onDownloadRequested(payload: {
  url: string
  title: string
  streams: QualityStreams
  size: QualitySize
  qualityLabel: string
}): void {
  const displayTitle = `«${payload.title}» — ${payload.qualityLabel}`
  void downloadTaskStore.start(
    { url: payload.url, title: payload.title, streams: payload.streams, size: payload.size },
    displayTitle,
  )
}
</script>

<template>
  <ExitConfirmDialog
    v-if="showExitConfirm && exitConfirmTask && exitConfirmProgress"
    :display-title="exitConfirmTask.displayTitle"
    :progress="exitConfirmProgress"
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
        :download-blocked="isDownloadActive"
        @download="onDownloadRequested"
      />

      <!--
        Секция «Текущая загрузка» (дизайн E3) — рендерится тогда и только
        тогда, когда задача существует (с момента клика «Скачать» до
        «Скрыть»/новой загрузки) **или** есть отказ команды управления
        загрузкой, который ещё не скрыт (ревью TL-45, «Достижимый путь
        к молчаливому отказу»): отказ `start_download` возможен и без
        существующей задачи (слот и не должен был занять что-то), поэтому
        секция не привязана только к наличию `downloadTask`. Пока ни того,
        ни другого нет, макет не резервирует под секцию пустое место
        (дизайн, «Где живёт задача экрана»).
      -->
      <template v-if="(downloadTask && downloadProgress) || downloadCommandError">
        <hr class="screen__divider">

        <!--
          Без собственного aria-live здесь (ревью TL-45, «Заметки»):
          `DownloadCommandErrorBlock` несёт role="alert", `DownloadPanel` —
          свою единственную живую зону для нетерминальных фаз и role="status"
          для терминальных. Обёртка секции с ещё одним aria-live поверх них
          дала бы вложенные регионы и задвоенные объявления одного и того
          же текста.
        -->
        <section class="download-section">
          <h2 class="download-section__title">
            Текущая загрузка
          </h2>
          <DownloadCommandErrorBlock
            v-if="downloadCommandError"
            :error="downloadCommandError"
            @hide="downloadTaskStore.dismissCommandError"
          />
          <DownloadPanel
            v-if="downloadTask && downloadProgress"
            :display-title="downloadTask.displayTitle"
            :plan="downloadTask.plan"
            :progress="downloadProgress"
            :soft-stall-seconds="softStallSeconds"
            @cancel="downloadTaskStore.cancel"
            @retry="downloadTaskStore.retry"
            @hide="downloadTaskStore.hide"
          />
        </section>
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
  border-top: 1px solid #ddd;
}

.version {
  margin: 0 0 1rem;
  color: #555;
}

.screen__footer {
  margin-top: 1rem;
}

.download-section__title {
  margin: 0 0 0.5rem;
  font-size: 1rem;
  font-weight: 600;
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
  outline: 2px solid #1a73e8;
  outline-offset: 2px;
}
</style>
