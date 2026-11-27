use crate::{
    navigation,
    storage::{self, Accent, Language, Preferences, SearchEngine, Theme},
};
use cef::{CefString, base64_encode, uriencode};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

pub(super) const LOGO_SVG: &str = include_str!("../../assets/logo.svg");
pub(super) const LOGO_LIGHT_SVG: &str = include_str!("../../assets/logo-light.svg");
static UPDATED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Eq)]
struct StartPageKey {
    theme: Theme,
    accent: Accent,
    language: Language,
    search_engine: SearchEngine,
    show_tips: bool,
    updated: bool,
}

static START_PAGE_CACHE: Mutex<Option<(StartPageKey, String)>> = Mutex::new(None);

pub fn mark_updated() {
    UPDATED.store(true, Ordering::Relaxed);
}

#[allow(dead_code)]
pub fn start_page_url() -> String {
    start_page_url_for(&storage::load_preferences())
}

pub fn start_page_url_for(preferences: &Preferences) -> String {
    let key = StartPageKey {
        theme: preferences.theme,
        accent: preferences.accent,
        language: preferences.language,
        search_engine: preferences.search_engine,
        show_tips: preferences.show_tips,
        updated: UPDATED.load(Ordering::Relaxed),
    };
    {
        let cache = START_PAGE_CACHE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((cached_key, cached_url)) = cache.as_ref()
            && *cached_key == key
        {
            return cached_url.clone();
        }
    }
    let url = data_url(&start_page_html(preferences, key.updated));
    let mut cache = START_PAGE_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    *cache = Some((key, url.clone()));
    url
}

pub fn is_internal_page_url(url: &str) -> bool {
    url.starts_with("data:text/html")
}

#[allow(dead_code)]
pub fn error_page_url(failed_url: &str, message: &str, code: Option<i32>) -> String {
    error_page_url_for(failed_url, message, code, &storage::load_preferences())
}

pub fn error_page_url_for(
    failed_url: &str,
    message: &str,
    code: Option<i32>,
    preferences: &Preferences,
) -> String {
    data_url(&error_page_html(failed_url, message, code, preferences))
}

fn accent_hex(accent: Accent) -> &'static str {
    match accent {
        Accent::Olive => "#727c45",
        Accent::Blue => "#527a9e",
        Accent::Graphite => "#666b70",
    }
}

fn error_page_html(
    failed_url: &str,
    message: &str,
    code: Option<i32>,
    preferences: &Preferences,
) -> String {
    let english = preferences.language == Language::English;
    let dark = preferences.theme == Theme::Dark;
    let logo_svg = if dark { LOGO_LIGHT_SVG } else { LOGO_SVG };
    let color_scheme = if dark { "dark" } else { "light" };
    let background = if dark { "#171917" } else { "#f7f7f4" };
    let surface = if dark { "#222521" } else { "#fcfcfa" };
    let foreground = if dark { "#eceee8" } else { "#171815" };
    let muted = if dark { "#a2a89c" } else { "#747a6e" };
    let line = if dark { "#3c4239" } else { "#dedfd7" };
    let heading_color = if dark { "#e57c76" } else { "#8b2e2e" };
    let accent = accent_hex(preferences.accent);
    let title = if english {
        "Unable to open page"
    } else {
        "Не удалось открыть страницу"
    };
    let failed_url = escape_html(failed_url);
    let message = escape_html(message);
    let code = code
        .map(|value| {
            if english {
                format!("<p class=\"code\">Chromium error code: {value}</p>")
            } else {
                format!("<p class=\"code\">Код ошибки Chromium: {value}</p>")
            }
        })
        .unwrap_or_default();
    format!(
        r#"<!doctype html>
<html lang="{}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title}</title><style>
:root {{ color-scheme: {color_scheme}; --accent: {accent}; }} * {{ box-sizing: border-box; }}
body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; padding: 32px; background: {background}; color: {foreground}; font-family: "Segoe UI", Arial, sans-serif; }}
main {{ width: min(620px, 100%); padding: 32px; border: 1px solid {line}; border-radius: 16px; background: {surface}; }}
.mark {{ width: 48px; height: 48px; margin-bottom: 24px; }} .mark svg {{ display: block; width: 100%; height: 100%; }}
h1 {{ margin: 0 0 16px; font-size: 24px; font-weight: 500; letter-spacing: -.02em; color: {heading_color}; }}
p {{ margin: 0 0 12px; font-size: 15px; line-height: 1.6; }} p:last-child {{ margin-bottom: 0; }}
.url {{ overflow-wrap: anywhere; color: {foreground}; padding: 8px 12px; border-radius: 8px; border: 1px solid {line}; border-left: 3px solid var(--accent); }}
.code {{ color: {muted}; font-size: 13px; }}
</style></head><body><main><div class="mark">{logo_svg}</div><h1>{title}</h1><p>{message}</p><p class="url">{failed_url}</p>{code}</main>
<script>requestAnimationFrame(() => requestAnimationFrame(() => console.log('__kockers_first_paint__')));</script></body></html>"#,
        preferences.language.as_str()
    )
}

