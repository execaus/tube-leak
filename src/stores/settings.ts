import { invoke } from '@tauri-apps/api/core'
import { defineStore } from 'pinia'
import { ref } from 'vue'

import type {
  DestinationFolder,
  Settings,
  SettingsField,
  SettingsView,
  TemplatePreview,
  TemplateProblem,
} from '@/types/generated/settings'
import { getPreviewFailureDisplay } from '@/utils/settingsFieldTexts'
import { toSettingsCommandFailure, type SettingsCommandFailure } from '@/utils/settingsCommandFailure'

const SETTINGS_GET_COMMAND = 'settings_get'
const SETTINGS_SET_COMMAND = 'settings_set'
const PREVIEW_NAME_TEMPLATE_COMMAND = 'preview_name_template'

/**
 * Задержка перед вызовом `preview_name_template` после изменения черновика
 * шаблона (дизайн E5, «Шаблон имени»: «с небольшой задержкой (порядка
 * нескольких сотен миллисекунд после последнего нажатия)») — то же число,
 * что и debounce разбора ссылки в `useProbe.ts` (`DEBOUNCE_MS`), не общая
 * константа между файлами: совпадение числа не делает их одним понятием
 * (один — задержка перед запуском yt-dlp, другой — перед вызовом
 * предпросмотра имени, они не обязаны меняться синхронно).
 */
const PREVIEW_DEBOUNCE_MS = 400

/**
 * Домен «настройки» (эпик E5, TL-94; Ф-9…Ф-13, С-5…С-9) — один Pinia-стор
 * на домен (CLAUDE.md), отдельный от `history.ts`/`downloadTask.ts`. Ядро —
 * источник истины (Ф-9: «настройки — состояние в Rust»): стор — проекция
 * последнего ответа `settings_get`/`settings_set`, не накопитель черновиков
 * — черновой ввод (текст шаблона, текст числа попыток) живёт в
 * `SettingsScreen.vue`, а не здесь (тот же приём, что url в `useProbe.ts`
 * живёт в композабле, а не в сторе очереди).
 *
 * # Один файл настроек — успешное сохранение любого поля гасит обе пометки
 *
 * `resetFields`/`wholeFileReset` **не** держатся собственным состоянием
 * поверх ответа команд — стор просто отражает то, что вернула последняя
 * `settings_get`/успешный `settings_set` (дизайн E5, п. 3: «в успешном
 * ответе `settings_set` они всегда пусты и `false`» — это гарантия
 * контракта, не то, что должен обеспечивать клиент).
 *
 * # Живой пример — debounce плюс сторож по поколениям
 *
 * {@link requestPreview} — тот же приём, что `useLinkProbe` в `useProbe.ts`
 * (`generation`, TL-27 review, К-4): гонка быстрого ввода не должна
 * позволить более раннему ответу `preview_name_template` перезаписать уже
 * показанный результат более позднего запроса. Счётчик увеличивается на
 * каждый вызов {@link requestPreview}, и результат применяется, только
 * если он всё ещё совпадает с текущим на момент разрешения промиса.
 */
