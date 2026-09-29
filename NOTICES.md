# Уведомления об авторских правах

Файл собирается машинно и едет вместе с приложением. Он существует
потому, что MIT, BSD, ISC, Zlib и Apache-2.0 требуют **сохранять
уведомление об авторских правах** при распространении — это условие
гранта, а не оформление. Полные тексты самих лицензий — в
`THIRD-PARTY-LICENSES.md` рядом; дублировать их здесь по разу на пакет
незачем, а уведомления у пакетов разные, и они здесь.

**Как собрано.** Для каждого пакета, который едет в поставку (ведро
`shipped` в `licenses.lock.json`), взяты строки копирайта из файла
лицензии самого пакета. Где апстрим файла не поставляет — взяты авторы
из метаданных пакета, и запись об этом прямо говорит. Где нет ни того,
ни другого — пакет назван в отдельном списке внизу, со ссылкой на его
репозиторий.

Не редактировать руками: пересборка — `npm run check-licenses -- --write`.

Итог этой сборки: всего записей 328, из них из файла пакета
244, восстановлено из метаданных 72,
недоступно ни одним способом 12.

## Rust-крейты

### adler2 2.0.1 — 0BSD OR MIT OR Apache-2.0

```
Copyright (C) Jonas Schievink <jonasschievink@gmail.com>
```

### aho-corasick 1.1.5 — Unlicense OR MIT

```
Copyright (c) 2015 Andrew Gallant
```

### alloc-no-stdlib 2.0.4 — BSD-3-Clause

```
Copyright (c) 2016 Dropbox, Inc.
```

### alloc-stdlib 0.2.4 — BSD-3-Clause

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Daniel Reiter Horn <danielrh@dropbox.com>
```

### anyhow 1.0.104 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### atk 0.18.2 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### atk-sys 0.18.2 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### base64 0.21.7 — MIT OR Apache-2.0

```
Copyright (c) 2015 Alice Maz
```

### base64 0.22.1 — MIT OR Apache-2.0

```
Copyright (c) 2015 Alice Maz
```

### base64 0.23.1 — MIT OR Apache-2.0

```
Copyright (c) 2025 Alice Maz, Marshall Pierce
```

### bit-set 0.8.0 — Apache-2.0 OR MIT

```
Copyright (c) 2023 The Rust Project Developers
```

### bit-vec 0.8.0 — Apache-2.0 OR MIT

```
Copyright (c) 2023 The Rust Project Developers
```

### bitflags 1.3.2 — MIT/Apache-2.0

```
Copyright (c) 2014 The Rust Project Developers
```

### bitflags 2.13.1 — MIT OR Apache-2.0

```
Copyright (c) 2014 The Rust Project Developers
```

### block-buffer 0.10.4 — MIT OR Apache-2.0

```
Copyright (c) 2018-2019 The RustCrypto Project Developers
```

### block2 0.6.2 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Mads Marquart <mads@marquart.dk>
```

### brotli 8.0.4 — BSD-3-Clause AND MIT

```
Copyright (c) 2016 Dropbox, Inc.
Copyright (c) 2009, 2010, 2013-2016 by the Brotli Authors.
```

### brotli-decompressor 5.0.3 — BSD-3-Clause/MIT

```
Copyright (c) 2016 Dropbox, Inc.
```

### byteorder 1.5.0 — Unlicense OR MIT

```
Copyright (c) 2015 Andrew Gallant
```

### bytes 1.12.1 — MIT

```
Copyright (c) 2018 Carl Lerche
```

### cairo-rs 0.18.5 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### cairo-sys-rs 0.18.2 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### camino 1.2.5 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Without Boats <saoirse@without.boats>
Ashley Williams <ashley666ashley@gmail.com>
Steve Klabnik <steve@steveklabnik.com>
Rain <rain@sunshowers.io>
```

### cargo-platform 0.1.9 — MIT OR Apache-2.0

Уведомление недоступно: файл лицензии в пакете есть, но строки с уведомлением об авторских правах в нём нет, и авторы в метаданных не указаны. Репозиторий: https://github.com/rust-lang/cargo.

### cargo_metadata 0.19.2 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Oliver Schneider <git-spam-no-reply9815368754983@oli-obk.de>
```

### cfb 0.7.3 — MIT

```
Copyright (c) 2017 Matthew D. Steele
```

### cfg-if 1.0.4 — MIT OR Apache-2.0

```
Copyright (c) 2014 Alex Crichton
```

### cookie 0.18.2 — MIT OR Apache-2.0

```
Copyright 2017 Sergio Benitez
Copyright 2014 Alex Chricton
Copyright (c) 2017 Sergio Benitez
Copyright (c) 2014 Alex Crichton
```

### core-foundation 0.10.1 — MIT OR Apache-2.0

```
Copyright (c) 2012-2013 Mozilla Foundation
```

### core-foundation-sys 0.8.7 — MIT OR Apache-2.0

```
Copyright (c) 2012-2013 Mozilla Foundation
```

### core-graphics 0.25.0 — MIT OR Apache-2.0

```
Copyright (c) 2012-2013 Mozilla Foundation
```

### core-graphics-types 0.2.0 — MIT OR Apache-2.0

```
Copyright (c) 2012-2013 Mozilla Foundation
```

### cpufeatures 0.2.17 — MIT OR Apache-2.0

