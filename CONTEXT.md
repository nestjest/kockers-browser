# Контекст проекта Kockers Browser

## Текущее состояние

- Актуальная версия: **0.2.4.3**.
- Репозиторий: `https://github.com/nestjest/kockers-browser`.
- Релиз: `https://github.com/nestjest/kockers-browser/releases/tag/v0.2.4.3`.
- Целевая платформа: 64-bit Windows 10/11.
- Приложение: нативный браузер на Rust с Chromium Embedded Framework, без Electron, Node.js, WebView, `wry` или iframe.

## Технологии

- Rust stable, edition 2024.
- `cef = 156.4.0`, binary distribution CEF `156.0.26`.
- `windows-sys 0.61` для Win32, GDI/GDI+, WinHTTP, Windows CNG и запуска процессов.
- `url 2.5` для разбора URL обновлений.
- Inno Setup 6 для установщика.
- Sandbox и web security включены.

Собственный `kockers_browser.exe` не является точкой входа приложения. В дистрибутиве это CEF bootstrap, загружающий `kockers_browser.dll`; Rust реализует экспорт `RunWinMain`. Нельзя запускать настройки проекта через `target/debug/kockers_browser.exe` напрямую.

## Структура

- `src/lib.rs` — версия приложения и вход CEF DLL.
- `src/app.rs` — browser/auxiliary processes, инициализация CEF, фоновые параметры Chromium и message loop.
- `src/browser.rs` — вкладки, CEF client callbacks, загрузки, разрешения, навигация и жизненный цикл browser objects.
- `src/navigation.rs` — нормализация адресов и поисковых запросов.
- `src/storage.rs` — настройки, сессия, профиль, закладки и история.
- `src/update.rs` — GitHub Releases, сравнение версий, WinHTTP, SHA-256 и запуск установщика.
- `src/platform/windows.rs` — собственное Win32-окно, панели, вкладки, ввод и отрисовка.
- `src/platform/icons.rs` — Material SVG path parser и отрисовка иконок через GDI+.
- `src/ui/pages.rs` — стартовая страница и страница ошибок.
- `src/ui/internal.rs` — профиль, настройки и история как внутренние HTML-страницы.
- `src/bin/bundle.rs` — перенос ресурсов и сборка самодостаточного `target/bundle`.
- `installer/KockersBrowser.iss` — установщик Inno Setup.
- `scripts/build.ps1`, `run.ps1`, `cargo.ps1`, `installer.ps1` — стандартные команды MSVC/Cargo/упаковки.

## Архитектура интерфейса

### Главное окно и CEF

Главное окно — собственное Win32 `KockersBrowserWindow` с `WS_CLIPCHILDREN`. Адресная строка — дочерний EDIT, страницы — отдельные дочерние CEF-окна. Активная CEF-страница показывается сразу после создания; нельзя скрывать её до `on_load_end`, иначе появляется пустой промежуток. Фоновые CEF-окна скрываются.

### Панели меню и загрузок

Панели — owned top-level `KockersBrowserOverlay` со стилями `WS_POPUP`, `WS_EX_LAYERED`, `WS_EX_TOOLWINDOW`, `WS_EX_NOACTIVATE`.

- Показ и скрытие используют системный `AnimateWindow` с `AW_BLEND` и `AW_BLEND | AW_HIDE`.
- Длительность — 120 мс; настройка «Уменьшить анимации» отключает переходы.
- Нет собственного покадрового цикла панелей, покадрового alpha или перемещения через `WM_TIMER`.
- `WM_PRINTCLIENT` рисует содержимое для системных кадров.
- После fade-in alpha явно возвращается к 255.
- При ошибке `AnimateWindow` есть fallback на `ShowWindow`.
- Владелец получается через `GetWindow(..., GW_OWNER)`, не через `GetParent`.
- Позиция панели переводится из client coordinates главного окна через `ClientToScreen`; `WM_MOVE` обновляет её положение.
- Popup обрабатывает `MA_NOACTIVATE`, `WM_LBUTTONDOWN` с `SetCapture` и `WM_LBUTTONUP` с `ReleaseCapture`. Это необходимо для работоспособности команд меню.

### Вкладки и анимации

