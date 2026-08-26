<script setup lang="ts">
/**
 * Отказ команды управления загрузкой (`start_download`/`cancel_download`/
 * `retry_download`) — ревью TL-45, «Достижимый путь к молчаливому отказу»:
 * раньше отказ гасился только в консоль, и пользователь, нажавший
 * «Скачать», не видел ничего — тот же класс дефекта, что «не отвечает»
 * в E1.
 *
 * Чисто презентационный блок, тот же приём, что `ProbeErrorBlock` в E2:
 * заголовок и пояснение по классу (`downloadCommandErrorTexts.ts`), без
 * кнопки «Повторить» — ни один из шести классов не решается повтором
 * того же вызова (см. doc `DownloadCommandErrorKind` в
 * `src/types/download.ts`).
 */
import { computed } from 'vue'

import type { DownloadCommandFailure } from '@/stores/downloadTask'
import {
  getDownloadCommandErrorText,
  NON_CONTRACTUAL_COMMAND_ERROR_TEXT,
} from '@/utils/downloadCommandErrorTexts'

const props = defineProps<{
  error: DownloadCommandFailure
}>()

defineEmits<{
  hide: []
}>()

const text = computed(() =>
  props.error.kind === undefined
    ? NON_CONTRACTUAL_COMMAND_ERROR_TEXT
    : getDownloadCommandErrorText(props.error.kind),
)
</script>

<template>
  <section
    class="download-command-error"
    role="alert"
    aria-live="assertive"
  >
    <p class="download-command-error__title">
      ✕ {{ text.title }}
    </p>
    <p class="download-command-error__explanation">
      {{ text.explanation }}
    </p>
    <div class="download-command-error__actions">
      <button
        type="button"
        class="tap-target"
        @click="$emit('hide')"
      >
        Скрыть
      </button>
    </div>
  </section>
</template>

<style scoped>
.download-command-error {
  margin-top: 0.75rem;
  padding: 0.75rem 0;
}

.download-command-error__title {
  margin: 0 0 0.5rem;
  font-weight: 600;
}

.download-command-error__explanation {
  margin: 0 0 0.75rem;
  max-width: 40rem;
  line-height: 1.4;
  color: #333;
}

.download-command-error__actions {
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