```
Copyright (c) 2020-2025 The RustCrypto Project Developers
```

### crc32fast 1.5.1 — MIT OR Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) 2018 Sam Rijs, Alex Crichton and contributors
```

### crossbeam-channel 0.5.16 — MIT OR Apache-2.0

```
Copyright (c) 2019 The Crossbeam Project Developers
Copyright (c) 2009 The Go Authors. All rights reserved.
```

### crossbeam-utils 0.8.22 — MIT OR Apache-2.0

```
Copyright (c) 2019 The Crossbeam Project Developers
```

### crypto-common 0.1.7 — MIT OR Apache-2.0

```
Copyright (c) 2021 RustCrypto Developers
```

### cssparser 0.36.0 — MPL-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Simon Sapin <simon.sapin@exyr.org>
```

### ctor 0.8.0 — Apache-2.0 OR MIT

```
Copyright {yyyy} {name of copyright owner}
```

### darling 0.23.0 — MIT

```
Copyright (c) 2017 Ted Driggs
```

### darling_core 0.23.0 — MIT

```
Copyright (c) 2017 Ted Driggs
```

### dbus 0.9.12 — Apache-2.0/MIT

```
Copyright 2014-2018 David Henningsson <diwic@ubuntu.com> and other contributors
Copyright (c) 2014-2018 David Henningsson <diwic@ubuntu.com> and other contributors
```

### deranged 0.5.8 — MIT OR Apache-2.0

```
Copyright 2024 Jacob Pratt et al.
Copyright (c) 2024 Jacob Pratt et al.
```

### derive_more 2.1.1 — MIT

```
Copyright (c) 2016 Jelte Fennema
```

### digest 0.10.7 — MIT OR Apache-2.0

```
Copyright (c) 2017 Artyom Pavlov
```

### dirs 6.0.0 — MIT OR Apache-2.0

```
Copyright (c) 2018-2019 dirs-rs contributors
```

### dirs-sys 0.5.0 — MIT OR Apache-2.0

```
Copyright (c) 2018-2019 dirs-rs contributors
```

### dispatch2 0.3.1 — Zlib OR Apache-2.0 OR MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Mads Marquart <mads@marquart.dk>
Mary <mary@mary.zone>
```

### dlopen2 0.8.2 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Szymon Wieloch <szymon.wieloch@gmail.com>
Ahmed Masud <ahmed.masud@saf.ai>
OpenByte <development.openbyte@gmail.com>
```

### dom_query 0.27.0 — MIT

```
Copyright (c) 2023 Mykola Humanov
```

### dpi 0.1.2 — Apache-2.0 AND MIT

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) 2018 Jorge Aparicio
Copyright © 2005-2020 Rich Felker, et al.
Copyright © 1993,2004 Sun Microsystems or
Copyright © 2003-2011 David Schultz or
Copyright © 2003-2009 Steven G. Kargl or
Copyright © 2003-2009 Bruce D. Evans or
Copyright © 2008 Stephen L. Moshier or
Copyright © 2017-2018 Arm Limited
```

### dtoa 1.0.11 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### dtoa-short 0.3.5 — MPL-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Xidorn Quan <me@upsuper.org>
```

### dunce 1.0.5 — CC0-1.0 OR MIT-0 OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Kornel <kornel@geekhood.net>
```

### dyn-clone 1.0.20 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### embed_plist 1.2.2 — MIT OR Apache-2.0

```
Copyright (c) 2020 Nikolai Vazquez
```

### equivalent 1.0.2 — Apache-2.0 OR MIT

```
Copyright (c) 2016--2023
```

### erased-serde 0.4.10 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### errno 0.3.14 — MIT OR Apache-2.0

```
Copyright (c) 2014 Chris Wong
```

### fallible-iterator 0.3.0 — MIT/Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) 2015 The rust-openssl-verify Developers
```

### fallible-streaming-iterator 0.1.9 — MIT/Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) 2016 The fallible-streaming-iterator Developers
```

### fastrand 2.5.0 — Apache-2.0 OR MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Stjepan Glavina <stjepang@gmail.com>
```

### fdeflate 0.3.7 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The image-rs Developers
```

### field-offset 0.3.6 — MIT OR Apache-2.0

```
Copyright (c) 2016-2021 Diggory Blake, and other contributors.
```

### flate2 1.1.9 — MIT OR Apache-2.0

```
Copyright (c) 2014-2026 Alex Crichton
```

### fnv 1.0.7 — Apache-2.0 / MIT

```
Copyright (c) 2017 Contributors
```

### foldhash 0.2.0 — Zlib

```
Copyright (c) 2024 Orson Peters
```

### foreign-types 0.5.0 — MIT/Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) 2017 The foreign-types Developers
```

