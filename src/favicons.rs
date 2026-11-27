use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    io::{self, Write},
    iter,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};
use url::Url;
use windows_sys::Win32::{
    Networking::WinHttp::*,
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
};

const MAX_ICON_SIZE: u64 = 1024 * 1024;
const MAX_CANDIDATES: usize = 8;
const MAX_URL_LENGTH: usize = 8192;
const MAX_CACHED_ENTRIES: usize = 512;
const HTTP_TIMEOUT_MS: i32 = 5_000;

#[derive(Clone)]
pub struct FaviconCache {
    directory: Option<PathBuf>,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    pending: HashSet<String>,
    cached: HashMap<String, Option<PathBuf>>,
}

impl State {
    fn insert_cached(&mut self, key: String, value: Option<PathBuf>) {
        if self.cached.len() >= MAX_CACHED_ENTRIES && !self.cached.contains_key(&key) {
            self.cached.clear();
        }
        self.cached.insert(key, value);
    }
}

impl FaviconCache {
    pub fn new() -> Self {
        Self {
            directory: crate::storage::data_dir().map(|path| path.join("Favicons")),
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    /// Запускает загрузку или присоединяется к уже активной. Возвращает false
    /// только для недопустимого URL, недоступного хранилища или ошибки запуска потока.
    pub fn request(&self, page_url: &str, candidate_urls: &[String]) -> bool {
        let Some((page_url, candidates)) = prepare_request(page_url, candidate_urls) else {
            return false;
        };
        let Some(directory) = self.directory.as_ref() else {
            return false;
        };
        let key = page_url.to_string();
        let path = cache_path(directory, &key);
        if is_cached_file(&path) && cached_source_matches(&path, &candidates) {
            lock_state(&self.state).insert_cached(key, Some(path));
            return true;
        }

        {
            let mut state = lock_state(&self.state);
            if !state.pending.insert(key.clone()) {
                return true;
            }
        }

        let state = Arc::clone(&self.state);
        let worker_key = key.clone();
        let spawned = thread::Builder::new()
            .name("favicon-cache".to_owned())
            .spawn(move || {
                let mut updated = false;
                if let Some((source, bytes)) = download_first(&candidates)
                    && write_atomic(&path, &bytes).is_ok()
                {
                    let _ = write_atomic(&source_path(&path), source.as_bytes());
                    updated = true;
                }
                {
                    let mut state = lock_state(&state);
                    if updated {
                        state.insert_cached(worker_key.clone(), Some(path));
                    }
                    state.pending.remove(&worker_key);
                }
                if updated {
                    crate::platform::invalidate_image_cache();
                }
            });
        if spawned.is_err() {
            lock_state(&self.state).pending.remove(&key);
            return false;
        }
        true
    }

    pub fn cached(&self, page_url: &str) -> Option<PathBuf> {
        let page_url = normalize_page_url(page_url)?;
        let key = page_url.as_str();
        if let Some(entry) = lock_state(&self.state).cached.get(key) {
            return entry.clone();
        }
        let path = cache_path(self.directory.as_ref()?, key);
        let result = is_cached_file(&path).then_some(path);
        lock_state(&self.state).insert_cached(key.to_owned(), result.clone());
        result
    }

    pub fn is_pending(&self, page_url: &str) -> bool {
        let Some(page_url) = normalize_page_url(page_url) else {
            return false;
        };
        lock_state(&self.state).pending.contains(page_url.as_str())
    }
}

impl Default for FaviconCache {
    fn default() -> Self {
        Self::new()
    }
}

fn lock_state(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|error| error.into_inner())
}

fn prepare_request(page_url: &str, candidate_urls: &[String]) -> Option<(Url, Vec<Url>)> {
    let page_url = normalize_page_url(page_url)?;
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for candidate in candidate_urls {
        let candidate = candidate.trim();
        if candidate.is_empty() || candidate.len() > MAX_URL_LENGTH {
            continue;
        }
        let Ok(mut url) = page_url.join(candidate) else {
            continue;
        };
        if !is_http_url(&url) {
            continue;
        }
        url.set_fragment(None);
        if seen.insert(url.to_string()) {
            candidates.push(url);
            if candidates.len() == MAX_CANDIDATES - 1 {
                break;
            }
        }
    }

    let fallback = page_url.join("/favicon.ico").ok()?;
    if seen.insert(fallback.to_string()) {
        candidates.push(fallback);
    }
    Some((page_url, candidates))
}

fn normalize_page_url(value: &str) -> Option<Url> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_URL_LENGTH {
        return None;
    }
    let mut url = Url::parse(value).ok()?;
    if !is_http_url(&url) {
        return None;
    }
    url.set_fragment(None);
    Some(url)
}

