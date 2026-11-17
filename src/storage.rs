use std::{
    fs, iter,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

const MAX_HISTORY: usize = 2_000;
const MAX_CLOSED_TABS: usize = 20;
const MAX_BOOKMARKS: usize = 200;

macro_rules! text_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }
            fn parse(value: &str) -> Option<Self> {
                match value { $($value => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

text_enum!(SearchEngine { Google => "google", Bing => "bing", DuckDuckGo => "duckduckgo", Yandex => "yandex" });
text_enum!(Theme { Light => "light", Dark => "dark" });
text_enum!(Accent { Olive => "olive", Blue => "blue", Graphite => "graphite" });
text_enum!(Language { Russian => "ru", English => "en" });
text_enum!(StartupMode { NewTab => "new_tab", Restore => "restore", Custom => "custom" });

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preferences {
    pub save_history: bool,
    pub ask_download_path: bool,
    pub show_bookmarks: bool,
    pub search_engine: SearchEngine,
    pub theme: Theme,
    pub accent: Accent,
    pub language: Language,
    pub startup_mode: StartupMode,
    pub startup_url: String,
    pub download_dir: String,
    pub reduce_animations: bool,
    pub smooth_scrolling: bool,
    pub background_throttling: bool,
    pub lazy_restore: bool,
    pub continue_background: bool,
    pub hardware_acceleration: bool,
    pub show_tips: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            save_history: true,
            ask_download_path: false,
            show_bookmarks: true,
            search_engine: SearchEngine::Google,
            theme: Theme::Light,
            accent: Accent::Olive,
            language: Language::Russian,
            startup_mode: StartupMode::Restore,
            startup_url: String::new(),
            download_dir: downloads_dir()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            reduce_animations: false,
            smooth_scrolling: true,
            background_throttling: true,
            lazy_restore: true,
            continue_background: false,
            hardware_acceleration: true,
            show_tips: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    pub nickname: String,
    pub description: String,
    pub avatar_path: String,
    pub banner_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub visited_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTab {
    pub url: String,
    pub title: String,
    pub pinned: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    pub tabs: Vec<SessionTab>,
    pub active: usize,
}

pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("Kockers Browser"))
}

pub fn downloads_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|path| path.join("Downloads"))
}

pub fn load_preferences() -> Preferences {
    let Some(path) = data_dir().map(|path| path.join("preferences.txt")) else {
        return Preferences::default();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Preferences::default();
    };
    let mut preferences = Preferences::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let boolean = || value == "1";
        match key {
            "history" => preferences.save_history = boolean(),
            "ask_download" => preferences.ask_download_path = boolean(),
            "bookmarks" => preferences.show_bookmarks = boolean(),
            "reduce_animations" => preferences.reduce_animations = boolean(),
            "smooth_scrolling" => preferences.smooth_scrolling = boolean(),
            "background_throttling" => preferences.background_throttling = boolean(),
            "lazy_restore" => preferences.lazy_restore = boolean(),
            "continue_background" => preferences.continue_background = boolean(),
            "hardware_acceleration" => preferences.hardware_acceleration = boolean(),
            "show_tips" => preferences.show_tips = boolean(),
            "search" => {
                if let Some(value) = SearchEngine::parse(value) {
                    preferences.search_engine = value;
                }
            }
            "theme" => {
                if let Some(value) = Theme::parse(value) {
                    preferences.theme = value;
                }
            }
            "accent" => {
                if let Some(value) = Accent::parse(value) {
                    preferences.accent = value;
                }
            }
            "language" => {
                if let Some(value) = Language::parse(value) {
                    preferences.language = value;
                }
            }
            "startup" => {
                if let Some(value) = StartupMode::parse(value) {
                    preferences.startup_mode = value;
                }
            }
            "startup_url" => {
                if let Some(value) = decode_field(value) {
                    preferences.startup_url = value;
                }
            }
            "download_dir" => {
                if let Some(value) = decode_field(value) {
                    preferences.download_dir = value;
                }
            }
            _ => {}
        }
    }
    preferences
}

