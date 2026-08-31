<script setup lang="ts">
/**
 * Строка честно ожидающей задачи (эпик E4, дизайн «Пять состояний одной
 * задачи»). Новый лёгкий компонент, а не ветка `DownloadPanel` (E3): у
 * ожидающей задачи физически нет ни одной вещи, которую рисует степпер —
 * yt-dlp для неё ещё не запускался, «Готовим загрузку…» было бы неверным
 * текстом. `DownloadPanel` не трогается ни строкой (обещание дизайна E3,
 * подтверждённое дизайном E4) — задача начинает рисоваться им только с
 * момента реального перехода в `fetching`.
 *
 * Чисто презентационный компонент — тот же приём, что `DownloadPanel`/
 * `DownloadCommandErrorBlock` (эпик E3): состояние и IPC-вызовы живут в
 * сторе, сюда приходят готовые пропсы, клик только эмитится.
 */
import { computed } from 'vue'

import { getWaitingStatusText } from '@/utils/queueTexts'

const props = defineProps<{
  /** ««Название» — качество» — тот же формат, что и заголовок `DownloadPanel` (дизайн E4). */
  displayTitle: string
  /**
   * Число нетерминальных задач строго впереди этой, не считая активную
   * (дизайн: «ноль задач впереди даёт вариант без „и ещё N“»). Считается
   * фронтендом по порядку присланного списка (см. doc `QueueSnapshot.tasks`
   * в `src/types/generated/queue.ts`), не отдельным полем контракта.
   */
  aheadCount: number
  /**
   * Очередь восстановлена после перезапуска и ещё не продолжена (Р-3) —
   * меняет формулировку целиком («как только вы продолжите»).
   */
  awaitingContinue: boolean
}>()

defineEmits<{
  cancel: []
}>()

/**
 * `aria-live="polite"` (не сам компонент, а его статус-строка) — текст
 * позиции меняется по мере продвижения очереди, но структурные события
 * заметно реже прогресса внутри «Скачивание» (дизайн E4, «Доступность»).
 */
const statusText = computed(() => getWaitingStatusText(props.aheadCount, props.awaitingContinue))
</script>

<template>
  <div class="queue-waiting-row">
    <p class="queue-waiting-row__title">
      {{ displayTitle }}
    </p>
    <p
      class="queue-waiting-row__status"
      aria-live="polite"
    >
      {{ statusText }}
    </p>
    <div class="queue-waiting-row__actions">
      <button
        type="button"
        class="tap-target"
        @click="$emit('cancel')"
      >
        Отменить
      </button>
    </div>
  </div>
</template>

<style scoped>
.queue-waiting-row {
  padding: 0.75rem 0;
  border-bottom: 1px solid #eee;
}

.queue-waiting-row__title {
  margin: 0 0 0.25rem;
  font-weight: 600;
}

.queue-waiting-row__status {
  margin: 0 0 0.5rem;
  color: #555;
}

.queue-waiting-row__actions {
  display: flex;
  gap: 0.5rem;
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
  outline: 2px solid #1a73e8;
  outline-offset: 2px;
}
</style>