### foreign-types-shared 0.3.1 — MIT/Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) 2017 The foreign-types Developers
```

### form_urlencoded 1.2.2 — MIT OR Apache-2.0

```
Copyright (c) 2013-2016 The rust-url developers
```

### fs4 1.1.0 — MIT OR Apache-2.0

```
Copyright (c) 2015 The Rust Project Developers
```

### futures-channel 0.3.34 — MIT OR Apache-2.0

```
Copyright (c) 2016 Alex Crichton
Copyright (c) 2017 The Tokio Authors
```

### futures-core 0.3.34 — MIT OR Apache-2.0

```
Copyright (c) 2016 Alex Crichton
Copyright (c) 2017 The Tokio Authors
```

### futures-executor 0.3.34 — MIT OR Apache-2.0

```
Copyright (c) 2016 Alex Crichton
Copyright (c) 2017 The Tokio Authors
```

### futures-io 0.3.34 — MIT OR Apache-2.0

```
Copyright (c) 2016 Alex Crichton
Copyright (c) 2017 The Tokio Authors
```

### futures-task 0.3.34 — MIT OR Apache-2.0

```
Copyright (c) 2016 Alex Crichton
Copyright (c) 2017 The Tokio Authors
```

### futures-util 0.3.34 — MIT OR Apache-2.0

```
Copyright (c) 2016 Alex Crichton
Copyright (c) 2017 The Tokio Authors
```

### gdk 0.18.2 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### gdk-pixbuf 0.18.5 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### gdk-pixbuf-sys 0.18.0 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### gdk-sys 0.18.2 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### gdkwayland-sys 0.18.2 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### gdkx11 0.18.2 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### gdkx11-sys 0.18.2 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### generic-array 0.14.7 — MIT

```
Copyright (c) 2015 Bartłomiej Kamiński
```

### getrandom 0.2.17 — MIT OR Apache-2.0

```
Copyright (c) 2018-2024 The rust-random Project Developers
Copyright (c) 2014 The Rust Project Developers
```

### getrandom 0.3.4 — MIT OR Apache-2.0

```
Copyright (c) 2018-2025 The rust-random Project Developers
Copyright (c) 2014 The Rust Project Developers
```

### getrandom 0.4.3 — MIT OR Apache-2.0

```
Copyright (c) 2018-2026 The rust-random Project Developers
Copyright (c) 2014 The Rust Project Developers
```

### gio 0.18.4 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### gio-sys 0.18.1 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### glib 0.18.5 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### glib-sys 0.18.1 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### glob 0.3.4 — MIT OR Apache-2.0

```
Copyright (c) 2014 The Rust Project Developers
```

### gobject-sys 0.18.0 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### gtk 0.18.2 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### gtk-sys 0.18.2 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### hashbrown 0.12.3 — MIT OR Apache-2.0

```
Copyright (c) 2016 Amanieu d'Antras
```

### hashbrown 0.17.1 — MIT OR Apache-2.0

```
Copyright (c) 2016 Amanieu d'Antras
```

### heck 0.4.1 — MIT OR Apache-2.0

```
Copyright (c) 2015 The Rust Project Developers
```

### heck 0.5.0 — MIT OR Apache-2.0

```
Copyright (c) 2015 The Rust Project Developers
```

### html5ever 0.38.0 — MIT OR Apache-2.0

```
Copyright (c) 2014 The html5ever Project Developers
```

### http 1.5.0 — MIT OR Apache-2.0

```
Copyright 2017 http-rs authors
Copyright (c) 2017 http-rs authors
```

### httparse 1.10.1 — MIT OR Apache-2.0

```
Copyright (c) 2015-2025 Sean McArthur
```

### ico 0.5.0 — MIT

```
Copyright (c) 2018 Matthew D. Steele
```

### icu_collections 2.3.0 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### icu_locale_core 2.3.0 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### icu_normalizer 2.3.0 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### icu_normalizer_data 2.3.0 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### icu_properties 2.3.0 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### icu_properties_data 2.3.0 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### icu_provider 2.3.1 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### ident_case 1.0.1 — MIT/Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Ted Driggs <ted.driggs@outlook.com>
```

### idna 1.1.0 — MIT OR Apache-2.0

```
Copyright (c) 2013-2025 The rust-url developers
```

### idna_adapter 1.2.2 — Apache-2.0 OR MIT

```
Copyright (c) The rust-url developers
```

### indexmap 1.9.3 — Apache-2.0 OR MIT

```
Copyright (c) 2016--2017
```

### indexmap 2.14.0 — Apache-2.0 OR MIT

```
Copyright (c) 2016--2017
```

### infer 0.19.0 — MIT

```
Copyright (c) 2019 Bojan
```

### itoa 1.0.18 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### javascriptcore-rs 1.1.2 — MIT

```
Copyright (c) 2013-2021, The Gtk-rs Project Developers.
Copyright (c) 2021, Tauri Programme within The Commons Conservancy.
```

### javascriptcore-rs-sys 1.1.1 — MIT

```
Copyright (c) 2013-2017, The Gtk-rs Project Developers.
```

### json-patch 3.0.1 — MIT/Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) 2017 Ivan Dubrov
```

### jsonptr 0.6.3 — MIT OR Apache-2.0

```
Copyright 2024 Chance Dinkins
Copyright (c) 2022 Chance Dinkins
```

### keyboard-types 0.7.0 — MIT OR Apache-2.0

```
Copyright (c) 2017 Pyfisch
```

### libc 0.2.189 — MIT OR Apache-2.0

```
Copyright (c) The Rust Project Developers
```

### libdbus-sys 0.2.7 — Apache-2.0/MIT

```
Copyright 2014-2018 David Henningsson <diwic@ubuntu.com> and other contributors
Copyright (c) 2014-2018 David Henningsson <diwic@ubuntu.com> and other contributors
```

### libsqlite3-sys 0.38.2 — MIT

```
Copyright (c) 2014 The rusqlite developers
```

### linux-raw-sys 0.12.1 — Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Dan Gohman <dev@sunfishcode.online>
```

