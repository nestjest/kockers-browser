use std::{
    fs,
    io::{Read, Write},
    iter, mem,
    path::{Path, PathBuf},
    ptr,
    sync::{Mutex, MutexGuard, OnceLock},
    thread,
    time::Duration,
};
use url::Url;
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError},
    Networking::WinHttp::*,
    Security::Cryptography::*,
    System::Threading::{CREATE_NO_WINDOW, CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW},
};

const REPOSITORY: &str = "nestjest/kockers-browser";
const MAX_SUMS_BYTES: usize = 64 * 1024;
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;
const CURRENT_VERSION: &str = crate::APP_VERSION;
static STATUS: OnceLock<Mutex<Status>> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Checking,
    Downloading {
        version: String,
        received: u64,
        total: Option<u64>,
    },
    Verifying {
        version: String,
    },
    Installing {
        version: String,
    },
    UpToDate,
    Error(String),
}

impl Status {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Checking
                | Self::Downloading { .. }
                | Self::Verifying { .. }
                | Self::Installing { .. }
        )
    }
}

pub fn snapshot() -> Status {
    status().clone()
}

pub fn start() {
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(12));
        start_check(true);
    });
}

pub fn check_now() {
    start_check(false);
}

fn start_check(only_if_idle: bool) {
    {
        let mut current = status();
        if current.is_active() || (only_if_idle && *current != Status::Idle) {
            return;
        }
        *current = Status::Checking;
    }
    write_status("Проверка обновлений");
    thread::spawn(|| {
        if let Err(error) = check_for_update() {
            set_status(Status::Error(error.clone()));
            write_status(&format!("Обновление не установлено: {error}"));
        }
    });
}

fn status() -> MutexGuard<'static, Status> {
    STATUS
        .get_or_init(|| Mutex::new(Status::Idle))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn set_status(value: Status) {
    *status() = value;
}

fn check_for_update() -> Result<(), String> {
    let releases_url = format!("https://github.com/{REPOSITORY}/releases.atom");
    let feed = download_text(&releases_url, 512 * 1024)?;
    let tag = latest_release_tag(&feed)?;
    let version = tag.strip_prefix('v').unwrap_or(&tag);
    let candidate = Version::parse(version)?;
    if candidate <= Version::parse(CURRENT_VERSION)? {
        set_status(Status::UpToDate);
        write_status(&format!(
            "Актуальная версия {CURRENT_VERSION}; новых обновлений нет"
        ));
        return Ok(());
    }

    let asset = format!("KockersBrowser-{version}-windows-x64-setup.exe");
    let base = format!("https://github.com/{REPOSITORY}/releases/download/{tag}");
    let sums = download_text(&format!("{base}/SHA256SUMS.txt"), MAX_SUMS_BYTES)?;
    let expected = expected_hash(&sums, &asset)?;
    let directory = update_directory()?;
    let installer = directory.join(&asset);
    let partial = directory.join(format!("{asset}.download"));
    download_file(
        &format!("{base}/{asset}"),
        &partial,
        MAX_INSTALLER_BYTES,
        version,
    )?;
    set_status(Status::Verifying {
        version: version.to_owned(),
    });
    let actual = sha256_file(&partial)?;
    if !actual.eq_ignore_ascii_case(expected) {
        let _ = fs::remove_file(&partial);
        return Err("SHA-256 установщика не совпал с релизом".to_owned());
    }
    replace_file(&partial, &installer)?;
    set_status(Status::Installing {
        version: version.to_owned(),
    });
    write_status(&format!("Запуск проверенного обновления {version}"));
    launch_installer(&installer)?;
    Ok(())
}

fn latest_release_tag(feed: &str) -> Result<String, String> {
    let prefix = format!("https://github.com/{REPOSITORY}/releases/tag/");
    let mut latest: Option<(Version, String)> = None;
    let mut remainder = feed;
    while let Some(start) = remainder.find("href=\"") {
        remainder = &remainder[start + 6..];
        let Some(end) = remainder.find('"') else {
            return Err("Некорректная ссылка в списке релизов".to_owned());
        };
        let link = &remainder[..end];
        remainder = &remainder[end + 1..];
        let Some(tag) = link.strip_prefix(&prefix) else {
            continue;
        };
        if tag.is_empty()
            || tag.contains('/')
            || !tag.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
            })
        {
            continue;
        }
        let Ok(version) = Version::parse(tag.strip_prefix('v').unwrap_or(tag)) else {
            continue;
        };
        if latest
            .as_ref()
            .is_none_or(|(current, _)| version > *current)
        {
            latest = Some((version, tag.to_owned()));
        }
    }
    latest
        .map(|(_, tag)| tag)
        .ok_or_else(|| "GitHub не вернул ни одного корректного релиза".to_owned())
}

