use cef::{CefString, CommandLine, ImplCommandLine};
use std::{
    collections::{BTreeMap, HashSet},
    fs, mem,
    path::{Component, Path, PathBuf},
    process::Command,
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use windows_sys::Win32::{Security::Cryptography::*, System::Threading::CREATE_NO_WINDOW};

const MAX_CRX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_JSON_DEPTH: usize = 128;
static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

struct StagingDirGuard(PathBuf);

impl Drop for StagingDirGuard {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

type JsonObject = BTreeMap<String, JsonValue>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonValue {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<JsonValue>),
    Object(JsonObject),
}

impl JsonValue {
    fn into_object(self) -> Option<JsonObject> {
        match self {
            Self::Object(map) => Some(map),
            _ => None,
        }
    }

    fn as_object(&self) -> Option<&JsonObject> {
        match self {
            Self::Object(map) => Some(map),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.as_str()),
            _ => None,
        }
    }

    fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(raw) => raw.parse::<i64>().ok(),
            _ => None,
        }
    }
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> JsonParser<'a> {
    fn new(text: &'a str) -> Self {
        let text = text.trim_start_matches('\u{FEFF}');
        Self {
            bytes: text.as_bytes(),
            position: 0,
        }
    }

    fn skip_whitespace(&mut self) {
        while self
            .bytes
            .get(self.position)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.position += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_whitespace();
        self.bytes.get(self.position).copied()
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), String> {
        if self.consume(expected) {
            Ok(())
        } else {
            Err(format!("Ожидался символ '{}' в JSON.", expected as char))
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<JsonValue, String> {
        if depth > MAX_JSON_DEPTH {
            return Err("Превышена глубина вложенности JSON.".to_owned());
        }
        match self.peek() {
            Some(b'{') => self.parse_object(depth + 1).map(JsonValue::Object),
            Some(b'[') => self.parse_array(depth + 1).map(JsonValue::Array),
            Some(b'"') => self.parse_string().map(JsonValue::String),
            Some(b't') => {
                self.expect_literal(b"true")?;
                Ok(JsonValue::Bool(true))
            }
            Some(b'f') => {
                self.expect_literal(b"false")?;
                Ok(JsonValue::Bool(false))
            }
            Some(b'n') => {
                self.expect_literal(b"null")?;
                Ok(JsonValue::Null)
            }
            Some(b'-' | b'0'..=b'9') => self.parse_number().map(JsonValue::Number),
            _ => Err("Неожиданный токен в JSON.".to_owned()),
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<JsonObject, String> {
        self.expect(b'{')?;
        let mut map = BTreeMap::new();
        if self.consume(b'}') {
            return Ok(map);
        }
        loop {
            let key = self.parse_string()?;
            self.expect(b':')?;
            let value = self.parse_value(depth)?;
            map.insert(key, value);
            if self.consume(b'}') {
                return Ok(map);
            }
            self.expect(b',')?;
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<Vec<JsonValue>, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        if self.consume(b']') {
            return Ok(items);
        }
        loop {
            items.push(self.parse_value(depth)?);
            if self.consume(b']') {
                return Ok(items);
            }
            self.expect(b',')?;
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut result = String::new();
        while let Some(byte) = self.bytes.get(self.position).copied() {
            self.position += 1;
            match byte {
                b'"' => return Ok(result),
                b'\\' => {
                    let escaped = self
                        .bytes
                        .get(self.position)
                        .copied()
                        .ok_or_else(|| "Незавершённая escape-последовательность JSON.".to_owned())?;
                    self.position += 1;
                    match escaped {
                        b'"' => result.push('"'),
                        b'\\' => result.push('\\'),
                        b'/' => result.push('/'),
                        b'b' => result.push('\u{0008}'),
                        b'f' => result.push('\u{000C}'),
                        b'n' => result.push('\n'),
                        b'r' => result.push('\r'),
                        b't' => result.push('\t'),
                        b'u' => {
                            let code = self.parse_hex4()?;
                            if (0xD800..=0xDBFF).contains(&code) {
                                if self.bytes.get(self.position..self.position + 2) != Some(b"\\u")
                                {
                                    return Err("Некорректная суррогатная пара в JSON.".to_owned());
                                }
                                self.position += 2;
                                let low = self.parse_hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err("Некорректная суррогатная пара в JSON.".to_owned());
                                }
                                let scalar = 0x10000
                                    + ((u32::from(code) - 0xD800) << 10)
                                    + (u32::from(low) - 0xDC00);
                                let character = char::from_u32(scalar)
                                    .ok_or_else(|| "Некорректный Unicode в JSON.".to_owned())?;
                                result.push(character);
                            } else if (0xDC00..=0xDFFF).contains(&code) {
                                return Err("Изолированный суррогат в JSON.".to_owned());
                            } else {
                                let character = char::from_u32(u32::from(code))
                                    .ok_or_else(|| "Некорректный Unicode в JSON.".to_owned())?;
                                result.push(character);
                            }
                        }
                        _ => return Err("Неизвестная escape-последовательность JSON.".to_owned()),
                    }
                }
                0x00..=0x1F => return Err("Управляющий символ в строке JSON.".to_owned()),
                _ => {
                    let start = self.position - 1;
                    let width = match byte {
                        0x20..=0x7F => 1,
                        0xC2..=0xDF => 2,
                        0xE0..=0xEF => 3,
                        0xF0..=0xF4 => 4,
                        _ => return Err("Некорректный UTF-8 в JSON.".to_owned()),
                    };
                    let end = start + width;
                    let slice = self
                        .bytes
                        .get(start..end)
                        .ok_or_else(|| "Некорректный UTF-8 в JSON.".to_owned())?;
                    let text = std::str::from_utf8(slice)
                        .map_err(|_| "Некорректный UTF-8 в JSON.".to_owned())?;
                    result.push_str(text);
                    self.position = end;
                }
            }
        }
        Err("Незавершённая строка JSON.".to_owned())
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        let digits = self
            .bytes
            .get(self.position..self.position + 4)
            .ok_or_else(|| "Некорректный Unicode escape в JSON.".to_owned())?;
        let digits = std::str::from_utf8(digits)
            .map_err(|_| "Некорректный Unicode escape в JSON.".to_owned())?;
        let value = u16::from_str_radix(digits, 16)
            .map_err(|_| "Некорректный Unicode escape в JSON.".to_owned())?;
        self.position += 4;
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<String, String> {
        self.skip_whitespace();
        let start = self.position;
        if self.bytes.get(self.position) == Some(&b'-') {
            self.position += 1;
        }
        match self.bytes.get(self.position) {
            Some(b'0') => self.position += 1,
            Some(b'1'..=b'9') => {
                self.position += 1;
                while self
                    .bytes
                    .get(self.position)
                    .is_some_and(u8::is_ascii_digit)
                {
                    self.position += 1;
                }
            }
            _ => return Err("Некорректное число в JSON.".to_owned()),
        }
        if self.bytes.get(self.position) == Some(&b'.') {
            self.position += 1;
            if !self
                .bytes
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                return Err("Некорректная дробная часть числа в JSON.".to_owned());
            }
            while self
                .bytes
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
            }
        }
        if matches!(self.bytes.get(self.position), Some(b'e' | b'E')) {
            self.position += 1;
            if matches!(self.bytes.get(self.position), Some(b'+' | b'-')) {
                self.position += 1;
            }
            if !self
                .bytes
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                return Err("Некорректный порядок числа в JSON.".to_owned());
            }
            while self
                .bytes
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
            }
        }
        std::str::from_utf8(&self.bytes[start..self.position])
            .map(str::to_owned)
            .map_err(|_| "Некорректное число в JSON.".to_owned())
    }

    fn expect_literal(&mut self, literal: &[u8]) -> Result<(), String> {
        self.skip_whitespace();
        if self.bytes.get(self.position..self.position + literal.len()) == Some(literal) {
            self.position += literal.len();
            Ok(())
        } else {
            Err("Некорректный литерал в JSON.".to_owned())
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        self.skip_whitespace();
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err("Лишние данные после JSON.".to_owned())
        }
    }
}

fn parse_json_str(text: &str) -> Result<JsonValue, String> {
    let mut parser = JsonParser::new(text);
    let value = parser.parse_value(0)?;
    parser.finish()?;
    Ok(value)
}

fn write_json_pretty(value: &JsonValue) -> String {
    let mut output = String::new();
    format_json_value(value, 0, &mut output);
    output
}

fn format_json_value(value: &JsonValue, indent: usize, out: &mut String) {
    match value {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        JsonValue::Number(raw) => out.push_str(raw),
        JsonValue::String(text) => format_json_string(text, out),
        JsonValue::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            let next = indent + 2;
            for (index, item) in items.iter().enumerate() {
                push_indent(out, next);
                format_json_value(item, next, out);
                if index + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            push_indent(out, indent);
            out.push(']');
        }
        JsonValue::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            let next = indent + 2;
            for (index, (key, value)) in map.iter().enumerate() {
                push_indent(out, next);
                format_json_string(key, out);
                out.push_str(": ");
                format_json_value(value, next, out);
                if index + 1 < map.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            push_indent(out, indent);
            out.push('}');
        }
    }
}

fn push_indent(out: &mut String, spaces: usize) {
    for _ in 0..spaces {
        out.push(' ');
    }
}

fn format_json_string(value: &str, out: &mut String) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        output.push(char::from(TABLE[((triple >> 18) & 0x3F) as usize]));
        output.push(char::from(TABLE[((triple >> 12) & 0x3F) as usize]));
        if chunk.len() > 1 {
            output.push(char::from(TABLE[((triple >> 6) & 0x3F) as usize]));
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(char::from(TABLE[(triple & 0x3F) as usize]));
        } else {
            output.push('=');
        }
    }
    output
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub pinned: bool,
    pub popup_url: Option<String>,
    pub options_url: Option<String>,
    pub icon_path: Option<PathBuf>,
}