Нативные вкладки используют простой `Transition` с линейной интерполяцией за 120 мс, без пружин, bounce и overshoot.

- `TabVisual.x` управляет положением, `TabVisual.width` — раскрытием и схлопыванием ширины.
- Новая вкладка начинается с нулевой ширины и раскрывается; соседние вкладки и кнопка «+» используют текущие значения анимации.
- `TabVisual.closing` сначала запускает out-переход до нулевой ширины, затем вызывается фактический `BrowserController::close_tab`.
- Крестик, средняя кнопка, `Ctrl+W` и контекстное меню должны использовать этот общий путь закрытия.
- Текст обрезается по фактической ширине через clip DC.
- Таймер анимации полностью останавливается в покое.

### Области перерисовки

Нельзя вызывать `InvalidateRect` на всём главном окне при каждом изменении вкладки.

- `refresh_tabs` возвращает `false`, если snapshots не изменились.
- Изменённые вкладки invalidate только верхние 54 px.
- Во время анимации `paint_toolbar(..., tabs_only = true)` рисует только логотип, вкладки и кнопки заголовка.
- Адресная строка и состояние загрузки используют узкие области invalidate.
- `on_after_created` не повторяет синхронизацию активной вкладки, уже выполненную при создании/активации.
- Фоновые вкладки не синхронизируют активный адрес.

### Иконки и кнопки заголовка

- Иконки рисуются через GDI+ из встроенных Material paths.
- Подсветка сворачивания, развёртывания и закрытия — плашка 36×36 px со скруглением 10 px и отступами внутри области нажатия.
- Крестик закрытия при наведении всегда белый.
- Основные оконные иконки загружаются с `LR_SHARED`; логотип темы остаётся отдельным ресурсом с явным `DestroyIcon`.

## Данные и память

Данные находятся в `%LOCALAPPDATA%\Kockers Browser`, установка по умолчанию — `%LOCALAPPDATA%\Programs\Kockers Browser`.

- При включённой отложенной загрузке восстановленные и новые фоновые вкладки не создают CEF-контент до активации.
- Нативный UI не хранит URL всех вкладок в snapshots и не клонирует их для переключения.
- Снимки загрузок освобождаются при закрытии панели.
- Приватные окна используют изолированный in-memory request context.
- Ограничения: история — 2000 записей, закрытые вкладки — 20, закладки — 200, восстановление сессии — 50 вкладок, список загрузок — 50.

## Обновления

`src/update.rs` читает `releases.atom`, выбирает максимальную поддерживаемую версию, скачивает установщик и `SHA256SUMS.txt` только по HTTPS с GitHub, проверяет SHA-256 через Windows CNG и запускает:

```text
/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /UPDATE
```

Файлы скачиваются в `%LOCALAPPDATA%\Kockers Browser\Updates`, статус записывается в `update-status.txt`. Поддерживаются четырёхсегментные hotfix-версии и старый суффикс `a`.

В Inno Setup используется `CloseApplications=force`: Restart Manager не мог закрыть все CEF-процессы, а suppressed message box выбирал Abort с кодом 5. `[Run]` с `/updated` перезапускает браузер после установки.

## Проверка и сборка

```powershell
powershell -ExecutionPolicy Bypass -File scripts/cargo.ps1 test --lib
powershell -Command '& "./scripts/cargo.ps1" -CargoArgs @("clippy","--all-targets","--","-D","warnings")'
powershell -ExecutionPolicy Bypass -File scripts/build.ps1 -Release
powershell -ExecutionPolicy Bypass -File scripts/installer.ps1
```

Перед релизом проверяются тесты, Clippy, release-сборка, установка и перезапуск. UI-изменения дополнительно проверяются на реальном окне: открытие/закрытие панелей, выполнение команд меню, кадры открытия/закрытия вкладок и читаемость адресной строки.

Временные скрипты, журналы и снимки помещаются в `target` и удаляются после проверки. Подробные правила — в `KIMI.md`.

## Ограничения

- Поддерживается только Windows x64.
- Нет облачной синхронизации и интеграции с Safe Browsing/антивирусом.
- Бинарники пока не подписаны, SmartScreen может показывать предупреждение.