fn update_directory() -> Result<PathBuf, String> {
    let directory = crate::storage::data_dir()
        .ok_or("LOCALAPPDATA недоступен")?
        .join("Updates");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Не удалось создать каталог обновлений: {error}"))?;
    Ok(directory)
}

fn write_status(message: &str) {
    if let Some(directory) = crate::storage::data_dir() {
        let _ = fs::create_dir_all(&directory);
        let _ = fs::write(directory.join("update-status.txt"), format!("{message}\n"));
    }
}

fn download_text(url: &str, limit: usize) -> Result<String, String> {
    let response = HttpResponse::open(url)?;
    let mut bytes = Vec::new();
    response.read(limit as u64, |chunk| {
        bytes.extend_from_slice(chunk);
        Ok(())
    })?;
    String::from_utf8(bytes).map_err(|_| "SHA256SUMS.txt содержит не UTF-8 данные".to_owned())
}

fn download_file(url: &str, path: &Path, limit: u64, version: &str) -> Result<(), String> {
    let response = HttpResponse::open(url)?;
    let total = response.content_length;
    if total.is_some_and(|total| total > limit) {
        return Err("Размер загрузки превышает разрешённый предел".to_owned());
    }
    set_status(Status::Downloading {
        version: version.to_owned(),
        received: 0,
        total,
    });
    let mut file = fs::File::create(path)
        .map_err(|error| format!("Не удалось создать файл обновления: {error}"))?;
    let mut received = 0u64;
    let result = response.read(limit, |chunk| {
        file.write_all(chunk)
            .map_err(|error| format!("Не удалось записать обновление: {error}"))?;
        received += chunk.len() as u64;
        set_status(Status::Downloading {
            version: version.to_owned(),
            received,
            total,
        });
        Ok(())
    });
    if let Err(error) = result {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    if total.is_some_and(|total| total != received) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err("Загрузка оборвалась до получения всего установщика".to_owned());
    }
    file.sync_all()
        .map_err(|error| format!("Не удалось сохранить обновление на диск: {error}"))
}

fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    let source_wide = wide(&source.to_string_lossy());
    let target_wide = wide(&target.to_string_lossy());
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            source_wide.as_ptr(),
            target_wide.as_ptr(),
            windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(format!(
            "Не удалось завершить файл обновления: {}",
            windows_error()
        ))
    } else {
        Ok(())
    }
}

fn launch_installer(path: &Path) -> Result<(), String> {
    let application = wide(&path.to_string_lossy());
    let mut command = wide(&format!(
        "\"{}\" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /UPDATE",
        path.to_string_lossy()
    ));
    let directory = wide(&path.parent().unwrap_or(Path::new("")).to_string_lossy());
    let startup = STARTUPINFOW {
        cb: mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_NO_WINDOW,
            ptr::null(),
            directory.as_ptr(),
            &startup,
            &mut process,
        )
    };
    if created == 0 {
        return Err(format!(
            "Не удалось запустить установщик: {}",
            windows_error()
        ));
    }
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
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
    fn open(url: &str) -> Result<Self, String> {
        let url = Url::parse(url).map_err(|_| "Некорректный URL обновления")?;
        if url.scheme() != "https" || url.host_str() != Some("github.com") {
            return Err("Для обновления разрешён только HTTPS github.com".to_owned());
        }
        let host = url.host_str().ok_or("Отсутствует хост обновления")?;
        let port = url
            .port_or_known_default()
            .ok_or("Отсутствует порт HTTPS")?;
        let object = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        };
        unsafe {
            let agent = wide(&format!("KockersBrowser/{CURRENT_VERSION}"));
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
            WinHttpSetTimeouts(session.0, 10_000, 15_000, 30_000, 60_000);
            let host_wide = wide(host);
            let connection = HttpHandle(WinHttpConnect(session.0, host_wide.as_ptr(), port, 0));
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
                WINHTTP_FLAG_SECURE,
            ));
            if request.0.is_null() {
                return Err(format!("WinHttpOpenRequest: {}", windows_error()));
            }
            let headers = wide("Accept: */*\r\n");
            if WinHttpSendRequest(request.0, headers.as_ptr(), u32::MAX, ptr::null(), 0, 0, 0) == 0
            {
                return Err(format!("WinHttpSendRequest: {}", windows_error()));
            }
            if WinHttpReceiveResponse(request.0, ptr::null_mut()) == 0 {
                return Err(format!("WinHttpReceiveResponse: {}", windows_error()));
            }
            let mut status = 0u32;
            let mut status_size = mem::size_of::<u32>() as u32;
            if WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                ptr::null(),
                &mut status as *mut u32 as *mut core::ffi::c_void,
                &mut status_size,
                ptr::null_mut(),
            ) == 0
            {
                return Err(format!(
                    "Не удалось прочитать HTTP-статус: {}",
                    windows_error()
                ));
            }
            if status != 200 {
                return Err(format!("GitHub вернул HTTP {status}"));
            }
            let mut content_length = 0u32;
            let mut content_length_size = mem::size_of::<u32>() as u32;
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

    fn read(
        &self,
        limit: u64,
        mut sink: impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut total = 0u64;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            let result = unsafe {
                WinHttpReadData(
                    self.request.0,
                    buffer.as_mut_ptr() as *mut core::ffi::c_void,
                    buffer.len() as u32,
                    &mut read,
                )
            };
            if result == 0 {
                return Err(format!("Ошибка чтения обновления: {}", windows_error()));
            }
            if read == 0 {
                break;
            }
            total += read as u64;
            if total > limit {
                return Err("Размер загрузки превышает разрешённый предел".to_owned());
            }
            sink(&buffer[..read as usize])?;
        }
        Ok(())
    }
}

