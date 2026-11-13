# Kockers Browser

Нативный браузер на Rust с Chromium Embedded Framework. Windows-версия использует собственное скруглённое Win32-окно, вкладки и windowed CEF для содержимого. Sandbox и web security не отключаются. Восстановленные фоновые вкладки не создают браузерный контент до активации, а таймер анимаций останавливается в покое.

## Установка для пользователей Windows

1. Скачайте `KockersBrowser-0.2.0-windows-x64-setup.exe` из [релиза 0.2.0](https://github.com/nestjest/kockers-browser/releases/tag/v0.2.0).
2. Запустите установщик и следуйте шагам мастера. Установка выполняется для текущего пользователя и не требует прав администратора.
3. По умолчанию браузер устанавливается в `%LOCALAPPDATA%\Programs\Kockers Browser`; ярлык создаётся в меню «Пуск».

После установки браузер автоматически проверяет GitHub Releases при запуске. Новый установщик скачивается по HTTPS, сверяется с SHA-256 из `SHA256SUMS.txt`, устанавливается без дополнительных действий и перезапускает браузер. Для диагностики проверку можно отключить аргументом `--disable-update-check`.

Установщик и браузер пока не имеют цифровой подписи, поэтому Windows SmartScreen может показать предупреждение. Удаление выполняется через «Установленные приложения» в настройках Windows.

## Технологии

- Rust stable, Cargo, edition 2024.
- `cef = 156.4.0+156.0.26` (`tauri-apps/cef-rs`).
- CEF binary distribution `156.0.26` скачивается автоматически официальным `download-cef` при первой сборке.
- `windows-sys` для нативного Win32-окна. Electron, Node.js, WebView, `wry`, iframe и off-screen rendering не используются.

## Требования

Текущая целевая платформа: 64-bit Windows 10/11.

1. Установите Rust stable: <https://rustup.rs/>.
2. Установите MSVC Build Tools и Windows SDK, например:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --accept-package-agreements --accept-source-agreements --override "--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

`VCTools` включает необходимые CMake/Ninja. Скрипт `scripts/cargo.ps1` сам находит Visual Studio через `vswhere` и импортирует MSVC-окружение.

## CEF

Ручная установка CEF не нужна. Первая сборка скачивает и распаковывает pinned-версию в Cargo `OUT_DIR`, компилирует `libcef_dll_wrapper` и копирует runtime рядом с target-файлами. Загрузка занимает некоторое время: `libcef.dll` и ресурсы имеют большой размер.

Для использования уже установленной совместимой distribution можно задать `CEF_PATH` на её корневой каталог перед вызовом скрипта. Версия должна соответствовать `cef-dll-sys`; при несовпадении build script загрузит правильную версию в подкаталог.

## Сборка и запуск

Из корня проекта:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build.ps1
.\target\bundle\kockers_browser.exe
```

Сборка и немедленный запуск:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/run.ps1
```

Release:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/run.ps1 -Release
```

Сборка EXE-установщика:

```powershell
winget install --id JRSoftware.InnoSetup --exact
powershell -ExecutionPolicy Bypass -File scripts/installer.ps1
```

Скрипт собирает release и создаёт установщик в `dist`. Inno Setup включает весь необходимый CEF runtime и локальную копию Microsoft `vcruntime140.dll`, но исключает PDB, LIB и временные build-файлы.

Для отдельных команд Cargo:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/cargo.ps1 test --lib
powershell -ExecutionPolicy Bypass -Command '& "./scripts/cargo.ps1" -CargoArgs @("clippy","--all-targets","--","-D","warnings")'
```

На Windows с включённым CEF sandbox приложение собирается как `kockers_browser.dll`. В дистрибутиве `kockers_browser.exe` — это официальный CEF `bootstrap.exe`, который загружает DLL. Не запускайте `target/debug/kockers_browser.exe` напрямую и не переносите DLL без runtime-файлов.

## Дистрибутив

Каталог `target/bundle` самодостаточен:

```text
kockers_browser.exe          # CEF bootstrap
kockers_browser.dll          # Rust UI, навигация и CEF client
kockers_browser.exe.manifest
libcef.dll                   # Chromium/CEF
chrome_elf.dll
icudtl.dat
resources.pak
v8_context_snapshot.bin
*.dll                        # графика и остальные runtime-компоненты CEF
locales/
```

Debug-сборка также содержит PDB. Для распространения копируйте каталог целиком. Иконка приложения и компактный знак K встроены в DLL через `build.rs`.

## Управление

- Введите полный HTTP/HTTPS URL, домен без схемы или обычный поисковый запрос и нажмите Enter.
- `Ctrl+L` — адресная строка; `Ctrl+R` или `F5` — обновить; `Escape` — остановить загрузку.
- `Alt+Left`, `Alt+Right` — назад/вперёд; `F12` — отдельное окно DevTools.
- `Ctrl+T` — новая вкладка; `Ctrl+W` — закрыть; `Ctrl+Shift+T` — восстановить закрытую.
- `Ctrl+Tab` и `Ctrl+Shift+Tab` — следующая/предыдущая вкладка; `Ctrl+1`…`Ctrl+9` — выбор по номеру.
- `Ctrl+Shift+N` — приватное окно с изолированным контекстом без записи истории и сессии.
- Вкладки можно перетаскивать, закрывать средней кнопкой, закреплять через контекстное меню.
- Кнопка `↓` открывает панель загрузок с паузой, отменой и открытием завершённых файлов. Перед запуском исполняемых файлов показывается предупреждение.

Меню `⋮` управляет сохранением истории, выбором пути загрузки и очисткой данных. Обычная сессия, настройки и история хранятся в `%LOCALAPPDATA%\Kockers Browser`; загрузки по умолчанию сохраняются в `%USERPROFILE%\Downloads`.

Поисковые запросы открываются через Google. Стартовая страница и страница ошибок отображаются самим Chromium. Popup без действия пользователя блокируется, а разрешения сайтов требуют отдельного подтверждения. HTTP отмечается предупреждающим значком в адресной строке.

## Структура исходников

```text
src/
  lib.rs                  # RunWinMain sandbox DLL
  main.rs                 # сообщение при неправильном прямом запуске
  app.rs                  # browser/auxiliary process и жизненный цикл CEF
  browser.rs              # вкладки, CEF callbacks, разрешения и загрузки
  navigation.rs           # чистая модель URL/поиска
  storage.rs              # настройки, история и сессия
  update.rs               # GitHub Releases, WinHTTP и проверка SHA-256
  platform/windows.rs     # собственное Win32-окно, панель и анимации
  ui/pages.rs             # стартовая страница и страница ошибок
  bin/bundle.rs           # упаковка через cef::build_util
assets/
  logo.svg
  logo.png
  logo-small.png
  app.ico                 # иконка приложения на светлой плашке: exe, ярлыки, установщик
  app-icon.svg            # исходник app.ico
  logo-mark.ico           # «K» без плашки для панели вкладок
scripts/
  cargo.ps1
  build.ps1
  run.ps1
```

CEF-вызовы выполняются после освобождения mutex контроллера. Закрытие окна использует документированный цикл `try_close_browser`/`WM_CLOSE`; отдельная вкладка завершает закрытие через `do_close` и уничтожение собственного дочернего HWND. Message loop завершается после закрытия последнего окна.

## Ограничения

- Реализована только Windows x64; платформенный код изолирован в `src/platform`.
- Нет закладок, расширений, синхронизации и страницы просмотра истории.
- Поисковая система фиксирована. Нет интеграции с внешним Safe Browsing или антивирусом; предупреждения не заменяют проверку источника файла.
- Для production-релиза ещё нужны подпись кода, политика обновлений CEF и дополнительное UX-тестирование DPI/доступности.