pub fn installed_extensions() -> Vec<ExtensionInfo> {
    let Some(root) = crate::storage::data_dir().map(|path| path.join("Extensions")) else {
        return Vec::new();
    };
    let pinned = read_pinned(&root);
    let mut extensions = fs::read_dir(&root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let id = entry.file_name().to_str()?.to_ascii_lowercase();
            extension_id(&id)?;
            read_extension(&entry.path(), &id, pinned.contains(&id))
        })
        .collect::<Vec<_>>();
    extensions.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    extensions
}

pub fn set_pinned(id: &str, pinned: bool) -> Result<(), String> {
    let root = crate::storage::data_dir()
        .ok_or_else(|| "Каталог данных недоступен.".to_owned())?
        .join("Extensions");
    if extension_id(id).is_none() || !root.join(id).join("manifest.json").is_file() {
        return Err("Расширение не установлено.".to_owned());
    }
    let mut ids = read_pinned(&root);
    if pinned {
        ids.insert(id.to_owned());
    } else {
        ids.remove(id);
    }
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let mut ids = ids.into_iter().collect::<Vec<_>>();
    ids.sort();
    let contents = if ids.is_empty() {
        String::new()
    } else {
        format!("{}\n", ids.join("\n"))
    };
    fs::write(root.join("pinned.txt"), contents)
        .map_err(|error| format!("Не удалось сохранить закрепления расширений: {error}"))
}

