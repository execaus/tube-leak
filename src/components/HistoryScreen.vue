<script setup lang="ts">
/**
 * Экран истории (эпик E5, TL-93; Ф-4…Ф-6, Ф-8, С-1…С-4, С-10; дизайн E5,
 * раздел «2. Экран истории»). Заголовок «История» (фокус-цель переключения
 * вкладок, К-14) остаётся в `App.vue` — этот компонент рисует всё, что
 * дизайн кладёт под ним, тем же разделением ответственности, что
 * `ProbeSection`/`QueueSection` под `<h1>` «Главного».
 *
 * Состояние и все вызовы `invoke` живут в `useHistoryStore` (один Pinia-стор
 * на домен, CLAUDE.md) — компонент только читает срез стора и эмитит клики
 * в его действия, тот же приём, что `QueueSection`/`downloadTaskStore`.
 *
 * # Обновление первой страницы по активации вкладки (Б-2/С-1, правки ревью
 * TL-93, второй раунд)
 *
 * `App.vue` держит все три панели смонтированными постоянно (`v-show`,
 * К-14) — у `HistoryScreen` поэтому нет своего события «монтирования при
 * переходе на вкладку», и решение ведущего подключить обновление первой
 * страницы «при активации вкладки «История»» реализовано пропом
 * `active: boolean` (`App.vue` передаёт `activeTab === 'history'`), а не
 * вторым `onMounted`/`watch(() => ...)` внутри самого `App.vue`, читающим
 * приватности стора извне: компонент, которому нужно действие, сам решает,
 * когда его вызвать, `App.vue` лишь сообщает факт «эта вкладка теперь
 * видна» — тот же приём, что уже передаёт эту секцию через `v-show`.
 */
import { computed, nextTick, onMounted, ref, watch } from 'vue'

import { useHistoryStore } from '@/stores/history'
import type { HistoryEntry } from '@/types/generated/history'
import { getFolderDisplayText } from '@/utils/downloadOutcomeTexts'
import { fileContainer } from '@/utils/fileContainer'
import { formatExactSize } from '@/utils/formatExactSize'
import { formatHistoryMessage } from '@/utils/formatHistoryMessage'
import { formatRelativeTime } from '@/utils/formatRelativeTime'
import {
  getHistoryCommandErrorText,
  NON_CONTRACTUAL_HISTORY_COMMAND_ERROR_TEXT,
} from '@/utils/historyCommandErrorTexts'
import { getHistoryFileStatusText } from '@/utils/historyFileStatusTexts'
import { getHistoryNoticeText } from '@/utils/historyNoticeTexts'
import { getHistoryUnavailableText, HISTORY_IPC_FAILURE_TEXT } from '@/utils/historyUnavailableTexts'
import {
  getLauncherFailureDetails,
  getShowInFolderErrorText,
  HISTORY_ROW_GONE_TEXT,
  NON_CONTRACTUAL_SHOW_IN_FOLDER_ERROR_TEXT,
} from '@/utils/historyShowInFolderTexts'
import { formatTaskDisplayTitle } from '@/utils/queueTaskTitle'
import { unixSecsToIso } from '@/utils/unixSecsToIso'

import ClearHistoryConfirmDialog from './ClearHistoryConfirmDialog.vue'

const props = withDefaults(
  defineProps<{
    /** Активна ли сейчас вкладка «История» (`App.vue`, `activeTab === 'history'`) — см. doc-класса выше. */
    active?: boolean
  }>(),
  {
    // По умолчанию `false` — тесты, которым переход между вкладками
    // безразличен (большинство `HistoryScreen.test.ts`), монтируют
    // компонент без этого пропа вовсе; `App.vue` всегда передаёт его явно.
    active: false,
  },
)

const emit = defineEmits<{
  /**
   * С-7 (правки ревью TL-93, второй раунд): «если записей нет — на
   * заголовок экрана». Заголовок `<h2>История</h2>` — узел `App.vue`
   * (К-14), этот компонент не владеет им и не должен: эмит — то же
   * разделение ответственности, что и у пропа `active` выше, только в
   * обратную сторону (ребёнок просит родителя подвинуть фокус на элемент,
   * которым родитель управляет).
   */
  requestHeadingFocus: []
}>()

const store = useHistoryStore()

onMounted(() => {
  void store.initialize()
})

/**
 * Активация вкладки «История» перезапрашивает первую страницу (решение
 * ведущего, ревью TL-93, второй раунд, п. 7) — `watch`, не `immediate`:
 * самая первая загрузка уже покрыта `onMounted` выше независимо от того,
 * какая вкладка активна изначально («Главный»); здесь важен только
 * переход `false → true`, переход в обратную сторону (уход со вкладки)
 * ничего не запрашивает.
 */