fn is_http_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}

fn cached_source_matches(path: &Path, candidates: &[Url]) -> bool {
    let Ok(source) = fs::read_to_string(source_path(path)) else {
        return false;
    };
    candidates.iter().any(|url| url.as_str() == source)
}

fn is_cached_file(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && (1..=MAX_ICON_SIZE).contains(&metadata.len()))
}

fn source_path(path: &Path) -> PathBuf {
    path.with_extension("source")
}

fn cache_path(directory: &Path, page_url: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    page_url.hash(&mut hasher);
    let name = format!("{:016x}-{}.icon", hasher.finish(), sanitize_url(page_url));
    directory.join(name)
}

fn sanitize_url(url: &str) -> String {
    let mut result = String::with_capacity(48);
    for byte in url.bytes() {
        if byte.is_ascii_alphanumeric() {
            result.push(char::from(byte.to_ascii_lowercase()));
        } else if !result.is_empty() && !result.ends_with('-') {
            result.push('-');
        }
        if result.len() == 48 {
            break;
        }
    }
    let result = result.trim_matches('-');
    if result.is_empty() {
        "page".to_owned()
    } else {
        result.to_owned()
    }
}

fn download_first(candidates: &[Url]) -> Option<(String, Vec<u8>)> {
    for url in candidates {
        if let Ok(bytes) = download(url)
            && is_supported_image(&bytes)
        {
            return Some((url.to_string(), bytes));
        }
    }
    None
}

fn download(url: &Url) -> Result<Vec<u8>, String> {
    let response = HttpResponse::open(url)?;
    if response
        .content_length
        .is_some_and(|length| length > MAX_ICON_SIZE)
    {
        return Err("Favicon превышает ограничение размера".to_owned());
    }
    response.read()
}

struct HttpHandle(*mut core::ffi::c_void);

impl Drop for HttpHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

struct HttpResponse {
    request: HttpHandle,
    _connection: HttpHandle,
    _session: HttpHandle,
    content_length: Option<u64>,
}

impl HttpResponse {
    fn open(url: &Url) -> Result<Self, String> {
        let host = url.host_str().ok_or("Отсутствует хост favicon")?;
        let port = url
            .port_or_known_default()
            .ok_or("Отсутствует порт favicon")?;
        let object = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        };
        let secure = url.scheme() == "https";
        unsafe {
            let agent = wide("KockersBrowser");
            let session = HttpHandle(WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                ptr::null(),
                ptr::null(),
                0,
            ));
            if session.0.is_null() {
                return Err(format!("WinHttpOpen: {}", windows_error()));
            }
            WinHttpSetTimeouts(
                session.0,
                HTTP_TIMEOUT_MS,
                HTTP_TIMEOUT_MS,
                HTTP_TIMEOUT_MS,
                HTTP_TIMEOUT_MS,
            );
            let host = wide(host);
            let connection = HttpHandle(WinHttpConnect(session.0, host.as_ptr(), port, 0));
            if connection.0.is_null() {
                return Err(format!("WinHttpConnect: {}", windows_error()));
            }
            let verb = wide("GET");
            let object = wide(&object);
            let request = HttpHandle(WinHttpOpenRequest(
                connection.0,
                verb.as_ptr(),
                object.as_ptr(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                if secure { WINHTTP_FLAG_SECURE } else { 0 },
            ));
            if request.0.is_null() {
                return Err(format!("WinHttpOpenRequest: {}", windows_error()));
            }
            let headers = wide(
                "Accept: image/png,image/x-icon,image/vnd.microsoft.icon,image/gif,image/jpeg,image/webp,*/*;q=0.5\r\n",
            );
            if WinHttpSendRequest(request.0, headers.as_ptr(), u32::MAX, ptr::null(), 0, 0, 0) == 0
            {
                return Err(format!("WinHttpSendRequest: {}", windows_error()));
            }
            if WinHttpReceiveResponse(request.0, ptr::null_mut()) == 0 {
                return Err(format!("WinHttpReceiveResponse: {}", windows_error()));
            }
            let mut status = 0u32;
            let mut status_size = size_of::<u32>() as u32;
            if WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                ptr::null(),
                &mut status as *mut u32 as *mut core::ffi::c_void,
                &mut status_size,
                ptr::null_mut(),
            ) == 0
            {
                return Err(format!("HTTP-статус favicon: {}", windows_error()));
            }
            if status != 200 {
                return Err(format!("Favicon вернул HTTP {status}"));
            }
            let mut content_length = 0u32;
            let mut content_length_size = size_of::<u32>() as u32;
            let content_length = (WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER,
                ptr::null(),
                &mut content_length as *mut u32 as *mut core::ffi::c_void,
                &mut content_length_size,
                ptr::null_mut(),
            ) != 0)
                .then_some(u64::from(content_length));
            Ok(Self {
                request,
                _connection: connection,
                _session: session,
                content_length,
            })
        }
    }

    fn read(&self) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            if unsafe {
                WinHttpReadData(
                    self.request.0,
                    buffer.as_mut_ptr() as *mut core::ffi::c_void,
                    buffer.len() as u32,
                    &mut read,
                )
            } == 0
            {
                return Err(format!("Ошибка чтения favicon: {}", windows_error()));
            }
            if read == 0 {
                break;
            }
            if bytes.len() as u64 + u64::from(read) > MAX_ICON_SIZE {
                return Err("Favicon превышает ограничение размера".to_owned());
            }
            bytes.extend_from_slice(&buffer[..read as usize]);
        }
        if bytes.is_empty() {
            return Err("Пустой favicon".to_owned());
        }
        Ok(bytes)
    }
}

