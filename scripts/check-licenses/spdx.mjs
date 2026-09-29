// Разбор поля `license` крейта/пакета в набор лицензий, которые
// ДЕЙСТВИТЕЛЬНО применяются к нам (TL-136, #143).
//
// Зачем разбор, а не строка целиком. В графе 28 разных выражений, и
// почти все они составные. Считать строку идентификатором лицензии —
// ровно та ошибка, из-за которой #143 и открыт: `Apache-2.0 AND ISC`
// не «одна экзотическая лицензия», а ДВЕ обычные, и обе требуют своего
// раздела; `MIT OR Apache-2.0` — наоборот, одна на выбор получателя
// прав, и раздела хватит одного.
//
// Разница между AND и OR здесь не косметическая:
//
// - `AND` — условия складываются, выполнять нужно ВСЕ. Пропустив
//   конъюнкт, мы не выполняем чужую лицензию (ring: Apache-2.0 AND ISC).
// - `OR` — условия на выбор, выполнять нужно ОДНО, и выбираем его мы.
//   Выбор обязан быть одинаковым в документе и в стороже, иначе сторож
//   потребует раздел под лицензию, которую мы не выбирали.
//
// Поэтому выбор задан правилом, а не списком: предпочитаем MIT, затем
// Apache-2.0 — те две, под которыми и так идёт подавляющая часть графа,
// то есть новых обязательств такой выбор не создаёт. Если ни одной из
// них в дизъюнкции нет, берём первую по алфавиту: лишь бы выбор был
// воспроизводим, а не зависел от порядка слов в чужом Cargo.toml.

/**
 * Порядок предпочтения при выборе из дизъюнкции (`OR`). Первое
 * совпадение выигрывает; остальное — по алфавиту.
 */
export const PREFERRED = Object.freeze(['MIT', 'Apache-2.0'])

/**
 * Приводит запись лицензии к списку конъюнктов, каждый из которых —
 * список альтернатив.
 *
 * Поддерживает исторический разделитель `/` (в старых крейтах —
 * `MIT/Apache-2.0`, `Apache-2.0 / MIT`): cargo трактует его как `OR`,
 * и таких записей в нашем графе 19.
 *
 * @param {string} expression значение поля `license`
 * @returns {string[][]} конъюнкты; каждый — непустой список альтернатив
 */
export function parseSpdx(expression) {
  const text = expression.trim()
  if (text === '') return []

  // Скобки нужны ровно для одного выражения в графе —
  // `(MIT OR Apache-2.0) AND Unicode-3.0`, — но разбирать их надо
  // честно: без этого `AND` внутри скобок разорвал бы группу.
  const conjuncts = splitTopLevel(text, 'AND')
  return conjuncts.map((conjunct) => {
    const unwrapped = unwrapParens(conjunct)
    return splitTopLevel(unwrapped.replaceAll('/', ' OR '), 'OR').map((alternative) =>
      unwrapParens(alternative),
    )
  })
}

/**
 * Лицензии, которые мы обязаны выполнить для этого пакета.
 *
 * @param {string} expression значение поля `license`
 * @returns {string[]} отсортированный список без повторов
 */
export function effectiveLicenses(expression) {
  const chosen = parseSpdx(expression).map((alternatives) => choose(alternatives))
  return [...new Set(chosen)].sort()
}

/**
 * Выбор одной альтернативы из дизъюнкции.
 *
 * @param {string[]} alternatives
 * @returns {string}
 */
export function choose(alternatives) {
  for (const preferred of PREFERRED) {
    if (alternatives.includes(preferred)) return preferred
  }
  return [...alternatives].sort()[0]
}

/**
 * Делит выражение по оператору верхнего уровня, не заглядывая внутрь
 * скобок.
 *
 * @param {string} text
 * @param {'AND' | 'OR'} operator
 * @returns {string[]}
 */
function splitTopLevel(text, operator) {
  const parts = []
  let depth = 0
  let current = ''
  const tokens = text.split(/\s+/)
  for (const token of tokens) {
    if (depth === 0 && token === operator) {
      parts.push(current.trim())
      current = ''
      continue
    }
    depth += (token.match(/\(/g) ?? []).length
    depth -= (token.match(/\)/g) ?? []).length
    current += (current === '' ? '' : ' ') + token
  }
  parts.push(current.trim())
  return parts.filter((part) => part !== '')
}

/**
 * @param {string} text
 * @returns {string}
 */
function unwrapParens(text) {
  let result = text.trim()
  while (result.startsWith('(') && result.endsWith(')')) {
    result = result.slice(1, -1).trim()
  }
  return result
}
