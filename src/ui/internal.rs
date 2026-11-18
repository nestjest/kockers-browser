use super::pages::{data_url, escape_html};
use crate::storage::{self, Accent, Language, Preferences, Theme};
use cef::{CefString, base64_encode};
use std::{fs, path::Path};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Sync,
    Performance,
    Appearance,
    Search,
    DefaultBrowser,
    Startup,
    Languages,
    Downloads,
    System,
    Reset,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InternalPage {
    Profile,
    Settings(Section),
    History,
}

impl InternalPage {
    pub fn url(self) -> String {
        let preferences = storage::load_preferences();
        let html = match self {
            Self::Profile => profile_html(&preferences),
            Self::Settings(section) => settings_html(section, &preferences),
            Self::History => history_html(&preferences),
        };
        data_url(&html)
    }
}

pub fn is_action_url(url: &str) -> bool {
    url.starts_with("kockers-action:")
}

fn tr(language: Language, russian: &'static str, english: &'static str) -> &'static str {
    match language {
        Language::Russian => russian,
        Language::English => english,
    }
}

fn document(_language: Language, title: &str, body: &str, preferences: &Preferences) -> String {
    let dark = preferences.theme == Theme::Dark;
    let accent = match preferences.accent {
        Accent::Olive => "#727c45",
        Accent::Blue => "#527a9e",
        Accent::Graphite => "#666b70",
    };
    let motion = if preferences.reduce_animations {
        "none"
    } else {
        "page-in .12s linear both"
    };
    format!(
        r#"<!doctype html>
<html lang="{}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{}</title><style>
:root{{color-scheme:{};--bg:{};--surface:{};--raised:{};--text:{};--muted:{};--line:{};--hover:{};--accent:{accent}}}
*{{box-sizing:border-box}}body{{margin:0;min-height:100vh;background:radial-gradient(circle at 75% 0,var(--raised),var(--bg) 52%);color:var(--text);font:14px "Segoe UI",Arial,sans-serif}}
a{{color:inherit}}button,input,textarea,select{{font:inherit}}button,select,input,textarea{{color:var(--text)}}button{{cursor:pointer}}
.shell{{display:grid;grid-template-columns:230px minmax(0,850px);gap:32px;max-width:1160px;margin:auto;padding:34px 28px 64px}}
.sidebar{{position:sticky;top:28px;align-self:start}}.brand{{display:flex;align-items:center;gap:10px;margin:0 8px 26px;font-size:17px;font-weight:600;letter-spacing:.12em}}
.brand svg{{width:26px;height:26px}}.nav{{display:grid;gap:4px}}.nav a{{display:flex;align-items:center;gap:11px;padding:10px 12px;border-radius:11px;text-decoration:none;color:var(--muted);transition:background .16s,color .16s}}
.nav a:hover{{background:var(--hover);color:var(--text)}}.nav a.active{{background:color-mix(in srgb,var(--accent) 22%,var(--hover));color:var(--text)}}.nav svg,.icon{{width:18px;height:18px;fill:currentColor;stroke:none;flex:none}}
main{{min-width:0;animation:{motion}}}.heading{{margin:4px 0 24px}}h1{{font-size:27px;font-weight:600;letter-spacing:-.025em;margin:0 0 7px}}h2{{font-size:16px;font-weight:600;margin:0}}p{{line-height:1.55}}.subtitle{{margin:0;color:var(--muted)}}
.card{{background:color-mix(in srgb,var(--surface) 92%,transparent);border:1px solid var(--line);border-radius:18px;box-shadow:0 14px 45px #00000009;overflow:hidden;margin-bottom:18px}}
.row{{display:flex;align-items:center;justify-content:space-between;gap:24px;min-height:62px;padding:14px 20px;border-bottom:1px solid var(--line)}}.row:last-child{{border:0}}.row.stacked{{align-items:stretch;flex-direction:column;gap:10px}}.label{{font-weight:500}}.hint{{display:block;color:var(--muted);font-size:12px;margin-top:4px;line-height:1.45}}
select,input[type=text],input[type=url],textarea{{border:1px solid var(--line);background:var(--raised);border-radius:10px;padding:9px 11px;outline:none;min-width:180px}}textarea{{width:100%;resize:vertical;min-height:105px;line-height:1.5}}input:focus,textarea:focus,select:focus{{border-color:var(--accent);box-shadow:0 0 0 3px color-mix(in srgb,var(--accent) 15%,transparent)}}
.switch{{appearance:none;width:42px;height:24px;min-width:42px;border:0;border-radius:20px;background:var(--line);position:relative;cursor:pointer;transition:background .12s linear;flex:none}}.switch:before{{content:"";position:absolute;width:18px;height:18px;left:3px;top:3px;border-radius:50%;background:white;box-shadow:0 1px 4px #0003;transition:transform .12s linear}}.switch:checked{{background:var(--accent)}}.switch:checked:before{{transform:translateX(18px)}}
.btn{{border:1px solid var(--line);background:var(--raised);border-radius:10px;padding:9px 14px;text-decoration:none;display:inline-flex;align-items:center;justify-content:center;gap:8px;transition:background .12s linear}}.btn:hover{{background:var(--hover)}}.btn.primary{{background:var(--accent);border-color:var(--accent);color:white}}.btn.danger{{color:#b04d47}}.actions{{display:flex;gap:10px;flex-wrap:wrap}}.note{{padding:14px 18px;color:var(--muted);font-size:12px}}
.profile-banner{{height:210px;background:linear-gradient(125deg,var(--accent),var(--raised));background-size:cover;background-position:center;border-radius:18px 18px 0 0}}.avatar{{width:112px;height:112px;border-radius:50%;margin:-58px 0 14px 24px;border:5px solid var(--surface);background:var(--hover) center/cover;display:grid;place-items:center;font-size:36px;font-weight:600}}.profile-body{{padding:0 24px 24px}}.profile-name{{font-size:23px;font-weight:600;margin:0 0 5px}}.profile-description{{white-space:pre-wrap;color:var(--muted);margin:0 0 20px}}.field{{display:grid;gap:7px;margin-bottom:14px}}.field span{{color:var(--muted);font-size:12px}}
.history-tools{{display:flex;gap:10px;margin-bottom:18px}}.history-tools input{{flex:1}}.history-entry{{display:grid;grid-template-columns:105px minmax(0,1fr) 36px;align-items:center;gap:12px;padding:13px 18px;border-bottom:1px solid var(--line)}}.history-entry:last-child{{border:0}}.history-time{{color:var(--muted);font-size:12px}}.history-link{{text-decoration:none;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}.history-url{{display:block;color:var(--muted);font-size:11px;margin-top:3px;overflow:hidden;text-overflow:ellipsis}}.icon-button{{border:0;background:transparent;color:var(--muted);border-radius:8px;padding:8px}}.icon-button:hover{{background:var(--hover);color:var(--text)}}.empty{{text-align:center;color:var(--muted);padding:56px 20px}}
@keyframes page-in{{from{{opacity:0}}to{{opacity:1}}}}
@media(max-width:850px){{.shell{{grid-template-columns:1fr;padding:20px}}.sidebar{{position:static}}.nav{{grid-template-columns:repeat(2,minmax(0,1fr))}}.brand{{margin-bottom:14px}}}}
@media(prefers-reduced-motion:reduce){{main{{animation:none}}*{{transition:none!important}}}}
</style></head><body>{}<script>
function action(path,params={{}}){{const query=new URLSearchParams(params).toString();location.href='kockers-action:'+path+(query?'?'+query:'');}}
function setting(key,value){{action('settings/set',{{key,value}});}}
function saveProfile(event){{event.preventDefault();action('profile/save',{{nickname:document.getElementById('nickname').value,description:document.getElementById('description').value}});}}
const historySearch=document.getElementById('history-search');if(historySearch){{historySearch.addEventListener('input',()=>{{const query=historySearch.value.toLowerCase();document.querySelectorAll('.history-entry').forEach(entry=>entry.hidden=!entry.dataset.search.includes(query));}});}}
document.querySelectorAll('[data-time]').forEach(element=>{{const date=new Date(Number(element.dataset.time)*1000);element.textContent=date.toLocaleString({{}},{{day:'numeric',month:'short',hour:'2-digit',minute:'2-digit'}});element.title=date.toLocaleString();}});
</script></body></html>"#,
        preferences.language.as_str(),
        escape_html(title),
        if dark { "dark" } else { "light" },
        if dark { "#171917" } else { "#f3f4ef" },
        if dark { "#222521" } else { "#fcfcfa" },
        if dark { "#292d28" } else { "#ffffff" },
        if dark { "#eceee8" } else { "#20221d" },
        if dark { "#a2a89c" } else { "#747a6e" },
        if dark { "#3c4239" } else { "#dfe2d9" },
        if dark { "#32372f" } else { "#e8ebe2" },
        body
    )
}

fn sidebar(language: Language, active: &str) -> String {
    let entries = [
        (
            "profile",
            "profile",
            "Профиль",
            "Profile",
            "M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z",
        ),
        (
            "settings/sync",
            "sync",
            "Синхронизация",
            "Sync",
            "M12 4V1L8 5l4 4V6c3.31 0 6 2.69 6 6 0 1.01-.25 1.97-.7 2.8l1.46 1.46C19.54 15.03 20 13.57 20 12c0-4.42-3.58-8-8-8zm0 14c-3.31 0-6-2.69-6-6 0-1.01.25-1.97.7-2.8L5.24 7.74C4.46 8.97 4 10.43 4 12c0 4.42 3.58 8 8 8v3l4-4-4-4v3z",
        ),
        (
            "settings/performance",
            "performance",
            "Производительность",
            "Performance",
            "M7 2v11h3v9l7-12h-4l4-8z",
        ),
        (
            "settings/appearance",
            "appearance",
            "Внешний вид",
            "Appearance",
            "M12 3a9 9 0 0 0 0 18c1.1 0 2-.9 2-2 0-.5-.2-1-.5-1.3-.3-.3-.5-.8-.5-1.3 0-1.1.9-2 2-2h2.4c3.1 0 5.6-2.5 5.6-5.6C23 5.2 18 3 12 3zM6.5 12c-.83 0-1.5-.67-1.5-1.5S5.67 9 6.5 9 8 9.67 8 10.5 7.33 12 6.5 12zm3-4C8.67 8 8 7.33 8 6.5S8.67 5 9.5 5 11 5.67 11 6.5 10.33 8 9.5 8zm5 0c-.83 0-1.5-.67-1.5-1.5S13.67 5 14.5 5 16 5.67 16 6.5 15.33 8 14.5 8zm3 4c-.83 0-1.5-.67-1.5-1.5S16.67 9 17.5 9 19 9.67 19 10.5 18.33 12 17.5 12z",
        ),
        (
            "settings/search",
            "search",
            "Поисковая система",
            "Search engine",
            "M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z",
        ),
        (
            "settings/default",
            "default",
            "Браузер по умолчанию",
            "Default browser",
            "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 17.93c-3.95-.49-7-3.85-7-7.93 0-.62.08-1.21.21-1.79L9 15v1c0 1.1.9 2 2 2v1.93zm6.9-2.54c-.26-.81-1-1.39-1.9-1.39h-1v-3c0-.55-.45-1-1-1H8v-2h2c.55 0 1-.45 1-1V7h2c1.1 0 2-.9 2-2v-.41c2.93 1.19 5 4.06 5 7.41 0 2.08-.8 3.97-2.1 5.39z",
        ),
        (
            "settings/startup",
            "startup",
            "Запуск браузера",
            "Startup",
            "M19 19H5V5h7V3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z",
        ),
        (
            "settings/languages",
            "languages",
            "Языки",
            "Languages",
            "M12.87 15.07l-2.54-2.51.03-.03c1.74-1.94 2.98-4.17 3.71-6.53H17V4h-7V2H8v2H1v1.99h11.17C11.5 7.92 10.44 9.75 9 11.35 8.07 10.32 7.3 9.19 6.69 8h-2c.73 1.63 1.73 3.17 2.98 4.56l-5.09 5.02L4 19l5-5 3.11 3.11.76-2.04zM18.5 10h-2L12 22h2l1.12-3h4.75L21 22h2l-4.5-12zm-2.62 7l1.62-4.33L19.12 17h-3.24z",
        ),
        (
            "settings/downloads",
            "downloads",
            "Скачанные файлы",
            "Downloads",
            "M5 20h14v-2H5v2zM19 9h-4V3H9v6H5l7 7 7-7z",
        ),
        (
            "settings/system",
            "system",
            "Система",
            "System",
            "M20 18c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2H4c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2H0v2h24v-2h-4zM4 6h16v10H4V6z",
        ),
        (
            "settings/reset",
            "reset",
            "Сбросить настройки",
            "Reset settings",
            "M12 5V2L8 6l4 4V7c3.31 0 6 2.69 6 6 0 2.97-2.17 5.43-5 5.91v2.02c3.95-.49 7-3.85 7-7.93 0-4.42-3.58-8-8-8zm-6 8c0-1.66.67-3.16 1.76-4.24L6.34 7.34C4.9 8.79 4 10.79 4 13c0 4.08 3.05 7.44 7 7.93v-2.02c-2.83-.48-5-2.94-5-5.91z",
        ),
        (
            "history",
            "history",
            "История браузера",
            "History",
            "M13 3c-4.97 0-9 4.03-9 9H1l3.89 3.89.07.14L9 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42C8.27 19.99 10.51 21 13 21c4.97 0 9-4.03 9-9s-4.03-9-9-9zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z",
        ),
    ];
    let mut nav = String::new();
    for (path, id, russian, english, icon) in entries {
        nav.push_str(&format!(r#"<a class="{}" href="kockers-action:open/{}"><svg class="icon" viewBox="0 0 24 24"><path d="{}"/></svg>{}</a>"#,
            if active == id { "active" } else { "" }, path, icon, tr(language, russian, english)));
    }
    format!(
        r#"<aside class="sidebar"><div class="brand">KOCKERS</div><nav class="nav">{}</nav></aside>"#,
        nav
    )
}

fn settings_html(section: Section, preferences: &Preferences) -> String {
    let language = preferences.language;
    let (id, title, subtitle) = match section {
        Section::Sync => (
            "sync",
            tr(language, "Синхронизация аккаунта", "Account sync"),
            tr(
                language,
                "Переносите настройки и данные профиля между компьютерами.",
                "Move settings and profile details between computers.",
            ),
        ),
        Section::Performance => (
            "performance",
            tr(language, "Производительность", "Performance"),
            tr(
                language,
                "Снижайте нагрузку и настройте отзывчивость браузера.",
                "Balance resource usage and responsiveness.",
            ),
        ),
        Section::Appearance => (
            "appearance",
            tr(language, "Внешний вид", "Appearance"),
            tr(
                language,
                "Тема, цвет и элементы панели.",
                "Theme, color and toolbar elements.",
            ),
        ),
        Section::Search => (
            "search",
            tr(language, "Поисковая система", "Search engine"),
            tr(
                language,
                "Используется для запросов из адресной строки.",
                "Used for address bar searches.",
            ),
        ),
        Section::DefaultBrowser => (
            "default",
            tr(language, "Браузер по умолчанию", "Default browser"),
            tr(
                language,
                "Открывайте ссылки в Kockers Browser.",
                "Open links in Kockers Browser.",
            ),
        ),
        Section::Startup => (
            "startup",
            tr(
                language,
                "Запуск Kockers Browser",
                "Kockers Browser startup",
            ),
            tr(
                language,
                "Выберите вкладки, которые откроются при запуске.",
                "Choose which tabs open at startup.",
            ),
        ),
        Section::Languages => (
            "languages",
            tr(language, "Языки", "Languages"),
            tr(
                language,
                "Язык внутренних страниц и основного меню.",
                "Language of internal pages and the main menu.",
            ),
        ),
        Section::Downloads => (
            "downloads",
            tr(language, "Скачанные файлы", "Downloaded files"),
            tr(
                language,
                "Папка и поведение загрузок.",
                "Download location and behavior.",
            ),
        ),
        Section::System => (
            "system",
            tr(language, "Система", "System"),
            tr(
                language,
                "Интеграция с Windows и работа в фоне.",
                "Windows integration and background behavior.",
            ),
        ),
        Section::Reset => (
            "reset",
            tr(language, "Сбросить настройки", "Reset settings"),
            tr(
                language,
                "Верните исходные параметры браузера.",
                "Restore the original browser settings.",
            ),
        ),
    };
    let content = match section {
        Section::Sync => format!(
            "{}{}",
            row(
                tr(language, "Экспорт конфигурации", "Export configuration"),
                tr(
                    language,
                    "Сохраните JSON-файл в надёжном месте.",
                    "Save a JSON file in a safe place."
                ),
                button(
                    "config/export",
                    tr(language, "Экспорт JSON", "Export JSON"),
                    false
                )
            ),
            row(
                tr(language, "Импорт конфигурации", "Import configuration"),
                tr(
                    language,
                    "Текущие настройки будут заменены после выбора файла.",
                    "Current settings will be replaced after file selection."
                ),
                button(
                    "config/import",
                    tr(language, "Импорт JSON", "Import JSON"),
                    false
                )
            )
        ),
        Section::Performance => format!(
            "{}{}{}{}",
            toggle(
                "reduce_animations",
                tr(language, "Уменьшить анимации", "Reduce animations"),
                tr(
                    language,
                    "Отключает плавные переходы панелей и страниц.",
                    "Disables smooth transitions on panels and pages."
                ),
                preferences.reduce_animations
            ),
            toggle(
                "smooth_scrolling",
                tr(language, "Плавная прокрутка", "Smooth scrolling"),
                tr(
                    language,
                    "Изменение применится после перезапуска.",
                    "A restart is required."
                ),
                preferences.smooth_scrolling
            ),
            toggle(
                "background_throttling",
                tr(
                    language,
                    "Экономить ресурсы фоновых вкладок",
                    "Throttle background tabs"
                ),
                tr(
                    language,
                    "Ограничивает таймеры неактивных страниц.",
                    "Limits timers on inactive pages."
                ),
                preferences.background_throttling
            ),
            toggle(
                "lazy_restore",
                tr(
                    language,
                    "Откладывать фоновые вкладки",
                    "Defer background tabs"
                ),
                tr(
                    language,
                    "Не загружать восстановленные и новые фоновые вкладки до активации.",
                    "Load restored and new background tabs only when activated."
                ),
                preferences.lazy_restore
            )
        ),
        Section::Appearance => format!(
            "{}{}{}",
            select(
                "theme",
                tr(language, "Тема", "Theme"),
                &[
                    ("light", tr(language, "Светлая", "Light")),
                    ("dark", tr(language, "Тёмная", "Dark"))
                ],
                preferences.theme.as_str()
            ),
            select(
                "accent",
                tr(language, "Акцентный цвет", "Accent color"),
                &[
                    ("olive", tr(language, "Оливковый", "Olive")),
                    ("blue", tr(language, "Синий", "Blue")),
                    ("graphite", tr(language, "Графитовый", "Graphite"))
                ],
                preferences.accent.as_str()
            ),
            toggle(
                "bookmarks",
                tr(language, "Показывать панель закладок", "Show bookmarks bar"),
                "",
                preferences.show_bookmarks
            )
        ),
        Section::Search => select(
            "search",
            tr(language, "Поисковая система", "Search engine"),
            &[
                ("google", "Google"),
                ("bing", "Bing"),
                ("duckduckgo", "DuckDuckGo"),
                ("yandex", tr(language, "Яндекс", "Yandex")),
            ],
            preferences.search_engine.as_str(),
        ),
        Section::DefaultBrowser => row(
            tr(language, "Настройка в Windows", "Windows setting"),
            tr(
                language,
                "Windows попросит подтвердить выбор в системных параметрах.",
                "Windows asks you to confirm the choice in Settings.",
            ),
            button(
                "system/default-browser",
                tr(language, "Открыть параметры", "Open Settings"),
                false,
            ),
        ),
        Section::Startup => format!(
            "{}{}",
            select(
                "startup",
                tr(language, "При запуске", "On startup"),
                &[
                    (
                        "new_tab",
                        tr(language, "Открывать новую вкладку", "Open the new tab page")
                    ),
                    (
                        "restore",
                        tr(
                            language,
                            "Продолжить с места выхода",
                            "Continue where you left off"
                        )
                    ),
                    (
                        "custom",
                        tr(
                            language,
                            "Открывать заданную страницу",
                            "Open a specific page"
                        )
                    )
                ],
                preferences.startup_mode.as_str()
            ),
            text_setting(
                "startup_url",
                tr(language, "Адрес стартовой страницы", "Startup page URL"),
                tr(
                    language,
                    "Например: https://example.com",
                    "For example: https://example.com"
                ),
                &preferences.startup_url
            )
        ),
        Section::Languages => select(
            "language",
            tr(language, "Язык интерфейса", "Display language"),
            &[("ru", "Русский"), ("en", "English")],
            preferences.language.as_str(),
        ),
        Section::Downloads => format!(
            "{}{}",
            row(
                tr(language, "Папка загрузок", "Download folder"),
                &preferences.download_dir,
                button(
                    "downloads/choose",
                    tr(language, "Выбрать папку", "Choose folder"),
                    false
                )
            ),
            toggle(
                "ask_download",
                tr(
                    language,
                    "Спрашивать путь перед загрузкой",
                    "Ask where to save each file"
                ),
                "",
                preferences.ask_download_path
            )
        ),
        Section::System => format!(
            "{}{}{}{}",
            toggle(
                "continue_background",
                tr(
                    language,
                    "Не отключать фоновые сервисы при закрытии браузера",
                    "Keep background services running after closing"
                ),
                tr(
                    language,
                    "Повторный запуск откроет браузер быстрее.",
                    "The next launch will reopen the browser faster."
                ),
                preferences.continue_background
            ),
            toggle(
                "hardware_acceleration",
                tr(
                    language,
                    "Использовать аппаратное ускорение графики",
                    "Use hardware graphics acceleration"
                ),
                tr(
                    language,
                    "Если доступно. Требуется перезапуск.",
                    "When available. A restart is required."
                ),
                preferences.hardware_acceleration
            ),
            toggle(
                "show_tips",
                tr(
                    language,
                    "Показывать системные уведомления и советы",
                    "Show system notifications and tips"
                ),
                tr(
                    language,
                    "Новости об обновлениях и подсказки на стартовой странице.",
                    "Update news and hints on the start page."
                ),
                preferences.show_tips
            ),
            row(
                tr(language, "Прокси-сервер", "Proxy server"),
                tr(
                    language,
                    "Открыть сетевые настройки компьютера.",
                    "Open computer network settings."
                ),
                button(
                    "system/proxy",
                    tr(language, "Открыть настройки", "Open settings"),
                    false
                )
            )
        ),
        Section::Reset => row(
            tr(language, "Сбросить все настройки", "Reset all settings"),
            tr(
                language,
                "История, закладки, вкладки и фотографии профиля сохранятся.",
                "History, bookmarks, tabs and profile photos are preserved.",
            ),
            r#"<button class="btn danger" onclick="if(confirm('Сбросить настройки браузера?'))action('settings/reset')">Сбросить настройки</button>"#,
        ),
    };
    let body = format!(
        r#"<div class="shell">{}<main><header class="heading"><h1>{}</h1><p class="subtitle">{}</p></header><section class="card">{}</section></main></div>"#,
        sidebar(language, id),
        title,
        subtitle,
        content
    );
    document(language, title, &body, preferences)
}

fn profile_html(preferences: &Preferences) -> String {
    let language = preferences.language;
    let profile = storage::load_profile();
    let nickname = if profile.nickname.is_empty() {
        tr(language, "Ваш профиль", "Your profile")
    } else {
        &profile.nickname
    };
    let description = if profile.description.is_empty() {
        tr(
            language,
            "Добавьте пару слов о себе.",
            "Add a few words about yourself.",
        )
    } else {
        &profile.description
    };
    let banner = image_data(&profile.banner_path)
        .map(|value| format!("background-image:url('{value}')"))
        .unwrap_or_default();
    let avatar = image_data(&profile.avatar_path)
        .map(|value| format!("background-image:url('{value}')"))
        .unwrap_or_default();
    let initial = if profile.avatar_path.is_empty() {
        escape_html(&profile.nickname.chars().next().unwrap_or('K').to_string())
    } else {
        String::new()
    };
    let body = format!(
        r#"<div class="shell">{}<main><header class="heading"><h1>{}</h1><p class="subtitle">{}</p></header>
<section class="card"><div class="profile-banner" style="{}"></div><div class="avatar" style="{}">{}</div><div class="profile-body"><p class="profile-name">{}</p><p class="profile-description">{}</p><div class="actions">{} {}</div></div></section>
<form class="card" onsubmit="saveProfile(event)"><div class="row stacked"><h2>{}</h2></div><div class="row stacked"><label class="field"><span>{}</span><input id="nickname" type="text" maxlength="60" value="{}" placeholder="{}"></label><label class="field"><span>{}</span><textarea id="description" maxlength="500" placeholder="{}">{}</textarea></label><div class="actions"><button class="btn primary" type="submit">{}</button></div></div></form></main></div>"#,
        sidebar(language, "profile"),
        tr(language, "Профиль", "Profile"),
        tr(
            language,
            "Ваши фотография, шапка, имя и описание.",
            "Your photo, banner, name and description."
        ),
        banner,
        avatar,
        initial,
        escape_html(nickname),
        escape_html(description),
        button(
            "profile/avatar",
            tr(language, "Изменить фото", "Change photo"),
            false
        ),
        button(
            "profile/banner",
            tr(language, "Изменить шапку", "Change banner"),
            false
        ),
        tr(language, "Личные данные", "Profile details"),
        tr(language, "Nickname", "Nickname"),
        escape_html(&profile.nickname),
        tr(
            language,
            "Как к вам обращаться?",
            "What should we call you?"
        ),
        tr(language, "Описание", "Description"),
        tr(language, "Расскажите о себе", "Tell us about yourself"),
        escape_html(&profile.description),
        tr(language, "Сохранить", "Save")
    );
    document(
        language,
        tr(language, "Профиль", "Profile"),
        &body,
        preferences,
    )
}

fn history_html(preferences: &Preferences) -> String {
    let language = preferences.language;
    let mut entries = storage::load_history();
    entries.reverse();
    let mut html = String::new();
    for entry in entries {
        let title = if entry.title.is_empty() {
            &entry.url
        } else {
            &entry.title
        };
        let search = escape_html(&format!(
            "{} {}",
            title.to_lowercase(),
            entry.url.to_lowercase()
        ));
        html.push_str(&format!(r#"<div class="history-entry" data-search="{}"><time class="history-time" data-time="{}"></time><div><a class="history-link" href="{}">{}</a><span class="history-url">{}</span></div><button class="icon-button" title="{}" onclick="action('history/remove',{{url:'{}'}})">×</button></div>"#,search,entry.visited_at,escape_html(&entry.url),escape_html(title),escape_html(&entry.url),tr(language,"Удалить","Remove"),js_string(&entry.url)));
    }
    if html.is_empty() {
        html = format!(
            r#"<div class="empty">{}</div>"#,
            tr(language, "История пока пуста", "No history yet")
        );
    }
    let body = format!(
        r#"<div class="shell">{}<main><header class="heading"><h1>{}</h1><p class="subtitle">{}</p></header><div class="history-tools"><input id="history-search" type="text" placeholder="{}" autofocus><button class="btn danger" onclick="if(confirm('{}'))action('history/clear')">{}</button></div><section class="card">{}</section></main></div>"#,
        sidebar(language, "history"),
        tr(language, "История браузера", "Browser history"),
        tr(
            language,
            "Поиск по посещённым страницам.",
            "Search visited pages."
        ),
        tr(language, "Поиск в истории", "Search history"),
        tr(language, "Очистить всю историю?", "Clear all history?"),
        tr(language, "Очистить историю", "Clear history"),
        html
    );
    document(
        language,
        tr(language, "История", "History"),
        &body,
        preferences,
    )
}

fn row(label: &str, hint: &str, control: impl AsRef<str>) -> String {
    format!(
        r#"<div class="row"><div><span class="label">{}</span><span class="hint">{}</span></div><div>{}</div></div>"#,
        escape_html(label),
        escape_html(hint),
        control.as_ref()
    )
}

fn toggle(key: &str, label: &str, hint: &str, checked: bool) -> String {
    row(
        label,
        hint,
        format!(
            r#"<input class="switch" type="checkbox" role="switch" {} onchange="setting('{}',this.checked)">"#,
            if checked { "checked" } else { "" },
            key
        ),
    )
}

fn select(key: &str, label: &str, options: &[(&str, &str)], selected: &str) -> String {
    let options = options
        .iter()
        .map(|(value, label)| {
            format!(
                r#"<option value="{}" {}>{}</option>"#,
                value,
                if *value == selected { "selected" } else { "" },
                escape_html(label)
            )
        })
        .collect::<String>();
    row(
        label,
        "",
        format!(
            r#"<select onchange="setting('{}',this.value)">{}</select>"#,
            key, options
        ),
    )
}

fn text_setting(key: &str, label: &str, hint: &str, value: &str) -> String {
    row(
        label,
        hint,
        format!(
            r#"<input type="url" value="{}" placeholder="https://" onchange="setting('{}',this.value)">"#,
            escape_html(value),
            key
        ),
    )
}

fn button(path: &str, label: &str, primary: bool) -> String {
    format!(
        r#"<a class="btn{}" href="kockers-action:{}">{}</a>"#,
        if primary { " primary" } else { "" },
        path,
        escape_html(label)
    )
}

fn image_data(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let bytes = fs::read(Path::new(path)).ok()?;
    let mime = match Path::new(path)
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return None,
    };
    let encoded = CefString::from(&base64_encode(Some(&bytes))).to_string();
    Some(format!("data:{mime};base64,{encoded}"))
}

fn js_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace(['\r', '\n'], "")
}