pub fn save_preferences(preferences: &Preferences) {
    if let Some(path) = data_dir().map(|path| path.join("preferences.txt")) {
        let mut lines = vec![
            format!("history={}", u8::from(preferences.save_history)),
            format!("ask_download={}", u8::from(preferences.ask_download_path)),
            format!("bookmarks={}", u8::from(preferences.show_bookmarks)),
            format!("search={}", preferences.search_engine.as_str()),
            format!("theme={}", preferences.theme.as_str()),
            format!("accent={}", preferences.accent.as_str()),
            format!("language={}", preferences.language.as_str()),
            format!("startup={}", preferences.startup_mode.as_str()),
            format!("startup_url={}", encode_field(&preferences.startup_url)),
            format!("download_dir={}", encode_field(&preferences.download_dir)),
            format!(
                "reduce_animations={}",
                u8::from(preferences.reduce_animations)
            ),
            format!(
                "smooth_scrolling={}",
                u8::from(preferences.smooth_scrolling)
            ),
            format!(
                "background_throttling={}",
                u8::from(preferences.background_throttling)
            ),
            format!("lazy_restore={}", u8::from(preferences.lazy_restore)),
            format!(
                "continue_background={}",
                u8::from(preferences.continue_background)
            ),
            format!(
                "hardware_acceleration={}",
                u8::from(preferences.hardware_acceleration)
            ),
            format!("show_tips={}", u8::from(preferences.show_tips)),
        ];
        lines.push(String::new());
        write_atomic(&path, lines.join("\n").as_bytes());
    }
}

pub fn load_profile() -> Profile {
    let Some(path) = data_dir().map(|path| path.join("profile.txt")) else {
        return Profile::default();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Profile::default();
    };
    let mut profile = Profile::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('\t') else {
            continue;
        };
        let Some(value) = decode_field(value) else {
            continue;
        };
        match key {
            "nickname" => profile.nickname = value,
            "description" => profile.description = value,
            "avatar" => profile.avatar_path = value,
            "banner" => profile.banner_path = value,
            _ => {}
        }
    }
    profile
}

pub fn save_profile(profile: &Profile) {
    if let Some(path) = data_dir().map(|path| path.join("profile.txt")) {
        let text = format!(
            "nickname\t{}\ndescription\t{}\navatar\t{}\nbanner\t{}\n",
            encode_field(&profile.nickname),
            encode_field(&profile.description),
            encode_field(&profile.avatar_path),
            encode_field(&profile.banner_path),
        );
        write_atomic(&path, text.as_bytes());
    }
}

pub fn store_profile_image(source: &Path, banner: bool) -> Result<String, String> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|value| matches!(value.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif"))
        .ok_or_else(|| "Выберите изображение PNG, JPEG, WebP или GIF.".to_owned())?;
    let metadata = fs::metadata(source).map_err(|error| error.to_string())?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Err("Изображение должно быть не больше 16 МБ.".to_owned());
    }
    let directory = data_dir()
        .ok_or_else(|| "Каталог данных недоступен.".to_owned())?
        .join("Profile");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let name = if banner { "banner" } else { "avatar" };
    let target = directory.join(format!("{name}.{extension}"));
    fs::copy(source, &target).map_err(|error| error.to_string())?;
    Ok(target.to_string_lossy().into_owned())
}

pub fn export_config() -> String {
    let preferences = load_preferences();
    let profile = load_profile();
    let fields = vec![
        "\"version\":1".to_owned(),
        format!("\"history\":{}", preferences.save_history),
        format!("\"ask_download\":{}", preferences.ask_download_path),
        format!("\"bookmarks_bar\":{}", preferences.show_bookmarks),
        format!("\"search\":\"{}\"", preferences.search_engine.as_str()),
        format!("\"theme\":\"{}\"", preferences.theme.as_str()),
        format!("\"accent\":\"{}\"", preferences.accent.as_str()),
        format!("\"language\":\"{}\"", preferences.language.as_str()),
        format!("\"startup\":\"{}\"", preferences.startup_mode.as_str()),
        format!(
            "\"startup_url\":\"{}\"",
            json_escape(&preferences.startup_url)
        ),
        format!(
            "\"download_dir\":\"{}\"",
            json_escape(&preferences.download_dir)
        ),
        format!("\"reduce_animations\":{}", preferences.reduce_animations),
        format!("\"smooth_scrolling\":{}", preferences.smooth_scrolling),
        format!(
            "\"background_throttling\":{}",
            preferences.background_throttling
        ),
        format!("\"lazy_restore\":{}", preferences.lazy_restore),
        format!(
            "\"continue_background\":{}",
            preferences.continue_background
        ),
        format!(
            "\"hardware_acceleration\":{}",
            preferences.hardware_acceleration
        ),
        format!("\"show_tips\":{}", preferences.show_tips),
        format!("\"nickname\":\"{}\"", json_escape(&profile.nickname)),
        format!("\"description\":\"{}\"", json_escape(&profile.description)),
    ];
    format!("{{\n  {}\n}}\n", fields.join(",\n  "))
}