fn start_page_html(preferences: &Preferences, updated: bool) -> String {
    let english = preferences.language == Language::English;
    let update_notice = if updated && preferences.show_tips {
        format!(
            "<div class=\"update\" role=\"status\"><span>{} {}</span><a href=\"https://github.com/nestjest/kockers-browser/releases/tag/v{1}\">{}</a></div>",
            if english {
                "Installed version"
            } else {
                "Установлена версия"
            },
            crate::APP_VERSION,
            if english {
                "What's new"
            } else {
                "Что нового"
            }
        )
    } else {
        String::new()
    };
    let search_prefix = navigation::search_url(preferences.search_engine, "").unwrap_or_default();
    let dark = preferences.theme == Theme::Dark;
    let logo_svg = if dark { LOGO_LIGHT_SVG } else { LOGO_SVG };
    let color_scheme = if dark { "dark" } else { "light" };
    let background = if dark { "#171917" } else { "#f7f7f4" };
    let surface = if dark { "#292d28" } else { "#ffffffdf" };
    let foreground = if dark { "#eceee8" } else { "#171815" };
    let muted = if dark { "#a2a89c" } else { "#8b8d83" };
    let line = if dark { "#3c4239" } else { "#dedfd7" };
    let hover = if dark { "#32372f" } else { "#e7e9e0" };
    let accent = accent_hex(preferences.accent);
    let placeholder = if english {
        "Address or search"
    } else {
        "Адрес или поиск"
    };
    let title = if english {
        "New tab"
    } else {
        "Новая вкладка"
    };
    format!(
        r#"<!doctype html>
<html lang="{}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title}</title><style>
:root {{ color-scheme: {color_scheme}; --accent: {accent}; }} * {{ box-sizing: border-box; }}
body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; padding: 32px; overflow: hidden;
  background: radial-gradient(circle at 50% 35%, {surface} 0, {background} 62%); color: {foreground}; font-family: "Segoe UI", Arial, sans-serif; }}
