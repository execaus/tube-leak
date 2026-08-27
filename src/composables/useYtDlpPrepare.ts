import { invoke } from '@tauri-apps/api/core'
import { listen, type Event as TauriEvent, type UnlistenFn } from '@tauri-apps/api/event'
import { onUnmounted, ref, type Ref } from 'vue'

import type {
  YtDlpPrepareError,
  YtDlpPrepareErrorKind,
  YtDlpPrepareEvent,
  YtDlpPrepared,
  YtDlpPrepareStage,
} from '@/types/generated/ytdlp'
import { assertNever } from '@/utils/assertNever'
import { knownKindsOf } from '@/utils/knownKinds'

const PREPARE_YTDLP_COMMAND = 'prepare_ytdlp'

/**
 * Имя события хода подготовки, эмитится Rust-стороной под этим именем
 * (`crate::ytdlp::prepare::PREPARE_EVENT`, зеркалируется здесь, а не
 * импортируется — граница Rust↔TS не делится константами напрямую).
 */
const PREPARE_EVENT_NAME = 'ytdlp://prepare'

/**
 * Единственная точка входа фронтенда к Tauri-команде `prepare_ytdlp`
 * (TL-17). Фронтенд не запускает процессы и не трогает файловую систему
 * напрямую (CLAUDE.md, Ф-3) — только через `invoke`. Вызывать безопасно
 * всегда: обе двери внутрь подготовки (эта команда и автозапуск при
 * старте приложения) ведут в одну идемпотентную функцию под мьютексом на
 * Rust-стороне, поэтому повторный вызов дождётся уже идущей подготовки.
 */
export async function prepareYtDlp(): Promise<YtDlpPrepared> {
  return invoke<YtDlpPrepared>(PREPARE_YTDLP_COMMAND)
}

/**
 * То, что реально может оказаться в `error` composable'а. Контрактный путь
 * честный ({@link YtDlpPrepareError}), но неконтрактный отказ существует и
 * должен быть учтён без каста вслепую (ревью TL-17, #18, «Обязательно»):
 * паника внутри команды, отказ самого IPC-вызова `plugin:event|listen` при
 * попытке подписаться, и т.п. приходят как что угодно — строка, `Error`,
 * произвольный объект. В этих случаях `kind` не заполняется: показывать
 * пользователю какую-то из шести конкретных причин было бы неправдой,
 * а не диагностикой. `message` — лучшее, что удалось извлечь, для блока
 * «Подробнее»; при полном отсутствии текста используется общая заглушка.
 */
export type PrepareFailure = YtDlpPrepareError | { kind?: undefined; message: string }

/**
 * Белый список семи классов `YtDlpPrepareErrorKind`, выведенный из
 * сгенерированного типа (TL-52, см. doc `@/utils/knownKinds`) — тот самый
 * список, ручная версия которого пропустила `notEnoughSpace` в TL-18 и
 * тихо подменила текст ошибки заглушкой (см. doc-комментарий
 * `src-tauri/src/types/bindings.rs`).
 */
const KNOWN_ERROR_KINDS = knownKindsOf({
  dataDirUnavailable: true,
  archiveMissing: true,
  archiveCorrupted: true,
  notEnoughSpace: true,
  unpackFailed: true,
  layoutUnexpected: true,
  warmupFailed: true,
} satisfies Record<YtDlpPrepareErrorKind, true>)

function isYtDlpPrepareError(value: unknown): value is YtDlpPrepareError {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Record<string, unknown>
  return (
    typeof candidate.kind === 'string' &&
    (KNOWN_ERROR_KINDS as readonly string[]).includes(candidate.kind) &&
    typeof candidate.message === 'string'
  )
}

/** Настоящее сужение типа реджекта, а не `as YtDlpPrepareError` вслепую. */
function toPrepareFailure(err: unknown): PrepareFailure {
  if (isYtDlpPrepareError(err)) return err
  if (err instanceof Error) return { message: err.message }
  if (typeof err === 'string' && err.length > 0) return { message: err }
  return { message: 'Подготовка yt-dlp не удалась по нераспознанной причине.' }
}

/**
 * Нетерминальные этапы подготовки — выведены из сгенерированного
 * `YtDlpPrepareStage` через `Exclude`, а не переписаны как отдельный
 * литеральный union руками (ревью TL-52: до этой правки `'unpacking' |
 * 'warmingUp'` были продублированы здесь и ещё раз в проп `YtDlpPrepareScreen.vue`
 * — два места, которые ничего не связывало на уровне типов). Если Rust
 * когда-нибудь переименует `ready`/`failed` или добавит третье
 * терминальное значение, `Exclude` подхватит это без правки, потому что
 * вычисляется от актуального контракта, а не переписывает его список
 * вручную.
 */
export type NonTerminalYtDlpPrepareStage = Exclude<YtDlpPrepareStage, 'ready' | 'failed'>

/**
 * Является ли этап нетерминальным — единственное место, решающее это
 * (используется и здесь, в `handleEvent`, и как источник правды для типа
 * пропса `YtDlpPrepareScreen.vue`, doc {@link NonTerminalYtDlpPrepareStage}).
 *
 * Перечисляет все четыре значения `YtDlpPrepareStage` явно, а не два
 * нетерминальных через `||` (это и было дырой TL-52: `event.payload.stage
 * === 'unpacking' || ... === 'warmingUp'` компилировался бы, даже если
 * контракт обзаведётся новым нетерминальным этапом, и просто никогда не
 * присваивал бы его в `stage.value` — экран продолжал бы молчать о новом
 * этапе, а не падать на сборке). `assertNever` в `default` держит границу:
 * он недостижим, пока перечислены все четыре, и перестаёт собираться, как
 * только контракт добавит пятое значение.
 */