pub fn import_config(text: &str) -> Result<(Preferences, Profile), String> {
    let mut parser = JsonParser::new(text);
    parser.expect(b'{')?;
    let mut preferences = Preferences::default();
    let mut profile = load_profile();
    if parser.consume(b'}') {
        return Err("Файл конфигурации пуст.".to_owned());
    }
    loop {
        let key = parser.string()?;
        parser.expect(b':')?;
        let value = parser.value()?;
        let boolean = || match value {
            JsonValue::Bool(value) => Ok(value),
            _ => Err(format!("Поле {key} должно быть true или false.")),
        };
        let string = || match &value {
            JsonValue::String(value) => Ok(value.clone()),
            _ => Err(format!("Поле {key} должно быть строкой.")),
        };
        match key.as_str() {
            "version" => {}
            "history" => preferences.save_history = boolean()?,
            "ask_download" => preferences.ask_download_path = boolean()?,
            "bookmarks_bar" => preferences.show_bookmarks = boolean()?,
            "reduce_animations" => preferences.reduce_animations = boolean()?,
            "smooth_scrolling" => preferences.smooth_scrolling = boolean()?,
            "background_throttling" => preferences.background_throttling = boolean()?,
            "lazy_restore" => preferences.lazy_restore = boolean()?,
            "continue_background" => preferences.continue_background = boolean()?,
            "hardware_acceleration" => preferences.hardware_acceleration = boolean()?,
            "show_tips" => preferences.show_tips = boolean()?,
            "search" => {
                preferences.search_engine = SearchEngine::parse(&string()?)
                    .ok_or_else(|| "Неизвестная поисковая система.".to_owned())?;
            }
            "theme" => {
                preferences.theme =
                    Theme::parse(&string()?).ok_or_else(|| "Неизвестная тема.".to_owned())?;
            }
            "accent" => {
                preferences.accent = Accent::parse(&string()?)
                    .ok_or_else(|| "Неизвестный акцентный цвет.".to_owned())?;
            }
            "language" => {
                preferences.language =
                    Language::parse(&string()?).ok_or_else(|| "Неизвестный язык.".to_owned())?;
            }
            "startup" => {
                preferences.startup_mode = StartupMode::parse(&string()?)
                    .ok_or_else(|| "Неизвестный режим запуска.".to_owned())?;
            }
            "startup_url" => preferences.startup_url = string()?,
            "download_dir" => preferences.download_dir = string()?,
            "nickname" => profile.nickname = string()?,
            "description" => profile.description = string()?,
            _ => {}
        }
        if parser.consume(b'}') {
            break;
        }
        parser.expect(b',')?;
    }
    parser.finish()?;
    Ok((preferences, profile))
}

pub fn load_bookmarks() -> Vec<Bookmark> {
    let Some(path) = data_dir().map(|path| path.join("bookmarks.txt")) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut bookmarks = text
        .lines()
        .filter_map(|line| {
            let (url, title) = line.split_once('\t')?;
            let url = decode_field(url)?;
            if !url.starts_with("https://") && !url.starts_with("http://") {
                return None;
            }
            let title = decode_field(title).unwrap_or_else(|| url.clone());
            Some(Bookmark { url, title })
        })
        .collect::<Vec<_>>();
    bookmarks.truncate(MAX_BOOKMARKS);
    bookmarks
}

pub fn save_bookmarks(bookmarks: &[Bookmark]) {
    let Some(path) = data_dir().map(|path| path.join("bookmarks.txt")) else {
        return;
    };
    let mut text = String::new();
    for bookmark in bookmarks.iter().take(MAX_BOOKMARKS) {
        if bookmark.url.starts_with("https://") || bookmark.url.starts_with("http://") {
            text.push_str(&format!(
                "{}\t{}\n",
                encode_field(&bookmark.url),
                encode_field(&bookmark.title)
            ));
        }
    }
    write_atomic(&path, text.as_bytes());
}

