<script setup lang="ts">
/**
 * Диалог подтверждения очистки истории (С-4, дизайн E5, «Очистить —
 * подтверждение») — единственный необратимый шаг эпика. Новый компонент по
 * образцу `ExitConfirmDialog` (визуальный язык, ручная фокус-ловушка), а не
 * переиспользование того компонента с новыми пропсами: разная область
 * ответственности (окно/очередь против истории), разный набор данных и
 * разное событие-получатель (дизайн, «Что решено дизайном» у пункта 2) —
 * общий только визуальный язык и код фокус-ловушки, дешевле скопировать,
 * чем изгибать один компонент под два самостоятельных сценария.
 *
 * Фокус-ловушка и Esc — рукописные по той же причине, что у
 * `ExitConfirmDialog` (jsdom не реализует `HTMLDialogElement.showModal()`).
 */
import { nextTick, onMounted, onUnmounted, ref } from 'vue'

import { getClearHistoryConfirmBody } from '@/utils/historyClearDialogTexts'

const props = defineProps<{
  /**
   * Число уже загруженных на экран записей, если это заведомо все записи
   * целиком (нет `nextCursor` — раздел «Показать ещё» дизайна, «число
   * записей... то же, что уже известно экрану из уже загруженных
   * страниц»); `undefined`, когда список не долистан и точное число
   * неизвестно — тогда текст обходится без числа.
   */
  knownRecordCount?: number
}>()

const emit = defineEmits<{
  cancel: []
  confirm: []
}>()

const cancelButton = ref<HTMLButtonElement>()
const confirmButton = ref<HTMLButtonElement>()

let previouslyFocused: HTMLElement | null = null

onMounted(() => {
  previouslyFocused = document.activeElement instanceof HTMLElement ? document.activeElement : null
  // «Отмена» — кнопка по умолчанию и получает фокус (дизайн: безопасный
  // выбор по умолчанию, тот же приём, что «Остаться» в `ExitConfirmDialog`).
  void nextTick(() => {
    cancelButton.value?.focus()
  })
})

onUnmounted(() => {
  previouslyFocused?.focus()
})

/** Ручная фокус-ловушка: ровно два интерактивных элемента, Tab с последнего переносит на первый и наоборот. */
function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    event.preventDefault()
    emit('cancel')
    return
  }
  if (event.key !== 'Tab') return

  const first = cancelButton.value
  const last = confirmButton.value
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
  <div class="clear-history-confirm-backdrop">
    <div
      class="clear-history-confirm-dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="clear-history-confirm-heading"
      aria-describedby="clear-history-confirm-body"
      @keydown="onKeydown"
    >
      <p
        id="clear-history-confirm-heading"
        class="clear-history-confirm-dialog__heading"
      >
        Очистить всю историю?
      </p>
      <p
        id="clear-history-confirm-body"
        class="clear-history-confirm-dialog__body"
      >
        {{ getClearHistoryConfirmBody(props.knownRecordCount) }}
      </p>
      <div class="clear-history-confirm-dialog__actions">
        <button
          ref="cancelButton"
          type="button"
          class="tap-target clear-history-confirm-dialog__cancel"
          @click="emit('cancel')"
        >
          Отмена
        </button>
        <button
          ref="confirmButton"
          type="button"
          class="tap-target clear-history-confirm-dialog__confirm"
          @click="emit('confirm')"
        >
          Очистить всё
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.clear-history-confirm-backdrop {
  position: fixed;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--color-overlay);
  z-index: 100;
}

.clear-history-confirm-dialog {
  max-width: 28rem;
  margin: 1rem;
  padding: 1.25rem;
  background: var(--color-surface);
  border-radius: 0.5rem;
  box-shadow: 0 0.5rem 1.5rem var(--color-shadow);
}

.clear-history-confirm-dialog__heading {
  margin: 0 0 0.5rem;
  font-weight: 600;
}

.clear-history-confirm-dialog__body {
  margin: 0 0 1rem;
  line-height: 1.4;
  color: var(--color-text-secondary);
}

.clear-history-confirm-dialog__actions {
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
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}

.clear-history-confirm-dialog__cancel {
  font-weight: 600;
  border: 1px solid var(--color-accent);
  background: var(--color-accent);
  color: var(--color-on-accent);
  border-radius: 0.3rem;
}

.clear-history-confirm-dialog__confirm {
  border: none;
  background: transparent;
  color: var(--color-error);
}
</style>
