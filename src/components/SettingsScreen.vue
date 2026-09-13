<script setup lang="ts">
/**
 * Экран настроек (эпик E5, TL-94; Ф-9…Ф-13, С-5…С-9; дизайн E5, раздел
 * «3. Экран настроек»). Заголовок «Настройки» (фокус-цель переключения
 * вкладок, К-14) остаётся в `App.vue` — этот компонент рисует всё, что
 * дизайн кладёт под ним (тот же приём, что `HistoryScreen.vue`).
 *
 * Три поля, у каждого свой независимый цикл «изменить → сохранить/сбросить»
 * (дизайн, п. 3, «Что решено дизайном»): папка сохраняется сразу по выбору
 * диалогом, шаблон и число попыток — по кнопке «Сохранить» у поля.
 * Черновой ввод (текст шаблона, текст числа попыток) живёт здесь, а не в
 * `useSettingsStore` — стор несёт только то, что уже подтверждено ядром
 * (doc-класс стора, «Один Pinia-стор на домен»).
 *
 * # Обновление по активации вкладки (по образцу `HistoryScreen.vue`)
 *
 * `App.vue` держит все три панели смонтированными постоянно (`v-show`,
 * К-14) — проп `active: boolean` сообщает компоненту факт «эта вкладка
 * теперь видна», а решение перезапросить настройки принимает сам компонент
 * (тот же приём, что и у `HistoryScreen.vue`, doc-класс там же).
 */
import { computed, onMounted, ref, watch } from 'vue'

import { pickDestinationFolder } from '@/composables/usePickFolder'
import { useSettingsStore } from '@/stores/settings'
import type { SettingsField } from '@/types/generated/settings'
import { clampAttempts, parseAttemptsInput } from '@/utils/parseAttemptsInput'
import { validateNameTemplateDraft } from '@/utils/nameTemplateClientCheck'
import {
  getAttemptsSaveErrorText,
  getDestinationFolderPathText,
  getFolderSaveErrorText,
  getTemplateProblemText,
  getTemplateSaveErrorText,
  PREVIEW_UNAVAILABLE_TEXT,
} from '@/utils/settingsFieldTexts'

const props = withDefaults(defineProps<{ active?: boolean }>(), { active: false })

const store = useSettingsStore()

onMounted(() => {
  void store.fetchSettings()
})

watch(
  () => props.active,
  (isActive) => {
    if (isActive) void store.fetchSettings()
  },
)

/** Пометка «сброшено к умолчанию» у конкретного поля (дизайн, п. 3) — не показывается вместе с общим баннером `wholeFileReset`, у которого пострадал сам факт чтения, а не отдельное значение. */
function isFieldReset(field: SettingsField): boolean {
  return !store.wholeFileReset && store.resetFields.includes(field)
}

const FIELD_RESET_BADGE_TEXT = 'Сброшено к значению по умолчанию: сохранённое значение было недопустимым.'

// --- Папка назначения (Ф-11, С-5, С-6) ---------------------------------

const folderPathText = computed(() => (store.settings ? getDestinationFolderPathText(store.settings.destinationFolder) : ''))
const canResetFolder = computed(() => store.settings?.destinationFolder.kind === 'custom')
const folderMissingWarningVisible = computed(() => store.settings !== undefined && !store.destinationFolderExists)
const folderLengthWarningVisible = computed(() => {
  const folder = store.settings?.destinationFolder
  return folder !== undefined && folder.kind === 'custom' && folder.path.length > 200
})
const folderErrorText = computed(() => (store.folderError ? getFolderSaveErrorText(store.folderError) : undefined))

async function onPickFolder(): Promise<void> {
  const path = await pickDestinationFolder()
  if (path === null) return
  await store.setDestinationFolder({ kind: 'custom', path })
}

async function onResetFolder(): Promise<void> {
  await store.setDestinationFolder({ kind: 'system' })
}

// --- Шаблон имени файла (Ф-12, С-7) -------------------------------------

const nameTemplateDraft = ref('')