fn read_pinned(root: &Path) -> HashSet<String> {
    fs::read_to_string(root.join("pinned.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter_map(extension_id)
        .collect()
}

fn read_extension(root: &Path, id: &str, pinned: bool) -> Option<ExtensionInfo> {
    let manifest = read_json(&root.join("manifest.json"))?;
    let name = localized_name(root, &manifest);
    let action = manifest
        .get("action")
        .and_then(JsonValue::as_object)
        .or_else(|| manifest.get("browser_action").and_then(JsonValue::as_object));
    let popup = action
        .and_then(|action| non_empty_string(action, "default_popup"))
        .and_then(|path| extension_url(id, &path));
    let options = non_empty_string(&manifest, "options_page")
        .or_else(|| {
            manifest
                .get("options_ui")
                .and_then(JsonValue::as_object)
                .and_then(|options| non_empty_string(options, "page"))
        })
        .and_then(|path| extension_url(id, &path));
    let icon = action
        .and_then(|action| {
            non_empty_string(action, "default_icon").or_else(|| {
                action
                    .get("default_icon")
                    .and_then(JsonValue::as_object)
                    .and_then(select_icon)
            })
        })
        .or_else(|| {
            manifest
                .get("icons")
                .and_then(JsonValue::as_object)
                .and_then(select_icon)
        })
        .and_then(|path| safe_resource_path(root, &path));
    Some(ExtensionInfo {
        id: id.to_owned(),
        name,
        pinned,
        popup_url: popup,
        options_url: options,
        icon_path: icon,
    })
}

fn read_json(path: &Path) -> Option<JsonObject> {
    let text = fs::read_to_string(path).ok()?;
    parse_json_str(&text).ok()?.into_object()
}

fn non_empty_string(dictionary: &JsonObject, key: &str) -> Option<String> {
    let value = dictionary.get(key)?.as_str()?;
    (!value.trim().is_empty()).then(|| value.to_owned())
}

fn is_valid_extension_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let Some(manifest) = read_json(&path.join("manifest.json")) else {
        return false;
    };
    let valid_version = manifest
        .get("manifest_version")
        .and_then(JsonValue::as_i64)
        .is_some_and(|version| matches!(version, 2 | 3));
    valid_version && non_empty_string(&manifest, "name").is_some()
}

fn localized_name(root: &Path, manifest: &JsonObject) -> String {
    let short_name = non_empty_string(manifest, "short_name");
    let name = non_empty_string(manifest, "name")
        .or_else(|| short_name.clone())
        .unwrap_or_else(|| "Расширение".to_owned());
    let default_locale = non_empty_string(manifest, "default_locale");
    let current_locale = crate::storage::load_preferences().language.as_str();
    localized_name_with(
        &name,
        default_locale.as_deref(),
        current_locale,
        short_name.as_deref(),
        |locale, key| {
            let path = safe_resource_path(root, &format!("_locales/{locale}/messages.json"))?;
            localized_message(&path, key)
        },
    )
}

fn localized_message(path: &Path, key: &str) -> Option<String> {
    let messages = read_json(path)?;
    read_nested_message(
        &messages,
        |messages| messages.get(key).and_then(JsonValue::as_object),
        |message| non_empty_string(message, "message"),
    )
}

fn read_nested_message<'a, P, C>(
    parent: &'a P,
    read_entry: impl FnOnce(&'a P) -> Option<C>,
    read_message: impl FnOnce(&C) -> Option<String>,
) -> Option<String> {
    let entry = read_entry(parent)?;
    read_message(&entry)
}

fn localized_name_with(
    name: &str,
    default_locale: Option<&str>,
    current_locale: &str,
    short_name: Option<&str>,
    mut read_message: impl FnMut(&str, &str) -> Option<String>,
) -> String {
    let Some(key) = name
        .strip_prefix("__MSG_")
        .and_then(|name| name.strip_suffix("__"))
    else {
        return name.to_owned();
    };
    for locale in locale_candidates(current_locale, default_locale) {
        if let Some(localized) = read_message(&locale, key) {
            return localized;
        }
    }
    short_name
        .filter(|name| {
            !name.trim().is_empty() && !(name.starts_with("__MSG_") && name.ends_with("__"))
        })
        .unwrap_or("Расширение")
        .to_owned()
}

fn locale_candidates(current_locale: &str, default_locale: Option<&str>) -> Vec<String> {
    let mut candidates = Vec::new();
    for locale in [Some(current_locale), default_locale].into_iter().flatten() {
        let locale = locale.trim().replace('-', "_");
        if locale.is_empty()
            || !locale
                .bytes()
                .all(|character| character.is_ascii_alphanumeric() || character == b'_')
        {
            continue;
        }
        let language = locale.split_once('_').map(|(language, _)| language);
        for candidate in [Some(locale.as_str()), language].into_iter().flatten() {
            if !candidate.is_empty()
                && !candidates
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(candidate))
            {
                candidates.push(candidate.to_owned());
            }
        }
    }
    candidates
}