### litemap 0.8.3 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### lock_api 0.4.14 — MIT OR Apache-2.0

```
Copyright (c) 2016 The Rust Project Developers
```

### log 0.4.34 — MIT OR Apache-2.0

```
Copyright (c) 2014 The Rust Project Developers
```

### markup5ever 0.38.0 — MIT OR Apache-2.0

```
Copyright (c) 2014 The html5ever Project Developers
```

### memchr 2.8.3 — Unlicense OR MIT

```
Copyright (c) 2015 Andrew Gallant
```

### memoffset 0.9.1 — MIT

```
Copyright (c) 2017 Gilad Naaman
```

### mime 0.3.17 — MIT OR Apache-2.0

```
Copyright (c) 2014 Sean McArthur
```

### miniz_oxide 0.8.9 — MIT OR Zlib OR Apache-2.0

```
Copyright 2013-2014 RAD Game Tools and Valve Software
Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC
Copyright (c) 2017 Frommi
Copyright (c) 2017-2024 oyvindln
Copyright (c) 2020 Frommi
```

### mio 1.2.2 — MIT

```
Copyright (c) 2014 Carl Lerche and other MIO contributors
```

### muda 0.19.3 — Apache-2.0 OR MIT

```
Copyright (c) 2022-2022 Tauri Programme within The Commons Conservancy
```

### new_debug_unreachable 1.0.6 — MIT

```
Copyright (c) 2015 Jonathan Reem
```

### num-conv 0.2.2 — MIT OR Apache-2.0

```
Copyright (c) Jacob Pratt
```

### objc2 0.6.4 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Mads Marquart <mads@marquart.dk>
```

### objc2-app-kit 0.3.2 — Zlib OR Apache-2.0 OR MIT

Уведомление недоступно: апстрим не поставляет ни файла лицензии, ни авторов в метаданных. Репозиторий: https://github.com/madsmtm/objc2.

### objc2-core-foundation 0.3.2 — Zlib OR Apache-2.0 OR MIT

Уведомление недоступно: апстрим не поставляет ни файла лицензии, ни авторов в метаданных. Репозиторий: https://github.com/madsmtm/objc2.

### objc2-encode 4.1.0 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Mads Marquart <mads@marquart.dk>
```

### objc2-exception-helper 0.1.1 — Zlib OR Apache-2.0 OR MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Mads Marquart <mads@marquart.dk>
```

### objc2-foundation 0.3.2 — MIT

Уведомление недоступно: апстрим не поставляет ни файла лицензии, ни авторов в метаданных. Репозиторий: https://github.com/madsmtm/objc2.

### objc2-web-kit 0.3.2 — Zlib OR Apache-2.0 OR MIT

Уведомление недоступно: апстрим не поставляет ни файла лицензии, ни авторов в метаданных. Репозиторий: https://github.com/madsmtm/objc2.

### once_cell 1.21.4 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Aleksey Kladov <aleksey.kladov@gmail.com>
```

### option-ext 0.2.0 — MPL-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Simon Ochsenreither <simon@ochsenreither.de>
```

### pango 0.18.3 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### pango-sys 0.18.0 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The gtk-rs Project Developers
```

### parking_lot 0.12.5 — MIT OR Apache-2.0

```
Copyright (c) 2016 The Rust Project Developers
```

### parking_lot_core 0.9.12 — MIT OR Apache-2.0

```
Copyright (c) 2016 The Rust Project Developers
```

### percent-encoding 2.3.2 — MIT OR Apache-2.0

```
Copyright (c) 2013-2025 The rust-url developers
```

### phf 0.13.1 — MIT

```
Copyright (c) 2014-2022 Steven Fackler, Yuki Okushi
```

### phf_generator 0.13.1 — MIT

```
Copyright (c) 2014-2022 Steven Fackler, Yuki Okushi
```

### phf_shared 0.13.1 — MIT

```
Copyright (c) 2014-2022 Steven Fackler, Yuki Okushi
```

### pin-project-lite 0.2.17 — Apache-2.0 OR MIT

Уведомление недоступно: файл лицензии в пакете есть, но строки с уведомлением об авторских правах в нём нет, и авторы в метаданных не указаны. Репозиторий: https://github.com/taiki-e/pin-project-lite.

### plist 1.10.0 — MIT

```
Copyright (c) 2015 Edward Barnard
```

### png 0.17.16 — MIT OR Apache-2.0

```
Copyright (c) 2015 nwin
```

### png 0.18.1 — MIT OR Apache-2.0

```
Copyright (c) 2015 nwin
```

### potential_utf 0.1.6 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### powerfmt 0.2.0 — MIT OR Apache-2.0

```
Copyright 2023 Jacob Pratt et al.
Copyright (c) 2023 Jacob Pratt et al.
```

### precomputed-hash 0.1.1 — MIT

```
Copyright (c) 2017 Emilio Cobos Álvarez
```

### proc-macro-crate 1.3.1 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Bastian Köcher <git@kchr.de>
```

### proc-macro-crate 2.0.2 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Bastian Köcher <git@kchr.de>
```

### proc-macro-error 1.0.4 — MIT OR Apache-2.0

```
Copyright 2019-2020 CreepySkeleton <creepy-skeleton@yandex.ru>
Copyright (c) 2019-2020 CreepySkeleton
```

