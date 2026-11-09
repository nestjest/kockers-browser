use cef::{CefString, base64_encode, uriencode};
use std::sync::OnceLock;

const LOGO_SVG: &str = include_str!("../../assets/logo.svg");

pub fn start_page_url() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| data_url(&start_page_html()))
}

pub fn error_page_url(failed_url: &str, message: &str, code: Option<i32>) -> String {
    let failed_url = escape_html(failed_url);
    let message = escape_html(message);
    let code = code
        .map(|value| format!("<p class=\"code\">Код ошибки Chromium: {value}</p>"))
        .unwrap_or_default();
    let html = format!(
        r#"<!doctype html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Не удалось открыть страницу</title>
<style>
:root {{ color-scheme: light; }}
* {{ box-sizing: border-box; }}
body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; padding: 32px;
  background: #F7F6F3; color: #020202; font-family: "Segoe UI", Arial, sans-serif; }}
main {{ width: min(620px, 100%); }}
.mark {{ width: 48px; height: 48px; margin-bottom: 28px; }}
.mark svg {{ display: block; width: 100%; height: 100%; }}
h1 {{ margin: 0 0 16px; font-size: 24px; font-weight: 500; letter-spacing: -.02em; color: #8B2E2E; }}
p {{ margin: 0 0 12px; font-size: 15px; line-height: 1.6; }}
.url {{ overflow-wrap: anywhere; color: #020202; }}
.code {{ color: #8D8D89; font-size: 13px; }}
</style>
</head>
<body><main>
<div class="mark">{LOGO_SVG}</div>
<h1>Не удалось открыть страницу</h1>
<p>{message}</p>
<p class="url">{failed_url}</p>
{code}
</main></body></html>"#
    );
    data_url(&html)
}

fn start_page_html() -> String {
    format!(
        r#"<!doctype html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>KOCKERS SOCIETY</title>
<style>
:root {{ color-scheme: light; }}
* {{ box-sizing: border-box; }}
body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; padding: 32px;
  background: #F7F6F3; color: #020202; font-family: "Segoe UI", Arial, sans-serif; }}
main {{ width: min(520px, 100%); text-align: center; transform: translateY(-4vh); }}
.logo {{ width: 112px; height: 112px; margin: 0 auto 24px; }}
.logo svg {{ display: block; width: 100%; height: 100%; }}
h1 {{ margin: 0; font-size: 20px; font-weight: 500; letter-spacing: .34em; text-indent: .34em; }}
.subtitle {{ margin: 14px 0 34px; color: #8D8D89; font-size: 13px; letter-spacing: .04em; }}
form {{ margin: 0; }}
input {{ width: 100%; height: 44px; border: 1px solid #E3E1DC; border-radius: 0;
  outline: none; padding: 0 14px; background: #F7F6F3; color: #020202;
  font: 14px "Segoe UI", Arial, sans-serif; text-align: center; }}
input:focus {{ border-color: #020202; }}
input::placeholder {{ color: #8D8D89; }}
</style>
</head>
<body><main>
<div class="logo">{LOGO_SVG}</div>
<h1>KOCKERS SOCIETY</h1>
<p class="subtitle">Минимальный браузер на Chromium</p>
<form id="navigation">
<input id="address" type="text" autofocus autocomplete="off" spellcheck="false"
  aria-label="URL или поисковый запрос" placeholder="URL или поисковый запрос">
</form>
</main>
<script>
const form = document.getElementById('navigation');
const address = document.getElementById('address');
form.addEventListener('submit', (event) => {{
  event.preventDefault();
  const value = address.value.trim();
  if (!value) return;
  let target;
  if (/^https?:\/\//i.test(value)) {{
    target = value;
  }} else if (/^[^\s]+\.[^\s]+$/.test(value) || /^localhost(?::\d+)?(?:[/?#]|$)/i.test(value) || /^\d+(?:\.\d+){{3}}(?::\d+)?(?:[/?#]|$)/.test(value)) {{
    target = 'https://' + value;
  }} else {{
    target = 'https://www.google.com/search?q=' + encodeURIComponent(value);
  }}
  window.location.href = target;
}});
address.focus();
</script>
</body></html>"#
    )
}

fn data_url(html: &str) -> String {
    let encoded = base64_encode(Some(html.as_bytes()));
    let encoded = CefString::from(&encoded);
    let encoded = CefString::from(&uriencode(Some(&encoded), 0)).to_string();
    format!("data:text/html;charset=utf-8;base64,{encoded}")
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
