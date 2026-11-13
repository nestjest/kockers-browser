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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preferences {
    pub save_history: bool,
    pub ask_download_path: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            save_history: true,
            ask_download_path: false,
        }
    }
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
        match line {
            "history=0" => preferences.save_history = false,
            "history=1" => preferences.save_history = true,
            "ask_download=0" => preferences.ask_download_path = false,
            "ask_download=1" => preferences.ask_download_path = true,
            _ => {}
        }
    }
    preferences
}

pub fn save_preferences(preferences: Preferences) {
    if let Some(path) = data_dir().map(|path| path.join("preferences.txt")) {
        let text = format!(
            "history={}\nask_download={}\n",
            u8::from(preferences.save_history),
            u8::from(preferences.ask_download_path)
        );
        write_atomic(&path, text.as_bytes());
    }
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

fn load_history() -> Vec<HistoryEntry> {
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
}