### proc-macro2 1.0.107 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
Alex Crichton <alex@alexcrichton.com>
```

### quick-xml 0.41.0 — MIT

```
Copyright (c) 2016 Johann Tuffe
```

### quote 1.0.47 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### raw-window-handle 0.6.2 — MIT OR Apache-2.0 OR Zlib

```
Copyright (c) 2019 Osspial
Copyright (c) 2020 Osspial
```

### regex 1.13.1 — MIT OR Apache-2.0

```
Copyright (c) 2014 The Rust Project Developers
```

### regex-automata 0.4.18 — MIT OR Apache-2.0

```
Copyright (c) 2014 The Rust Project Developers
```

### regex-syntax 0.8.11 — MIT OR Apache-2.0

```
Copyright (c) 2014 The Rust Project Developers
```

### rfd 0.16.0 — MIT

```
Copyright (c) 2022 Bartłomiej Maryńczak
```

### ring 0.17.14 — Apache-2.0 AND ISC

```
Copyright (c) 2009 The Go Authors. All rights reserved.
Copyright 2015 The Chromium Authors. All rights reserved.
Copyright 2015-2025 Brian Smith.
```

### rusqlite 0.40.2 — MIT

```
Copyright (c) 2014 The rusqlite developers
```

### rustc-hash 2.1.3 — Apache-2.0 OR MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The Rust Project Developers
```

### rustix 1.1.4 — Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Dan Gohman <dev@sunfishcode.online>
Jakub Konka <kubkon@jakubkonka.com>
```

### rustls 0.23.43 — Apache-2.0 OR ISC OR MIT

```
Copyright (c) 2016, Joseph Birr-Pixton <jpixton@gmail.com>
Copyright (c) 2016 Joseph Birr-Pixton <jpixton@gmail.com>
```

### rustls-pki-types 1.15.1 — MIT OR Apache-2.0

```
Copyright 2023 Dirkjan Ochtman
Copyright (c) 2023 Dirkjan Ochtman <dirkjan@ochtman.nl>
```

### rustls-webpki 0.103.15 — ISC

```
Copyright 2015 Brian Smith.
```

### same-file 1.0.6 — Unlicense/MIT

```
Copyright (c) 2017 Andrew Gallant
```

### schemars 0.8.22 — MIT

```
Copyright (c) 2019 Graham Esau
```

### scopeguard 1.2.0 — MIT OR Apache-2.0

```
Copyright (c) 2016-2019 Ulrik Sverdrup "bluss" and scopeguard developers
```

### selectors 0.36.1 — MPL-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The Servo Project Developers
```

### semver 1.0.28 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### serde 1.0.229 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Erick Tryzelaar <erick.tryzelaar@gmail.com>
David Tolnay <dtolnay@gmail.com>
```

### serde-untagged 0.1.9 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### serde_core 1.0.229 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Erick Tryzelaar <erick.tryzelaar@gmail.com>
David Tolnay <dtolnay@gmail.com>
```

### serde_derive_internals 0.29.1 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Erick Tryzelaar <erick.tryzelaar@gmail.com>
David Tolnay <dtolnay@gmail.com>
```

### serde_json 1.0.151 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Erick Tryzelaar <erick.tryzelaar@gmail.com>
David Tolnay <dtolnay@gmail.com>
```

### serde_spanned 0.6.9 — MIT OR Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) Individual contributors
```

### serde_spanned 1.1.1 — MIT OR Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) Individual contributors
```

### serde_with 3.22.0 — MIT OR Apache-2.0

```
Copyright (c) 2015
```

### serialize-to-javascript 0.1.2 — MIT OR Apache-2.0

```
Copyright (c) 2021 Chip Reed
```

### servo_arc 0.4.3 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The Servo Project Developers
```

### sha2 0.10.9 — MIT OR Apache-2.0

```
Copyright (c) 2006-2009 Graydon Hoare
Copyright (c) 2009-2013 Mozilla Foundation
Copyright (c) 2016 Artyom Pavlov
```

### signal-hook-registry 1.4.8 — MIT OR Apache-2.0

```
Copyright (c) 2017 tokio-jsonrpc developers
```

### simd-adler32 0.3.10 — MIT

```
Copyright (c) [2021] [Marvin Countryman]
```

### siphasher 1.0.3 — MIT/Apache-2.0

```
Copyright 2012-2016 The Rust Project Developers.
Copyright 2016-2026 Frank Denis.
```

### slab 0.4.12 — MIT

```
Copyright (c) 2019 Carl Lerche
```

### smallvec 1.15.2 — MIT OR Apache-2.0

```
Copyright (c) 2018 The Servo Project Developers
```

### softbuffer 0.4.8 — MIT OR Apache-2.0

```
Copyright 2022 Kirill Chibisov
```

### soup3 0.5.0 — MIT

```
Copyright (c) 2013-2017, The Gtk-rs Project Developers.
```

### soup3-sys 0.5.0 — MIT

```
Copyright (c) 2013-2017, The Gtk-rs Project Developers.
```

### stable_deref_trait 1.2.1 — MIT OR Apache-2.0

```
Copyright (c) 2017 Robert Grosse
```

### string_cache 0.9.0 — MIT OR Apache-2.0

```
Copyright (c) 2012-2013 Mozilla Foundation
```

### strsim 0.11.1 — MIT

```
Copyright (c) 2015 Danny Guo
Copyright (c) 2016 Titus Wormer <tituswormer@gmail.com>
Copyright (c) 2018 Akash Kurdekar
```

### subtle 2.6.1 — BSD-3-Clause

```
Copyright (c) 2016-2017 Isis Agora Lovecruft, Henry de Valence. All rights reserved.
Copyright (c) 2016-2024 Isis Agora Lovecruft. All rights reserved.
```

### swift-rs 1.0.8 — MIT OR Apache-2.0

```
Copyright 2023 The swift-rs developers
Copyright (c) 2023 The swift-rs Developers
```

### syn 1.0.109 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### syn 2.0.119 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### syn 3.0.4 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### synstructure 0.13.2 — MIT

```
Copyright 2016 Nika Layzell
```

### tao 0.35.3 — Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
```

