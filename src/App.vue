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
import { computed, onMounted } from 'vue'

import ProbeSection from '@/components/ProbeSection.vue'
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
</script>

<template>
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
        Разделитель — единственное, что явно отделяет «служебную» часть
        экрана (E1, про инструменты) от «рабочей» (про конкретный ролик,
        E2), чтобы ошибка ffmpeg выше не путалась с состоянием разбора
        ниже (дизайн E2, «Где живёт поле ссылки»).
      -->
      <hr class="screen__divider">

      <ProbeSection :yt-dlp-state="ytDlpState" />
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
