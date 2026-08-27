import { invoke } from '@tauri-apps/api/core'
import { onUnmounted, ref, watch, type Ref } from 'vue'

import type { ProbeError, ProbeErrorKind, ProbeResult } from '@/types/generated/probe'
import { knownKindsOf } from '@/utils/knownKinds'
import { looksLikeUrl } from '@/utils/looksLikeUrl'

const PROBE_URL_COMMAND = 'probe_url'
const CANCEL_PROBE_COMMAND = 'cancel_probe'

/**
 * Единственная точка входа фронтенда к команде `probe_url` (эпик E2,
 * TL-32 — команда ещё не существует на момент TL-33, экран строится
 * против контракта TL-29, см. бриф TL-33). Фронтенд не запускает процессы
 * и не трогает файловую систему напрямую (CLAUDE.md) — только `invoke`.
 */
export async function probeUrl(url: string): Promise<ProbeResult> {
  return invoke<ProbeResult>(PROBE_URL_COMMAND, { url })
}

/**
 * Отмена текущего разбора без старта нового (дизайн E2, «Управляющие
 * вызовы») — нужна отдельно от {@link probeUrl}, потому что очистка поля
 * не сопровождается новой ссылкой, которую можно было бы передать вместо
 * неё.
 */
export async function cancelProbe(): Promise<void> {
  return invoke<void>(CANCEL_PROBE_COMMAND)
}

/**
 * Белый список девяти классов `ProbeErrorKind`, выведенный из
 * сгенерированного типа (TL-52, см. doc `@/utils/knownKinds`): пропуск
 * нового варианта в объекте ниже роняет `npm run type-check`, а не
 * превращает его молча в неконтрактный фолбэк, как это было с ручным
 * массивом до TL-52.
 */
const KNOWN_ERROR_KINDS = knownKindsOf<ProbeErrorKind>({
  notAUrl: true,
  videoUnavailable: true,
  signInRequired: true,
  regionBlocked: true,
  networkUnavailable: true,
  playlistUnsupported: true,
  liveUnsupported: true,
  ytDlpFailure: true,
  timeout: true,
})

/**
 * То, что реально может оказаться отказом `probe_url`, **кроме** `notAUrl`
 * — этот класс, даже если его вернуло ядро (Ф-2 отдаёт полную валидацию в
 * Rust, фронтовая проверка — только быстрая подсказка), сводится к тому же
 * состоянию `LinkProbeState['notAUrl']`, что и мгновенная фронтовая
 * проверка, а не рисуется блоком ошибки (см. {@link useLinkProbe},
 * "notAUrl из ядра"). Поэтому здесь он структурно исключён — `kind`
 * сужен через `Exclude`, а не оставлен на совесть компонента.
 *
 * Контрактный путь честный ({@link ProbeError} без `notAUrl`), но
 * неконтрактный отказ существует (паника команды, отказ самого
 * IPC-вызова) и должен быть учтён без каста вслепую — тот же приём, что
 * `PrepareFailure` в `useYtDlpPrepare.ts` (E1).
 */
export type ProbeFailure =
  | (Omit<ProbeError, 'kind'> & { kind: Exclude<ProbeErrorKind, 'notAUrl'> })
  | { kind?: undefined; message: string }

function isProbeError(value: unknown): value is ProbeError {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Record<string, unknown>
  return (
    typeof candidate.kind === 'string' &&
    (KNOWN_ERROR_KINDS as readonly string[]).includes(candidate.kind) &&
    typeof candidate.message === 'string'
  )
}

/**
 * Превращает отклонение `probeUrl` в {@link ProbeFailure}. Вызывающая
 * сторона обязана сама развести `notAUrl` на состояние `notAUrl` ещё
 * *до* вызова этой функции (см. {@link dispatchProbe}) — здесь он
 * трактуется как неконтрактный случай и падает в общий фолбэк, что
 * является защитой на случай нарушения этого протокола, а не основным
 * путём.
 */
function toProbeFailure(err: unknown): ProbeFailure {
  // Проверка `err.kind !== 'notAUrl'` сужает тип свойства `err.kind`, но не
  // тип всего `err` (`ProbeError` — плоский интерфейс, не размеченное
  // объединение по вариантам) — TS не свяжет это с более узким `ProbeFailure`
  // автоматически. Каст безопасен: на этой строке рантайм уже гарантировал,
  // что `kind` не `notAUrl`.
  if (isProbeError(err) && err.kind !== 'notAUrl') return err as ProbeFailure
  if (err instanceof Error) return { message: err.message }
  if (typeof err === 'string' && err.length > 0) return { message: err }
  return { message: 'Разбор ролика завершился нераспознанной ошибкой.' }
}