fn expected_hash<'a>(sums: &'a str, asset: &str) -> Result<&'a str, String> {
    for line in sums.lines() {
        let mut fields = line.split_whitespace();
        let hash = fields.next().unwrap_or_default();
        let name = fields
            .next()
            .map(|name| name.trim_start_matches('*'))
            .unwrap_or_default();
        if name == asset {
            if hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Ok(hash);
            }
            return Err("Хеш установщика имеет неверный формат".to_owned());
        }
    }
    Err("SHA256SUMS.txt не содержит установщик текущей версии".to_owned())
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

fn sha256_file(path: &Path) -> Result<String, String> {
    unsafe {
        let mut algorithm = AlgorithmHandle(ptr::null_mut());
        let algorithm_name = wide("SHA256");
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
        let mut file = fs::File::open(path)
            .map_err(|error| format!("Не удалось открыть обновление для проверки: {error}"))?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("Ошибка чтения SHA-256: {error}"))?;
            if read == 0 {
                break;
            }
            let status = BCryptHashData(hash.0, buffer.as_ptr(), read as u32, 0);
            if status < 0 {
                return Err(format!("BCryptHashData: {status:#x}"));
            }
        }
        let mut output = vec![0u8; hash_length];
        let status = BCryptFinishHash(hash.0, output.as_mut_ptr(), output.len() as u32, 0);
        if status < 0 {
            return Err(format!("BCryptFinishHash: {status:#x}"));
        }
        Ok(output.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}