### tauri 2.11.5 — Apache-2.0 OR MIT

```
Copyright (c) 2017 - Present Tauri Apps Contributors
```

### tauri-codegen 2.6.3 — Apache-2.0 OR MIT

```
Copyright (c) 2017 - Present Tauri Apps Contributors
```

### tauri-plugin-dialog 2.7.3 — Apache-2.0 OR MIT

```
Copyright (c) 2017 - Present Tauri Apps Contributors
```

### tauri-plugin-fs 2.5.2 — Apache-2.0 OR MIT

```
Copyright (c) 2017 - Present Tauri Apps Contributors
```

### tauri-runtime 2.11.3 — Apache-2.0 OR MIT

```
Copyright (c) 2017 - Present Tauri Apps Contributors
```

### tauri-runtime-wry 2.11.4 — Apache-2.0 OR MIT

```
Copyright (c) 2017 - Present Tauri Apps Contributors
```

### tauri-utils 2.9.3 — Apache-2.0 OR MIT

```
Copyright (c) 2017 - Present Tauri Apps Contributors
```

### tendril 0.5.1 — MIT OR Apache-2.0

```
Copyright (c) 2015 Keegan McAllister
```

### thiserror 1.0.69 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### thiserror 2.0.20 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### time 0.3.55 — MIT OR Apache-2.0

```
Copyright (c) Jacob Pratt et al.
```

### time-core 0.1.9 — MIT OR Apache-2.0

```
Copyright (c) Jacob Pratt et al.
```

### tinystr 0.8.4 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### tokio 1.53.1 — MIT

```
Copyright (c) Tokio Contributors
```

### toml 1.1.4+spec-1.1.0 — MIT OR Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) Individual contributors
```

### toml_datetime 0.6.3 — MIT OR Apache-2.0

```
Copyright (c) 2014 Alex Crichton
```

### toml_datetime 1.1.1+spec-1.1.0 — MIT OR Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) Individual contributors
```

### toml_edit 0.19.15 — MIT OR Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) Individual contributors
```

### toml_edit 0.20.2 — MIT OR Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) Individual contributors
```

### toml_parser 1.1.3+spec-1.1.0 — MIT OR Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) Individual contributors
```

### toml_writer 1.1.2+spec-1.1.0 — MIT OR Apache-2.0

```
Copyright {yyyy} {name of copyright owner}
Copyright (c) Individual contributors
```

### tracing 0.1.44 — MIT

```
Copyright (c) 2019 Tokio Contributors
```

### tracing-core 0.1.36 — MIT

```
Copyright (c) 2019 Tokio Contributors
```

### typeid 1.0.3 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

### typenum 1.20.1 — MIT OR Apache-2.0

```
Copyright 2014 Paho Lurie-Gregg
Copyright (c) 2014 Paho Lurie-Gregg
```

### unic-char-property 0.9.0 — MIT/Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The UNIC Project Developers
```

### unic-char-range 0.9.0 — MIT/Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The UNIC Project Developers
```

### unic-common 0.9.0 — MIT/Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The UNIC Project Developers
```

### unic-ucd-ident 0.9.0 — MIT/Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The UNIC Project Developers
```

### unic-ucd-version 0.9.0 — MIT/Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
The UNIC Project Developers
```

### unicode-ident 1.0.24 — (MIT OR Apache-2.0) AND Unicode-3.0

```
Copyright © 1991-2023 Unicode, Inc.
```

### unicode-segmentation 1.13.3 — MIT OR Apache-2.0

```
Copyright (c) 2015 The Rust Project Developers
```

### untrusted 0.9.0 — ISC

```
Copyright 2015-2016 Brian Smith.
```

### ureq 3.4.0 — MIT OR Apache-2.0

```
Copyright (c) 2019 Martin Algesten
```

### ureq-proto 0.6.1 — MIT OR Apache-2.0

```
Copyright 2022 Martin Algesten
```

### url 2.5.8 — MIT OR Apache-2.0

```
Copyright (c) 2013-2025 The rust-url developers
```

### urlpattern 0.3.0 — MIT

```
Copyright (c) 2021 the Deno authors
```

