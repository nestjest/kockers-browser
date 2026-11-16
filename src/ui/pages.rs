use cef::{CefString, base64_encode, uriencode};
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

const LOGO_SVG: &str = include_str!("../../assets/logo.svg");
static UPDATED: AtomicBool = AtomicBool::new(false);

pub fn mark_updated() {
    UPDATED.store(true, Ordering::Relaxed);
}

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
    let update_notice = if UPDATED.load(Ordering::Relaxed) {
        format!(
            "<div class=\"update\" role=\"status\"><span>Установлена версия {}</span><a href=\"https://github.com/nestjest/kockers-browser/releases/tag/v{0}\">Что нового</a></div>",
            env!("CARGO_PKG_VERSION")
        )
    } else {
        String::new()
    };
    format!(
        r#"<!doctype html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>KOCKERS</title>
<style>
:root {{ color-scheme: light; }}
* {{ box-sizing: border-box; }}
body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; padding: 32px;
  overflow: hidden; background: radial-gradient(circle at 50% 35%, #fff 0, #f7f7f4 58%, #efefe9 100%);
  color: #171815; font-family: "Segoe UI", Arial, sans-serif; }}
main {{ position: relative; top: -4vh; width: min(560px, 100%); text-align: center; }}
.brand {{ will-change: transform, opacity; }}
.logo {{ width: 104px; height: 104px; margin: 0 auto 22px; filter: drop-shadow(0 12px 24px #25281a18); }}
.logo svg {{ display: block; width: 100%; height: 100%; }}
h1 {{ margin: 0 0 34px; font-size: 20px; font-weight: 600; letter-spacing: .32em; text-indent: .32em; }}
form {{ margin: 0; will-change: transform, opacity; }}
input {{ width: 100%; height: 54px; border: 1px solid #dedfd7; border-radius: 27px;
  outline: none; padding: 0 24px; background: #ffffffdf; color: #171815;
  box-shadow: 0 8px 30px #25281a0c; font: 15px "Segoe UI", Arial, sans-serif;
  transition: border-color .18s, box-shadow .18s, background .18s; }}
input:focus {{ border-color: #9da36f; background: #fff; box-shadow: 0 10px 34px #25281a14; }}
input::placeholder {{ color: #8b8d83; }}
.update {{ display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-top: 16px;
  padding: 12px 16px; border: 1px solid #dedfd7; border-radius: 14px; background: #ffffffb8;
  color: #606456; font-size: 13px; text-align: left; animation: notice-in .24s ease-out both; }}
.update a {{ flex: none; border-radius: 8px; padding: 7px 10px; color: #56602f; font-weight: 600;
  text-decoration: none; transition: background .16s, color .16s; }}
.update a:hover {{ background: #e7e9e0; color: #20221d; }}
.update a:focus-visible {{ outline: 2px solid #727c45; outline-offset: 2px; }}
@keyframes notice-in {{ from {{ opacity: 0; transform: translateY(8px); }} to {{ opacity: 1; transform: translateY(0); }} }}
@media (prefers-reduced-motion: reduce) {{ .update {{ animation: none; }} input, .update a {{ transition: none; }} }}
</style>
</head>
<body><main>
<div class="brand"><div class="logo">{LOGO_SVG}</div><h1>KOCKERS</h1></div>
<form id="navigation">
<input id="address" type="text" autofocus autocomplete="off" spellcheck="false"
  aria-label="Адрес или поиск" placeholder="Адрес или поиск">
</form>
{update_notice}
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
if (!matchMedia('(prefers-reduced-motion: reduce)').matches) {{
  const items = [document.querySelector('.brand'), form];
  items.forEach((item, index) => {{
    let position = 28 + index * 10;
    let velocity = 0;
    let previous;
    item.style.opacity = '0';
    function frame(now) {{
      if (previous === undefined) {{
        previous = now + index * 65;
        return requestAnimationFrame(frame);
      }}
      const dt = Math.min(Math.max((now - previous) / 1000, 0), .05);
      previous = now;
      if (dt <= 0) return requestAnimationFrame(frame);
      velocity += (-190 * position - 15 * velocity) * dt;
      position += velocity * dt;
      item.style.transform = `translateY(${{position}}px)`;
      item.style.opacity = String(Math.max(0, Math.min(1, 1 - position / 40)));
      if (Math.abs(position) > .02 || Math.abs(velocity) > .02) requestAnimationFrame(frame);
      else {{ item.style.transform = 'none'; item.style.opacity = '1'; item.style.willChange = 'auto'; }}
    }}
    requestAnimationFrame(frame);
  }});
}}
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