watch(
  () => props.active,
  (isActive) => {
    if (isActive) void store.refreshFirst()
  },
)

function entryMetaLine(entry: HistoryEntry): string {
  const parts = [
    fileContainer(entry.fileName),
    formatExactSize(entry.sizeBytes),
    formatRelativeTime(unixSecsToIso(entry.finishedAtUnixSecs)),
  ].filter((part) => part.length > 0)
  return parts.join(' · ')
}

/** Третья строка записи: путь папки для `present`, статус файла для `missing` (Ф-5, таблица трёх случаев). */
function entryThirdLine(entry: HistoryEntry): string {
  return getHistoryFileStatusText(entry.fileStatus, entry.folderDisplay) ?? getFolderDisplayText(entry.folderDisplay)
}

/** «Показать в папке» — отсутствует ровно для «нет файла, нет папки» (Ф-8, таблица трёх случаев, строка 3). */
function showButtonVisible(entry: HistoryEntry): boolean {
  return entry.fileStatus.kind === 'present' || entry.fileStatus.folderExists
}

function deleteAriaLabel(entry: HistoryEntry): string {
  return `Удалить запись ${formatTaskDisplayTitle(entry.title, entry.quality)} из истории`
}

/** Готовая строка построчной ошибки «Показать в папке» (С-4: форма «Заголовок: пояснение», без двоеточия при пустом пояснении). */
function showInFolderErrorMessage(entry: HistoryEntry): string | undefined {
  const failure = store.showInFolderErrors[entry.id]
  if (!failure) return undefined
  if (failure.kind === 'goneFromHistory') return HISTORY_ROW_GONE_TEXT
  const text = failure.kind === undefined ? NON_CONTRACTUAL_SHOW_IN_FOLDER_ERROR_TEXT : getShowInFolderErrorText(failure)
  return formatHistoryMessage(text.title, text.explanation)
}

function showInFolderErrorDetails(entry: HistoryEntry) {
  const failure = store.showInFolderErrors[entry.id]
  if (!failure || failure.kind === undefined || failure.kind === 'goneFromHistory') return undefined
  return getLauncherFailureDetails(failure)
}

/** Готовая строка баннера отказа `delete_history_record`/`clear_history` (С-4, та же форма). */
const commandErrorMessage = computed(() => {
  const failure = store.commandError
  if (!failure) return undefined
  const text = failure.kind === undefined ? NON_CONTRACTUAL_HISTORY_COMMAND_ERROR_TEXT : getHistoryCommandErrorText(failure)
  return formatHistoryMessage(text.title, text.explanation)
})

/**
 * Число для тела диалога очистки (пункт 2 дизайна, «Очистить —
 * подтверждение»): только когда список точно долистан до конца
 * (`nextCursor` отсутствует) — иначе оно не гарантированно полное.
 */
const knownRecordCountForClear = computed(() => (store.nextCursor ? undefined : store.entries.length))

const showClearConfirm = ref(false)

/**
 * Корень компонента (С-7) — нужен только чтобы после удаления строки
 * найти кнопки «Удалить» уже обновлённого списка (см.
 * {@link focusAfterRowRemoval}); искать их через `document` целиком было
 * бы правильно только в продакшене, но не в модульных тестах, где на
 * странице бывает не один экземпляр экрана подряд.
 */
const rootEl = ref<HTMLElement>()

/**
 * После «Удалить» (С-7, правки ревью TL-93, второй раунд): фокус уходит на
 * «Удалить» следующей записи; если следующей нет — на «Удалить»
 * предыдущей; если записей не осталось вовсе — на заголовок экрана.
 *
 * `removedIndex` — позиция удалённой строки в **старом** списке. После
 * удаления кнопки последующих строк сдвигаются на одну позицию вверх, так
 * что кнопка на той же позиции `removedIndex` в **новом**, уже
 * перерисованном списке — это кнопка ровно той записи, что раньше шла
 * следующей; если удалённая была последней, эта позиция уже вне границ
 * нового (более короткого) списка, и в дело идёт предыдущая позиция.
 */
function focusAfterRowRemoval(removedIndex: number): void {
  const buttons = rootEl.value?.querySelectorAll<HTMLButtonElement>('.history-screen__delete-button')
  if (!buttons || buttons.length === 0) {
    emit('requestHeadingFocus')
    return
  }
  const target = buttons[removedIndex] ?? buttons[removedIndex - 1]
  target?.focus()
}

