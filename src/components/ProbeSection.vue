<script setup lang="ts">
/**
 * Секция «ссылка + карточка» (эпик E2, TL-33). Встраивается в `App.vue`
 * ниже существующих строк sidecar (E1, компоненты не меняются) — см.
 * дизайн, раздел «Где живёт поле ссылки».
 *
 * Поле ссылки имеет три состояния, зависящие только от статуса **yt-dlp**
 * (`ytDlpState`, передаётся родителем из `useSidecarCheck`) — статус
 * ffmpeg его не блокирует, потому что разбор ролика ffmpeg не использует.
 *
 * # Ретрансляция запроса скачивания (эпик E3, TL-45)
 *
 * `VideoCard` эмитит снимок «что скачивать» без ссылки на сам ролик — она
 * известна только здесь, в `useLinkProbe`. `ProbeSection` дополняет снимок
 * текущим значением поля `url` и ретранслирует его наверх, в `App.vue`,
 * где живёт стор задачи скачивания: сама секция разбора задачу не
 * запускает и ничего о ней не хранит.
 */
import { computed, useId } from 'vue'

import type { QualitySize, QualityStreams } from '@/types/generated/probe'
import type { SelectedQuality } from '@/types/generated/queue'
import { useLinkProbe } from '@/composables/useProbe'

import ProbeErrorBlock from './ProbeErrorBlock.vue'
import VideoCard from './VideoCard.vue'

const props = defineProps<{
  /**
   * - `checking` — проверка yt-dlp ещё идёт (`report` не пришёл, `isLoading`).
   * - `blocked` — yt-dlp не в порядке (`status !== 'ok'`); ошибка уже видна
   *   в строке `yt-dlp` выше, здесь текст не повторяется.
   * - `ready` — yt-dlp `ok`, поле активно.
   */
  ytDlpState: 'checking' | 'blocked' | 'ready'
}>()

const emit = defineEmits<{
  download: [
    payload: { url: string; title: string; streams: QualityStreams; size: QualitySize; quality: SelectedQuality },
  ]
}>()

const { url, state, retry } = useLinkProbe()

function onDownload(payload: {
  title: string
  streams: QualityStreams
  size: QualitySize
  quality: SelectedQuality
}): void {
  // `url.value` — то, что буквально лежит в поле (для отображения); разбор
  // (`useLinkProbe`) уже давно решает по обрезанной строке (`evaluate`
  // делает `value.trim()` перед проверками). Ссылка, вставленная с
  // завершающим переносом строки/пробелом, давала нормальную карточку (её
  // строит обрезанное значение), но затем попадала в команду старта
  // необрезанной — и core честно отклонял её как `invalidUrl` молча
  // (ревью TL-45, «Достижимый путь к молчаливому отказу»). Обрезаем здесь,
  // в точке эмита, а не полагаемся на то, что где-то выше её обрежут ещё раз.
  emit('download', { ...payload, url: url.value.trim() })
}

/** Стабильный id, связывающий видимый `<label>` с полем (доступность: не только aria-label). */
const inputId = useId()

const disabled = computed(() => props.ytDlpState !== 'ready')

const placeholder = computed(() => {
  switch (props.ytDlpState) {
    case 'checking':
      return 'Проверяем yt-dlp…'
    case 'blocked':
      return 'Разбор ссылок недоступен, пока не решена проблема с yt-dlp выше'
    case 'ready':
      return 'Вставьте ссылку на ролик YouTube'
  }
  return ''
})

/** Инлайн-текст под полем (aria-live="polite") — привязан к вводу, обновляется на каждое изменение. */
const inlineNotAUrlText = computed(() =>
  state.value.kind === 'notAUrl'
    ? 'Это не похоже на ссылку на ролик YouTube. Проверьте, что скопировали именно адрес страницы (https://…).'
    : '',
)
</script>

<template>
  <section class="probe-section">
    <label
      :for="inputId"
      class="probe-section__label"
    >
      Ссылка на видео
    </label>
    <input
      :id="inputId"
      v-model="url"
      type="text"
      class="probe-section__input tap-target"
      :disabled="disabled"
      :placeholder="placeholder"
    >

    <!--
      Персистентный контейнер (не v-if) — так aria-live гарантированно
      подхватывает изменение текста скринридером (дизайн E2, «Клавиатурная
      доступность ошибок»).
    -->
    <p
      class="probe-section__inline-error"
      aria-live="polite"
    >
      {{ inlineNotAUrlText }}
    </p>

    <div class="probe-section__body">
      <template v-if="state.kind === 'empty'">
        <p class="probe-section__hint">
          например, https://www.youtube.com/watch?v=…
        </p>
        <p class="probe-section__disclaimer">
          Скачивайте только то, на что у вас есть право; обход платных ограничений, DRM и
          региональных блокировок не поддерживается.
        </p>
      </template>

      <template v-else-if="state.kind === 'loading'">
        <p class="probe-section__loading">
          <span
            class="spinner"
            aria-hidden="true"
          >○</span>
          Получаем данные о ролике…
        </p>
        <p
          v-if="state.slow"
          class="probe-section__loading-slow"
        >
          Для роликов с большим числом доступных качеств это иногда занимает больше времени
        </p>
      </template>

      <VideoCard
        v-else-if="state.kind === 'success'"
        :result="state.result"
        @download="onDownload"
      />

      <ProbeErrorBlock
        v-else-if="state.kind === 'error'"
        :error="state.error"
        @retry="retry"
      />
    </div>
  </section>
</template>

<style scoped>
.probe-section {
  margin-top: 1rem;
}

.probe-section__label {
  margin: 0 0 0.5rem;
  font-size: 1rem;
  font-weight: 600;
}

.probe-section__input {
  width: 100%;
  box-sizing: border-box;
  padding: 0.5rem 0.75rem;
  font-size: 1rem;
}

.probe-section__input:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}

.probe-section__inline-error {
  min-height: 1.2em;
  margin: 0.35rem 0 0;
  color: var(--color-error);
}

.probe-section__body {
  margin-top: 0.5rem;
}

.probe-section__hint {
  margin: 0;
  color: var(--color-text-muted);
}

.probe-section__disclaimer {
  margin: 0.75rem 0 0;
  font-size: 0.8rem;
  color: var(--color-text-subtle);
}

.probe-section__loading {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin: 0;
}

.probe-section__loading-slow {
  margin: 0.35rem 0 0;
  font-size: 0.85rem;
  color: var(--color-text-muted);
}

.spinner {
  display: inline-block;
  width: 1.25em;
  text-align: center;
  animation: probe-section-spin 1.2s linear infinite;
}

@keyframes probe-section-spin {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}

.tap-target {
  min-width: 40px;
  min-height: 40px;
}
</style>
