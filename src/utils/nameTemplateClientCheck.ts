import type { TemplateProblem } from '@/types/generated/settings'

/**
 * Белый список переменных шаблона (Ф-12, дизайн E5 п. 3, «Доступно:
 * {title}, {id}, {quality}, {date}.») — тот же список, что и в `types.rs`
 * (TL-86), продублированный здесь намеренно: это быстрая клиентская
 * проверка формы («баланс скобок, только известные имена», дизайн, п. 3),
 * а не второй источник истины взамен сервера в теории — но на практике
 * блокировку кнопки «Сохранить» держит **только** эта функция (см.
 * `canSaveTemplate` в `SettingsScreen.vue`): ответ `preview_name_template`
 * на неё не влияет никак, даже когда предпросмотр успешно посчитал
 * результат для того же черновика. Расхождение с сервером здесь не
 * симметрично: если сервер строже этой функции, ложно разрешённый ввод
 * всё равно отклонит `settings_set` (и покажет свой текст под полем) — это
 * самоисправляющаяся ошибка на один клик. Если эта функция строже сервера
 * (например, отстала от нового варианта `TemplateProblem` или разошлась в
 * позиции символа), кнопка «Сохранить» недоступна **навсегда** для такого
 * шаблона — сервер здесь ни при чём, ему просто никогда не дают шанс
 * возразить. Постоянного сторожа этому совпадению на момент ревью TL-94
 * нет: правильность проверена разово, не защищена от будущего дрейфа —
 * сверка — #107, #108 (общая фикстура вердиктов `name_template`, читаемая
 * и Rust-тестом, и этим модулем).
 */
const KNOWN_TEMPLATE_VARIABLES: ReadonlySet<string> = new Set(['title', 'id', 'quality', 'date'])

/**
 * Предел длины шаблона в символах Unicode (скалярах, `[...s].length`) — тот
 * же, что `NAME_TEMPLATE_MAX_CHARS` в `src-tauri/src/storage/settings.rs`
 * (TL-87). Ядро проверяет длину **до** разбора (`check_template_length`, и у
 * сохранения, и у предпросмотра, TL-91), поэтому и здесь длина проверяется
 * первой. В контракт число не выведено — сверка значения с ядром входит в
 * общую фикстуру #107/#108.
 */
export const NAME_TEMPLATE_MAX_CHARS = 200

/**
 * Быстрая клиентская проверка формы шаблона (дизайн E5, «Шаблон имени»,
 * п. 3): гейт кнопки «Сохранить» до окончательной проверки на сервере.
 * Возвращает `undefined`, если по форме шаблон выглядит допустимым —
 * это не гарантия принятия сервером (например, `sanitized_stem` может
 * найти проблему, которую эта функция не видит), а лишь то, что «Сохранить»
 * не отправляет заведомо разбитый по форме ввод молча.
 *
 * Позиции — как в {@link TemplateProblem} (в символах Unicode, с единицы):
 * итерация `for...of`/`Array.from` по строке уже даёт по одному элементу на
 * кодовую точку, а не на UTF-16 code unit.
 *
 * Пустая строка — тоже `noVariables` (в ней нет ни одной переменной), а не
 * отдельный случай: пустой шаблон и шаблон из одних литералов эквивалентны
 * по последствию («все файлы получили бы одно имя»).
 */
export function validateNameTemplateDraft(template: string): TemplateProblem | undefined {
  const chars = Array.from(template)
  if (chars.length > NAME_TEMPLATE_MAX_CHARS) {
    return { kind: 'tooLong', max: NAME_TEMPLATE_MAX_CHARS }
  }
  let sawVariable = false
  let i = 0

  while (i < chars.length) {
    const ch = chars[i]

    if (ch === '}') {
      return { kind: 'strayClosingBrace', position: i + 1 }
    }

    if (ch === '{') {
      const openPosition = i + 1
      let j = i + 1
      let name = ''
      while (j < chars.length && chars[j] !== '}' && chars[j] !== '{') {
        name += chars[j]
        j++
      }
      if (j >= chars.length || chars[j] === '{') {
        return { kind: 'unclosedBrace', position: openPosition }
      }
      if (!KNOWN_TEMPLATE_VARIABLES.has(name)) {
        return { kind: 'unknownVariable', position: openPosition, name }
      }
      sawVariable = true
      i = j + 1
      continue
    }

    i++
  }

  if (!sawVariable) return { kind: 'noVariables' }
  return undefined
}