fn bcrypt_property(algorithm: &AlgorithmHandle, name: &str) -> Result<u32, String> {
    unsafe {
        let property = wide(name);
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    revision: u64,
    prerelease: Option<String>,
}

impl Version {
    fn parse(value: &str) -> Result<Self, String> {
        let (numeric, prerelease) = value
            .split_once('-')
            .map(|(numeric, prerelease)| (numeric, Some(prerelease)))
            .unwrap_or((value, None));
        let numbers = numeric.split('.').collect::<Vec<_>>();
        if !matches!(numbers.len(), 3 | 4)
            || numbers
                .iter()
                .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
            || prerelease.is_some_and(|part| part.is_empty())
        {
            return Err(format!("Некорректная версия: {value}"));
        }
        Ok(Self {
            major: numbers[0]
                .parse()
                .map_err(|_| "Слишком большая major-версия")?,
            minor: numbers[1]
                .parse()
                .map_err(|_| "Слишком большая minor-версия")?,
            patch: numbers[2]
                .parse()
                .map_err(|_| "Слишком большая patch-версия")?,
            revision: numbers
                .get(3)
                .map(|part| part.parse())
                .transpose()
                .map_err(|_| "Слишком большая hotfix-версия")?
                .unwrap_or(0),
            prerelease: prerelease.map(str::to_owned),
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(std::cmp::Ord::cmp(self, other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cmp_numeric(other)
    }
}

impl Version {
    fn cmp_numeric(&self, other: &Self) -> std::cmp::Ordering {
        (self.major, self.minor, self.patch, self.revision)
            .cmp(&(other.major, other.minor, other.patch, other.revision))
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(left), Some(right)) => compare_prerelease(left, right),
                _ => std::cmp::Ordering::Equal,
            })
    }
}

fn compare_prerelease(left: &str, right: &str) -> std::cmp::Ordering {
    let left = left.split('.').collect::<Vec<_>>();
    let right = right.split('.').collect::<Vec<_>>();
    for (left, right) in left.iter().zip(&right) {
        let left_numeric = left.bytes().all(|byte| byte.is_ascii_digit());
        let right_numeric = right.bytes().all(|byte| byte.is_ascii_digit());
        let ordering = match (left_numeric, right_numeric) {
            (true, true) => left
                .parse::<u64>()
                .unwrap_or(u64::MAX)
                .cmp(&right.parse::<u64>().unwrap_or(u64::MAX)),
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (false, false) => left.cmp(right),
        };
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}

fn windows_error() -> u32 {
    unsafe { GetLastError() }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_stable_and_prerelease_versions() {
        let current = Version::parse("0.1.0-alpha.1").unwrap();
        let stable = Version::parse("0.1.0").unwrap();
        let next = Version::parse("0.2.0").unwrap();
        assert!(stable > current);
        assert!(next > stable);
        assert!(
            Version::parse("0.2.0-alpha.10").unwrap() > Version::parse("0.2.0-alpha.2").unwrap()
        );
        assert!(Version::parse("0.1.0").unwrap() > Version::parse("0.1.0-rc.1").unwrap());
        assert!(Version::parse("0.2.2.1").unwrap() > Version::parse("0.2.2-alpha").unwrap());
        assert!(Version::parse("0.2.2.2").unwrap() > Version::parse("0.2.2.1").unwrap());
    }

    #[test]
    fn rejects_invalid_versions() {
        assert!(Version::parse("1.0").is_err());
        assert!(Version::parse("1.0.0-").is_err());
        assert!(Version::parse("1.0.x").is_err());
        assert!(Version::parse("1.0.0.0.0").is_err());
    }

    #[test]
    fn release_feed_includes_prereleases() {
        let feed = r#"<feed><entry><link href="https://github.com/nestjest/kockers-browser/releases/tag/v0.2.2-alpha" /></entry><entry><link href="https://github.com/nestjest/kockers-browser/releases/tag/v0.2.1-alpha" /></entry></feed>"#;
        assert_eq!(latest_release_tag(feed).as_deref(), Ok("v0.2.2-alpha"));
        assert!(latest_release_tag("<feed></feed>").is_err());
    }

    #[test]
    fn finds_only_the_exact_asset_hash() {
        let sums = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  KockersBrowser-0.2.0-windows-x64-setup.exe\n";
        assert!(expected_hash(sums, "KockersBrowser-0.2.0-windows-x64-setup.exe").is_ok());
        assert!(expected_hash(sums, "KockersBrowser-0.3.0-windows-x64-setup.exe").is_err());
        assert!(expected_hash("not-a-hash  file.exe", "file.exe").is_err());
    }

    #[test]
    fn hashes_files_with_windows_cng() {
        let path = PathBuf::from("target").join(format!("sha256-test-{}", std::process::id()));
        fs::write(&path, []).unwrap();
        let result = sha256_file(&path);
        let _ = fs::remove_file(path);
        assert_eq!(
            result.as_deref(),
            Ok("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
    }

    #[test]
    #[ignore = "downloads the 0.2.3 installer from GitHub"]
    fn downloads_github_asset_and_reports_progress() {
        let version = "0.2.3";
        let asset = format!("KockersBrowser-{version}-windows-x64-setup.exe");
        let base = format!("https://github.com/{REPOSITORY}/releases/download/v{version}");
        let path = PathBuf::from("target").join(format!("update-test-{}.exe", std::process::id()));
        let result = (|| {
            download_file(
                &format!("{base}/{asset}"),
                &path,
                MAX_INSTALLER_BYTES,
                version,
            )?;
            let Status::Downloading {
                received, total, ..
            } = snapshot()
            else {
                return Err("После загрузки не сохранился её прогресс".to_owned());
            };
            assert_eq!(Some(received), total);
            assert!(received > 0);
            let sums = download_text(&format!("{base}/SHA256SUMS.txt"), MAX_SUMS_BYTES)?;
            let expected = expected_hash(&sums, &asset)?;
            assert!(sha256_file(&path)?.eq_ignore_ascii_case(expected));
            Ok(())
        })();
        let _ = fs::remove_file(path);
        result.unwrap();
    }
}