export const useSettingsStore = defineStore('settings', () => {
  const settings = ref<Settings>()
  const defaults = ref<Settings>()
  const resetFields = ref<SettingsField[]>([])
  const wholeFileReset = ref(false)
  const destinationFolderExists = ref(true)

  /** `true` после самого первого ответа `settings_get` (успешного или нет) — гейт «загружаем» на экране. */
  const loaded = ref(false)
  /** Отказ `settings_get`, не различающий причину (контракт не описывает Result для этой команды) — неконтрактный сбой самого IPC-вызова. */
  const ipcFailure = ref(false)

  const folderError = ref<SettingsCommandFailure>()
  const folderSaving = ref(false)
  const templateError = ref<SettingsCommandFailure>()
  const templateSaving = ref(false)
  const attemptsError = ref<SettingsCommandFailure>()
  const attemptsSaving = ref(false)

  const previewResult = ref<string>()
  const previewProblem = ref<TemplateProblem>()
  const previewUnavailable = ref(false)

  let previewGeneration = 0
  let previewDebounceTimer: ReturnType<typeof setTimeout> | undefined

  function applyView(view: SettingsView): void {
    settings.value = view.settings
    defaults.value = view.defaults
    resetFields.value = view.resetFields
    wholeFileReset.value = view.wholeFileReset
    destinationFolderExists.value = view.destinationFolderExists
  }

  /** Разовое чтение настроек — вызывается из `SettingsScreen.vue` при монтировании и при активации вкладки (дизайн E5, «Данные для API», по образцу `HistoryScreen`/`history_page`). */
  async function fetchSettings(): Promise<void> {
    try {
      const view = await invoke<SettingsView>(SETTINGS_GET_COMMAND)
      applyView(view)
      ipcFailure.value = false
    } catch (err) {
      console.error('settings_get rejected', err)
      ipcFailure.value = true
    } finally {
      loaded.value = true
    }
  }

  function clearPreviewDebounce(): void {
    if (previewDebounceTimer !== undefined) {
      clearTimeout(previewDebounceTimer)
      previewDebounceTimer = undefined
    }
  }

  async function dispatchPreview(myGeneration: number, template: string): Promise<void> {
    try {
      const preview = await invoke<TemplatePreview>(PREVIEW_NAME_TEMPLATE_COMMAND, { template })
      if (myGeneration !== previewGeneration) return
      previewResult.value = preview.result
      previewProblem.value = undefined
      previewUnavailable.value = false
    } catch (err) {
      if (myGeneration !== previewGeneration) return
      const failure = toSettingsCommandFailure(err)
      const display = getPreviewFailureDisplay(failure)
      if (display.kind === 'problem') {
        previewProblem.value = display.problem
        previewResult.value = undefined
        previewUnavailable.value = false
      } else {
        previewUnavailable.value = true
        previewResult.value = undefined
        previewProblem.value = undefined
      }
    }
  }

  /**
   * Живой пример по черновому (ещё не сохранённому) шаблону — не пишет файл
   * (Ф-12, дизайн «Шаблон имени»). По умолчанию — debounce
   * `PREVIEW_DEBOUNCE_MS`, вызывается из `SettingsScreen.vue` на каждое
   * пользовательское изменение поля.
   *
   * `immediate: true` — для программной синхронизации черновика с уже
   * подтверждённым ядром значением (первая загрузка настроек, успешный
   * `settings_set`/сброс), а не с вводом пользователя: без немедленного
   * вызова здесь этот путь всё равно поставил бы в очередь таймер на
   * `PREVIEW_DEBOUNCE_MS`, который сработал бы позже, в произвольный момент
   * относительно остального теста/сценария — ровно то, что уже один раз
   * столкнуло этот стор с чужим `mockImplementationOnce` в
   * `App.tabs.test.ts` (мутация ревью: убрать `immediate` — тот тест
   * снова красный).
   */
  function requestPreview(template: string, opts: { immediate?: boolean } = {}): void {
    clearPreviewDebounce()
    previewGeneration += 1
    const myGeneration = previewGeneration
    if (opts.immediate) {
      void dispatchPreview(myGeneration, template)
    } else {
      previewDebounceTimer = setTimeout(() => {
        void dispatchPreview(myGeneration, template)
      }, PREVIEW_DEBOUNCE_MS)
    }
  }

  /** Сохранение папки назначения (Ф-11) — по успешному выбору диалога либо по «Сбросить» (`{kind:'system'}`), оба мгновенно, без черновика. */
  async function setDestinationFolder(folder: DestinationFolder): Promise<boolean> {
    folderSaving.value = true
    try {
      const view = await invoke<SettingsView>(SETTINGS_SET_COMMAND, { destinationFolder: folder })
      applyView(view)
      folderError.value = undefined
      return true
    } catch (err) {
      folderError.value = toSettingsCommandFailure(err)
      return false
    } finally {
      folderSaving.value = false
    }
  }

  /** Сохранение шаблона имени (Ф-12) — по клику «Сохранить»/«Сбросить» в `SettingsScreen.vue`, окончательная проверка на сервере. */
  async function setNameTemplate(template: string): Promise<boolean> {
    templateSaving.value = true
    try {
      const view = await invoke<SettingsView>(SETTINGS_SET_COMMAND, { nameTemplate: template })
      applyView(view)
      templateError.value = undefined
      return true
    } catch (err) {
      templateError.value = toSettingsCommandFailure(err)
      return false
    } finally {
      templateSaving.value = false
    }
  }

  /** Сохранение числа попыток (Ф-13) — вход уже целое число (клиентская проверка формы — `parseAttemptsInput.ts`, до вызова этого действия). */
  async function setMaxAttempts(value: number): Promise<boolean> {
    attemptsSaving.value = true
    try {
      const view = await invoke<SettingsView>(SETTINGS_SET_COMMAND, { maxAttempts: value })
      applyView(view)
      attemptsError.value = undefined
      return true
    } catch (err) {
      attemptsError.value = toSettingsCommandFailure(err)
      return false
    } finally {
      attemptsSaving.value = false
    }
  }

  return {
    settings,
    defaults,
    resetFields,
    wholeFileReset,
    destinationFolderExists,
    loaded,
    ipcFailure,
    folderError,
    folderSaving,
    templateError,
    templateSaving,
    attemptsError,
    attemptsSaving,
    previewResult,
    previewProblem,
    previewUnavailable,
    fetchSettings,
    requestPreview,
    setDestinationFolder,
    setNameTemplate,
    setMaxAttempts,
  }
})