/**
 * Состояние карточной области экрана (дизайн E2, раздел «Состояния») —
 * ровно одно одновременно с полем ссылки. `notAUrl` покрывает и
 * мгновенную фронтовую проверку, и класс `notAUrl`, пришедший из ядра
 * (см. doc {@link useLinkProbe}) — представление одно и то же независимо
 * от источника.
 */
export type LinkProbeState =
  | { kind: 'empty' }
  | { kind: 'notAUrl' }
  | { kind: 'loading'; slow: boolean }
  | { kind: 'success'; result: ProbeResult }
  | { kind: 'error'; error: ProbeFailure }

/** Задержка тишины ввода перед стартом разбора (дизайн E2 — стартовая точка, подлежит калибровке на bundle). */
const DEBOUNCE_MS = 400
/** Порог, после которого «Получаем данные…» обзаводится второй строкой пояснения. */
const SLOW_HINT_MS = 6000

export interface UseLinkProbeReturn {
  url: Ref<string>
  state: Ref<LinkProbeState>
  /** Повторяет разбор для текущего содержимого поля немедленно (без ожидания тишины ввода). */
  retry: () => void
}

/**
 * Composable экрана разбора ссылки (TL-33, эпик E2).
 *
 * # Одно общее правило отмены (дизайн E2, «Триггер разбора и отмена»)
 *
 * {@link evaluate} — единственное место, решающее судьбу поля по его
 * текущему содержимому: пусто → пустое состояние; не похоже на ссылку →
 * мгновенная ошибка без запуска процесса; похоже на ссылку → карточная
 * область немедленно переходит в «получаем данные…» (старая
 * карточка/ошибка не остаётся видна ни на долю секунды — «замена
 * карточки, а не наложение поверх старой»), а сам вызов `probeUrl`
 * откладывается на ~400 мс тишины ввода (кроме retry — тот стартует
 * немедленно). Очистка поля, вторая ссылка и правка уже вставленной — не
 * три ветки, а один и тот же путь с разным содержимым на входе.
 *
 * # `notAUrl` из ядра — то же состояние, что и мгновенная проверка (блокер ревью)
 *
 * Фронтовая проверка {@link looksLikeUrl} — не полная валидация (Ф-2
 * отдаёт её Rust целиком, здесь только быстрая подсказка «стоит ли вообще
 * пробовать»). Значит, ядро может вернуть класс `notAUrl` даже после того,
 * как фронт счёл содержимое похожим на ссылку и запустил разбор — путь
 * реальный: достаточно набрать `https://` руками и замереть на 400 мс,
 * не успев дописать хост. Такой отказ **не должен** рисоваться блоком
 * ошибки (там нет технических деталей — процесс не запускался вовсе, и
 * заголовка «Это не ссылка» в таблице дизайна для блочного представления
 * попросту нет, только инлайн) — он сводится к тому же `state.value =
 * {kind:'notAUrl'}`, что и локальная мгновенная проверка, той же веткой
 * рендера в `ProbeSection.vue`.
 *
 * # Сторож по поколениям (обязателен, TL-27 review, К-4)
 *
 * Наивная защита «есть ли активный запрос» не закрывает случай, когда
 * разбор A успевает завершиться в ядре **раньше**, чем вызов по ссылке B
 * вообще доходит до ядра: ответ на A приходит совершенно легитимным
 * успехом уже после того, как поле показывает B. Сравнение по URL тоже не
 * годится — одна и та же ссылка, вставленная дважды подряд, и расхождения
 * нормализации ломают его.
 *
 * Поэтому здесь — монотонный счётчик `generation`, увеличиваемый в
 * {@link evaluate} на **каждое** решение по содержимому поля (на каждое
 * изменение — evaluate вызывается и для мгновенных веток, и для той, что
 * готовит debounce; `retry()` тоже проходит через {@link evaluate} и
 * получает собственное поколение). Каждый вызов {@link dispatchProbe}
 * запоминает поколение, с которым он был запущен, и при
 * разрешении/отклонении промиса `probeUrl` сверяет его с текущим —
 * несовпадение отбрасывает результат целиком, **и resolve, и reject**, не
 * трогая `state`. Ключ — именно счётчик, не URL: одна и та же ссылка,
 * вставленная дважды, и расхождения нормализации не ломают сравнение.
 */