pub fn load_session() -> Session {
    let Some(path) = data_dir().map(|path| path.join("session.txt")) else {
        return Session::default();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Session::default();
    };
    let mut session = Session::default();
    for line in text.lines() {
        if let Some(active) = line.strip_prefix("A\t") {
            session.active = active.parse().unwrap_or(0);
        } else if let Some(fields) = line.strip_prefix("T\t") {
            let mut fields = fields.split('\t');
            let pinned = fields.next() == Some("1");
            let Some(url) = fields.next().and_then(decode_field) else {
                continue;
            };
            if !url.starts_with("https://") && !url.starts_with("http://") {
                continue;
            }
            let title = fields.next().and_then(decode_field).unwrap_or_default();
            session.tabs.push(SessionTab { url, title, pinned });
        }
    }
    session.tabs.truncate(50);
    if session.tabs.is_empty() {
        session.active = 0;
    } else {
        session.active = session.active.min(session.tabs.len() - 1);
    }
    session
}

pub fn save_session(session: &Session) {
    let Some(path) = data_dir().map(|path| path.join("session.txt")) else {
        return;
    };
    let mut text = String::from("KOCKERS-SESSION\t1\n");
    text.push_str(&format!("A\t{}\n", session.active));
    for tab in session.tabs.iter().take(50) {
        if tab.url.starts_with("https://") || tab.url.starts_with("http://") {
            text.push_str(&format!(
                "T\t{}\t{}\t{}\n",
                u8::from(tab.pinned),
                encode_field(&tab.url),
                encode_field(&tab.title)
            ));
        }
    }
    write_atomic(&path, text.as_bytes());
}

pub fn add_history(url: &str, title: &str) {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return;
    }
    let mut entries = load_history();
    entries.retain(|entry| entry.url != url);
    entries.push(HistoryEntry {
        url: url.to_owned(),
        title: title.to_owned(),
        visited_at: now(),
    });
    if entries.len() > MAX_HISTORY {
        entries.drain(..entries.len() - MAX_HISTORY);
    }
    save_history(&entries);
}

pub fn remove_history(url: &str) {
    let entries = load_history()
        .into_iter()
        .filter(|entry| entry.url != url)
        .collect::<Vec<_>>();
    save_history(&entries);
}

pub fn clear_history_since(seconds: Option<u64>) {
    if seconds.is_none() {
        if let Some(path) = data_dir().map(|path| path.join("history.txt")) {
            let _ = fs::remove_file(path);
        }
        return;
    }
    let cutoff = now().saturating_sub(seconds.unwrap_or(0));
    let entries = load_history()
        .into_iter()
        .filter(|entry| entry.visited_at < cutoff)
        .collect::<Vec<_>>();
    save_history(&entries);
}

pub fn load_history() -> Vec<HistoryEntry> {
    let Some(path) = data_dir().map(|path| path.join("history.txt")) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let visited_at = fields.next()?.parse().ok()?;
            let url = decode_field(fields.next()?)?;
            let title = decode_field(fields.next()?).unwrap_or_default();
            Some(HistoryEntry {
                url,
                title,
                visited_at,
            })
        })
        .collect()
}

fn save_history(entries: &[HistoryEntry]) {
    let Some(path) = data_dir().map(|path| path.join("history.txt")) else {
        return;
    };
    let mut text = String::new();
    for entry in entries {
        text.push_str(&format!(
            "{}\t{}\t{}\n",
            entry.visited_at,
            encode_field(&entry.url),
            encode_field(&entry.title)
        ));
    }
    write_atomic(&path, text.as_bytes());
}

pub fn closed_tab_limit() -> usize {
    MAX_CLOSED_TABS
}

fn write_atomic(path: &Path, bytes: &[u8]) {
    let Some(parent) = path.parent() else { return };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let temporary = path.with_extension("tmp");
    if fs::write(&temporary, bytes).is_err() {
        return;
    }
    let source = temporary
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let target = path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        let _ = fs::remove_file(temporary);
    }
}

fn encode_field(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn decode_field(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            value if value.is_control() => escaped.push_str(&format!("\\u{:04X}", value as u32)),
            value => escaped.push(value),
        }
    }
    escaped
}

