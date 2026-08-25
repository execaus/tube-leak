<script setup lang="ts">
/**
 * Служебный экран проверки sidecar (Ф-9, Н-6, TL-8), предварённый экраном
 * подготовки yt-dlp при первом запуске (TL-17).
 *
 * # Порядок вызовов — критично (см. ревью TL-12, #18)
 *
 * `check_sidecar` не вызывается, пока не разрешился промис `prepare_ytdlp`
 * — ни при каком сценарии, включая повторную проверку по кнопке. Во время
 * подготовки резолв пути к yt-dlp честно возвращает `notFound`, и
 * `check_sidecar` показал бы «Не нашли файл yt-dlp по ожидаемому пути» —
 * ложное утверждение при совершенно нормальном первом запуске. Поэтому обе
 * команды идут строго последовательно в {@link runPrepareAndCheckSidecar},
 * а не параллельно с гонкой на отрисовку.
 *
 * # Поднятие экрана подготовки
 *
 * Экран поднимается по приходу первого события `ytdlp://prepare`
 * (`stage` становится `unpacking` или `warmingUp`), а не по факту вызова
 * `prepare_ytdlp` — на тёплом запуске (обычный случай) событий нет вовсе,
 * и промис резолвится за доли секунды: показывать экран подготовки в этом
 * случае было бы обманом, он бы мигнул зря. Пока промис ещё не разрешился
 * и ни одного события не пришло (сверхкороткое окно между вызовом команды
 * и первым событием либо самим разрешением), экран показывает нейтральное
 * «Запускаем…» — не пустое окно, но и не утверждение о конкретном этапе.
 */
import { computed, onMounted } from 'vue'

import SidecarStatusRow from '@/components/SidecarStatusRow.vue'
import YtDlpPrepareError from '@/components/YtDlpPrepareError.vue'
import YtDlpPrepareScreen from '@/components/YtDlpPrepareScreen.vue'
import { useSidecarCheck } from '@/composables/useSidecarCheck'
import { useYtDlpPrepare } from '@/composables/useYtDlpPrepare'

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

/** Узкий тип этапа, для которого показывается прогресс (терминальные — вне этого экрана). */
const preparingStage = computed<'unpacking' | 'warmingUp' | undefined>(() => {
  return stage.value === 'unpacking' || stage.value === 'warmingUp' ? stage.value : undefined
})

type ScreenState = 'starting' | 'preparing' | 'prepareError' | 'ready'

const screenState = computed<ScreenState>(() => {
  if (prepareError.value) return 'prepareError'
  // `preparingStage` — это последнее полученное событие, а не признак того,
  // что подготовка ещё идёт: после разрешения промиса `prepare()` событие
  // остаётся тем же (`ready` может не прийти вовсе), поэтому без проверки
  // `isPreparing` экран подготовки завис бы навсегда даже после успеха.
  if (isPreparing.value && preparingStage.value) return 'preparing'
  if (isPreparing.value) return 'starting'
  return 'ready'
})

/**
 * Кнопка «Повторить проверку» — одна на весь экран (Ф-9 — одна команда на
 * оба бинарника сразу). Показывается тогда и только тогда, когда отчёт уже
 * пришёл и хотя бы одна из строк не в состоянии «в порядке». Достижима
 * только из состояния `ready`, то есть после того как `prepare_ytdlp` уже
 * разрешился, — повторный клик снова зовёт только `check_sidecar`.
 */
const showRetry = computed(() => {
  const r = report.value
  if (!r) return false
  return r.ytDlp.status !== 'ok' || r.ffmpeg.status !== 'ok'
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
</script>

<template>
  <main class="screen">
    <header>
      <h1>tube-leak</h1>
      <p
        v-if="screenState === 'ready'"
        class="version"
      >
        версия {{ APP_VERSION }}
      </p>
    </header>

    <YtDlpPrepareScreen
      v-if="screenState === 'preparing'"
      :stage="preparingStage!"
      :percent="percent"
      :eta-secs="etaSecs"
    />

    <p
      v-else-if="screenState === 'starting'"
      class="starting"
    >
      Запускаем…
    </p>

    <YtDlpPrepareError
      v-else-if="screenState === 'prepareError' && prepareError"
      :error="prepareError"
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
    </template>
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

.starting {
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