/**
 * Синхронизация черновика с ядром (первая загрузка `settings_get`, успешный
 * `settings_set`/сброс) — предпросмотр запрашивается **немедленно**
 * (`{immediate: true}`), не через debounce набора текста ниже: это не
 * пользовательский ввод, а уже подтверждённое значение, и откладывать вызов
 * не за чем (см. doc {@link import('@/stores/settings').useSettingsStore},
 * «Живой пример», и doc `requestPreview` — там же цена немедленного вызова
 * здесь, а не по таймеру).
 */
watch(
  () => store.settings?.nameTemplate,
  (value) => {
    if (value === undefined) return
    nameTemplateDraft.value = value
    store.requestPreview(value, { immediate: true })
  },
  { immediate: true },
)

/**
 * Единственный путь пользовательского ввода в поле шаблона — ручная
 * привязка, не `v-model`+`watch` (тот же приём, что у `onAttemptsInput`):
 * так «программная» синхронизация выше и «набор текста» здесь не могут
 * задвоить вызов `requestPreview` на одно и то же значение — они физически
 * разные обработчики, а не один наблюдатель за той же переменной.
 */
function onNameTemplateInput(event: Event): void {
  const value = (event.target as HTMLInputElement).value
  nameTemplateDraft.value = value
  store.requestPreview(value)
}

const templateClientProblem = computed(() => validateNameTemplateDraft(nameTemplateDraft.value))

const canSaveTemplate = computed(
  () => store.settings !== undefined && nameTemplateDraft.value !== store.settings.nameTemplate && templateClientProblem.value === undefined,
)

/** Живой пример (дизайн, «Шаблон имени») — приоритет: ошибка последнего ответа `preview_name_template`, затем недоступность, затем успешный результат; пока ответа ещё не было — ничего не показывается. */
const previewLine = computed<string | undefined>(() => {
  if (store.previewProblem) return getTemplateProblemText(store.previewProblem)
  if (store.previewUnavailable) return PREVIEW_UNAVAILABLE_TEXT
  if (store.previewResult !== undefined) return `Пример: «${store.previewResult}»`
  return undefined
})

const templateErrorText = computed(() => (store.templateError ? getTemplateSaveErrorText(store.templateError) : undefined))

async function onSaveTemplate(): Promise<void> {
  if (!canSaveTemplate.value) return
  await store.setNameTemplate(nameTemplateDraft.value)
}

async function onResetTemplate(): Promise<void> {
  if (!store.defaults) return
  await store.setNameTemplate(store.defaults.nameTemplate)
}

// --- Число попыток (Ф-13, С-8) ------------------------------------------

const attemptsDraft = ref('')

/**
 * Ручная привязка вместо `v-model` (мутация ревью — сломался бы молча):
 * компилятор Vue 3 у `<input v-model>` со **статическим** `type="number"`
 * сам подставляет модификатор `.number` (`looseToNumber`) — обратная сторона
 * того самого поведения, которое отправляет `null`/`NaN` в `settings_set`
 * при пустом или нечисловом вводе, ровно то, что запрещает добавка к
 * критерию ревью TL-83. `:value`/`@input` читают `event.target.value` как
 * есть — та самая строка, которую разбирает `parseAttemptsInput.ts`.
 */
function onAttemptsInput(event: Event): void {
  attemptsDraft.value = (event.target as HTMLInputElement).value
}

watch(
  () => store.settings?.maxAttempts,
  (value) => {
    if (value !== undefined) attemptsDraft.value = String(value)
  },
  { immediate: true },
)

const parsedAttemptsDraft = computed(() => parseAttemptsInput(attemptsDraft.value))

/** Гейт «Сохранить» (добавка к критерию ревью TL-83): пустое, нечисловое и дробное значение не отправляются вовсе. */
const canSaveAttempts = computed(
  () => store.settings !== undefined && parsedAttemptsDraft.value !== undefined && parsedAttemptsDraft.value !== store.settings.maxAttempts,
)

const attemptsErrorText = computed(() => (store.attemptsError ? getAttemptsSaveErrorText(store.attemptsError) : undefined))

const canDecrementAttempts = computed(() => parsedAttemptsDraft.value === undefined || parsedAttemptsDraft.value > 1)
const canIncrementAttempts = computed(() => parsedAttemptsDraft.value === undefined || parsedAttemptsDraft.value < 20)