fn select_icon(icons: &JsonObject) -> Option<String> {
    ["32", "24", "19", "16", "48", "128"]
        .iter()
        .find_map(|size| non_empty_string(icons, size))
}

fn extension_url(id: &str, path: &str) -> Option<String> {
    let path = path.strip_prefix('/').unwrap_or(path);
    let candidate = Path::new(path);
    if path.is_empty()
        || path.contains([':', '\\'])
        || candidate
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    Some(format!("chrome-extension://{id}/{path}"))
}

fn safe_resource_path(root: &Path, path: &str) -> Option<PathBuf> {
    let relative = Path::new(path);
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    let path = root.join(relative);
    path.is_file().then_some(path)
}

pub fn configure_command_line(command_line: &CommandLine) {
    let switch = CefString::from("load-extension");
    if command_line.has_switch(Some(&switch)) != 0 {
        return;
    }
    let Some(root) = crate::storage::data_dir().map(|path| path.join("Extensions")) else {
        return;
    };
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .and_then(extension_id)
                .is_some()
        })
        .map(|entry| entry.path())
        .filter(|path| is_valid_extension_dir(path))
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    paths.sort();
    if !paths.is_empty() {
        command_line.append_switch_with_value(
            Some(&switch),
            Some(&CefString::from(paths.join(",").as_str())),
        );
    }
}

pub fn is_web_store_download(url: &str, name: &str) -> bool {
    if !name.to_ascii_lowercase().ends_with(".crx") {
        return false;
    }
    url::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && matches!(
                url.host_str(),
                Some("clients2.google.com" | "clients2.googleusercontent.com")
            )
    })
}

pub fn install_crx(path: &Path, suggested_name: &str) -> Result<String, String> {
    let _install_guard = INSTALL_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let id = extension_id(suggested_name)
        .or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(extension_id)
        })
        .ok_or_else(|| "Не удалось определить идентификатор расширения.".to_owned())?;
    let bytes = fs::read(path).map_err(|error| format!("Не удалось прочитать CRX: {error}"))?;
    if bytes.len() as u64 > MAX_CRX_BYTES {
        return Err("Пакет расширения больше 256 МБ.".to_owned());
    }
    let zip_offset = crx_zip_offset(&bytes)?;
    let public_key = crx_public_key(&bytes, &id)?;
    let root = crate::storage::data_dir()
        .ok_or_else(|| "Каталог данных недоступен.".to_owned())?
        .join("Extensions");
    let staging = root.join(format!(
        ".installing-{id}-{}",
        STAGING_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    let _staging_guard = StagingDirGuard(staging.clone());
    let package = staging.join("package.zip");
    fs::write(&package, &bytes[zip_offset..])
        .map_err(|error| format!("Не удалось распаковать CRX: {error}"))?;
    validate_archive(&package)?;
    extract_archive(&package, &staging)?;
    fs::remove_file(&package).ok();
    let manifest = staging.join("manifest.json");
    if !manifest.is_file() {
        return Err("В пакете отсутствует manifest.json.".to_owned());
    }
    set_manifest_key(&manifest, &public_key)?;
    let destination = root.join(&id);
    let backup = root.join(format!(".backup-{id}"));
    if backup.exists() {
        fs::remove_dir_all(&backup).map_err(|error| error.to_string())?;
    }
    if destination.exists() {
        fs::rename(&destination, &backup).map_err(|error| error.to_string())?;
    }
    if let Err(error) = fs::rename(&staging, &destination) {
        if backup.exists() {
            fs::rename(&backup, &destination).ok();
        }
        return Err(format!("Не удалось сохранить расширение: {error}"));
    }
    if backup.exists() {
        fs::remove_dir_all(backup).ok();
    }
    Ok(id)
}

pub fn request_restart() {
    RESTART_REQUESTED.store(true, Ordering::SeqCst);
}

pub fn is_restart_requested() -> bool {
    RESTART_REQUESTED.load(Ordering::SeqCst)
}

pub fn restart_if_requested() {
    if !RESTART_REQUESTED.swap(false, Ordering::SeqCst) {
        return;
    }
    if let Ok(executable) = std::env::current_exe() {
        std::process::Command::new(executable).spawn().ok();
    }
}

fn extension_id(name: &str) -> Option<String> {
    let id = name.split('_').next()?;
    (id.len() == 32
        && id.bytes().all(|character| {
            character.is_ascii_alphabetic() && character.to_ascii_lowercase() <= b'p'
        }))
    .then(|| id.to_ascii_lowercase())
}

fn set_manifest_key(path: &Path, public_key: &[u8]) -> Result<(), String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("Не удалось прочитать manifest.json: {error}"))?;
    let mut manifest = parse_json_str(&text)
        .map_err(|_| "Не удалось разобрать manifest.json.".to_owned())?
        .into_object()
        .ok_or_else(|| "manifest.json не является JSON-объектом.".to_owned())?;
    let encoded = encode_base64(public_key);
    manifest.insert("key".to_owned(), JsonValue::String(encoded));
    let json = write_json_pretty(&JsonValue::Object(manifest));
    if json.trim().is_empty() {
        return Err("Не удалось сериализовать manifest.json.".to_owned());
    }
    fs::write(path, format!("{json}\n"))
        .map_err(|error| format!("Не удалось записать manifest.json: {error}"))
}