/**
 * Клик «Удалить» — оборачивает `store.deleteRecord` управлением фокуса
 * (С-7). Строка не считается удалённой только по факту вызова: отказ
 * `writeFailed`/неконтрактный сбой оставляет её на месте (см.
 * `useHistoryStore.deleteRecord`), и тогда фокус не трогается вовсе —
 * кнопка, на которую только что нажали, никуда не делась.
 */
async function onDeleteClick(entry: HistoryEntry): Promise<void> {
  const removedIndex = store.entries.findIndex((e) => e.id === entry.id)
  await store.deleteRecord(entry.id)
  await nextTick()
  const stillThere = store.entries.some((e) => e.id === entry.id)
  if (stillThere || removedIndex === -1) return
  focusAfterRowRemoval(removedIndex)
}

/**
 * Подтверждение «Очистить всё» (С-7): фокус на заголовок экрана — не на
 * то, что вернул бы `ClearHistoryConfirmDialog` сам по себе
 * (`previouslyFocused.focus()` на кнопке «Очистить» toolbar'а), потому
 * что после успешной очистки список пуст и toolbar вместе с той кнопкой
 * пропадает из DOM: `.focus()` на отсоединённом узле не делает ничего, и
 * фокус тихо падает на `<body>` (ровно так тест ревьюера A2 воспроизводил
 * дефект). Явный эмит здесь выполняется **после** того, как диалог уже
 * закрылся и его `onUnmounted` уже попытался восстановить фокус — так что
 * итоговое состояние всегда одно и то же, независимо от гонки между
 * закрытием диалога и ответом `clear_history`.
 */
async function onClearConfirmed(): Promise<void> {
  showClearConfirm.value = false
  await store.clearHistory()
  await nextTick()
  if (store.entries.length === 0) {
    emit('requestHeadingFocus')
  }
}
</script>

<template>
  <div
    ref="rootEl"
    class="history-screen"
  >
    <!--
      Живая зона структурных изменений списка (дизайн E5, «Доступность»):
      «не пересказом содержимого» — короткий текст, не перечитывание всего
      списка. Постоянный узел, не создаваемый по условию (тот же приём, что
      `queue-status-announcer` в `App.vue`): скринридер должен знать про
      регион заранее, иначе первое объявление теряется.
    -->
    <p
      class="visually-hidden history-screen__announcer"
      aria-live="polite"
    >
      {{ store.liveAnnouncement }}
    </p>

    <template v-if="store.availability">
      <p class="history-screen__unavailable">
        {{ getHistoryUnavailableText(store.availability.reason) }}
      </p>
    </template>

    <!--
      С-6 (правки ревью TL-93, второй раунд): исключение самого IPC-вызова
      на первой странице — тоже блокирующее состояние, не «История пуста»
      (см. doc {@link import('@/utils/historyUnavailableTexts').HISTORY_IPC_FAILURE_TEXT}).
    -->
    <template v-else-if="store.ipcFailure">
      <p class="history-screen__unavailable">
        {{ HISTORY_IPC_FAILURE_TEXT }}
      </p>
    </template>

    <template v-else>
      <ul
        v-if="store.notices.length > 0"
        class="history-screen__notices"
      >
        <!--
          `role="status"` — не на самом `<li>` (мелочи правок ревью TL-93,
          второй раунд): роль `status` на элементе списка стирает его
          неявную роль `listitem`, и скринридер перестаёт видеть список как
          список (не сообщает «список, N элементов»/номер элемента). Живая
          зона — на вложенном `<p>` с текстом, `<li>` остаётся обычным
          пунктом.
        -->
        <li
          v-for="notice in store.notices"
          :key="notice.kind"
          class="history-screen__banner"
        >
          <p
            class="history-screen__banner-text"
            role="status"
          >
            ⚠ {{ getHistoryNoticeText(notice) }}
          </p>
          <button
            type="button"
            class="tap-target"
            @click="store.dismissNotice(notice.kind)"
          >
            Скрыть
          </button>
        </li>
      </ul>

      <p
        v-if="commandErrorMessage"
        class="history-screen__command-error"
        role="alert"
      >
        {{ commandErrorMessage }}
        <button
          type="button"
          class="tap-target"
          @click="store.dismissCommandError"
        >
          Скрыть
        </button>
      </p>

      <template v-if="store.loaded">
        <p
          v-if="store.entries.length === 0"
          class="history-screen__empty"
        >
          История пуста. Здесь появятся ролики после первой завершённой загрузки.
        </p>

        <template v-else>
          <div class="history-screen__toolbar">
            <button
              type="button"
              class="tap-target"
              @click="showClearConfirm = true"
            >
              Очистить
            </button>
          </div>

          <ul class="history-screen__list">
            <li
              v-for="entry in store.entries"
              :key="entry.id"
              class="history-screen__entry"
            >
              <p class="history-screen__entry-title">
                {{ formatTaskDisplayTitle(entry.title, entry.quality) }}
              </p>
              <p class="history-screen__entry-meta">
                {{ entryMetaLine(entry) }}
              </p>
              <p class="history-screen__entry-line">
                {{ entryThirdLine(entry) }}
              </p>

              <p
                v-if="showInFolderErrorMessage(entry)"
                class="history-screen__entry-error"
                role="alert"
              >
                {{ showInFolderErrorMessage(entry) }}
              </p>
              <details v-if="showInFolderErrorDetails(entry)">
                <summary>Подробнее</summary>
                <dl class="history-screen__entry-error-details">
                  <template v-if="showInFolderErrorDetails(entry)?.exitCode !== undefined">
                    <dt>Код выхода</dt>
                    <dd>{{ showInFolderErrorDetails(entry)?.exitCode }}</dd>
                  </template>
                  <template v-if="showInFolderErrorDetails(entry)?.stderrTail">
                    <dt>stderr</dt>
                    <dd>{{ showInFolderErrorDetails(entry)?.stderrTail }}</dd>
                  </template>
                </dl>
              </details>

              <div class="history-screen__entry-actions">
                <button
                  v-if="showButtonVisible(entry)"
                  type="button"
                  class="tap-target"
                  @click="store.showInFolder(entry)"
                >
                  Показать в папке
                </button>
                <button
                  type="button"
                  class="tap-target history-screen__delete-button"
                  :aria-label="deleteAriaLabel(entry)"
                  @click="onDeleteClick(entry)"
                >
                  Удалить
                </button>
              </div>
            </li>
          </ul>

          <div
            v-if="store.nextCursor"
            class="history-screen__load-more"
          >
            <button
              type="button"
              class="tap-target"
              :disabled="store.isLoadingMore"
              @click="store.loadMore"
            >
              {{ store.isLoadingMore ? 'Загружаем…' : 'Показать ещё' }}
            </button>
          </div>
        </template>
      </template>
      <p
        v-else
        class="history-screen__loading"
        aria-live="polite"
      >
        Загружаем историю…
      </p>
    </template>

    <ClearHistoryConfirmDialog
      v-if="showClearConfirm"
      :known-record-count="knownRecordCountForClear"
      @cancel="showClearConfirm = false"
      @confirm="onClearConfirmed"
    />
  </div>