#[derive(Debug, PartialEq, Eq)]
enum JsonValue {
    String(String),
    Bool(bool),
    Number,
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> JsonParser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            position: 0,
        }
    }

    fn whitespace(&mut self) {
        while self
            .bytes
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        self.whitespace();
        if self.bytes.get(self.position) == Some(&expected) {
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
            Err(format!("Ожидался символ {} в JSON.", expected as char))
        }
    }

    fn string(&mut self) -> Result<String, String> {
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
                        .ok_or_else(|| "Незавершённая escape-последовательность.".to_owned())?;
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
                            let digits = self
                                .bytes
                                .get(self.position..self.position + 4)
                                .ok_or_else(|| "Некорректный Unicode escape.".to_owned())?;
                            let digits = std::str::from_utf8(digits)
                                .map_err(|_| "Некорректный Unicode escape.".to_owned())?;
                            let code = u16::from_str_radix(digits, 16)
                                .map_err(|_| "Некорректный Unicode escape.".to_owned())?;
                            let character = char::from_u32(code as u32)
                                .ok_or_else(|| "Некорректный Unicode escape.".to_owned())?;
                            result.push(character);
                            self.position += 4;
                        }
                        _ => return Err("Неизвестная escape-последовательность.".to_owned()),
                    }
                }
                0..=31 => return Err("Управляющий символ в строке JSON.".to_owned()),
                _ => {
                    let start = self.position - 1;
                    let width =
                        utf8_width(byte).ok_or_else(|| "Некорректный UTF-8 в JSON.".to_owned())?;
                    let end = start + width;
                    let text = self
                        .bytes
                        .get(start..end)
                        .ok_or_else(|| "Некорректный UTF-8 в JSON.".to_owned())?;
                    result.push_str(
                        std::str::from_utf8(text)
                            .map_err(|_| "Некорректный UTF-8 в JSON.".to_owned())?,
                    );
                    self.position = end;
                }
            }
        }
        Err("Незавершённая строка JSON.".to_owned())
    }

    fn value(&mut self) -> Result<JsonValue, String> {
        self.whitespace();
        match self.bytes.get(self.position) {
            Some(b'"') => self.string().map(JsonValue::String),
            Some(b't') => {
                self.literal(b"true")?;
                Ok(JsonValue::Bool(true))
            }
            Some(b'f') => {
                self.literal(b"false")?;
                Ok(JsonValue::Bool(false))
            }
            Some(b'-' | b'0'..=b'9') => {
                while self.bytes.get(self.position).is_some_and(|value| {
                    matches!(value, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                }) {
                    self.position += 1;
                }
                Ok(JsonValue::Number)
            }
            _ => Err("Неподдерживаемое значение JSON.".to_owned()),
        }
    }

    fn literal(&mut self, literal: &[u8]) -> Result<(), String> {
        if self.bytes.get(self.position..self.position + literal.len()) == Some(literal) {
            self.position += literal.len();
            Ok(())
        } else {
            Err("Некорректное значение JSON.".to_owned())
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        self.whitespace();
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err("Лишние данные после JSON.".to_owned())
        }
    }
}

fn utf8_width(byte: u8) -> Option<usize> {
    match byte {
        0x20..=0x7F => Some(1),
        0xC2..=0xDF => Some(2),
        0xE0..=0xEF => Some(3),
        0xF0..=0xF4 => Some(4),
        _ => None,
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_survive_unicode_and_delimiters() {
        let original = "Заголовок\tстрока\n100% ✓";
        assert_eq!(
            decode_field(&encode_field(original)).as_deref(),
            Some(original)
        );
    }

    #[test]
    fn rejects_incomplete_escape_sequences() {
        assert_eq!(decode_field("abc%2"), None);
        assert_eq!(decode_field("abc%XZ"), None);
    }

    #[test]
    fn session_defaults_are_safe() {
        let session = Session::default();
        assert_eq!(session.active, 0);
        assert!(session.tabs.is_empty());
        assert!(Preferences::default().save_history);
        assert!(!Preferences::default().ask_download_path);
    }

    #[test]
    fn json_roundtrip_preserves_settings_and_unicode() {
        let source = export_config();
        let (preferences, _) = import_config(&source).unwrap();
        assert_eq!(preferences, load_preferences());
        let (_, profile) = import_config("{\"nickname\":\"Тест ✓\"}").unwrap();
        assert_eq!(profile.nickname, "Тест ✓");
    }

    #[test]
    fn json_import_rejects_truncated_or_wrongly_typed_values() {
        assert!(import_config("{\"history\":").is_err());
        assert!(import_config("{\"history\":\"yes\"}").is_err());
        assert!(import_config("{\"theme\":\"blue\"} trailing").is_err());
    }
}