fn crx_public_key(bytes: &[u8], expected_id: &str) -> Result<Vec<u8>, String> {
    if bytes.get(..4) != Some(b"Cr24") || bytes.len() < 16 {
        return Err("Файл не является корректным CRX-пакетом.".to_owned());
    }
    let version = crx_number(bytes, 4)?;
    let public_keys = match version {
        2 => {
            let end = 16usize
                .checked_add(crx_number(bytes, 8)?)
                .ok_or_else(|| "Некорректный размер ключа CRX.".to_owned())?;
            vec![
                bytes
                    .get(16..end)
                    .filter(|key| !key.is_empty())
                    .ok_or_else(|| "В CRX2 отсутствует открытый ключ.".to_owned())?,
            ]
        }
        3 => {
            let end = 12usize
                .checked_add(crx_number(bytes, 8)?)
                .ok_or_else(|| "Некорректный размер заголовка CRX.".to_owned())?;
            let header = bytes
                .get(12..end)
                .ok_or_else(|| "Некорректный заголовок CRX3.".to_owned())?;
            let mut public_keys = Vec::new();
            for proof_field in [2, 3] {
                for proof in protobuf_bytes_fields(header, proof_field)? {
                    if let Some(key) = protobuf_bytes_fields(proof, 1)?
                        .into_iter()
                        .find(|key| !key.is_empty())
                    {
                        public_keys.push(key);
                    }
                }
            }
            if public_keys.is_empty() {
                return Err("В CRX3 отсутствует открытый ключ.".to_owned());
            }
            public_keys
        }
        _ => return Err("Неподдерживаемая версия CRX-пакета.".to_owned()),
    };
    let mut actual_ids = Vec::new();
    for public_key in public_keys {
        let actual_id = public_key_id(public_key)?;
        if actual_id == expected_id {
            return Ok(public_key.to_vec());
        }
        actual_ids.push(actual_id);
    }
    Err(format!(
        "ID открытых ключей CRX ({}) не совпадают с ID установки ({expected_id}).",
        actual_ids.join(", ")
    ))
}

fn crx_number(bytes: &[u8], offset: usize) -> Result<usize, String> {
    let number = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "Некорректный заголовок CRX.".to_owned())?;
    Ok(u32::from_le_bytes(number.try_into().expect("CRX number")) as usize)
}

fn protobuf_bytes_fields(bytes: &[u8], wanted_field: u32) -> Result<Vec<&[u8]>, String> {
    let mut position = 0usize;
    let mut result = Vec::new();
    while position < bytes.len() {
        let tag = protobuf_varint(bytes, &mut position)?;
        let field = u32::try_from(tag >> 3)
            .ok()
            .filter(|field| *field != 0)
            .ok_or_else(|| "Некорректное поле protobuf в CRX.".to_owned())?;
        match tag & 7 {
            0 => {
                protobuf_varint(bytes, &mut position)?;
            }
            1 => {
                position = position
                    .checked_add(8)
                    .filter(|position| *position <= bytes.len())
                    .ok_or_else(|| "Некорректное поле protobuf в CRX.".to_owned())?;
            }
            2 => {
                let length = usize::try_from(protobuf_varint(bytes, &mut position)?)
                    .map_err(|_| "Некорректная длина protobuf в CRX.".to_owned())?;
                let end = position
                    .checked_add(length)
                    .ok_or_else(|| "Некорректная длина protobuf в CRX.".to_owned())?;
                let value = bytes
                    .get(position..end)
                    .ok_or_else(|| "Некорректная длина protobuf в CRX.".to_owned())?;
                position = end;
                if field == wanted_field {
                    result.push(value);
                }
            }
            5 => {
                position = position
                    .checked_add(4)
                    .filter(|position| *position <= bytes.len())
                    .ok_or_else(|| "Некорректное поле protobuf в CRX.".to_owned())?;
            }
            _ => return Err("Неподдерживаемый тип поля protobuf в CRX.".to_owned()),
        }
    }
    Ok(result)
}

fn protobuf_varint(bytes: &[u8], position: &mut usize) -> Result<u64, String> {
    let mut value = 0u64;
    for index in 0..10 {
        let byte = *bytes
            .get(*position)
            .ok_or_else(|| "Некорректный varint protobuf в CRX.".to_owned())?;
        *position += 1;
        if index == 9 && byte > 1 {
            return Err("Переполнение varint protobuf в CRX.".to_owned());
        }
        value |= u64::from(byte & 0x7F) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("Некорректный varint protobuf в CRX.".to_owned())
}

struct AlgorithmHandle(*mut core::ffi::c_void);

impl Drop for AlgorithmHandle {
    fn drop(&mut self) {
        unsafe { BCryptCloseAlgorithmProvider(self.0, 0) };
    }
}

struct HashHandle(*mut core::ffi::c_void);

impl Drop for HashHandle {
    fn drop(&mut self) {
        unsafe { BCryptDestroyHash(self.0) };
    }
}

fn public_key_id(public_key: &[u8]) -> Result<String, String> {
    let hash = sha256(public_key)?;
    Ok(hash[..16]
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0F])
        .map(|nibble| char::from(b'a' + nibble))
        .collect())
}

