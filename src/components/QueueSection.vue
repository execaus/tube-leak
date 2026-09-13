<script setup lang="ts">
/**
 * Секция «Очередь загрузок» (эпик E4, TL-75) — сменяет секцию «Текущая
 * загрузка» (эпик E3, TL-45) на том же месте макета. Список задач вместо
 * ровно одной панели: активная и терминальные рисуются нетронутым
 * `DownloadPanel` (дизайн E3/E4, «Что проектируем и чего здесь нет»),
 * честно ожидающие — новым лёгким `QueueWaitingRow`.
 *
 * Чисто презентационный компонент, тот же приём, что `DownloadPanel`/
 * `DownloadCommandErrorBlock` (эпик E3): состояние и IPC-вызовы живут в
 * `useDownloadTaskStore`, сюда приходят готовые пропсы, клики только
 * эмитятся наверх с `taskId` строки, где применимо.
 */
import { computed } from 'vue'

import type { DownloadCommandFailure } from '@/stores/downloadTask'
import type { QueuePauseReason, QueueTask } from '@/types/generated/queue'
import { toDownloadProgress } from '@/utils/queueTaskProgress'
import { formatTaskDisplayTitle } from '@/utils/queueTaskTitle'
import { getResumeBannerText, YT_DLP_UPDATE_PAUSE_TEXT } from '@/utils/queueTexts'

import DownloadCommandErrorBlock from './DownloadCommandErrorBlock.vue'
import DownloadPanel from './DownloadPanel.vue'
import QueueWaitingRow from './QueueWaitingRow.vue'

const props = defineProps<{
  tasks: QueueTask[]
  awaitingContinue: boolean
  pauseReason?: QueuePauseReason
  commandError?: DownloadCommandFailure
  /** См. `useDownloadTaskStore.softStallSeconds` — относится ровно к активной задаче, если она есть. */
  softStallSeconds?: number
}>()

defineEmits<{
  cancel: [taskId: string]
  retry: [taskId: string]
  hide: [taskId: string]
  'hide-all-terminal': []
  resume: []
  'dismiss-command-error': []
}>()

function isTerminalPhase(phase: QueueTask['phase']): boolean {
  return phase === 'done' || phase === 'failed' || phase === 'cancelled'
}

/**
 * Активная/терминальная задача рисуется `DownloadPanel` без изменений
 * (дизайн E4, «Пять состояний»): ровно эти пять фаз (не `queued`).
 */
function isPanelPhase(phase: QueueTask['phase']): boolean {
  return phase !== 'queued'
}

/** Число задач в `queued`, стоящих строго впереди данной (дизайн, «Пять состояний»: не считая активную и терминальные). */
function aheadCountFor(taskId: string): number {
  let count = 0
  for (const t of props.tasks) {
    if (t.taskId === taskId) break
    if (t.phase === 'queued') count += 1
  }
  return count
}

const hasTerminalTask = computed(() => props.tasks.some((t) => isTerminalPhase(t.phase)))

const showSection = computed(() => props.tasks.length > 0 || props.commandError !== undefined)

const resumeBannerText = computed(() =>
  getResumeBannerText(props.tasks.filter((t) => !isTerminalPhase(t.phase)).length),
)
</script>

<template>
  <section
    v-if="showSection"
    class="queue-section"
  >
    <div class="queue-section__header">
      <h2 class="queue-section__title">
        Очередь загрузок
      </h2>
      <button
        v-if="hasTerminalTask"
        type="button"
        class="tap-target"
        @click="$emit('hide-all-terminal')"
      >
        Скрыть завершённые
      </button>
    </div>

    <!--
      Баннер продолжения после перезапуска (Р-3, дизайн «Продолжение после
      перезапуска») — одна кнопка на всю очередь: см. doc `resume()` в
      `useDownloadTaskStore` про то, почему кнопок на каждую задачу нет.
    -->
    <div
      v-if="awaitingContinue"
      class="queue-section__banner"
      role="status"
    >
      <p class="queue-section__banner-text">
        {{ resumeBannerText }}
      </p>
      <button
        type="button"
        class="tap-target"
        @click="$emit('resume')"
      >
        Продолжить очередь
      </button>
    </div>

    <DownloadCommandErrorBlock
      v-if="commandError"
      :error="commandError"
      @hide="$emit('dismiss-command-error')"
    />

    <!--
      Пауза на обновление yt-dlp между задачами (Р-7/С-8) — там, где
      обычно рисуется активная панель, но её в этот момент нет (дизайн,
      «Пауза на обновление yt-dlp между задачами»). Собственный текст
      очереди, не переиспользует `YtDlpUpdateBlock`/`ytdlp://update`.
    -->
    <p
      v-if="pauseReason === 'ytDlpUpdate'"
      class="queue-section__pause"
      aria-live="polite"
    >
      <span
        class="spinner"
        aria-hidden="true"
      >○</span>
      {{ ' ' }}{{ YT_DLP_UPDATE_PAUSE_TEXT }}
    </p>

    <ul class="queue-section__list">
      <li
        v-for="task in tasks"
        :key="task.taskId"
      >
        <QueueWaitingRow
          v-if="!isPanelPhase(task.phase)"
          :display-title="formatTaskDisplayTitle(task.title, task.quality)"
          :ahead-count="aheadCountFor(task.taskId)"
          :awaiting-continue="awaitingContinue"
          class="queue-section__task"
          @cancel="$emit('cancel', task.taskId)"
        />
        <DownloadPanel
          v-else
          :display-title="formatTaskDisplayTitle(task.title, task.quality)"
          :plan="task.plan"
          :progress="toDownloadProgress(task)"
          :soft-stall-seconds="softStallSeconds"
          class="queue-section__task"
          :class="{ 'queue-section__task--active': !isTerminalPhase(task.phase) }"
          @cancel="$emit('cancel', task.taskId)"
          @retry="$emit('retry', task.taskId)"
          @hide="$emit('hide', task.taskId)"
        />
      </li>
    </ul>
  </section>
</template>

<style scoped>
.queue-section {
  margin-top: 0.5rem;
}

.queue-section__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  margin-bottom: 0.5rem;
}

.queue-section__title {
  margin: 0;
  font-size: 1rem;
  font-weight: 600;
}

.queue-section__banner {
  padding: 0.75rem;
  margin-bottom: 0.75rem;
  background: var(--color-accent-soft);
  border-radius: 0.3rem;
}

.queue-section__banner-text {
  margin: 0 0 0.5rem;
  color: var(--color-text-secondary);
}

.queue-section__pause {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin: 0.75rem 0;
  color: var(--color-text-muted);
}

.queue-section__list {
  list-style: none;
  margin: 0;
  padding: 0;
}

.queue-section__task {
  margin-top: 0.5rem;
}

.queue-section__task--active {
  border-left: 3px solid var(--color-accent);
  padding-left: 0.75rem;
}

.spinner {
  display: inline-block;
  width: 1.25em;
  text-align: center;
  animation: queue-section-spin 1.2s linear infinite;
}

@keyframes queue-section-spin {
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
