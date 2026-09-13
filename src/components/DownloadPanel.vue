<script setup lang="ts">
/**
 * Панель задачи скачивания (дизайн E3, эпик TL-45): степпер фаз, прогресс
 * внутри «Скачивание», пауза перед повтором, терминальные состояния
 * Done/Failed/Cancelled.
 *
 * Чисто презентационный компонент — состояние и IPC-вызовы живут в
 * `useDownloadTaskStore` (`src/stores/downloadTask.ts`), сюда приходят уже
 * готовые пропсы; клики наверх эмитятся, ничего не вызывается напрямую.
 * Это тот же приём, что `ProbeErrorBlock`/`VideoCard` в эпике E2 — панель
 * тестируется пропсами, без мока `invoke`.
 *
 * `displayTitle` — снимок «название + качество», сделанный в момент клика
 * «Скачать» (требование С-13/TL-45 п.6): пропс, а не что-то, читаемое из
 * карточки повторно, поэтому компонент переживает замену карточки без
 * каких-либо изменений здесь.
 */
import { computed } from 'vue'

import type { DownloadPlan, DownloadProgress } from '@/types/generated/download'
import { assertNever } from '@/utils/assertNever'
import { formatEtaSecs } from '@/utils/formatEtaSecs'
import { formatSpeed } from '@/utils/formatSpeed'
import { getCancelledText, getFailedPartialDataNote, getFolderDisplayText } from '@/utils/downloadOutcomeTexts'
import { getDownloadErrorText } from '@/utils/downloadErrorTexts'

const props = defineProps<{
  displayTitle: string
  plan: DownloadPlan
  progress: DownloadProgress
  /**
   * Секунды без события `download://progress` — только пока
   * `progress.phase === 'downloading' && progress.state === 'running'`
   * (см. `useDownloadTaskStore`). `undefined` — индикатор не показан.
   */
  softStallSeconds?: number
}>()

const emit = defineEmits<{
  cancel: []
  retry: []
  hide: []
}>()

type StepKey = 'preparing' | 'downloading' | 'merging' | 'done'

/**
 * Список шагов степпера — «Склейка» не рисуется вовсе для «только
 * аудио»/прогрессивного формата (Ф-3, дизайн «Степпер фаз»), а не
 * показывается серым.
 */
const steps = computed<{ key: StepKey; label: string }[]>(() => {
  const list: { key: StepKey; label: string }[] = [
    { key: 'preparing', label: 'Подготовка' },
    { key: 'downloading', label: 'Скачивание' },
  ]
  if (props.plan === 'videoAndAudio') {
    list.push({ key: 'merging', label: 'Склейка' })
  }
  list.push({ key: 'done', label: 'Готово' })
  return list
})

/**
 * Шаг степпера, к которому относится нетерминальная фаза — `undefined`
 * для терминальных (степпер там не рисуется).
 *
 * Все семь значений `DownloadPhase` перечислены явно, включая `failed` и
 * `cancelled` (обе намеренно дают `undefined`) — раньше их накрывал общий
 * `default: return undefined`, и восьмая фаза, которую добавит очередь
 * (E4), тихо получила бы тот же `undefined`, не подсветив ни одного шага
 * степпера, и осталась бы незамеченной: `StepKey | undefined` разрешает
 * `undefined` уже сегодня, так что падение через конец `switch` не
 * ловится typecheck'ом сам по себе (ревью TL-52). `assertNever` в `default`
 * держит эту границу: он недостижим, пока перечислены все семь фаз, и
 * перестаёт собираться, как только контракт добавит восьмую.
 */
const currentStepKey = computed<StepKey | undefined>(() => {
  const phase = props.progress.phase
  switch (phase) {
    case 'queued':
    case 'fetching':
      return 'preparing'
    case 'downloading':
      return 'downloading'
    case 'merging':
      return 'merging'
    case 'done':
      return 'done'
    case 'failed':
    case 'cancelled':
      return undefined
    default:
      return assertNever(phase)
  }
})

const isTerminal = computed(
  () =>
    props.progress.phase === 'done' ||
    props.progress.phase === 'failed' ||
    props.progress.phase === 'cancelled',
)

function stepStatus(key: StepKey): 'done' | 'current' | 'upcoming' {
  const current = currentStepKey.value
  if (current === undefined) return 'upcoming'
  const list = steps.value
  const idx = list.findIndex((s) => s.key === key)
  const currentIdx = list.findIndex((s) => s.key === current)
  if (idx < currentIdx) return 'done'
  if (idx === currentIdx) return 'current'
  return 'upcoming'
}