fn is_supported_image(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x00\x00\x01\x00")
        || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || bytes.starts_with(b"\xff\xd8\xff")
        || bytes.starts_with(b"BM")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("У пути favicon отсутствует каталог"))?;
    fs::create_dir_all(parent)?;
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
    let temporary = parent.join(format!(
        ".{}.{}-{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        let source = wide_path(&temporary);
        let target = wide_path(path);
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect()
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(iter::once(0)).collect()
}

fn windows_error() -> io::Error {
    io::Error::last_os_error()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_are_relative_deduplicated_and_http_only() {
        let (_, candidates) = prepare_request(
            "https://example.com/articles/one#fragment",
            &[
                "../icon.png#large".to_owned(),
                "https://example.com/icon.png".to_owned(),
                "data:image/png;base64,AAAA".to_owned(),
                "file:///C:/icon.ico".to_owned(),
            ],
        )
        .unwrap();
        assert_eq!(
            candidates.iter().map(Url::as_str).collect::<Vec<_>>(),
            [
                "https://example.com/icon.png",
                "https://example.com/favicon.ico"
            ]
        );
    }

    #[test]
    fn explicit_fallback_is_not_downloaded_twice() {
        let (_, candidates) = prepare_request(
            "http://example.com:8080/page",
            &["http://example.com:8080/favicon.ico".to_owned()],
        )
        .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].as_str(),
            "http://example.com:8080/favicon.ico"
        );
    }

    #[test]
    fn fallback_is_reserved_when_candidate_limit_is_reached() {
        let provided = (0..20)
            .map(|index| format!("https://cdn.example.com/{index}.png"))
            .collect::<Vec<_>>();
        let (_, candidates) = prepare_request("https://example.com/page", &provided).unwrap();
        assert_eq!(candidates.len(), MAX_CANDIDATES);
        assert_eq!(
            candidates.last().unwrap().as_str(),
            "https://example.com/favicon.ico"
        );
    }

    #[test]
    fn invalid_pages_and_credentials_are_rejected() {
        assert!(prepare_request("file:///C:/page.html", &[]).is_none());
        assert!(prepare_request("https://user:secret@example.com", &[]).is_none());
        assert!(prepare_request("not a URL", &[]).is_none());
    }

    #[test]
    fn cache_names_are_stable_safe_and_distinct() {
        let directory = Path::new("cache");
        let first = cache_path(directory, "https://example.com/a?x=1");
        let same = cache_path(directory, "https://example.com/a?x=1");
        let second = cache_path(directory, "https://example.com/a?x=2");
        assert_eq!(first, same);
        assert_ne!(first, second);
        let name = first.file_name().unwrap().to_str().unwrap();
        assert!(name.len() <= 71);
        assert!(
            name.bytes()
                .all(|byte| { byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_') })
        );
        assert_eq!(first.parent(), Some(directory));
    }

    #[test]
    fn image_validation_rejects_html_and_truncated_data() {
        assert!(is_supported_image(b"\x00\x00\x01\x00rest"));
        assert!(is_supported_image(b"\x89PNG\r\n\x1a\nrest"));
        assert!(is_supported_image(b"RIFFsizeWEBPrest"));
        assert!(!is_supported_image(b"<html>not an icon</html>"));
        assert!(!is_supported_image(b"RIFFsizeWAVErest"));
        assert!(!is_supported_image(&[]));
    }
}
