use crate::{
    navigation,
    storage::{self, Language, Theme},
};
use cef::{CefString, base64_encode, uriencode};
use std::sync::atomic::{AtomicBool, Ordering};

const LOGO_SVG: &str = include_str!("../../assets/logo.svg");
const LOGO_LIGHT_SVG: &str = include_str!("../../assets/logo-light.svg");
static UPDATED: AtomicBool = AtomicBool::new(false);

pub fn mark_updated() {
    UPDATED.store(true, Ordering::Relaxed);
}

pub fn start_page_url() -> String {
    data_url(&start_page_html())
}

pub fn error_page_url(failed_url: &str, message: &str, code: Option<i32>) -> String {
    let failed_url = escape_html(failed_url);
    let message = escape_html(message);
    let code = code
        .map(|value| format!("<p class=\"code\">Код ошибки Chromium: {value}</p>"))
        .unwrap_or_default();
    let html = format!(
        r#"<!doctype html>
<html lang="ru"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Не удалось открыть страницу</title><style>
:root {{ color-scheme: light; }} * {{ box-sizing: border-box; }}
body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; padding: 32px; background: #F7F6F3; color: #020202; font-family: "Segoe UI", Arial, sans-serif; }}
main {{ width: min(620px, 100%); }} .mark {{ width: 48px; height: 48px; margin-bottom: 28px; }} .mark svg {{ display: block; width: 100%; height: 100%; }}
h1 {{ margin: 0 0 16px; font-size: 24px; font-weight: 500; letter-spacing: -.02em; color: #8B2E2E; }}
p {{ margin: 0 0 12px; font-size: 15px; line-height: 1.6; }} .url {{ overflow-wrap: anywhere; color: #020202; }} .code {{ color: #8D8D89; font-size: 13px; }}
</style></head><body><main><div class="mark">{LOGO_SVG}</div><h1>Не удалось открыть страницу</h1><p>{message}</p><p class="url">{failed_url}</p>{code}</main></body></html>"#
    );
    data_url(&html)
}

fn start_page_html() -> String {
    let preferences = storage::load_preferences();
    let english = preferences.language == Language::English;
    let update_notice = if UPDATED.load(Ordering::Relaxed) && preferences.show_tips {
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
    let animation = if preferences.reduce_animations {
        "none"
    } else {
        "glass-in .56s cubic-bezier(.2,1.28,.32,1) both"
    };
    let placeholder = if english {
        "Address or search"
    } else {
        "Адрес или поиск"
    };
    format!(
        r#"<!doctype html>
<html lang="{}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>KOCKERS</title><style>
:root {{ color-scheme: {color_scheme}; }} * {{ box-sizing: border-box; }}
body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; padding: 32px; overflow: hidden;
  background: radial-gradient(circle at 50% 35%, {surface} 0, {background} 62%); color: {foreground}; font-family: "Segoe UI", Arial, sans-serif; }}
main {{ position: relative; top: -4vh; width: min(560px, 100%); text-align: center; }}
.brand {{ animation: {animation}; }} .logo {{ width: 104px; height: 104px; margin: 0 auto 22px; filter: drop-shadow(0 12px 24px #25281a18); }} .logo svg {{ display: block; width: 100%; height: 100%; }}
h1 {{ margin: 0 0 34px; font-size: 20px; font-weight: 600; letter-spacing: .32em; text-indent: .32em; }}
form {{ margin: 0; animation: {animation}; animation-delay: .07s; }}
input {{ width: 100%; height: 54px; border: 1px solid {line}; border-radius: 27px; outline: none; padding: 0 24px; background: {surface}; color: {foreground}; box-shadow: 0 8px 30px #25281a0c; font: 15px "Segoe UI", Arial, sans-serif; transition: border-color .18s, box-shadow .18s, background .18s; }}
input:focus {{ border-color: #9da36f; box-shadow: 0 10px 34px #25281a14; }} input::placeholder {{ color: {muted}; }}
.update {{ display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-top: 16px; padding: 12px 16px; border: 1px solid {line}; border-radius: 14px; background: {surface}; color: {muted}; font-size: 13px; text-align: left; animation: {animation}; }}
.update a {{ flex: none; border-radius: 8px; padding: 7px 10px; color: {foreground}; font-weight: 600; text-decoration: none; }} .update a:hover {{ background: {hover}; }}
@keyframes glass-in {{ 0% {{ opacity: 0; transform: translateY(24px) scale(.97); }} 68% {{ opacity: 1; transform: translateY(-2px) scale(1.003); }} 100% {{ opacity: 1; transform: none; }} }}
@media (prefers-reduced-motion: reduce) {{ .brand, form, .update {{ animation: none; }} input {{ transition: none; }} }}
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