export function useLinkProbe(): UseLinkProbeReturn {
  const url = ref('')
  const state = ref<LinkProbeState>({ kind: 'empty' })

  let generation = 0
  let inFlight = false
  let debounceTimer: ReturnType<typeof setTimeout> | undefined
  let slowTimer: ReturnType<typeof setTimeout> | undefined

  function clearDebounceTimer(): void {
    if (debounceTimer !== undefined) {
      clearTimeout(debounceTimer)
      debounceTimer = undefined
    }
  }

  function clearSlowTimer(): void {
    if (slowTimer !== undefined) {
      clearTimeout(slowTimer)
      slowTimer = undefined
    }
  }

  /** Отменяет фактически запущенный разбор, если он есть — не более того («Экономия вызовов»). */
  function cancelIfInFlight(): void {
    if (inFlight) {
      inFlight = false
      void cancelProbe().catch(() => {})
    }
  }

  /** Собственно вызов `probeUrl` — то, что задерживается debounce'ом (не сам переход в «получаем данные…»). */
  function dispatchProbe(myGeneration: number, value: string): void {
    inFlight = true

    slowTimer = setTimeout(() => {
      // Поколение могло уже смениться, пока не сработал этот таймер.
      if (myGeneration !== generation) return
      state.value = { kind: 'loading', slow: true }
    }, SLOW_HINT_MS)

    probeUrl(value)
      .then((result) => {
        // Сторож по поколениям: отбрасываем resolve не текущего поколения
        // (см. doc {@link useLinkProbe}) — легитимный успех для ссылки,
        // которой в поле уже нет, не должен попасть на экран.
        if (myGeneration !== generation) return
        inFlight = false
        clearSlowTimer()
        state.value = { kind: 'success', result }
      })
      .catch((err: unknown) => {
        // И reject тоже отбрасывается тем же способом.
        if (myGeneration !== generation) return
        inFlight = false
        clearSlowTimer()

        // Блокер ревью: `notAUrl` из ядра — то же состояние, что и
        // мгновенная фронтовая проверка, не блок ошибки (см. doc выше).
        if (isProbeError(err) && err.kind === 'notAUrl') {
          state.value = { kind: 'notAUrl' }
          return
        }

        state.value = { kind: 'error', error: toProbeFailure(err) }
      })
  }

  function evaluate(value: string, opts: { immediate: boolean }): void {
    clearDebounceTimer()
    clearSlowTimer()
    generation += 1
    const myGeneration = generation

    const trimmed = value.trim()

    if (trimmed.length === 0) {
      cancelIfInFlight()
      state.value = { kind: 'empty' }
      return
    }

    if (!looksLikeUrl(trimmed)) {
      cancelIfInFlight()
      state.value = { kind: 'notAUrl' }
      return
    }

    // Похоже на ссылку — карточная область переходит в «получаем данные…»
    // немедленно, а не после тишины ввода: «карточная область немедленно
    // показывает» (дизайн) относится к решению по содержимому поля в
    // целом, а старая карточка/ошибка не обязана оставаться видна ни на
    // долю секунды дольше необходимого («замена карточки, а не наложение
    // поверх старой»). Задерживается debounce'ом только сам вызов
    // `probeUrl` — чтобы не запускать yt-dlp на каждое нажатие клавиши.
    state.value = { kind: 'loading', slow: false }

    if (opts.immediate) {
      dispatchProbe(myGeneration, trimmed)
    } else {
      debounceTimer = setTimeout(() => {
        dispatchProbe(myGeneration, trimmed)
      }, DEBOUNCE_MS)
    }
  }

  watch(url, (value) => {
    evaluate(value, { immediate: false })
  })

  /** Повтор по кнопке «Повторить» — сознательно без ожидания тишины ввода, но со своим поколением. */
  function retry(): void {
    evaluate(url.value, { immediate: true })
  }

  onUnmounted(() => {
    clearDebounceTimer()
    clearSlowTimer()
    // Отмена обязательна на каждом этапе (CLAUDE.md) — включая уход с
    // экрана: сегодня недостижимо (экраны E1 не поднимаются заново после
    // готовности), но E3/E4 добавят экраны, и полагаться на
    // недостижимость как на защиту — тот же класс риска, на котором
    // проект уже обжигался.
    cancelIfInFlight()
  })

  return { url, state, retry }
}