function stepAttempts(delta: number): void {
  const base = parsedAttemptsDraft.value ?? store.settings?.maxAttempts ?? 1
  attemptsDraft.value = String(clampAttempts(base + delta))
}

async function onSaveAttempts(): Promise<void> {
  const parsed = parsedAttemptsDraft.value
  if (parsed === undefined) return
  await store.setMaxAttempts(parsed)
}

async function onResetAttempts(): Promise<void> {
  if (!store.defaults) return
  await store.setMaxAttempts(store.defaults.maxAttempts)
}
</script>

<template>
  <div class="settings-screen">
    <template v-if="store.ipcFailure">
      <p class="settings-screen__unavailable">
        Настройки сейчас недоступны: не удалось обратиться к ядру. Загрузки продолжат работать на прежних значениях.
      </p>
    </template>

    <template v-else-if="!store.loaded">
      <p class="settings-screen__loading">
        Загружаем настройки…
      </p>
    </template>

    <template v-else>
      <p class="settings-screen__intro">
        Папка, шаблон и число попыток применяются к новым и ещё не начатым задачам очереди; та, что качается прямо
        сейчас, доводится по прежним значениям (см. подсказки под каждым полем).
      </p>

      <p
        v-if="store.wholeFileReset"
        class="settings-screen__whole-file-reset"
        role="status"
      >
        ⚠ Настройки сброшены к умолчаниям: файл не читался. Ниже — значения по умолчанию; при первом сохранении
        любого поля файл будет записан заново.
      </p>

      <!-- Папка назначения (Ф-11, С-5, С-6) -->
      <section class="settings-screen__field">
        <h3>Папка назначения</h3>
        <p
          v-if="isFieldReset('destinationFolder')"
          class="settings-screen__field-reset"
        >
          ⓘ {{ FIELD_RESET_BADGE_TEXT }}
        </p>
        <p class="settings-screen__folder-path">
          {{ folderPathText }}
        </p>
        <p
          v-if="folderMissingWarningVisible"
          class="settings-screen__warning"
        >
          ⚠ Папки сейчас нет — подключите диск или выберите другую папку.
        </p>
        <p
          v-if="folderLengthWarningVisible"
          class="settings-screen__warning"
        >
          Путь длиннее 200 символов — для некоторых сторонних программ вместе с именем файла это может превысить
          ограничение Windows на длину пути.
        </p>
        <p
          v-if="folderErrorText"
          class="settings-screen__error"
          role="alert"
        >
          {{ folderErrorText }}
        </p>
        <div class="settings-screen__actions">
          <button
            type="button"
            class="tap-target"
            :disabled="store.folderSaving"
            @click="onPickFolder"
          >
            Выбрать папку…
          </button>
          <button
            v-if="canResetFolder"
            type="button"
            class="tap-target"
            :disabled="store.folderSaving"
            @click="onResetFolder"
          >
            Сбросить к «Загрузки»
          </button>
        </div>
      </section>

      <!-- Шаблон имени файла (Ф-12, С-7) -->
      <section class="settings-screen__field">
        <h3>
          <label for="settings-template-input">Шаблон имени файла</label>
        </h3>
        <p
          v-if="isFieldReset('nameTemplate')"
          class="settings-screen__field-reset"
        >
          ⓘ {{ FIELD_RESET_BADGE_TEXT }}
        </p>
        <div class="settings-screen__actions">
          <input
            id="settings-template-input"
            type="text"
            :value="nameTemplateDraft"
            aria-describedby="settings-template-help settings-template-error"
            :aria-invalid="templateErrorText !== undefined"
            @input="onNameTemplateInput"
          >
          <button
            type="button"
            class="tap-target"
            :disabled="!canSaveTemplate || store.templateSaving"
            @click="onSaveTemplate"
          >
            Сохранить
          </button>
          <button
            type="button"
            class="tap-target"
            :disabled="store.templateSaving"
            @click="onResetTemplate"
          >
            Сбросить
          </button>
        </div>
        <p
          id="settings-template-help"
          class="settings-screen__hint"
        >
          Доступно: {title}, {id}, {quality}, {date}.
        </p>
        <p
          v-if="previewLine"
          class="settings-screen__preview"
        >
          {{ previewLine }}
        </p>
        <p
          v-if="templateErrorText"
          id="settings-template-error"
          class="settings-screen__error"
          role="alert"
        >
          ✕ {{ templateErrorText }}
        </p>
      </section>

      <!-- Число попыток (Ф-13, С-8) -->
      <section class="settings-screen__field">
        <h3>
          <label for="settings-attempts-input">Число попыток</label>
        </h3>
        <p
          v-if="isFieldReset('maxAttempts')"
          class="settings-screen__field-reset"
        >
          ⓘ {{ FIELD_RESET_BADGE_TEXT }}
        </p>
        <div class="settings-screen__actions">
          <input
            id="settings-attempts-input"
            type="number"
            min="1"
            max="20"
            step="1"
            :value="attemptsDraft"
            aria-describedby="settings-attempts-help settings-attempts-error"
            :aria-invalid="attemptsErrorText !== undefined"
            @input="onAttemptsInput"
          >
          <button
            type="button"
            class="tap-target"
            aria-label="Уменьшить число попыток"
            :disabled="!canDecrementAttempts"
            @click="stepAttempts(-1)"
          >
            −
          </button>
          <button
            type="button"
            class="tap-target"
            aria-label="Увеличить число попыток"
            :disabled="!canIncrementAttempts"
            @click="stepAttempts(1)"
          >
            +
          </button>
          <button
            type="button"
            class="tap-target"
            :disabled="!canSaveAttempts || store.attemptsSaving"
            @click="onSaveAttempts"
          >
            Сохранить
          </button>
          <button
            type="button"
            class="tap-target"
            :disabled="store.attemptsSaving"
            @click="onResetAttempts"
          >
            Сбросить
          </button>
        </div>
        <p
          id="settings-attempts-help"
          class="settings-screen__hint"
        >
          От 1 до 20. Чем больше, тем дольше приложение будет пытаться на плохой сети (до ≈16 минут при 20); чем
          меньше — тем быстрее честно сдаётся.
        </p>
        <p
          v-if="attemptsErrorText"
          id="settings-attempts-error"
          class="settings-screen__error"
          role="alert"
        >
          ✕ {{ attemptsErrorText }}
        </p>
      </section>
    </template>
  </div>
