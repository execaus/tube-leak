# Форма onedir-деревьев yt-dlp (TL-18)

Не вывод процесса, в отличие от соседних наборов, — **замер архивов**.
Сколько записей и сколько распакованных байт даёт каждый onedir-ассет
yt-dlp. Набор существует ради одного: потолки `MAX_UNPACKED_BYTES` и
`MAX_ENTRIES` в `src-tauri/src/ytdlp/unpack.rs` выведены из этих чисел, а
не из круглого числа, и должны краснеть, если реальность к ним подойдёт
вплотную.

Соседние наборы (`../ytdlp-download/`, `../ytdlp-probe/`,
`../ffmpeg-merge/`) — про поведение запущенного инструмента. Здесь
инструмент не запускался вовсе.

## Как снят

Диапазонными запросами к релизным ассетам GitHub, 2026-08-27: у каждого
zip забирается последний мегабайт, в нём находится `PK\x05\x06`, и
центральный каталог разбирается напрямую — размеры записей объявлены
именно там, и `zip`-крейт на рантайме читает их оттуда же. Архив целиком
не скачивается: шесть ассетов пинованного релиза — это 235 МБ, а нужны из
них 6 МБ хвостов.

Суффиксный диапазон (`Range: bytes=-1048576`) CDN релизов отвечает
**501 Not Implemented**, поэтому границы считаются от размера ассета,
взятого из API релизов.

Воспроизвести (нужен `gh`):

```sh
python3 - <<'PY'
import json, struct, subprocess
TAG, NAME = "2026.08.19", "yt-dlp_macos.zip"
rel = json.loads(subprocess.run(["gh","api",f"repos/yt-dlp/yt-dlp/releases/tags/{TAG}"],
                                capture_output=True, text=True).stdout)
size = next(a["size"] for a in rel["assets"] if a["name"] == NAME)
url = f"https://github.com/yt-dlp/yt-dlp/releases/download/{TAG}/{NAME}"
n = min(1 << 20, size)
buf = subprocess.run(["curl","-sL","-H",f"Range: bytes={size-n}-{size-1}",url],
                     capture_output=True).stdout
base = size - len(buf)
i = buf.rfind(b"PK\x05\x06")
entries = struct.unpack_from("<H", buf, i + 10)[0]
cd_off = struct.unpack_from("<II", buf, i + 12)[1]
p, files, total = cd_off - base, 0, 0
for _ in range(entries):
    csize, usize, nlen, elen, clen = struct.unpack_from("<II HHH", buf, p + 20)
    name = buf[p+46:p+46+nlen].decode()
    if not name.endswith("/"):
        files += 1; total += usize
    p += 46 + nlen + elen + clen
print(NAME, "entries", entries, "files", files, "unpacked", total)
PY
```

## Проверка на месте

Строка `yt-dlp_macos.zip` релиза `2026.08.19` сверена с локальным
`src-tauri/resources/yt-dlp.zip` (тот же ассет, уложенный в бандл
`scripts/fetch-binaries`) чтением его центрального каталога уже с диска:
162 записи, 130 010 634 байта — совпало до байта. То есть разбор хвоста
по сети даёт то же, что полный архив на диске.

## Формат

```json
{
  "_capture": { "note": "…", "reality": "live", "method": "…",
                "capturedAt": "2026-08-27", "pinnedRelease": "2026.08.19" },
  "trees": [
    { "release": "2026.08.19", "asset": "yt-dlp_macos.zip",
      "shippedAs": "macos (оба таргета, universal2)",
      "assetBytes": 53923637, "entries": 162, "files": 134,
      "directories": 28, "unpackedBytes": 130010634,
      "compressedBytes": 53887487 }
  ]
}
```

- `shippedAs` — какому таргету приложения этот ассет достаётся по
  `binaries.lock.json`; `null` — ассет апстрима, который мы не вкладываем
  (арм-сборки под Windows и Linux, musl). Их замеры всё равно здесь:
  потолок общий на все платформы, и знать разброс по семейству полезнее,
  чем по трём выбранным.
- `entries` — записи центрального каталога целиком, вместе с каталожными;
  именно это число сравнивается с `MAX_ENTRIES`, потому что именно оно
  задаёт границу цикла распаковки.
- `unpackedBytes` — сумма объявленных размеров **файлов**, без каталогов.
- Ожидаемых значений в фикстуре нет: она хранит факты, ответы хранят
  тесты — тот же приём, что во всех остальных наборах проекта.

## Состав

Шесть строк релиза `2026.08.19` — все onedir-ассеты пина, включая те, что
в бандл не едут. Четыре строки `yt-dlp_macos.zip` из релизов
`2024.12.03`, `2025.09.05`, `2025.12.08`, `2026.02.04` — та же платформа
на протяжении двадцати месяцев назад, чтобы потолок стоял на диапазоне
изменения, а не на одном снимке.

Самое большое дерево набора — 145,1 МиБ и 186 записей (macOS,
`2026.02.04`). Разброс по macOS за двадцать месяцев — 124,0…145,1 МиБ,
около ±17 %.

## Что сторожат тесты (`src/ytdlp/unpack.rs`)

- `the_pinned_release_is_the_one_the_sizes_were_measured_from` — версия в
  `_capture.pinnedRelease` сверяется с `ytDlp.version` из
  `binaries.lock.json`. Смена пина обязана ломать тест: она может
  изменить и форму дерева, и переснимать замеры надо вместе с ней.
- `the_ceilings_clear_every_tree_upstream_has_ever_shipped` — обе границы
  проверяются на трёхкратный запас над максимумом набора. Потолок,
  подошедший к реальности ближе, — это отложенная поломка обновления
  yt-dlp у всех пользователей сразу (E6), и заметить её надо здесь, а не
  там.
- `no_measured_tree_is_refused_by_the_unpacker` — каждая строка набора
  прогоняется через те же проверки, которыми распаковка отвергает архив.
  Тест отвечает на вопрос «а не отказали бы мы настоящему ассету?» кодом
  распаковки, а не арифметикой в голове.