function stepMarker(status: 'done' | 'current' | 'upcoming'): string {
  switch (status) {
    case 'done':
      return '✓'
    case 'current':
      return '●'
    case 'upcoming':
      return '○'
  }
}

/** Подпись подпотока — только когда потоков два (дизайн: «иначе просто «Скачиваем…»»). */
const streamLabel = computed(() => {
  const p = props.progress
  if (p.phase !== 'downloading' || p.state !== 'running') return undefined
  if (props.plan === 'videoAndAudio' && p.stream !== undefined) {
    return p.stream === 'video' ? 'Скачиваем видео' : 'Скачиваем звук'
  }
  return 'Скачиваем'
})

/**
 * Строка состава «Скачивание» (Ф-2, «Зафиксировано анализом» — компоновка
 * дизайна): любой отсутствующий элемент опускается целиком, а не рисуется
 * прочерком или нулём.
 *
 * Во время мягкого индикатора зависания (`softStallSeconds`) скорость и
 * оценка времени не показываются вовсе — застывшее число здесь тот же
 * класс лжи, что «не отвечает» в E1 (дизайн, «Три разных нет движения»,
 * пункт 1, явно про скорость; ETA скрывается тем же принципом — оценка,
 * посчитанная по уже неактуальной скорости, была бы такой же неправдой).
 */
const runningLineParts = computed<string[]>(() => {
  const p = props.progress
  if (p.phase !== 'downloading' || p.state !== 'running') return []

  const parts: string[] = []
  if (streamLabel.value) parts.push(streamLabel.value)

  const stalled = props.softStallSeconds !== undefined
  if (stalled) {
    parts.push('медленно или не отвечает')
  } else {
    if (p.speedBytesPerSec !== undefined) parts.push(formatSpeed(p.speedBytesPerSec))
    if (p.etaSecs !== undefined) parts.push(`осталось ${formatEtaSecs(p.etaSecs)}`)
  }

  // Номер попытки — только начиная со второй (дизайн: не загромождать
  // обычный путь числом, которое почти всегда «1»).
  if (p.attempt !== undefined && p.attempt.number > 1) {
    parts.push(`попытка ${p.attempt.number} из ${p.attempt.total}`)
  }

  return parts
})

const errorText = computed(() => {
  if (props.progress.phase !== 'failed') return undefined
  return getDownloadErrorText(props.progress.error.kind, props.progress.error.reason)
})

const failedPartialNote = computed(() => {
  if (props.progress.phase !== 'failed') return undefined
  return getFailedPartialDataNote(props.progress.error.partialData)
})

const cancelledText = computed(() => {
  if (props.progress.phase !== 'cancelled') return undefined
  return getCancelledText(props.progress.partialData)
})

const showDetailsToggle = computed(() => {
  const p = props.progress
  if (p.phase !== 'failed') return false
  return p.error.details?.stderrTail !== undefined || p.error.details?.exitCode !== undefined
})
</script>

