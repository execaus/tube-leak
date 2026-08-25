<script setup lang="ts">
/**
 * Служебный экран проверки sidecar (Ф-9, Н-6, TL-8). Единственный экран
 * эпика E1 — не ведёт никуда дальше (см. дизайн эпика E1).
 *
 * Окно не ждёт результата проверки перед отрисовкой (Н-6): обе строки
 * рендерятся сразу в состоянии «Проверяем…», а запрос к бэкенду уходит
 * асинхронно после монтирования (`onMounted`), не до него.
 */
import { computed, onMounted } from 'vue'

import SidecarStatusRow from '@/components/SidecarStatusRow.vue'
import { useSidecarCheck } from '@/composables/useSidecarCheck'

// Версия приложения известна локально и не зависит от sidecar (дизайн E1,
// «Компоновка»). Держим в синхроне с `package.json` вручную — единственное
// поле, дублировать которое через JSON-импорт ради одной строки избыточно.
const APP_VERSION = '0.1.0'

const { report, isLoading, check } = useSidecarCheck()

/**
 * Кнопка «Повторить проверку» — одна на весь экран (Ф-9 — одна команда на
 * оба бинарника сразу). Показывается тогда и только тогда, когда отчёт уже
 * пришёл и хотя бы одна из строк не в состоянии «в порядке».
 */
const showRetry = computed(() => {
  const r = report.value
  if (!r) return false
  return r.ytDlp.status !== 'ok' || r.ffmpeg.status !== 'ok'
})

onMounted(() => {
  void check()
})
</script>

<template>
  <main class="screen">
    <header>
      <h1>tube-leak</h1>
      <p class="version">
        версия {{ APP_VERSION }}
      </p>
    </header>

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
  </main>
</template>

<style scoped>
.screen {
  max-width: 40rem;
  margin: 0 auto;
  padding: 1.5rem;
  font-family: system-ui, -apple-system, sans-serif;
}

.version {
  margin: 0 0 1rem;
  color: #555;
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
  outline: 2px solid #1a73e8;
  outline-offset: 2px;
}
</style>
