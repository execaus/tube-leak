<script setup lang="ts">
/**
 * Экран этапа загрузки при первой подготовке yt-dlp (TL-17).
 *
 * Показывается только пока идёт реальная работа (`unpacking`/`warmingUp`) —
 * решение о том, показывать ли этот экран вообще, принимает родитель
 * (`App.vue`) по факту первого полученного события `ytdlp://prepare`, а не
 * по факту вызова команды: на тёплом запуске событий нет вовсе, и этот
 * компонент не монтируется.
 *
 * Только отображает то, что передали props, — сам `invoke`/`listen` не
 * вызывает (это делает `useYtDlpPrepare` в родителе).
 */
import { computed } from 'vue'

import type { NonTerminalYtDlpPrepareStage } from '@/composables/useYtDlpPrepare'
import { assertNever } from '@/utils/assertNever'

const props = defineProps<{
  /**
   * Тип — `NonTerminalYtDlpPrepareStage` из `useYtDlpPrepare.ts`, а не
   * повторённый здесь `'unpacking' | 'warmingUp'` (ревью TL-52: те же два
   * литерала жили в двух не связанных типами местах — composable и этот
   * проп могли разойтись молча).
   */
  stage: NonTerminalYtDlpPrepareStage
  /** Сквозной прогресс всей подготовки (0..100), не прогресс текущего этапа. */
  percent: number
  etaSecs?: number
}>()

const label = computed(() => {
  switch (props.stage) {
    case 'unpacking':
      return 'Распаковываем yt-dlp…'
    case 'warmingUp':
      return 'Готовим yt-dlp к первому запуску…'
    default:
      // `NonTerminalYtDlpPrepareStage` сегодня — ровно два значения выше;
      // `assertNever` — тот же сторож, что в `useYtDlpPrepare.ts`
      // (`isNonTerminalStage`), а не текст-заглушка на случай будущего
      // варианта.
      return assertNever(props.stage)
  }
})

/**
 * Первый и единственный запуск после установки (или после того, как ОС
 * забыла кэш проверки подписей) занимает заметно дольше обычного — это не
 * зависание, объясняем явно, чтобы экран не выглядел как «не отвечает».
 */
const hint = 'Это происходит один раз. Следующий запуск будет быстрым.'

function formatEta(seconds: number): string {
  if (seconds < 60) {
    return `осталось ~${seconds} с`
  }
  const minutes = Math.floor(seconds / 60)
  const rest = seconds % 60
  return rest === 0 ? `осталось ~${minutes} мин` : `осталось ~${minutes} мин ${rest} с`
}

const etaText = computed(() => (props.etaSecs !== undefined ? formatEta(props.etaSecs) : undefined))
</script>

<template>
  <section class="prepare-screen">
    <p
      class="prepare-screen__label"
      role="status"
      aria-live="polite"
    >
      {{ label }}
    </p>

    <div
      class="prepare-screen__bar"
      role="progressbar"
      :aria-label="label"
      :aria-valuenow="percent"
      aria-valuemin="0"
      aria-valuemax="100"
    >
      <div
        class="prepare-screen__bar-fill"
        :style="{ width: `${percent}%` }"
      />
    </div>

    <!--
      Проценты и ETA — вне живой области нарочно: ядро шлёт события ~раз в
      500 мс, и если бы вся секция была `aria-live`, VoiceOver тараторил бы
      числа все 35 секунд подряд. Живая область — только `__label`, он
      меняется всего пару раз за всю подготовку (ревью TL-17, #18).
    -->
    <p class="prepare-screen__percent">
      {{ percent }}%
      <span v-if="etaText"> · {{ etaText }}</span>
    </p>

    <p class="prepare-screen__hint">
      {{ hint }}
    </p>
  </section>
</template>

<style scoped>
.prepare-screen {
  padding: 1rem 0;
}

.prepare-screen__label {
  margin: 0 0 0.75rem;
  font-weight: 600;
}

.prepare-screen__bar {
  height: 0.5rem;
  border-radius: 0.25rem;
  background: rgba(0, 0, 0, 0.08);
  overflow: hidden;
}

.prepare-screen__bar-fill {
  height: 100%;
  background: #1a73e8;
  transition: width 0.2s ease-out;
}

.prepare-screen__percent {
  margin: 0.5rem 0 0;
  color: #555;
  font-variant-numeric: tabular-nums;
}

.prepare-screen__hint {
  margin: 0.75rem 0 0;
  color: #777;
  font-size: 0.9rem;
}
</style>