<template>
  <div class="download-panel">
    <p class="download-panel__title">
      {{ displayTitle }}
    </p>

    <template v-if="!isTerminal">
      <ol class="download-panel__steps">
        <li
          v-for="step in steps"
          :key="step.key"
          class="download-panel__step"
          :class="`download-panel__step--${stepStatus(step.key)}`"
          :aria-current="stepStatus(step.key) === 'current' ? 'step' : undefined"
        >
          <span
            class="download-panel__step-marker"
            aria-hidden="true"
          >{{ stepMarker(stepStatus(step.key)) }}</span>
          {{ step.label }}
        </li>
      </ol>

      <template v-if="progress.phase === 'queued' || progress.phase === 'fetching'">
        <p
          class="download-panel__row"
          aria-live="polite"
        >
          <span
            class="spinner"
            aria-hidden="true"
          >○</span>
          Готовим загрузку — yt-dlp запускается…
        </p>
      </template>

      <template v-else-if="progress.phase === 'downloading' && progress.state === 'running'">
        <!--
          Полоса рисуется только когда есть реальный процент — дизайн:
          отсутствующий элемент опускается целиком, не рисуется прочерком
          или нулём (ревью TL-45: нулевая ширина выглядела как «почти
          ничего не скачано», хотя данных попросту ещё нет).
        -->
        <div
          v-if="progress.percent !== undefined"
          class="download-panel__bar"
          role="progressbar"
          aria-valuemin="0"
          aria-valuemax="100"
          :aria-valuenow="progress.percent"
        >
          <div
            class="download-panel__bar-fill"
            :style="{ width: `${progress.percent}%` }"
          />
        </div>
        <p
          v-if="progress.percent !== undefined"
          class="download-panel__percent"
        >
          {{ Math.round(progress.percent) }} %
        </p>
        <!--
          Единственная живая зона фазы «Скачивание» — процент в неё
          сознательно не входит (ревью TL-45, «Заметки»): три вложенных
          региона, где процент объявлялся по несколько раз в секунду,
          устроили бы очередь объявлений ровно того рода, которого дизайн
          просил избежать.
        -->
        <div aria-live="polite">
          <p
            v-if="runningLineParts.length > 0"
            class="download-panel__row"
          >
            {{ runningLineParts.join(' · ') }}
          </p>
          <p
            v-if="softStallSeconds !== undefined"
            class="download-panel__stall"
          >
            Нет новых данных уже {{ softStallSeconds }} с — проверяем соединение…
          </p>
        </div>
      </template>

      <template v-else-if="progress.phase === 'downloading' && progress.state === 'waitingRetry'">
        <div
          v-if="progress.percent !== undefined"
          class="download-panel__bar"
          role="progressbar"
          aria-valuemin="0"
          aria-valuemax="100"
          aria-busy="true"
        >
          <div
            class="download-panel__bar-fill"
            :style="{ width: `${progress.percent}%` }"
          />
        </div>
        <p
          v-if="progress.percent !== undefined"
          class="download-panel__percent"
        >
          {{ Math.round(progress.percent) }} % (сохранено)
        </p>
        <p
          class="download-panel__row"
          aria-live="polite"
        >
          Соединение потеряно. Ждём повторной попытки ({{ progress.attempt.number }} из
          {{ progress.attempt.total }}) — через {{ progress.remainingSecs }} с…
        </p>
      </template>

      <template v-else-if="progress.phase === 'merging'">
        <!--
          Ревью TL-45: полоса, залитая на всю ширину (блочный `div` без
          заданной ширины), читалась как «готово», хотя remux ещё идёт.
          Дизайн для «Склейки» полосы не рисует вовсе — только нейтральный
          маркер; `aria-busy` держим на самой строке, не заводя пустой
          `role="progressbar"` без какого-либо визуального смысла.
        -->
        <p
          class="download-panel__row"
          aria-busy="true"
          aria-live="polite"
        >
          <span
            class="spinner"
            aria-hidden="true"
          >○</span>
          Склеиваем видео и звук…
        </p>
      </template>

      <div class="download-panel__actions">
        <button
          type="button"
          class="tap-target"
          @click="emit('cancel')"
        >
          Отменить
        </button>
      </div>
    </template>

    <template v-else-if="progress.phase === 'done'">
      <section
        class="download-panel__terminal"
        role="status"
      >
        <p class="download-panel__terminal-title">
          ✓ Готово
        </p>
        <p class="download-panel__row download-panel__row--wrap">
          «{{ progress.fileName }}» сохранён в папке {{ getFolderDisplayText(progress.folderDisplay) }}.
        </p>
        <div class="download-panel__actions">
          <button
            type="button"
            class="tap-target"
            @click="emit('hide')"
          >
            Скрыть
          </button>
        </div>
      </section>
    </template>

    <template v-else-if="progress.phase === 'failed'">
      <section
        class="download-panel__terminal"
        role="status"
      >
        <p class="download-panel__terminal-title">
          ✕ {{ errorText?.title }}
        </p>
        <p class="download-panel__row">
          {{ errorText?.explanation }} {{ failedPartialNote }}
        </p>

        <details v-if="showDetailsToggle">
          <summary>Подробнее</summary>
          <dl class="download-panel__details">
            <template v-if="progress.error.details?.exitCode !== undefined">
              <dt>Код выхода</dt>
              <dd>{{ progress.error.details.exitCode }}</dd>
            </template>
            <template v-if="progress.error.details?.stderrTail">
              <dt>stderr</dt>
              <dd>{{ progress.error.details.stderrTail }}</dd>
            </template>
          </dl>
        </details>

        <div class="download-panel__actions">
          <button
            v-if="progress.error.retryable"
            type="button"
            class="tap-target"
            @click="emit('retry')"
          >
            Повторить
          </button>
          <button
            type="button"
            class="tap-target"
            @click="emit('hide')"
          >
            Скрыть
          </button>
        </div>
      </section>
    </template>

    <template v-else-if="progress.phase === 'cancelled'">
      <section
        class="download-panel__terminal"
        role="status"
      >
        <p class="download-panel__terminal-title">
          ○ Отменено
        </p>
        <p class="download-panel__row">
          {{ cancelledText }}
        </p>
        <div class="download-panel__actions">
          <button
            type="button"
            class="tap-target"
            @click="emit('hide')"
          >
            Скрыть
          </button>
        </div>
      </section>
    </template>
  </div>