fn sha256(bytes: &[u8]) -> Result<[u8; 32], String> {
    unsafe {
        let mut algorithm = AlgorithmHandle(ptr::null_mut());
        let algorithm_name = wide_null("SHA256");
        let status =
            BCryptOpenAlgorithmProvider(&mut algorithm.0, algorithm_name.as_ptr(), ptr::null(), 0);
        if status < 0 {
            return Err(format!("BCryptOpenAlgorithmProvider: {status:#x}"));
        }
        let object_length = bcrypt_property(&algorithm, "ObjectLength")? as usize;
        let hash_length = bcrypt_property(&algorithm, "HashDigestLength")? as usize;
        if hash_length != 32 || object_length == 0 {
            return Err("Windows CNG вернул неверные параметры SHA-256".to_owned());
        }
        let mut object = vec![0u8; object_length];
        let mut hash = HashHandle(ptr::null_mut());
        let status = BCryptCreateHash(
            algorithm.0,
            &mut hash.0,
            object.as_mut_ptr(),
            object.len() as u32,
            ptr::null(),
            0,
            0,
        );
        if status < 0 {
            return Err(format!("BCryptCreateHash: {status:#x}"));
        }
        let status = BCryptHashData(hash.0, bytes.as_ptr(), bytes.len() as u32, 0);
        if status < 0 {
            return Err(format!("BCryptHashData: {status:#x}"));
        }
        let mut output = [0u8; 32];
        let status = BCryptFinishHash(hash.0, output.as_mut_ptr(), output.len() as u32, 0);
        if status < 0 {
            return Err(format!("BCryptFinishHash: {status:#x}"));
        }
        Ok(output)
    }
}