function isNonTerminalStage(stage: YtDlpPrepareStage): stage is NonTerminalYtDlpPrepareStage {
  switch (stage) {
    case 'unpacking':
    case 'warmingUp':
      return true
    case 'ready':
    case 'failed':
      return false
    default:
      return assertNever(stage)
  }
}

export interface UseYtDlpPrepareReturn {
  /**
   * Последний нетерминальный этап из события `ytdlp://prepare`;
   * `undefined` — событий ещё не было. Терминальные значения (`ready`,
   * `failed`) сюда намеренно не попадают — см. doc {@link useYtDlpPrepare}.
   */
  stage: Ref<NonTerminalYtDlpPrepareStage | undefined>
  /** Сквозной прогресс подготовки (0..100), из последнего полученного события. */
  percent: Ref<number>
  etaSecs: Ref<number | undefined>
  /** Итог успешной подготовки — заполняется по разрешению промиса команды. */
  result: Ref<YtDlpPrepared | undefined>
  /** Ошибка подготовки — заполняется по реджекту промиса команды или подписки. */
  error: Ref<PrepareFailure | undefined>
  /** `true` пока текущий вызов `prepare()` не разрешился. */
  isPending: Ref<boolean>
  prepare: () => Promise<void>
}

/**
 * Composable подготовки yt-dlp (TL-17): подписывается на событие хода
 * `ytdlp://prepare` и оборачивает команду `prepare_ytdlp`.
 *
 * # Порядок вызовов — критично
 *
 * Подписка на событие устанавливается и дожидается **до** вызова
 * `prepare_ytdlp` — иначе на первом запуске можно пропустить ранние
 * события (распаковка занимает ~1,3 с, событие может прийти почти сразу
 * после вызова команды). Именно поэтому `prepare()` сам управляет
 * подпиской, а не полагается на внешний `onMounted` вызывающего
 * компонента: два независимых `onMounted` не дают гарантии порядка между
 * собой, а гонка здесь означает пропущенный прогресс, а не просто
 * визуальную мелочь.
 *
 * Подписка живёт от первого вызова `prepare()` до размонтирования
 * компонента, использующего composable (`onUnmounted` снимает слушатель) —
 * повторные вызовы (retry по кнопке после ошибки) переиспользуют её, а не
 * создают новую при каждой попытке. Если сама подписка не удалась (отказ
 * IPC `listen`), memoized-промис сбрасывается, чтобы следующий `prepare()`
 * не был навечно привязан к уже отклонённому промису (блокер ревью TL-17).
 *
 * # Почему терминальные `stage` игнорируются
 *
 * Ядро эмитит `stage: 'ready'`/`'failed'` непосредственно перед тем, как
 * разрешить промис `prepare_ytdlp`, но порядок «доставка события в
 * webview» и «ответ команды» не гарантирован. Если брать `stage` из
 * события буквально, возможен видимый откат экрана назад в последний
 * момент 40-секундного ожидания. Терминальное состояние экрана вместо
 * этого берётся из `result`/`error` — из промиса, а не из события.
 */
export function useYtDlpPrepare(): UseYtDlpPrepareReturn {
  const stage = ref<NonTerminalYtDlpPrepareStage>()
  const percent = ref(0)
  const etaSecs = ref<number>()
  const result = ref<YtDlpPrepared>()
  const error = ref<PrepareFailure>()
  const isPending = ref(false)

  let unlisten: UnlistenFn | undefined
  let listening: Promise<void> | undefined

  function handleEvent(event: TauriEvent<YtDlpPrepareEvent>): void {
    percent.value = event.payload.percent
    etaSecs.value = event.payload.etaSecs
    if (isNonTerminalStage(event.payload.stage)) {
      stage.value = event.payload.stage
    }
  }

  function ensureListening(): Promise<void> {
    if (!listening) {
      listening = listen<YtDlpPrepareEvent>(PREPARE_EVENT_NAME, handleEvent)
        .then((fn) => {
          unlisten = fn
        })
        .catch((err: unknown) => {
          // Сбрасываем memoization: следующий prepare() (например, retry по
          // кнопке) должен снова попытаться подписаться, а не унаследовать
          // уже отклонённый промис навсегда.
          listening = undefined
          throw err
        })
    }
    return listening
  }

  async function prepare(): Promise<void> {
    isPending.value = true
    error.value = undefined
    stage.value = undefined
    percent.value = 0
    etaSecs.value = undefined

    try {
      // Ждём подтверждения подписки, прежде чем звать команду (см. doc
      // выше) — это и есть гарантия «check_sidecar не раньше разрешения
      // prepare_ytdlp» на уровне пропуска событий: без неё возможен
      // пропуск unpacking на самом первом запуске. Внутри try: отказ самой
      // подписки — тоже отказ подготовки, а не забытый вечный «Запускаем…»
      // (блокер ревью TL-17, #18).
      await ensureListening()
      result.value = await prepareYtDlp()
    } catch (err) {
      error.value = toPrepareFailure(err)
    } finally {
      isPending.value = false
    }
  }

  onUnmounted(() => {
    if (unlisten) {
      // Подписка уже подтверждена — снимаем сразу и синхронно.
      unlisten()
    } else if (listening) {
      // `listen()` мог ещё не успеть разрешиться (unmount раньше, чем
      // подтвердилась подписка) — цепляемся к тому же промису вместо того,
      // чтобы полагаться на уже присвоенное значение (гонка отписки, ревью
      // TL-17, #18, «Стоит поправить»). Если подписка в итоге отклонится,
      // отписывать нечего — `.catch` молча игнорирует.
      void listening.then(() => unlisten?.()).catch(() => {})
    }
  })

  return { stage, percent, etaSecs, result, error, isPending, prepare }
}