</template>

<style scoped>
/*
 * Скрыт визуально, виден скринридеру (`position: absolute` + `clip`, не
 * `display: none`/`visibility: hidden` — те убрали бы узел из дерева
 * доступности вместе с текстом). Тот же приём, что `.visually-hidden` в
 * `App.vue`/`SidecarStatusRow.vue` — не общий класс между файлами
 * (`<style scoped>` в каждом компоненте), а не дублирование намеренно.
 */
.visually-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}

.history-screen__unavailable {
  max-width: 40rem;
  line-height: 1.4;
}

.history-screen__notices {
  list-style: none;
  margin: 0 0 0.75rem;
  padding: 0;
}

.history-screen__banner {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.75rem;
  padding: 0.75rem;
  margin-bottom: 0.5rem;
  background: var(--color-accent-soft);
  border-radius: 0.3rem;
  box-shadow: inset 0 0 0 1px var(--color-banner-border);
}

.history-screen__banner-text {
  margin: 0;
  color: var(--color-text-secondary);
}

.history-screen__command-error {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin: 0 0 0.75rem;
  color: var(--color-error);
}

.history-screen__empty,
.history-screen__loading {
  color: var(--color-text-muted);
}

.history-screen__toolbar {
  display: flex;
  justify-content: flex-end;
  margin-bottom: 0.5rem;
}

.history-screen__list {
  list-style: none;
  margin: 0;
  padding: 0;
}

.history-screen__entry {
  padding: 0.75rem 0;
  border-bottom: 1px solid var(--color-border-subtle);
}

.history-screen__entry-title {
  margin: 0 0 0.25rem;
  font-weight: 600;
}

.history-screen__entry-meta {
  margin: 0 0 0.25rem;
  color: var(--color-text-muted);
}

.history-screen__entry-line {
  margin: 0 0 0.5rem;
  color: var(--color-text-secondary);
  word-break: break-word;
}

.history-screen__entry-error {
  margin: 0 0 0.5rem;
  color: var(--color-error);
}

.history-screen__entry-actions {
  display: flex;
  gap: 0.5rem;
}

.history-screen__load-more {
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
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}
</style>