### utf8-zero 0.8.1 — MIT OR Apache-2.0

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
Simon Sapin <simon.sapin@exyr.org>
Martin Algesten <martin@algesten.se>
```

### utf8_iter 1.0.4 — Apache-2.0 OR MIT

```
Copyright Mozilla Foundation
```

### uuid 1.25.0 — Apache-2.0 OR MIT

```
Copyright (c) 2014 The Rust Project Developers
Copyright (c) 2018 Ashley Mannix, Christopher Armstrong, Dylan DPC, Hunar Roop Kahlon
```

### walkdir 2.5.0 — Unlicense/MIT

```
Copyright (c) 2015 Andrew Gallant
```

### web_atoms 0.2.6 — MIT OR Apache-2.0

```
Copyright (c) 2014 The html5ever Project Developers
```

### webkit2gtk 2.0.2 — MIT

```
Copyright (c) 2016 Boucher, Antoni <bouanto@zoho.com>
Copyright (c) 2017-2021, The Gtk-rs Project Developers.
Copyright (c) 2021, Tauri Programme within The Commons Conservancy
```

### webkit2gtk-sys 2.0.2 — MIT

```
Copyright (c) 2016 Boucher, Antoni <bouanto@zoho.com>
```

### webpki-roots 1.0.9 — CDLA-Permissive-2.0

Уведомление недоступно: файл лицензии в пакете есть, но строки с уведомлением об авторских правах в нём нет, и авторы в метаданных не указаны. Репозиторий: https://github.com/rustls/webpki-roots.

### webview2-com 0.38.2 — MIT

Уведомление недоступно: апстрим не поставляет ни файла лицензии, ни авторов в метаданных. Репозиторий: https://github.com/wravery/webview2-rs.

### webview2-com-sys 0.38.2 — MIT

Уведомление недоступно: апстрим не поставляет ни файла лицензии, ни авторов в метаданных. Репозиторий: https://github.com/wravery/webview2-rs.

### winapi-util 0.1.11 — Unlicense OR MIT

```
Copyright (c) 2017 Andrew Gallant
```

### window-vibrancy 0.6.0 — Apache-2.0 OR MIT

```
Copyright (c) 2020-2022 Tauri Programme within The Commons Conservancy
```

### windows 0.61.3 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-collections 0.2.0 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-core 0.61.2 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-future 0.2.1 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-link 0.1.3 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-link 0.2.1 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-numerics 0.2.0 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-result 0.3.4 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-strings 0.4.2 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-sys 0.59.0 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-sys 0.60.2 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-sys 0.61.2 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-targets 0.52.6 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-targets 0.53.5 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-threading 0.1.0 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows-version 0.1.7 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows_x86_64_msvc 0.52.6 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### windows_x86_64_msvc 0.53.1 — MIT OR Apache-2.0

```
Copyright (c) Microsoft Corporation.
```

### winnow 0.5.40 — MIT

Уведомление недоступно: файл лицензии в пакете есть, но строки с уведомлением об авторских правах в нём нет, и авторы в метаданных не указаны. Репозиторий: https://github.com/winnow-rs/winnow.

### winnow 1.0.4 — MIT

Уведомление недоступно: файл лицензии в пакете есть, но строки с уведомлением об авторских правах в нём нет, и авторы в метаданных не указаны. Репозиторий: https://github.com/winnow-rs/winnow.

### writeable 0.6.4 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### wry 0.55.1 — Apache-2.0 OR MIT

```
Copyright (c) 2020-2023 Ngo Iok Ui & Tauri Programme within The Commons Conservancy
```

### x11 2.21.0 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
daggerbot <daggerbot@gmail.com>
Erle Pereira <erle@erlepereira.com>
AltF02 <contact@altf2.dev>
```

