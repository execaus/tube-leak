<script setup lang="ts">
/**
 * Экран отказа подготовки yt-dlp (TL-17): что произошло и что делать,
 * вместо бесконечного «загрузка». Решение — по типизированному `kind`
 * ошибки (CLAUDE.md, «Ошибки типизированные, не строки»), `message` —
 * диагностика в свёрнутом по умолчанию блоке «Подробнее», не для решения.
 *
 * `error` — не обязательно контрактный {@link YtDlpPrepareError}: реджект
 * может оказаться неконтрактным (паника команды, отказ IPC при подписке
 * на событие), и тогда `kind` не заполнен ({@link PrepareFailure} из
 * `useYtDlpPrepare`). Для таких случаев ниже есть отдельное общее
 * объяснение — молчаливый пустой текст был бы хуже честного «не удалось
 * разобрать причину» (ревью TL-17, #18, «Обязательно»).
 *
 * Только отображает то, что передали props, и сообщает о клике «Повторить»
 * наверх — сам `invoke` не вызывает.
 */
import { computed, ref } from 'vue'

import type { PrepareFailure } from '@/composables/useYtDlpPrepare'
import type { YtDlpPrepareErrorKind } from '@/types/generated/ytdlp'

const props = defineProps<{
  error: PrepareFailure
}>()

defineEmits<{
  retry: []
}>()

const detailsOpen = ref(false)

function toggleDetails(): void {
  detailsOpen.value = !detailsOpen.value
}

const explanations: Record<YtDlpPrepareErrorKind, string> = {
  dataDirUnavailable:
    'Не удалось создать рабочий каталог приложения. Проверьте, что диск, на котором установлен ' +
    'tube-leak, доступен для записи, и что права доступа не ограничены, затем попробуйте снова.',
  archiveMissing:
    'В установке tube-leak отсутствует часть с yt-dlp — похоже, установка повреждена. ' +
    'Переустановите tube-leak.',
  archiveCorrupted: 'Файл с yt-dlp в установке tube-leak повреждён. Переустановите tube-leak.',
  notEnoughSpace:
    'На диске не хватает места, чтобы распаковать yt-dlp. В отличие от других причин отказа ' +
    'распаковки, эта не требует переустановки: освободите место — сколько нужно и сколько ' +
    'свободно сейчас, указано ниже, в «Подробнее», — и нажмите «Повторить».',
  unpackFailed:
    'Не удалось распаковать yt-dlp — обычно это нехватка места на диске или прав на запись. ' +
    'Освободите место или проверьте права и попробуйте снова.',
  layoutUnexpected:
    'После распаковки yt-dlp не нашёлся ожидаемый исполняемый файл — похоже, установка ' +
    'повреждена. Переустановите tube-leak.',
  warmupFailed:
    'yt-dlp распаковался, но не запускается. Попробуйте ещё раз; если не поможет — ' +
    'переустановите tube-leak.',
}

/**
 * Фоллбэк для всего, что не подошло под контрактные шесть причин: реджект
 * не по контракту не должен рендерить пустую строку (ревью TL-17, #18).
 */
const FALLBACK_EXPLANATION =
  'Не удалось разобрать причину отказа. Попробуйте ещё раз; если не поможет — переустановите tube-leak.'

const explanation = computed(() => {
  const kind = props.error.kind
  return kind !== undefined ? explanations[kind] : FALLBACK_EXPLANATION
})

const kindLabel = computed(() => props.error.kind ?? 'неизвестно (не по контракту)')
</script>

<template>
  <section
    class="prepare-error"
    role="alert"
    aria-live="assertive"
  >
    <p class="prepare-error__title">
      Не удалось подготовить yt-dlp
    </p>
    <p class="prepare-error__explanation">
      {{ explanation }}
    </p>

    <div class="prepare-error__actions">
      <button
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
      class="prepare-error__details"
    >
      <dt>Код</dt>
      <dd>{{ kindLabel }}</dd>
      <dt>Сообщение</dt>
      <dd>{{ props.error.message }}</dd>
    </dl>
  </section>
</template>

<style scoped>
.prepare-error {
  padding: 1rem 0;
}

.prepare-error__title {
  margin: 0 0 0.5rem;
  font-weight: 600;
}

.prepare-error__explanation {
  margin: 0 0 0.75rem;
  max-width: 40rem;
  line-height: 1.4;
}

.prepare-error__actions {
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

.prepare-error__details {
  margin: 0.75rem 0 0;
  padding: 0.5rem;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.8rem;
  background: rgba(0, 0, 0, 0.04);
  white-space: pre-wrap;
  word-break: break-word;
}

.prepare-error__details dt {
  font-weight: 600;
}

.prepare-error__details dd {
  margin: 0 0 0.5rem;
}
</style>