</template>

<style scoped>
.settings-screen__unavailable,
.settings-screen__loading {
  max-width: 40rem;
  line-height: 1.4;
  color: var(--color-text-muted);
}

.settings-screen__intro {
  max-width: 40rem;
  line-height: 1.4;
  color: var(--color-text-secondary);
}

.settings-screen__whole-file-reset {
  padding: 0.75rem;
  margin: 0 0 1rem;
  background: var(--color-accent-soft);
  border-radius: 0.3rem;
  box-shadow: inset 0 0 0 1px var(--color-banner-border);
}

.settings-screen__field {
  margin: 1.5rem 0;
  padding-bottom: 1rem;
  border-bottom: 1px solid var(--color-border-subtle);
}

.settings-screen__field h3 {
  margin: 0 0 0.5rem;
}

.settings-screen__field-reset {
  margin: 0 0 0.5rem;
  color: var(--color-text-muted);
}

.settings-screen__folder-path {
  margin: 0 0 0.5rem;
  word-break: break-word;
}

.settings-screen__warning {
  margin: 0 0 0.5rem;
  color: var(--color-text-secondary);
}

.settings-screen__error {
  margin: 0.5rem 0 0;
  color: var(--color-error);
}

.settings-screen__hint {
  margin: 0.5rem 0 0;
  color: var(--color-text-muted);
}

.settings-screen__preview {
  margin: 0.5rem 0 0;
  color: var(--color-text-secondary);
}

.settings-screen__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem;
}

.settings-screen__actions input[type='text'] {
  min-height: 40px;
  padding: 0.4rem 0.6rem;
  box-sizing: border-box;
  flex: 1 1 16rem;
}

.settings-screen__actions input[type='number'] {
  min-height: 40px;
  width: 5rem;
  padding: 0.4rem 0.6rem;
  box-sizing: border-box;
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
