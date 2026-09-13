import type { TemplateProblem } from '@/types/generated/settings'

/**
 * Белый список переменных шаблона (Ф-12, дизайн E5 п. 3, «Доступно:
 * {title}, {id}, {quality}, {date}.») — тот же список, что и в `types.rs`
 * (TL-86), продублированный здесь намеренно: это быстрая клиентская
 * проверка формы («баланс скобок, только известные имена», дизайн, п. 3),
 * а не второй источник истины взамен сервера — окончательное решение
 * всегда за `settings_set`/`preview_name_template` (тот же валидатор,
 * доступный только за границей IPC). Расхождение с сервером здесь не
 * ломает контракт: худшее, что может случиться — кнопка «Сохранить»
 * ошибочно доступна для чего-то, что сервер всё равно отклонит (и покажет
 * свой текст под полем), либо ошибочно недоступна на доли секунды до
 * ответа живого примера — тест К-6/С-7 проверяет именно сохранение и
 * ошибки сервера, не совпадение этой функции с Rust побайтово.
 */
const KNOWN_TEMPLATE_VARIABLES: ReadonlySet<string> = new Set(['title', 'id', 'quality', 'date'])

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