main {{ position: relative; top: -4vh; width: min(560px, 100%); text-align: center; }}
.logo {{ width: 104px; height: 104px; margin: 0 auto 24px; filter: drop-shadow(0 12px 24px #25281a18); }} .logo svg {{ display: block; width: 100%; height: 100%; }}
h1 {{ margin: 0 0 32px; font-size: 20px; font-weight: 600; letter-spacing: .32em; text-indent: .32em; }}
form {{ margin: 0; }}
input {{ width: 100%; height: 56px; border: 1px solid {line}; border-radius: 9999px; outline: none; padding: 0 24px; background: {surface}; color: {foreground}; box-shadow: 0 8px 30px #25281a0c; font: 15px "Segoe UI", Arial, sans-serif; transition: border-color .12s linear, box-shadow .12s linear; }}
input:focus {{ border-color: var(--accent); box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent), 0 10px 34px #25281a14; }} input::placeholder {{ color: {muted}; }}
.update {{ display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-top: 16px; padding: 12px 16px; border: 1px solid {line}; border-radius: 16px; background: {surface}; color: {muted}; font-size: 13px; text-align: left; }}
.update a {{ flex: none; border-radius: 8px; padding: 8px 12px; color: {foreground}; font-weight: 600; text-decoration: none; }} .update a:hover {{ background: {hover}; }}
@media (prefers-reduced-motion: reduce) {{ input {{ transition: none; }} }}
</style></head><body><main><div class="brand"><div class="logo">{logo_svg}</div><h1>KOCKERS</h1></div>
<form id="navigation"><input id="address" type="text" autofocus autocomplete="off" spellcheck="false" aria-label="{placeholder}" placeholder="{placeholder}"></form>{update_notice}</main>
<script>
const form = document.getElementById('navigation');
const address = document.getElementById('address');
form.addEventListener('submit', (event) => {{
  event.preventDefault();
  const value = address.value.trim();
  if (!value) return;
  let target;
  if (/^https?:\/\//i.test(value)) {{ target = value; }}
  else if (/^[^\s]+\.[^\s]+$/.test(value) || /^localhost(?::\d+)?(?:[/?#]|$)/i.test(value) || /^\d+(?:\.\d+){{3}}(?::\d+)?(?:[/?#]|$)/.test(value)) {{ target = 'https://' + value; }}
  else {{ target = '{search_prefix}' + encodeURIComponent(value); }}
  window.location.href = target;
}});
address.focus();
requestAnimationFrame(() => requestAnimationFrame(() => console.log('__kockers_first_paint__')));
</script></body></html>"#,
        preferences.language.as_str()
    )
}

pub fn data_url(html: &str) -> String {
    let encoded = base64_encode(Some(html.as_bytes()));
    let encoded = CefString::from(&encoded);
    let encoded = CefString::from(&uriencode(Some(&encoded), 0)).to_string();
    format!("data:text/html;charset=utf-8;base64,{encoded}")
}

pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_page_cache_hits_and_invalidates_on_preference_change() {
        let mut prefs = Preferences::default();
        let first = start_page_url_for(&prefs);
        let second = start_page_url_for(&prefs);
        assert_eq!(first, second);
        assert!(is_internal_page_url(&first));

        prefs.theme = Theme::Dark;
        let dark = start_page_url_for(&prefs);
        assert_ne!(first, dark);

        prefs.accent = Accent::Blue;
        let blue = start_page_url_for(&prefs);
        assert_ne!(dark, blue);
    }

    #[test]
    fn start_and_error_pages_emit_first_paint_and_respect_preferences() {
        let mut prefs = Preferences {
            theme: Theme::Dark,
            accent: Accent::Blue,
            language: Language::English,
            ..Preferences::default()
        };
        let start = start_page_html(&prefs, false);
        assert!(start.contains("__kockers_first_paint__"));
        assert!(start.contains("#527a9e"));
        assert!(start.contains("New tab"));
        assert!(start.contains("color-scheme: dark"));

        let err = error_page_html("https://example.com", "ERR_NAME_NOT_RESOLVED", Some(-105), &prefs);
        assert!(err.contains("__kockers_first_paint__"));
        assert!(err.contains("Unable to open page"));
        assert!(err.contains("Chromium error code: -105"));
        assert!(err.contains("color-scheme: dark"));
        assert!(err.contains("#527a9e"));

        prefs.theme = Theme::Light;
        prefs.language = Language::Russian;
        let err_ru = error_page_html("https://example.com", "Ошибка", Some(-2), &prefs);
        assert!(err_ru.contains("Не удалось открыть страницу"));
        assert!(err_ru.contains("Код ошибки Chromium: -2"));
        assert!(err_ru.contains("color-scheme: light"));
    }
}
