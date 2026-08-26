<script setup lang="ts">
/**
 * Диалог подтверждения выхода при активной загрузке (Р-2, эпик E3, TL-46,
 * `role="alertdialog"`).
 *
 * Чисто презентационный компонент — видимость и различение «выйти»/
 * «отменить» решает `useExitConfirmation`, сюда приходят уже готовые пропсы
 * и клики только эмитятся, тот же приём, что `DownloadPanel`/
 * `DownloadCommandErrorBlock` (эпик E3, TL-45).
 *
 * # Фокус-ловушка и Esc — рукописные, не нативный `<dialog>`
 *
 * `HTMLDialogElement.showModal()` дал бы то же самое бесплатно в реальном
 * WebView, но jsdom (тестовое окружение проекта, `vitest.config.ts`) не
 * реализует `showModal` — компонент на нём просто упал бы, и требуемые
 * критерием приёмки тесты фокус-ловушки и Esc стали бы невозможны в
 * юнит-окружении целиком, а не частично. Рукописный трап на `keydown`
 * тестируется в jsdom как обычные события клавиатуры и фокуса.
 */
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'

import type { DownloadProgress } from '@/types/download'
import { getExitDialogText } from '@/utils/exitDialogTexts'

const props = defineProps<{
  displayTitle: string
  progress: DownloadProgress
}>()

const emit = defineEmits<{
  stay: []
  exitAnyway: []
}>()

const text = computed(() => getExitDialogText(props.displayTitle, props.progress))

const stayButton = ref<HTMLButtonElement>()
const exitButton = ref<HTMLButtonElement>()

let previouslyFocused: HTMLElement | null = null

onMounted(() => {
  previouslyFocused = document.activeElement instanceof HTMLElement ? document.activeElement : null
  // «Остаться» — кнопка по умолчанию и получает фокус (дизайн:
  // безопасный выбор по умолчанию, как и Esc/Enter).
  void nextTick(() => {
    stayButton.value?.focus()
  })
})

onUnmounted(() => {
  previouslyFocused?.focus()
})

/**
 * Ручная фокус-ловушка: в диалоге ровно два интерактивных элемента, Tab с
 * последнего переносит на первый и наоборот — без стороннего кода.
 */
function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    event.preventDefault()
    emit('stay')
    return
  }
  if (event.key !== 'Tab') return

  const first = stayButton.value
  const last = exitButton.value
  if (!first || !last) return

  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault()
    last.focus()
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault()
    first.focus()
  }
}
</script>

<template>
  <div class="exit-confirm-backdrop">
    <div
      class="exit-confirm-dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="exit-confirm-heading"
      aria-describedby="exit-confirm-body"
      @keydown="onKeydown"
    >
      <p
        id="exit-confirm-heading"
        class="exit-confirm-dialog__heading"
      >
        {{ text.heading }}
      </p>
      <p
        id="exit-confirm-body"
        class="exit-confirm-dialog__body"
      >
        {{ text.body }}
      </p>
      <div class="exit-confirm-dialog__actions">
        <button
          ref="stayButton"
          type="button"
          class="tap-target exit-confirm-dialog__stay"
          @click="emit('stay')"
        >
          Остаться
        </button>
        <button
          ref="exitButton"
          type="button"
          class="tap-target exit-confirm-dialog__exit"
          @click="emit('exitAnyway')"
        >
          Всё равно выйти
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.exit-confirm-backdrop {
  position: fixed;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.35);
  z-index: 100;
}

.exit-confirm-dialog {
  max-width: 28rem;
  margin: 1rem;
  padding: 1.25rem;
  background: #fff;
  border-radius: 0.5rem;
  box-shadow: 0 0.5rem 1.5rem rgba(0, 0, 0, 0.25);
}

.exit-confirm-dialog__heading {
  margin: 0 0 0.5rem;
  font-weight: 600;
}

.exit-confirm-dialog__body {
  margin: 0 0 1rem;
  line-height: 1.4;
  color: #333;
}

.exit-confirm-dialog__actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
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

.exit-confirm-dialog__stay {
  font-weight: 600;
  border: 1px solid #1a73e8;
  background: #1a73e8;
  color: #fff;
  border-radius: 0.3rem;
}

.exit-confirm-dialog__exit {
  border: none;
  background: transparent;
  color: #b3261e;
}
</style>
