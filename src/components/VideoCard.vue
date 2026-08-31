<script setup lang="ts">
/**
 * Карточка ролика — успешный результат разбора (дизайн E2, состояние 2).
 * Название рисуется как есть, без обрезания (К-1 требует посимвольного
 * совпадения с YouTube — `white-space: normal` вместо ellipsis).
 * Имя канала и превью не рисуются, если данных нет (Ф-5, Р-2).
 *
 * Превью — `alt=""` (декоративное): название уже показано текстом рядом,
 * иначе скринридер зачитал бы его дважды подряд.
 *
 * # Кнопка «Скачать» (эпик E3, дизайн «Кнопка Скачать»; правка TL-74/TL-75, Ф-2 E4)
 *
 * Живёт здесь же, под лестницей качеств — несколько строк, физически
 * часть той же карточки, не отдельная ui-задача (декомпозиция E3, «Что
 * сознательно не резалось мельче»). Неактивна ровно в одном случае —
 * ничего не выбрано в лестнице качеств (самообъясняющееся состояние,
 * подсказки не требует). Блокировка на случай «уже идёт другая задача»
 * (`downloadBlocked`) и её текст-подсказка — из дизайна E3 — сняты TL-74
 * как прямое следствие Ф-2 E4: постановка при занятом слоте больше не
 * отказ, а нормальный путь (очередь, эпик E4), и превентивно объяснять на
 * карточке нечего. Единственный оставшийся повод отказа — дубль (Ф-8
 * E4) — виден после клика через `DownloadCommandErrorBlock` (TL-75), не
 * здесь.
 *
 * Сама карточка не хранит и не запускает задачу скачивания — она лишь
 * эмитит снимок «что скачивать» в момент клика (заголовок, потоки,
 * подпись качества); что происходит с этим снимком дальше (стор
 * `useDownloadTaskStore`, независимая от карточки панель) её не касается —
 * это и есть требование С-13 «панель переживает замену карточки».
 *
 * `quality` в эмитируемом payload (TL-75, эпик E4) — тот же
 * {@link import('@/types/generated/queue').SelectedQuality}, что уносит
 * `StartDownloadRequest.quality`: два поля выбранного пункта лестницы
 * (`kind`/`heightPx`), без которых заголовок задачи не собрать заново
 * после перезапуска приложения (карточки и лестницы к этому моменту уже
 * нет, см. doc `StartDownloadRequest.quality` в
 * `src/types/generated/download.ts`).
 */
import { computed, ref } from 'vue'

import type { ProbeResult, QualityItem, QualitySize, QualityStreams } from '@/types/generated/probe'
import type { SelectedQuality } from '@/types/generated/queue'
import { formatDuration } from '@/utils/formatDuration'

import QualityLadder from './QualityLadder.vue'
import VideoThumbnail from './VideoThumbnail.vue'

const props = defineProps<{
  result: ProbeResult
}>()

const emit = defineEmits<{
  download: [
    payload: { title: string; streams: QualityStreams; size: QualitySize; quality: SelectedQuality },
  ]
}>()

const selected = ref<QualityItem>()

function onSelect(item: QualityItem | undefined): void {
  selected.value = item
}

/** Единственный оставшийся повод недоступности — ничего не выбрано в лестнице (TL-74, Ф-2 E4). */
const downloadDisabled = computed(() => selected.value === undefined)

function onDownloadClick(): void {
  const item = selected.value
  if (!item) return
  emit('download', {
    title: props.result.title,
    streams: item.streams,
    // Оценка размера того же пункта — стартовый знаменатель агрегации
    // прогресса в ядре (TL-41); берётся отсюда же, где и `streams`, не
    // собирается отдельно (правило контракта `StartDownloadRequest.size`).
    size: item.size,
    quality: { kind: item.kind, heightPx: item.heightPx },
  })
}

</script>

<template>
  <article class="video-card">
    <div class="video-card__top">
      <VideoThumbnail
        v-if="result.thumbnailUrl"
        :src="result.thumbnailUrl"
        alt=""
      />
      <div class="video-card__info">
        <h2 class="video-card__title">
          {{ result.title }}
        </h2>
        <p
          v-if="result.channel"
          class="video-card__channel"
        >
          {{ result.channel }}
        </p>
        <p class="video-card__duration">
          {{ formatDuration(result.durationSecs) }}
        </p>
      </div>
    </div>

    <QualityLadder
      :items="result.qualities"
      @update:selected="onSelect"
    />

    <button
      type="button"
      class="tap-target video-card__download"
      :disabled="downloadDisabled"
      @click="onDownloadClick"
    >
      Скачать
    </button>
  </article>
</template>

<style scoped>
.video-card {
  margin-top: 0.75rem;
}

.video-card__top {
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem;
  align-items: flex-start;
}

.video-card__info {
  min-width: 0;
  flex: 1;
}

.video-card__title {
  margin: 0 0 0.25rem;
  font-size: 1.05rem;
  font-weight: 600;
  line-height: 1.35;
  white-space: normal;
  overflow-wrap: break-word;
}

.video-card__channel {
  margin: 0 0 0.25rem;
  color: #555;
  font-size: 0.9rem;
}

.video-card__duration {
  margin: 0;
  color: #555;
}

.video-card__download {
  margin-top: 0.75rem;
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