</template>

<style scoped>
.download-panel {
  margin-top: 0.75rem;
}

.download-panel__title {
  margin: 0 0 0.5rem;
  font-weight: 600;
}

.download-panel__steps {
  display: flex;
  flex-wrap: wrap;
  gap: 0.4rem;
  margin: 0 0 0.75rem;
  padding: 0;
  list-style: none;
}

.download-panel__step {
  display: flex;
  align-items: center;
  gap: 0.3rem;
  color: var(--color-text-subtle);
}

.download-panel__step--current {
  color: var(--color-text-strong);
  font-weight: 600;
}

.download-panel__step--done {
  color: var(--color-text-muted);
}

.download-panel__step:not(:last-child)::after {
  content: '→';
  margin-left: 0.4rem;
  /*
   * Отдельный токен, а не `--color-text-faint` (ревью TL-22, Б-2): у
   * стрелки-разделителя был свой оттенок (`#aaa`), чуть светлее, чем у
   * плейсхолдера превью (`#999`) — совпадение по смыслу («едва заметно»),
   * а не по значению. `--color-text-faint` держит светлое значение `#999`
   * ради VideoThumbnail; сюда нужен именно `#aaa`, иначе светлая тема
   * меняется на пиксель.
   */
  color: var(--color-text-decorative);
}

.download-panel__bar {
  width: 100%;
  height: 0.6rem;
  background: var(--color-progress-track);
  border-radius: 999px;
  overflow: hidden;
}

.download-panel__bar-fill {
  height: 100%;
  background: var(--color-accent);
  transition: width 0.2s ease;
}

.download-panel__percent {
  margin: 0.35rem 0 0;
  font-weight: 600;
}

.download-panel__row {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  margin: 0.35rem 0 0;
  color: var(--color-text-secondary);
}

/*
 * Модификатор для строк, которые могут содержать длинный путь без
 * пробелов (TL-95, Done: `folderDisplay` с кастомной папкой). Путь не
 * сокращается (см. `getFolderDisplayText`), поэтому раскладку защищает
 * только CSS: `display: block` вместо строкового флекса (тут нет иконки
 * рядом с текстом, гнать его во флекс-строку незачем) и
 * `overflow-wrap: anywhere`, чтобы длинная последовательность без
 * пробелов переносилась посимвольно, а не раздвигала панель. Решение
 * дизайном не покрыто (документ молчит про перенос длинного пути в
 * панели, в отличие от пути в настройках/истории — там про сокращение
 * сказано явно, про перенос нет), поэтому выбран самый предсказуемый
 * вариант, а не многоточие с `title`, которое прятало бы факт вместо
 * того, чтобы его показывать.
 */
.download-panel__row--wrap {
  display: block;
  overflow-wrap: anywhere;
}

.download-panel__stall {
  margin: 0.35rem 0 0;
  color: var(--color-error);
}

.download-panel__actions {
  display: flex;
  gap: 0.5rem;
  margin-top: 0.75rem;
}

.download-panel__terminal-title {
  margin: 0 0 0.5rem;
  font-weight: 600;
}

.download-panel__details {
  margin: 0.5rem 0 0;
  padding: 0.5rem;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.8rem;
  background: var(--color-surface-subtle);
  white-space: pre-wrap;
  word-break: break-word;
}

.spinner {
  display: inline-block;
  width: 1.25em;
  text-align: center;
  animation: download-panel-spin 1.2s linear infinite;
}

@keyframes download-panel-spin {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}

.tap-target {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 40px;
  min-height: 40px;
  padding: 0.5rem 0.75rem;
  box-sizing: border-box;
}

.tap-target:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}
</style>
