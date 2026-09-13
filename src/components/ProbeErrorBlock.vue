<script setup lang="ts">
/**
 * Блок ошибки разбора (дизайн E2, раздел «Состояния», 8 классов из 9 —
 * `notAUrl` рисуется отдельно, инлайн под полем, не здесь, см.
 * `ProbeSection.vue`). Заголовок и пояснение — строго из таблицы дизайна
 * (`getProbeErrorText`), не из `error.message` (нормативно, раздел
 * «Решения по контракту, принятые на TL-27»): `message` уходит только в
 * свёрнутое «Подробнее», как техническая деталь.
 *
 * Только отображает то, что передали props, и сообщает о клике
 * «Повторить» наверх — сам `invoke` не вызывает.
 */
import { computed, ref } from 'vue'

import type { ProbeFailure } from '@/composables/useProbe'
import { getProbeErrorText, NON_CONTRACTUAL_FAILURE_TEXT } from '@/utils/probeErrorTexts'

const props = defineProps<{
  error: ProbeFailure
}>()

defineEmits<{
  retry: []
}>()

const detailsOpen = ref(false)

function toggleDetails(): void {
  detailsOpen.value = !detailsOpen.value
}

const text = computed(() => {
  const err = props.error
  if (err.kind === undefined) return NON_CONTRACTUAL_FAILURE_TEXT
  return getProbeErrorText(err.kind, err.reason, err.timeoutSecs)
})

const kindLabel = computed(() => props.error.kind ?? 'неизвестно (не по контракту)')

interface DetailEntry {
  label: string
  value: string
}

const details = computed<DetailEntry[]>(() => {
  const err = props.error
  const entries: DetailEntry[] = [
    { label: 'Код', value: kindLabel.value },
    { label: 'Сообщение', value: err.message },
  ]

  if (err.kind === undefined) return entries

  if (err.reason !== undefined) {
    entries.push({ label: 'Причина', value: err.reason })
  }
  if (err.timeoutSecs !== undefined) {
    entries.push({ label: 'Таймаут', value: `${err.timeoutSecs} с` })
  }
  if (err.details?.exitCode !== undefined) {
    entries.push({ label: 'Код выхода', value: String(err.details.exitCode) })
  }
  if (err.details?.stderrTail) {
    entries.push({ label: 'stderr', value: err.details.stderrTail })
  }

  return entries
})
</script>

<template>
  <section
    class="probe-error"
    role="alert"
    aria-live="assertive"
  >
    <p class="probe-error__title">
      {{ text.title }}
    </p>
    <p class="probe-error__explanation">
      {{ text.explanation }}
    </p>

    <div class="probe-error__actions">
      <button
        v-if="text.canRetry"
        type="button"
        class="tap-target"
        @click="$emit('retry')"
      >
        Повторить
      </button>
      <button
        type="button"
        class="tap-target"
        :aria-expanded="detailsOpen"
        @click="toggleDetails"
      >
        Подробнее {{ detailsOpen ? '▴' : '▾' }}
      </button>
    </div>

    <dl
      v-if="detailsOpen"
      class="probe-error__details"
    >
      <template
        v-for="entry in details"
        :key="entry.label"
      >
        <dt>{{ entry.label }}</dt>
        <dd>{{ entry.value }}</dd>
      </template>
    </dl>
  </section>
</template>

<style scoped>
.probe-error {
  padding: 0.75rem 0;
}

.probe-error__title {
  margin: 0 0 0.5rem;
  font-weight: 600;
}

.probe-error__explanation {
  margin: 0 0 0.75rem;
  max-width: 40rem;
  line-height: 1.4;
}

.probe-error__actions {
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
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}

.probe-error__details {
  margin: 0.75rem 0 0;
  padding: 0.5rem;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.8rem;
  background: var(--color-surface-subtle);
  white-space: pre-wrap;
  word-break: break-word;
}

.probe-error__details dt {
  font-weight: 600;
}

.probe-error__details dd {
  margin: 0 0 0.5rem;
}
</style>
