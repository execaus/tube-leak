# Фикстуры одного запуска `-f V,A` (эпик E3, TL-48)

Весь вывод **одного** запуска yt-dlp, заказывающего видео и звук через
запятую: stdout, stderr, код завершения и что лежало в папке назначения до
и после. На них стоят тесты оркестрации (`src/download/orchestrate_tests.rs`):
атрибуция строк по формату и имени файла, класс `staleFormat` при выпавшем
формате, сторож продвижения на стыке потоков, повтор после отказа одного
формата.

Соседние наборы: `../progress/` — строки прогресса одного потока, сняты с
YouTube; `../outcomes/` — код и stderr законченной попытки. Этот набор
отдельный, потому что его предмет — связь всех частей одного запуска, и
снят он иначе.

## Как сняты: без обращения к YouTube

YouTube с этой машины отвечал проверкой «не бот» (см. отчёт TL-48), а
обход ограничения вне границ продукта. Поэтому **строк извлечения с
YouTube в наборе нет**, и ни одного сетевого обмена за пределы машины при
съёмке не было:

- бинарник — настоящий вложенный yt-dlp **2026.08.19**:
  `src-tauri/binaries/yt-dlp-aarch64-apple-darwin.zip` (sha256 совпал с
  `binaries.lock.json`), распакованный во временный каталог, macOS,
  2026-09-13;
- argv — **аргументы приложения** (`download_args` в
  `src/download/orchestrate.rs`), кроме хвоста: вместо `-- <ссылка>` стоит
  `--load-info-json <infoJson>`. Совпадение проверяет тест
  `the_single_launch_fixtures_were_shot_with_the_arguments_of_the_app`;
- метаданные ролика — снятая живьём фикстура разбора
  `../../ytdlp-probe/4k-full-ladder.json`, в которой оставлены только
  форматы случая; у них `url` переписан на локальный HTTP-сервер,
  `protocol` — `http` вместо `https` (загрузчик yt-dlp у обоих один),
  `filesize` — размер отдаваемого файла; название — `_capture.title`;
- сервер — `127.0.0.1`, отдаёт валидные короткие mp4 (h264, 894 838 байт)
  и m4a (aac, 323 730 байт) кусками по 16 КиБ раз в 40 мс, чтобы строк
  прогресса было больше одной; понимает `Range`, отсутствующий файл —
  `404`. Все обращения к нему записаны в `_capture.requests`;
- процесс запущен под `sandbox-exec` с профилем
  `(deny network-outbound)(allow network-outbound (remote ip "localhost:*"))`
  и окружением `PATH=/usr/bin:/bin:/usr/sbin:/sbin` — как у `.app`,
  запущенного из Finder; ffmpeg в `PATH` нет, отсюда предупреждение
  `writing DASH m4a` в stderr;
- stdout и stderr читались двоичными дескрипторами (урок `../outcomes/`
  про universal newlines), время каждой строки от старта процесса — в
  `lineTimesMs`.

Единственная правка снятого текста — абсолютный путь временной папки
назначения заменён плейсхолдером `<destination>` (в stdout, stderr и argv).
Тесты подставляют на его место свою папку.

## Формат конверта

```json
{
  "_capture": {
    "note": "…чем интересна…",
    "reality": "live",
    "method": "local-server",
    "argv": ["--no-playlist", "…", "<progressTemplate>", "…", "--load-info-json", "<infoJson>"],
    "progressTemplate": "download:@tl-progress|…",
    "ytDlpVersion": "2026.08.19",
    "capturedAt": "2026-09-13",
    "select": "133,139",
    "served": { "133": { "file": "v133.mp4", "bytes": 894838 }, "139": "404" },
    "title": "Big Buck Bunny",
    "requests": [{ "path": "/v133.mp4", "range": null, "status": 200 }]
  },
  "exitCode": 0,
  "stdout": "…",
  "stderr": "…",
  "lineTimesMs": [486, 505],
  "listingBefore": [],
  "listingAfter": ["Big Buck Bunny.f133.mp4"]
}
```

## Состав

| Файл | `-f` | До запуска | Чем интересен |
|---|---|---|---|
| `video-and-audio.json` | `133,139` | пусто | Успех: видео целиком, затем звук, у каждого свои `Destination` и `finished`; код 0. |
| `video-only.json` | `133` | пусто | Повтор, заказывающий один поток. |
| `video-already-downloaded.json` | `133,139` | видео | `has already been downloaded`, следом `finished` с `downloaded_bytes` = `NA` (М-1), затем загрузка звука; код 0. |
| `one-format-missing.json` | `133,139` | пусто | В метаданных нет 139: перечень `[info] … Downloading 1 format(s): 133`, код 0, stderr пуст (Р-1). |
| `video-404.json` | `133,139` | пусто | Адрес видео отвечает 404: у 133 нет даже `Destination`, звук скачан целиком; код 1. |
| `phrase-in-title.json` | `133,139` | звук | Название само содержит `has already been downloaded` — фраза в строке дважды (М-3). |

## Что замерено попутно

- **Порядок строк на стыке.** С ffmpeg в `PATH` (вложенный, во временном
  каталоге) у `-f 133,139` постпроцессор `[FixupM4a]` потока идёт после его
  `finished` и до `Destination` следующего потока. Отсюда решение в
  `note_stream_start`: срок сторожа заводится заново на строке, называющей
  файл нового потока, а не на строках постпроцессора.
- **Когда после `has already been downloaded` приходит `finished`.** С
  argv приложения — всегда (`NA` вместо байт). С `-P temp:.` из прежней
  съёмки `../progress/already-downloaded.json` — нет: при отдельном
  временном каталоге yt-dlp находит готовый файл сам, не зовя загрузчик.
  Проверено на одних и тех же метаданных.

## Как переснять

Скрипт съёмки лежал во временном каталоге сессии и в репозиторий не
попал. Всё, что он делал, перечислено выше: собрать метаданные, поднять
сервер, запустить бинарник под `sandbox-exec` с argv из `_capture.argv`
(подставив `progressTemplate`, папку и файл метаданных), записать конверт.
Смена пина или шаблона роняет
`fixtures_are_the_output_of_this_template_and_of_the_pinned_yt_dlp`, смена
аргументов запуска — `the_single_launch_fixtures_were_shot_with_the_arguments_of_the_app`.