### x11-dl 2.21.0 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
daggerbot <daggerbot@gmail.com>
Erle Pereira <erle@erlepereira.com>
AltF02 <contact@altf2.dev>
```

### yoke 0.8.3 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### zerofrom 0.1.8 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### zeroize 1.9.0 — Apache-2.0 OR MIT

```
Copyright (c) 2018-2026 The RustCrypto Project Developers
```

### zerotrie 0.2.5 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### zerovec 0.11.8 — Unicode-3.0

```
Copyright © 2020-2024 Unicode, Inc.
```

### zip 4.6.1 — MIT

```
Copyright (c) 2014 Mathijs van de Nes
```

### zmij 1.0.23 — MIT

Уведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.

```
David Tolnay <dtolnay@gmail.com>
```

## npm-пакеты

### @babel/helper-string-parser 7.29.7 — MIT

```
Copyright (c) 2014-present Sebastian McKenzie and other contributors
```

### @babel/helper-validator-identifier 7.29.7 — MIT

```
Copyright (c) 2014-present Sebastian McKenzie and other contributors
```

### @babel/parser 7.29.8 — MIT

```
Copyright (C) 2012-2014 by various contributors (see AUTHORS)
```

### @babel/types 7.29.8 — MIT

```
Copyright (c) 2014-present Sebastian McKenzie and other contributors
```

### @jridgewell/sourcemap-codec 1.5.5 — MIT

```
Copyright 2024 Justin Ridgewell <justin@ridgewell.name>
```

### @tauri-apps/api 2.11.1 — Apache-2.0 OR MIT

```
Copyright (c) 2017 - Present Tauri Apps Contributors
```

### @tauri-apps/plugin-dialog 2.7.3 — MIT OR Apache-2.0

Уведомление недоступно: файл лицензии в пакете есть, но строки с уведомлением об авторских правах в нём нет, и авторы в метаданных не указаны. Репозиторий: https://github.com/tauri-apps/plugins-workspace.

### @vue/compiler-core 3.5.41 — MIT

```
Copyright (c) 2018-present, Yuxi (Evan) You
```

### @vue/compiler-dom 3.5.41 — MIT

```
Copyright (c) 2018-present, Yuxi (Evan) You
```

### @vue/compiler-sfc 3.5.41 — MIT

```
Copyright (c) 2018-present, Yuxi (Evan) You
```

### @vue/compiler-ssr 3.5.41 — MIT

```
Copyright (c) 2018-present, Yuxi (Evan) You
```

### @vue/devtools-api 8.2.1 — MIT

```
Copyright (c) 2023 webfansplz
```

### @vue/devtools-kit 8.2.1 — MIT

```
Copyright (c) 2023 webfansplz
```

### @vue/devtools-shared 8.2.1 — MIT

```
Copyright (c) 2023 webfansplz
```

### @vue/reactivity 3.5.41 — MIT

```
Copyright (c) 2018-present, Yuxi (Evan) You
```

### @vue/runtime-core 3.5.41 — MIT

```
Copyright (c) 2018-present, Yuxi (Evan) You
```

### @vue/runtime-dom 3.5.41 — MIT

```
Copyright (c) 2018-present, Yuxi (Evan) You
```

### @vue/server-renderer 3.5.41 — MIT

```
Copyright (c) 2018-present, Yuxi (Evan) You
```

### @vue/shared 3.5.41 — MIT

```
Copyright (c) 2018-present, Yuxi (Evan) You
```

### birpc 2.9.0 — MIT

```
Copyright (c) 2021 Anthony Fu <https://github.com/antfu>
```

### csstype 3.2.3 — MIT

```
Copyright (c) 2017-2018 Fredrik Nicol
```

### entities 7.0.1 — BSD-2-Clause

```
Copyright (c) Felix Böhm
```

### estree-walker 2.0.2 — MIT

```
Copyright (c) 2015-20 [these people](https://github.com/Rich-Harris/estree-walker/graphs/contributors)
```

### hookable 5.5.3 — MIT

```
Copyright (c) Pooya Parsa <pooya@pi0.io>
```

### magic-string 0.30.21 — MIT

```
Copyright 2018 Rich Harris
```

### nanoid 3.3.18 — MIT

```
Copyright 2017 Andrey Sitnik <andrey@sitnik.ru>
```

### nostics 1.2.0 — MIT

```
Copyright (c) 2026-present Vercel Inc.
```

### perfect-debounce 2.1.0 — MIT

```
Copyright (c) Pooya Parsa <pooya@pi0.io>
```

### picocolors 1.1.1 — ISC

```
Copyright (c) 2021-2024 Oleksii Raspopov, Kostiantyn Denysov, Anton Verinov
```

### pinia 4.0.3 — MIT

```
Copyright (c) 2019-present Eduardo San Martin Morote
```

### postcss 8.5.26 — MIT

```
Copyright 2013 Andrey Sitnik <andrey@sitnik.es>
```

### source-map-js 1.2.1 — BSD-3-Clause

```
Copyright (c) 2009-2011, Mozilla Foundation and contributors
```

### vue 3.5.41 — MIT

```
Copyright (c) 2018-present, Yuxi (Evan) You
```

## Пакеты, для которых уведомление недоступно

Выдумывать правообладателя мы не станем: ниже — честный список с адресом
репозитория, где уведомление можно получить у самого автора. Случая
два, и они разные:

- **файл лицензии есть, копирайта в нём нет** — пакет поставляет текст
  лицензии, но тот начинается сразу с условий, без строки об авторских
  правах, и авторы в метаданных не указаны;
- **файла лицензии нет вовсе** — лицензия заявлена только полем
  `license` в манифесте.
- `cargo-platform 0.1.9` (MIT OR Apache-2.0) — файл лицензии есть, копирайта в нём нет; https://github.com/rust-lang/cargo
- `objc2-app-kit 0.3.2` (Zlib OR Apache-2.0 OR MIT) — файла лицензии нет вовсе; https://github.com/madsmtm/objc2
- `objc2-core-foundation 0.3.2` (Zlib OR Apache-2.0 OR MIT) — файла лицензии нет вовсе; https://github.com/madsmtm/objc2
- `objc2-foundation 0.3.2` (MIT) — файла лицензии нет вовсе; https://github.com/madsmtm/objc2
- `objc2-web-kit 0.3.2` (Zlib OR Apache-2.0 OR MIT) — файла лицензии нет вовсе; https://github.com/madsmtm/objc2
- `pin-project-lite 0.2.17` (Apache-2.0 OR MIT) — файл лицензии есть, копирайта в нём нет; https://github.com/taiki-e/pin-project-lite
- `webpki-roots 1.0.9` (CDLA-Permissive-2.0) — файл лицензии есть, копирайта в нём нет; https://github.com/rustls/webpki-roots
- `webview2-com 0.38.2` (MIT) — файла лицензии нет вовсе; https://github.com/wravery/webview2-rs
- `webview2-com-sys 0.38.2` (MIT) — файла лицензии нет вовсе; https://github.com/wravery/webview2-rs
- `winnow 0.5.40` (MIT) — файл лицензии есть, копирайта в нём нет; https://github.com/winnow-rs/winnow
- `winnow 1.0.4` (MIT) — файл лицензии есть, копирайта в нём нет; https://github.com/winnow-rs/winnow
- `@tauri-apps/plugin-dialog 2.7.3` (MIT OR Apache-2.0) — файл лицензии есть, копирайта в нём нет; https://github.com/tauri-apps/plugins-workspace