fn bcrypt_property(algorithm: &AlgorithmHandle, name: &str) -> Result<u32, String> {
    unsafe {
        let property = wide_null(name);
        let mut value = 0u32;
        let mut written = 0u32;
        let status = BCryptGetProperty(
            algorithm.0,
            property.as_ptr(),
            &mut value as *mut u32 as *mut u8,
            mem::size_of::<u32>() as u32,
            &mut written,
            0,
        );
        if status < 0 || written != mem::size_of::<u32>() as u32 {
            Err(format!("BCryptGetProperty({name}): {status:#x}"))
        } else {
            Ok(value)
        }
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn crx_zip_offset(bytes: &[u8]) -> Result<usize, String> {
    if bytes.get(..4) != Some(b"Cr24") || bytes.len() < 16 {
        return Err("Файл не является корректным CRX-пакетом.".to_owned());
    }
    let number = |offset| {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("CRX header")) as usize
    };
    let version = number(4);
    let offset = match version {
        2 => 16usize
            .checked_add(number(8))
            .and_then(|offset| offset.checked_add(number(12))),
        3 => 12usize.checked_add(number(8)),
        _ => None,
    }
    .filter(|offset| *offset < bytes.len())
    .ok_or_else(|| "Неподдерживаемая версия CRX-пакета.".to_owned())?;
    if bytes.get(offset..offset + 2) != Some(b"PK") {
        return Err("В CRX отсутствуют данные расширения.".to_owned());
    }
    Ok(offset)
}

fn validate_archive(package: &Path) -> Result<(), String> {
    let output = tar()
        .arg("-tf")
        .arg(package)
        .output()
        .map_err(|error| format!("Не удалось проверить CRX: {error}"))?;
    if !output.status.success() {
        return Err("CRX содержит повреждённый ZIP-архив.".to_owned());
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    if listing.lines().count() > 8192 {
        return Err("Слишком много файлов в расширении.".to_owned());
    }
    for entry in listing.lines() {
        let path = entry.trim().replace('\\', "/");
        let candidate = Path::new(&path);
        if path.starts_with('/')
            || candidate
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
        {
            return Err("CRX содержит небезопасные пути файлов.".to_owned());
        }
    }
    Ok(())
}

fn extract_archive(package: &Path, destination: &Path) -> Result<(), String> {
    let status = tar()
        .arg("-xf")
        .arg(package)
        .arg("-C")
        .arg(destination)
        .status()
        .map_err(|error| format!("Не удалось распаковать CRX: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("Не удалось распаковать расширение.".to_owned())
    }
}

fn tar() -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new(r"C:\Windows\System32\tar.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve_name(
        name: &str,
        default_locale: Option<&str>,
        current_locale: &str,
        messages: &[(&str, &str, &str)],
    ) -> String {
        localized_name_with(name, default_locale, current_locale, None, |locale, key| {
            messages
                .iter()
                .find(|(message_locale, message_key, _)| {
                    *message_locale == locale && *message_key == key
                })
                .map(|(_, _, message)| (*message).to_owned())
        })
    }

    #[test]
    fn nested_message_is_read_before_the_json_owner_is_dropped() {
        use std::{cell::Cell, collections::HashMap, rc::Rc};

        struct Messages {
            entries: HashMap<String, HashMap<String, String>>,
            alive: Rc<Cell<bool>>,
        }

        impl Drop for Messages {
            fn drop(&mut self) {
                self.alive.set(false);
            }
        }

        let alive = Rc::new(Cell::new(true));
        let alive_during_read = alive.clone();
        {
            let messages = Messages {
                entries: HashMap::from([(
                    "extName".to_owned(),
                    HashMap::from([
                        ("message".to_owned(), "uBlock Origin Lite".to_owned()),
                        ("description".to_owned(), "extension name.".to_owned()),
                    ]),
                )]),
                alive: alive.clone(),
            };
            let resolved = read_nested_message(
                &messages,
                |messages| messages.entries.get("extName").cloned(),
                |message| {
                    assert!(alive_during_read.get());
                    message.get("message").cloned()
                },
            );
            assert_eq!(resolved.as_deref(), Some("uBlock Origin Lite"));
        }
        assert!(!alive.get());
    }

    #[test]
    fn unresolved_localization_uses_short_name() {
        let name = localized_name_with(
            "__MSG_extName__",
            Some("en"),
            "ru",
            Some("uBO Lite"),
            |_, _| None,
        );
        assert_eq!(name, "uBO Lite");
    }

    #[test]
    fn localization_prefers_the_current_locale() {
        let messages = [
            ("en", "extension_name", "English name"),
            ("ru", "extension_name", "Русское имя"),
        ];
        assert_eq!(
            resolve_name("__MSG_extension_name__", Some("en"), "ru", &messages,),
            "Русское имя"
        );
    }

    #[test]
    fn localization_uses_regional_and_language_fallbacks() {
        let messages = [("pt", "extension_name", "Nome em português")];
        assert_eq!(
            resolve_name("__MSG_extension_name__", Some("en"), "pt-BR", &messages,),
            "Nome em português"
        );
        let messages = [
            ("pt", "extension_name", "Nome em português"),
            ("pt_BR", "extension_name", "Nome brasileiro"),
        ];
        assert_eq!(
            resolve_name("__MSG_extension_name__", Some("en"), "pt-BR", &messages,),
            "Nome brasileiro"
        );
    }

    #[test]
    fn localization_falls_back_when_a_message_is_missing() {
        let messages = [
            ("ru", "other_message", "Другое"),
            ("en", "extension_name", "English name"),
        ];
        assert_eq!(
            resolve_name("__MSG_extension_name__", Some("en"), "ru", &messages,),
            "English name"
        );
    }

    #[test]
    fn localization_falls_back_to_the_default_language() {
        let messages = [("en", "extension_name", "English name")];
        assert_eq!(
            resolve_name("__MSG_extension_name__", Some("en_GB"), "fr-CA", &messages,),
            "English name"
        );
    }

    #[test]
    fn unresolved_localization_has_a_readable_fallback() {
        assert_eq!(
            resolve_name("__MSG_extension_name__", Some("en"), "ru", &[]),
            "Расширение"
        );
        assert_eq!(
            resolve_name("Plain extension", Some("en"), "ru", &[]),
            "Plain extension"
        );
    }

    #[test]
    fn locale_fallbacks_are_normalized_deduplicated_and_safe() {
        assert_eq!(
            locale_candidates("pt-BR", Some("en-GB")),
            ["pt_BR", "pt", "en_GB", "en"]
        );
        assert_eq!(locale_candidates("en_US", Some("en-US")), ["en_US", "en"]);
        assert_eq!(locale_candidates("../ru", Some("en")), ["en"]);
    }

    #[test]
    fn recognizes_only_official_web_store_downloads() {
        assert!(is_web_store_download(
            "https://clients2.google.com/service/update2/crx?response=redirect",
            "extension.crx"
        ));
        assert!(is_web_store_download(
            "https://clients2.googleusercontent.com/crx/blobs/package",
            "EXTENSION.CRX"
        ));
        assert!(!is_web_store_download(
            "https://example.com/service/update2/crx",
            "extension.crx"
        ));
        assert!(!is_web_store_download(
            "https://clients2.google.com/service/update2/crx",
            "extension.zip"
        ));
    }

    #[test]
    fn extension_resources_stay_inside_the_extension_origin() {
        let id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert_eq!(
            extension_url(id, "popup.html").as_deref(),
            Some("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/popup.html")
        );
        assert_eq!(extension_url(id, "../outside.html"), None);
        assert_eq!(extension_url(id, "https://example.com"), None);
        assert_eq!(extension_url(id, "C:/outside.html"), None);
    }

    #[test]
    fn extracts_crx2_public_keys_and_verifies_their_ids() {
        const ABC_ID: &str = "lkhibglpipabmpokebebeanofnkocccd";
        let key = b"abc";
        let mut crx = b"Cr24\x02\0\0\0".to_vec();
        crx.extend((key.len() as u32).to_le_bytes());
        crx.extend(1u32.to_le_bytes());
        crx.extend(key);
        crx.extend([0, b'P', b'K']);
        assert_eq!(public_key_id(key).as_deref(), Ok(ABC_ID));
        assert_eq!(crx_public_key(&crx, ABC_ID), Ok(key.to_vec()));
        assert!(crx_public_key(&crx, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_err());
    }

    #[test]
    fn extracts_crx3_public_keys_from_protobuf_headers() {
        const ABC_ID: &str = "lkhibglpipabmpokebebeanofnkocccd";
        fn varint(mut value: u64) -> Vec<u8> {
            let mut bytes = Vec::new();
            loop {
                let mut byte = (value & 0x7F) as u8;
                value >>= 7;
                if value != 0 {
                    byte |= 0x80;
                }
                bytes.push(byte);
                if value == 0 {
                    return bytes;
                }
            }
        }
        fn field(number: u32, value: &[u8]) -> Vec<u8> {
            let mut bytes = varint(u64::from(number) << 3 | 2);
            bytes.extend(varint(value.len() as u64));
            bytes.extend(value);
            bytes
        }
        let proof = field(1, b"abc");
        let other_proof = field(1, b"other");
        let mut header = field(2, &other_proof);
        header.extend(field(2, &proof));
        header.extend(field(10000, b"signed header"));
        let mut crx = b"Cr24\x03\0\0\0".to_vec();
        crx.extend((header.len() as u32).to_le_bytes());
        crx.extend(header);
        crx.extend(b"PK");
        assert_eq!(crx_public_key(&crx, ABC_ID), Ok(b"abc".to_vec()));
    }

    #[test]
    fn rejects_missing_or_truncated_crx_public_keys() {
        let mut crx2 = b"Cr24\x02\0\0\0".to_vec();
        crx2.extend(100u32.to_le_bytes());
        crx2.extend(0u32.to_le_bytes());
        crx2.extend(b"PK");
        assert!(crx_public_key(&crx2, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_err());

        let mut crx3 = b"Cr24\x03\0\0\0".to_vec();
        crx3.extend(2u32.to_le_bytes());
        crx3.extend([0x12, 0x05]);
        crx3.extend(b"PK");
        assert!(crx_public_key(&crx3, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_err());
        assert!(protobuf_varint(&[0x80], &mut 0).is_err());
    }

    #[test]
    fn parses_crx_headers_and_extension_ids() {
        assert_eq!(
            extension_id("DDKJIAHEJLHFCAFBDDMGIAHCPHECMPFH_2026.crx").as_deref(),
            Some("ddkjiahejlhfcafbddmgiahcphecmpfh")
        );
        assert_eq!(extension_id("not-an-extension.crx"), None);
        let mut crx = b"Cr24\x03\0\0\0\x04\0\0\0abcdPK\x05\x06".to_vec();
        assert_eq!(crx_zip_offset(&crx), Ok(16));
        crx[8] = 255;
        assert!(crx_zip_offset(&crx).is_err());
        assert!(crx_zip_offset(b"not a crx").is_err());
    }

    #[test]
    fn json_parser_handles_bom_escapes_surrogates_and_nested_values() {
        let input = "\u{FEFF}{\"manifest_version\": 3, \"name\": \"Rocket \\uD83D\\uDE80 \\\"Test\\\"\", \"nested\": {\"arr\": [true, false, null, -12.5e2]}}";
        let value = parse_json_str(input).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.get("manifest_version").and_then(JsonValue::as_i64), Some(3));
        assert_eq!(
            object.get("name").and_then(JsonValue::as_str),
            Some("Rocket 🚀 \"Test\"")
        );
        let nested = object.get("nested").and_then(JsonValue::as_object).unwrap();
        assert_eq!(
            nested.get("arr"),
            Some(&JsonValue::Array(vec![
                JsonValue::Bool(true),
                JsonValue::Bool(false),
                JsonValue::Null,
                JsonValue::Number("-12.5e2".to_owned()),
            ]))
        );
        // Reject lone surrogate and trailing garbage
        assert!(parse_json_str("\"\\uD83D\"").is_err());
        assert!(parse_json_str("{} extra").is_err());
        // Reject excessive nesting depth
        let deep = "[".repeat(MAX_JSON_DEPTH + 4) + &"]".repeat(MAX_JSON_DEPTH + 4);
        assert!(parse_json_str(&deep).is_err());
    }

    #[test]
    fn base64_and_manifest_key_roundtrip_preserve_fields() {
        assert_eq!(encode_base64(b"abc"), "YWJj");
        assert_eq!(encode_base64(b"ab"), "YWI=");
        assert_eq!(encode_base64(b"a"), "YQ==");

        let temp = std::env::temp_dir().join(format!(
            "kockers-ext-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();
        let manifest_path = temp.join("manifest.json");
        fs::write(
            &manifest_path,
            "\u{FEFF}{\"manifest_version\": 3, \"name\": \"Demo Extension\", \"action\": {\"default_popup\": \"popup.html\"}}",
        )
        .unwrap();

        assert!(is_valid_extension_dir(&temp));
        set_manifest_key(&manifest_path, b"abc").unwrap();
        let updated = read_json(&manifest_path).unwrap();
        assert_eq!(non_empty_string(&updated, "key").as_deref(), Some("YWJj"));
        assert_eq!(
            updated.get("manifest_version").and_then(JsonValue::as_i64),
            Some(3)
        );
        assert!(is_valid_extension_dir(&temp));

        let info = read_extension(&temp, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", true).unwrap();
        assert_eq!(info.name, "Demo Extension");
        assert_eq!(
            info.popup_url.as_deref(),
            Some("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/popup.html")
        );

        // Invalid manifest_version is rejected
        fs::write(&manifest_path, "{\"manifest_version\": 1, \"name\": \"Old\"}").unwrap();
        assert!(!is_valid_extension_dir(&temp));

        let _ = fs::remove_dir_all(&temp);
    }
}
