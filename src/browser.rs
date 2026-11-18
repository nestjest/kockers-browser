use crate::{
    navigation, platform, storage,
    ui::{self, pages},
};
use cef::*;
use std::{
    collections::VecDeque,
    path::PathBuf,
    ptr,
    sync::{Arc, Mutex, MutexGuard},
};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_F5, VK_F11, VK_F12, VK_LEFT, VK_RIGHT, VK_TAB},
};

pub type SharedController = Arc<Mutex<BrowserController>>;

#[derive(Clone, PartialEq, Eq)]
pub struct TabSnapshot {
    pub id: u64,
    pub title: String,
    pub pinned: bool,
    pub active: bool,
    pub loading: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    InProgress,
    Paused,
    Complete,
    Canceled,
    Interrupted,
}

#[derive(Clone)]
pub struct DownloadSnapshot {
    pub id: u32,
    pub name: String,
    pub path: PathBuf,
    pub received: i64,
    pub total: i64,
    pub percent: i32,
    pub status: DownloadStatus,
}

struct Tab {
    id: u64,
    browser: Option<Browser>,
    title: String,
    url: String,
    pinned: bool,
    closing: bool,
    is_loading: bool,
    can_go_back: bool,
    can_go_forward: bool,
    zoom_percent: i32,
    ready: bool,
}

struct ClosedTab {
    url: String,
    title: String,
    pinned: bool,
}

struct Download {
    snapshot: DownloadSnapshot,
    callback: Option<DownloadItemCallback>,
}

pub struct BrowserController {
    window: HWND,
    tabs: Vec<Tab>,
    active: Option<u64>,
    pending: VecDeque<u64>,
    closed: Vec<ClosedTab>,
    downloads: Vec<Download>,
    next_tab_id: u64,
    private: bool,
    request_context: Option<RequestContext>,
    preferences: storage::Preferences,
    bookmarks: Vec<storage::Bookmark>,
    closing_window: bool,
}

impl Default for BrowserController {
    fn default() -> Self {
        Self {
            window: ptr::null_mut(),
            tabs: Vec::new(),
            active: None,
            pending: VecDeque::new(),
            closed: Vec::new(),
            downloads: Vec::new(),
            next_tab_id: 1,
            private: false,
            request_context: None,
            preferences: storage::load_preferences(),
            bookmarks: storage::load_bookmarks(),
            closing_window: false,
        }
    }
}

#[allow(clippy::arc_with_non_send_sync)]
impl BrowserController {
    pub fn new(private: bool) -> Self {
        let request_context = private
            .then(|| {
                let settings = RequestContextSettings {
                    cache_path: CefString::default(),
                    persist_session_cookies: 0,
                    ..Default::default()
                };
                request_context_create_context(Some(&settings), None)
            })
            .flatten();
        Self {
            private,
            request_context,
            ..Default::default()
        }
    }

    pub fn set_window(&mut self, window: HWND) {
        self.window = window;
    }

    pub fn window(&self) -> HWND {
        self.window
    }

    pub fn is_private(&self) -> bool {
        self.private
    }

    pub fn tabs_snapshot(&self) -> Vec<TabSnapshot> {
        self.tabs
            .iter()
            .map(|tab| TabSnapshot {
                id: tab.id,
                title: tab.title.clone(),
                pinned: tab.pinned,
                active: self.active == Some(tab.id),
                loading: tab.is_loading,
            })
            .collect()
    }

    pub fn active_state(&self) -> (String, bool, bool, bool) {
        self.active_tab()
            .map(|tab| {
                (
                    tab.url.clone(),
                    tab.is_loading,
                    tab.can_go_back,
                    tab.can_go_forward,
                )
            })
            .unwrap_or_default()
    }

    pub fn tab_id_at(&self, index: usize) -> Option<u64> {
        self.tabs.get(index).map(|tab| tab.id)
    }

    pub fn last_tab_id(&self) -> Option<u64> {
        self.tabs.last().map(|tab| tab.id)
    }

    pub fn downloads_snapshot(&self) -> Vec<DownloadSnapshot> {
        self.downloads
            .iter()
            .rev()
            .map(|download| download.snapshot.clone())
            .collect()
    }

    pub fn preferences(&self) -> storage::Preferences {
        self.preferences.clone()
    }

    pub fn bookmarks_snapshot(&self) -> Vec<storage::Bookmark> {
        self.bookmarks.clone()
    }

    pub fn active_is_bookmarked(&self) -> bool {
        self.active_tab().is_some_and(|tab| {
            self.bookmarks
                .iter()
                .any(|bookmark| bookmark.url == tab.url)
        })
    }

    pub fn active_url(&self) -> String {
        self.active_tab()
            .map(|tab| tab.url.clone())
            .unwrap_or_default()
    }

    pub fn resize_content(&self) {
        let browsers = self
            .tabs
            .iter()
            .filter_map(|tab| {
                tab.browser.as_ref().and_then(Browser::host).map(|host| {
                    let handle = host.window_handle().0.cast::<std::ffi::c_void>();
                    (tab.id, handle)
                })
            })
            .collect::<Vec<_>>();
        unsafe { platform::resize_browsers(self.window, &browsers, self.active) };
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn navigate_input(controller: &SharedController, input: &str) {
        let engine = lock_controller(controller).preferences.search_engine;
        let target = match navigation::normalize_input_with(input, engine) {
            Ok(Some(url)) => url,
            Ok(None) => pages::start_page_url(),
            Err(error) => pages::error_page_url(input, &error.to_string(), None),
        };
        Self::load_url(controller, &target);
    }

    pub fn new_tab(controller: &SharedController, url: &str, background: bool) {
        Self::insert_tab(controller, url, "Новая вкладка", false, background, None);
    }

    pub fn restore_closed_tab(controller: &SharedController) {
        let closed = lock_controller(controller).closed.pop();
        if let Some(closed) = closed {
            Self::insert_tab(
                controller,
                &closed.url,
                &closed.title,
                closed.pinned,
                false,
                None,
            );
        }
    }

    fn insert_tab(
        controller: &SharedController,
        url: &str,
        title: &str,
        pinned: bool,
        background: bool,
        position: Option<usize>,
    ) {
        let (window, context, create) = {
            let mut state = lock_controller(controller);
            if state.closing_window {
                return;
            }
            let id = state.next_tab_id;
            state.next_tab_id += 1;
            let pinned_count = state.tabs.iter().filter(|tab| tab.pinned).count();
            let index = if pinned {
                pinned_count
            } else {
                position
                    .unwrap_or_else(|| {
                        state
                            .active_index()
                            .map(|index| index + 1)
                            .unwrap_or(state.tabs.len())
                    })
                    .max(pinned_count)
                    .min(state.tabs.len())
            };
            state.tabs.insert(
                index,
                Tab {
                    id,
                    browser: None,
                    title: title.to_owned(),
                    url: url.to_owned(),
                    pinned,
                    closing: false,
                    is_loading: false,
                    can_go_back: false,
                    can_go_forward: false,
                    zoom_percent: 100,
                    ready: false,
                },
            );
            if !background {
                state.active = Some(id);
            }
            let create = should_create_browser(background, state.preferences.lazy_restore);
            if create {
                state.pending.push_back(id);
            }
            state.persist_session();
            (state.window, state.request_context.clone(), create)
        };
        platform::refresh_tabs(window);
        platform::sync_active_tab(window);
        if create {
            create_browser(window, controller.clone(), url, context);
        }
    }

    pub fn activate_tab(controller: &SharedController, id: u64) {
        let create = {
            let mut state = lock_controller(controller);
            if state.closing_window || state.tab(id).is_none() {
                return;
            }
            state.active = Some(id);
            let create = if state.tab(id).is_some_and(|tab| tab.browser.is_none()) {
                let url = state.tab(id).map(|tab| tab.url.clone()).unwrap_or_default();
                state.pending.push_back(id);
                Some((url, state.request_context.clone()))
            } else {
                None
            };
            state.persist_session();
            create
        };
        let window = lock_controller(controller).window;
        apply_visibility(controller, window);
        platform::refresh_tabs(window);
        platform::sync_active_tab(window);
        if let Some((url, context)) = create {
            create_browser(window, controller.clone(), &url, context);
        }
    }

    pub fn next_tab(controller: &SharedController, reverse: bool) {
        let id = {
            let state = lock_controller(controller);
            let index = state.active_index();
            index.map(|index| {
                let next = if reverse {
                    (index + state.tabs.len() - 1) % state.tabs.len()
                } else {
                    (index + 1) % state.tabs.len()
                };
                state.tabs[next].id
            })
        };
        if let Some(id) = id {
            Self::activate_tab(controller, id);
        }
    }

    pub fn close_tab(controller: &SharedController, id: u64) {
        let host = {
            let mut state = lock_controller(controller);
            let Some(tab) = state.tab_mut(id) else { return };
            if tab.closing {
                return;
            }
            tab.closing = true;
            tab.browser.as_ref().and_then(Browser::host)
        };
        if let Some(host) = host {
            host.close_browser(0);
            if let Some(tab) = lock_controller(controller).tab_mut(id) {
                tab.closing = false;
            }
        } else {
            Self::finish_tab_close(controller, id);
        }
    }

    pub fn close_active_tab(controller: &SharedController) {
        let id = lock_controller(controller).active;
        if let Some(id) = id {
            Self::close_tab(controller, id);
        }
    }

    pub(crate) fn finish_tab_close(controller: &SharedController, id: u64) {
        let (window, was_active, closing_window, empty) = {
            let mut state = lock_controller(controller);
            let Some(index) = state.tabs.iter().position(|tab| tab.id == id) else {
                return;
            };
            let was_active = state.active == Some(id);
            let tab = state.tabs.remove(index);
            if !state.private && !state.closing_window && !tab.url.starts_with("data:") {
                state.closed.push(ClosedTab {
                    url: tab.url,
                    title: tab.title,
                    pinned: tab.pinned,
                });
                let limit = storage::closed_tab_limit();
                if state.closed.len() > limit {
                    let excess = state.closed.len() - limit;
                    state.closed.drain(..excess);
                }
            }
            if was_active {
                state.active = state
                    .tabs
                    .get(index)
                    .or_else(|| state.tabs.get(index.saturating_sub(1)))
                    .map(|tab| tab.id);
            }
            state.persist_session();
            (
                state.window,
                was_active,
                state.closing_window,
                state.tabs.is_empty(),
            )
        };
        if empty {
            if !closing_window {
                Self::new_tab(controller, &pages::start_page_url(), false);
            }
        } else {
            let active = if was_active && !closing_window {
                lock_controller(controller).active
            } else {
                None
            };
            if let Some(active) = active {
                Self::activate_tab(controller, active);
            }
            platform::refresh_tabs(window);
            platform::sync_active_tab(window);
        }
    }

    pub fn reorder_tab(controller: &SharedController, id: u64, target: usize) {
        let window = {
            let mut state = lock_controller(controller);
            let Some(from) = state.tabs.iter().position(|tab| tab.id == id) else {
                return;
            };
            let pinned = state.tabs[from].pinned;
            let target = target.min(state.tabs.len().saturating_sub(1));
            let valid = state
                .tabs
                .get(target)
                .is_some_and(|tab| tab.pinned == pinned);
            if !valid || from == target {
                return;
            }
            let tab = state.tabs.remove(from);
            state.tabs.insert(target, tab);
            state.persist_session();
            state.window
        };
        platform::refresh_tabs(window);
    }

    pub fn toggle_pin(controller: &SharedController, id: u64) {
        let window = {
            let mut state = lock_controller(controller);
            let Some(from) = state.tabs.iter().position(|tab| tab.id == id) else {
                return;
            };
            let pinned = !state.tabs[from].pinned;
            let mut tab = state.tabs.remove(from);
            tab.pinned = pinned;
            let index = state.tabs.iter().filter(|tab| tab.pinned).count();
            let index = index.min(state.tabs.len());
            state.tabs.insert(index, tab);
            state.persist_session();
            state.window
        };
        platform::refresh_tabs(window);
    }

    pub fn active_zoom_percent(&self) -> i32 {
        self.active_tab().map_or(100, |tab| tab.zoom_percent)
    }

    pub fn change_zoom(controller: &SharedController, direction: i32) {
        let (host, percent) = {
            let mut state = lock_controller(controller);
            let Some(tab) = state.active.and_then(|id| state.tab_mut(id)) else {
                return;
            };
            let steps = [
                50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400, 500,
            ];
            let current = steps
                .iter()
                .position(|step| *step == tab.zoom_percent)
                .unwrap_or(5) as i32;
            if direction != 0 {
                let next = (current + direction).clamp(0, steps.len() as i32 - 1);
                tab.zoom_percent = steps[next as usize];
            } else {
                tab.zoom_percent = 100;
            }
            (
                tab.browser.as_ref().and_then(Browser::host),
                tab.zoom_percent,
            )
        };
        if let Some(host) = host {
            let level = (percent as f64 / 100.0).ln() / 1.2_f64.ln();
            host.set_zoom_level(level);
        }
    }

    pub fn go_back(controller: &SharedController) {
        let browser = {
            let state = lock_controller(controller);
            state
                .active_tab()
                .filter(|tab| tab.can_go_back)
                .and_then(|tab| tab.browser.clone())
        };
        if let Some(browser) = browser {
            browser.go_back();
        }
    }

    pub fn go_forward(controller: &SharedController) {
        let browser = {
            let state = lock_controller(controller);
            state
                .active_tab()
                .filter(|tab| tab.can_go_forward)
                .and_then(|tab| tab.browser.clone())
        };
        if let Some(browser) = browser {
            browser.go_forward();
        }
    }

    pub fn reload_or_stop(controller: &SharedController) {
        let (browser, is_loading) = {
            let state = lock_controller(controller);
            let tab = state.active_tab();
            (
                tab.and_then(|tab| tab.browser.clone()),
                tab.is_some_and(|tab| tab.is_loading),
            )
        };
        if is_loading {
            if let Some(browser) = browser {
                browser.stop_load();
            }
        } else if let Some(browser) = browser {
            browser.reload();
        }
    }

    pub fn reload(controller: &SharedController) {
        let browser = lock_controller(controller)
            .active_tab()
            .and_then(|tab| tab.browser.clone());
        if let Some(browser) = browser {
            browser.reload();
        }
    }

    pub fn stop_loading(controller: &SharedController) {
        let browser = lock_controller(controller)
            .active_tab()
            .and_then(|tab| tab.browser.clone());
        if let Some(browser) = browser {
            browser.stop_load();
        }
    }

    pub fn go_home(controller: &SharedController) {
        Self::load_url(controller, &pages::start_page_url());
    }

    pub fn show_dev_tools(controller: &SharedController) {
        let host = lock_controller(controller)
            .active_tab()
            .and_then(|tab| tab.browser.as_ref())
            .and_then(Browser::host);
        let Some(host) = host else { return };
        let window_info =
            WindowInfo::default().set_as_popup(sys::HWND(ptr::null_mut()), "KOCKERS DevTools");
        let settings = BrowserSettings::default();
        let mut client = KockersClient::new(Arc::new(Mutex::new(BrowserController::default())));
        host.show_dev_tools(Some(&window_info), Some(&mut client), Some(&settings), None);
    }

    pub fn open_private_window() {
        let controller = Arc::new(Mutex::new(BrowserController::new(true)));
        let client = KockersClient::new(controller.clone());
        match platform::create_main_window(controller.clone()) {
            Ok(window) => create_main_browser(window, controller, client),
            Err(error) => platform::show_fatal_error(&error),
        }
    }

    pub fn close_requested(controller: &SharedController) -> bool {
        let hosts = {
            let mut state = lock_controller(controller);
            state.persist_session();
            state.closing_window = true;
            state.tabs.retain(|tab| tab.browser.is_some());
            state
                .tabs
                .iter_mut()
                .filter_map(|tab| {
                    tab.closing = true;
                    tab.browser.as_ref().and_then(Browser::host)
                })
                .collect::<Vec<_>>()
        };
        if hosts.is_empty() {
            return true;
        }
        let mut accepted = true;
        for host in hosts {
            accepted &= host.try_close_browser() == 1;
        }
        accepted
    }

    pub fn toggle_history(controller: &SharedController) {
        let preferences = {
            let mut state = lock_controller(controller);
            state.preferences.save_history = !state.preferences.save_history;
            state.preferences.clone()
        };
        storage::save_preferences(&preferences);
    }

    pub fn toggle_download_prompt(controller: &SharedController) {
        let preferences = {
            let mut state = lock_controller(controller);
            state.preferences.ask_download_path = !state.preferences.ask_download_path;
            state.preferences.clone()
        };
        storage::save_preferences(&preferences);
    }

    pub fn toggle_bookmarks_bar(controller: &SharedController) {
        let (window, preferences) = {
            let mut state = lock_controller(controller);
            state.preferences.show_bookmarks = !state.preferences.show_bookmarks;
            (state.window, state.preferences.clone())
        };
        storage::save_preferences(&preferences);
        platform::refresh_bookmarks(window);
    }

    pub fn toggle_bookmark(controller: &SharedController) {
        let (window, bookmarks) = {
            let mut state = lock_controller(controller);
            let Some(tab) = state.active_tab() else {
                return;
            };
            if !tab.url.starts_with("https://") && !tab.url.starts_with("http://") {
                return;
            }
            let url = tab.url.clone();
            let title = if tab.title.is_empty() {
                url.clone()
            } else {
                tab.title.clone()
            };
            if let Some(index) = state
                .bookmarks
                .iter()
                .position(|bookmark| bookmark.url == url)
            {
                state.bookmarks.remove(index);
            } else if state.bookmarks.len() < 200 {
                state.bookmarks.push(storage::Bookmark { url, title });
            } else {
                return;
            }
            (state.window, state.bookmarks.clone())
        };
        storage::save_bookmarks(&bookmarks);
        platform::refresh_bookmarks(window);
    }

    pub fn open_bookmark(controller: &SharedController, index: usize) {
        let url = lock_controller(controller)
            .bookmarks
            .get(index)
            .map(|bookmark| bookmark.url.clone());
        if let Some(url) = url {
            Self::load_url(controller, &url);
        }
    }

    pub fn remove_bookmark(controller: &SharedController, index: usize) {
        let (window, bookmarks) = {
            let mut state = lock_controller(controller);
            if index >= state.bookmarks.len() {
                return;
            }
            state.bookmarks.remove(index);
            (state.window, state.bookmarks.clone())
        };
        storage::save_bookmarks(&bookmarks);
        platform::refresh_bookmarks(window);
    }

    pub fn clear_history(seconds: Option<u64>) {
        storage::clear_history_since(seconds);
    }

    pub fn clear_site_data() {
        if let Some(context) = request_context_get_global_context() {
            context.clear_http_cache(None);
            context.clear_certificate_exceptions(None);
            context.clear_http_auth_credentials(None);
            if let Some(cookies) = context.cookie_manager(None) {
                cookies.delete_cookies(None, None, None);
            }
        }
    }

    pub fn open_internal_page(controller: &SharedController, page: ui::internal::InternalPage) {
        Self::load_url(controller, &page.url());
    }

    pub fn handle_internal_action(controller: &SharedController, action: &str) {
        let Ok(url) = url::Url::parse(action) else {
            return;
        };
        let parameters = url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        let parameter = |name: &str| {
            parameters
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        let path = url.path();
        let page = match path {
            "open/profile" => Some(ui::internal::InternalPage::Profile),
            "open/history" => Some(ui::internal::InternalPage::History),
            "open/settings/sync" => Some(ui::internal::InternalPage::Settings(
                ui::internal::Section::Sync,
            )),
            "open/settings/performance" => Some(ui::internal::InternalPage::Settings(
                ui::internal::Section::Performance,
            )),
            "open/settings/appearance" => Some(ui::internal::InternalPage::Settings(
                ui::internal::Section::Appearance,
            )),
            "open/settings/search" => Some(ui::internal::InternalPage::Settings(
                ui::internal::Section::Search,
            )),
            "open/settings/default" => Some(ui::internal::InternalPage::Settings(
                ui::internal::Section::DefaultBrowser,
            )),
            "open/settings/startup" => Some(ui::internal::InternalPage::Settings(
                ui::internal::Section::Startup,
            )),
            "open/settings/languages" => Some(ui::internal::InternalPage::Settings(
                ui::internal::Section::Languages,
            )),
            "open/settings/downloads" => Some(ui::internal::InternalPage::Settings(
                ui::internal::Section::Downloads,
            )),
            "open/settings/system" => Some(ui::internal::InternalPage::Settings(
                ui::internal::Section::System,
            )),
            "open/settings/reset" => Some(ui::internal::InternalPage::Settings(
                ui::internal::Section::Reset,
            )),
            _ => None,
        };
        if let Some(page) = page {
            Self::open_internal_page(controller, page);
            return;
        }
        match path {
            "settings/set" => {
                let mut preferences = lock_controller(controller).preferences();
                let key = parameter("key");
                let value = parameter("value");
                let enabled = value == "true";
                match key.as_str() {
                    "reduce_animations" => preferences.reduce_animations = enabled,
                    "smooth_scrolling" => preferences.smooth_scrolling = enabled,
                    "background_throttling" => preferences.background_throttling = enabled,
                    "lazy_restore" => preferences.lazy_restore = enabled,
                    "continue_background" => preferences.continue_background = enabled,
                    "hardware_acceleration" => preferences.hardware_acceleration = enabled,
                    "show_tips" => preferences.show_tips = enabled,
                    "bookmarks" => preferences.show_bookmarks = enabled,
                    "ask_download" => preferences.ask_download_path = enabled,
                    "theme" => {
                        preferences.theme = match value.as_str() {
                            "dark" => storage::Theme::Dark,
                            _ => storage::Theme::Light,
                        };
                    }
                    "accent" => {
                        preferences.accent = match value.as_str() {
                            "blue" => storage::Accent::Blue,
                            "graphite" => storage::Accent::Graphite,
                            _ => storage::Accent::Olive,
                        };
                    }
                    "language" => {
                        preferences.language = match value.as_str() {
                            "en" => storage::Language::English,
                            _ => storage::Language::Russian,
                        };
                    }
                    "search" => {
                        preferences.search_engine = match value.as_str() {
                            "bing" => storage::SearchEngine::Bing,
                            "duckduckgo" => storage::SearchEngine::DuckDuckGo,
                            "yandex" => storage::SearchEngine::Yandex,
                            _ => storage::SearchEngine::Google,
                        };
                    }
                    "startup" => {
                        preferences.startup_mode = match value.as_str() {
                            "new_tab" => storage::StartupMode::NewTab,
                            "custom" => storage::StartupMode::Custom,
                            _ => storage::StartupMode::Restore,
                        };
                    }
                    "startup_url" => preferences.startup_url = value,
                    _ => return,
                }
                let section = match key.as_str() {
                    "reduce_animations"
                    | "smooth_scrolling"
                    | "background_throttling"
                    | "lazy_restore" => ui::internal::Section::Performance,
                    "theme" | "accent" | "bookmarks" => ui::internal::Section::Appearance,
                    "language" => ui::internal::Section::Languages,
                    "search" => ui::internal::Section::Search,
                    "startup" | "startup_url" => ui::internal::Section::Startup,
                    "ask_download" => ui::internal::Section::Downloads,
                    _ => ui::internal::Section::System,
                };
                Self::apply_preferences(
                    controller,
                    preferences,
                    ui::internal::InternalPage::Settings(section),
                );
            }
            "profile/save" => {
                let mut profile = storage::load_profile();
                profile.nickname = parameter("nickname").chars().take(60).collect();
                profile.description = parameter("description").chars().take(500).collect();
                storage::save_profile(&profile);
                Self::open_internal_page(controller, ui::internal::InternalPage::Profile);
            }
            "profile/avatar" | "profile/banner" => {
                let window = lock_controller(controller).window();
                let banner = path == "profile/banner";
                if let Some(source) = platform::choose_image(window) {
                    match storage::store_profile_image(&source, banner) {
                        Ok(stored) => {
                            let mut profile = storage::load_profile();
                            if banner {
                                profile.banner_path = stored;
                            } else {
                                profile.avatar_path = stored;
                            }
                            storage::save_profile(&profile);
                            Self::open_internal_page(
                                controller,
                                ui::internal::InternalPage::Profile,
                            );
                        }
                        Err(error) => platform::show_fatal_error(&error),
                    }
                }
            }
            "config/export" => {
                let window = lock_controller(controller).window();
                if let Err(error) = platform::export_config(window, &storage::export_config()) {
                    platform::show_fatal_error(&error);
                }
            }
            "config/import" => {
                let window = lock_controller(controller).window();
                if let Some(text) = platform::import_config(window) {
                    match storage::import_config(&text) {
                        Ok((preferences, profile)) => {
                            storage::save_profile(&profile);
                            Self::apply_preferences(
                                controller,
                                preferences,
                                ui::internal::InternalPage::Settings(ui::internal::Section::Sync),
                            );
                        }
                        Err(error) => platform::show_fatal_error(&error),
                    }
                }
            }
            "downloads/choose" => {
                let window = lock_controller(controller).window();
                if let Some(directory) = platform::choose_download_folder(window) {
                    let mut preferences = lock_controller(controller).preferences();
                    preferences.download_dir = directory.to_string_lossy().into_owned();
                    Self::apply_preferences(
                        controller,
                        preferences,
                        ui::internal::InternalPage::Settings(ui::internal::Section::Downloads),
                    );
                }
            }
            "system/proxy" => platform::open_proxy_settings(),
            "system/default-browser" => platform::open_default_browser_settings(),
            "settings/reset" => {
                Self::apply_preferences(
                    controller,
                    storage::Preferences::default(),
                    ui::internal::InternalPage::Settings(ui::internal::Section::Reset),
                );
            }
            "history/remove" => {
                storage::remove_history(&parameter("url"));
                Self::open_internal_page(controller, ui::internal::InternalPage::History);
            }
            "history/clear" => {
                storage::clear_history_since(None);
                Self::open_internal_page(controller, ui::internal::InternalPage::History);
            }
            _ => {}
        }
    }

    fn apply_preferences(
        controller: &SharedController,
        preferences: storage::Preferences,
        page: ui::internal::InternalPage,
    ) {
        let window = {
            let mut state = lock_controller(controller);
            state.preferences = preferences.clone();
            state.window
        };
        storage::save_preferences(&preferences);
        platform::preferences_changed(window);
        Self::open_internal_page(controller, page);
    }

    pub fn cancel_download(controller: &SharedController, id: u32) {
        let callback = lock_controller(controller)
            .downloads
            .iter()
            .find(|download| download.snapshot.id == id)
            .and_then(|download| download.callback.clone());
        if let Some(callback) = callback {
            callback.cancel();
        }
    }

    pub fn toggle_download_pause(controller: &SharedController, id: u32) {
        let (callback, paused) = {
            let state = lock_controller(controller);
            let download = state
                .downloads
                .iter()
                .find(|download| download.snapshot.id == id);
            (
                download.and_then(|download| download.callback.clone()),
                download.is_some_and(|download| download.snapshot.status == DownloadStatus::Paused),
            )
        };
        if let Some(callback) = callback {
            if paused {
                callback.resume();
            } else {
                callback.pause();
            }
        }
    }

    pub fn open_download(controller: &SharedController, id: u32) {
        let download = lock_controller(controller)
            .downloads
            .iter()
            .find(|download| download.snapshot.id == id)
            .map(|download| download.snapshot.clone());
        if let Some(download) = download
            && download.status == DownloadStatus::Complete
        {
            platform::open_downloaded_file(&download.path);
        }
    }

    fn load_url(controller: &SharedController, url: &str) {
        let frame = lock_controller(controller)
            .active_tab()
            .and_then(|tab| tab.browser.as_ref())
            .and_then(Browser::main_frame);
        if let Some(frame) = frame {
            frame.load_url(Some(&CefString::from(url)));
        }
    }

    fn on_after_created(&mut self, browser: Option<&mut Browser>) -> Option<HWND> {
        let browser = browser?;
        if browser.is_popup() != 0 {
            return None;
        }
        let id = self.pending.pop_front()?;
        let tab = self.tab_mut(id)?;
        tab.browser = Some(browser.clone());
        tab.is_loading = true;
        Some(self.window)
    }

    fn on_address_change(
        &mut self,
        browser_id: i32,
        frame: Option<&mut Frame>,
        url: &str,
    ) -> Option<(HWND, bool)> {
        if frame.is_none_or(|frame| frame.is_main() == 0) {
            return None;
        }
        let index = self.tabs.iter().position(|tab| {
            tab.browser
                .as_ref()
                .is_some_and(|browser| browser.identifier() == browser_id)
        })?;
        self.tabs[index].url = url.to_owned();
        if !self.private && self.preferences.save_history {
            storage::add_history(url, &self.tabs[index].title);
        }
        self.persist_session();
        Some((self.window, self.active == Some(self.tabs[index].id)))
    }

    fn on_title_change(&mut self, browser_id: i32, title: &str) -> Option<(HWND, bool)> {
        let index = self.tabs.iter().position(|tab| {
            tab.browser
                .as_ref()
                .is_some_and(|browser| browser.identifier() == browser_id)
        })?;
        self.tabs[index].title = if title.is_empty() {
            "Новая вкладка".to_owned()
        } else {
            title.to_owned()
        };
        self.persist_session();
        Some((self.window, self.active == Some(self.tabs[index].id)))
    }

    fn on_loading_state_change(
        &mut self,
        browser_id: i32,
        is_loading: bool,
        can_go_back: bool,
        can_go_forward: bool,
    ) -> Option<(HWND, bool)> {
        let tab = self.tabs.iter_mut().find(|tab| {
            tab.browser
                .as_ref()
                .is_some_and(|browser| browser.identifier() == browser_id)
        })?;
        tab.is_loading = is_loading;
        tab.can_go_back = can_go_back;
        tab.can_go_forward = can_go_forward;
        Some((self.window, self.active == Some(tab.id)))
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.tab(id))
    }

    fn active_index(&self) -> Option<usize> {
        self.active
            .and_then(|id| self.tabs.iter().position(|tab| tab.id == id))
    }

    fn tab(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    fn browser_handles(&self) -> Vec<(u64, HWND)> {
        self.tabs
            .iter()
            .filter(|tab| tab.ready)
            .filter_map(|tab| {
                tab.browser
                    .as_ref()
                    .and_then(Browser::host)
                    .map(|host| (tab.id, host.window_handle().0.cast::<std::ffi::c_void>()))
            })
            .filter(|(_, handle)| !handle.is_null())
            .collect()
    }

    fn persist_session(&self) {
        if self.private || self.closing_window {
            return;
        }
        let session = storage::Session {
            tabs: self
                .tabs
                .iter()
                .filter(|tab| !tab.closing && !tab.url.starts_with("data:"))
                .map(|tab| storage::SessionTab {
                    url: tab.url.clone(),
                    title: tab.title.clone(),
                    pinned: tab.pinned,
                })
                .collect(),
            active: self.active_index().unwrap_or(0),
        };
        storage::save_session(&session);
    }
}

fn should_create_browser(background: bool, lazy_restore: bool) -> bool {
    !background || !lazy_restore
}

pub fn lock_controller(controller: &SharedController) -> MutexGuard<'_, BrowserController> {
    match controller.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub fn create_main_browser(window: HWND, controller: SharedController, mut client: Client) {
    {
        let mut state = lock_controller(&controller);
        state.set_window(window);
        if state.private && state.request_context.is_none() {
            drop(state);
            platform::show_fatal_error("Не удалось создать изолированный приватный контекст.");
            platform::finish_window_close(window);
            return;
        }
    }
    let (url, context, lazy_restore) = {
        let mut state = lock_controller(&controller);
        if !state.private && state.preferences.startup_mode == storage::StartupMode::Restore {
            let session = storage::load_session();
            if !session.tabs.is_empty() {
                for tab in session.tabs {
                    let id = state.next_tab_id;
                    state.next_tab_id += 1;
                    state.tabs.push(Tab {
                        id,
                        browser: None,
                        title: tab.title,
                        url: tab.url,
                        pinned: tab.pinned,
                        closing: false,
                        is_loading: false,
                        can_go_back: false,
                        can_go_forward: false,
                        zoom_percent: 100,
                        ready: false,
                    });
                }
                state.active = state.tabs.get(session.active).map(|tab| tab.id);
            }
        }
        if state.tabs.is_empty() {
            let initial_url = if state.preferences.startup_mode == storage::StartupMode::Custom {
                navigation::normalize_input_with(
                    &state.preferences.startup_url,
                    state.preferences.search_engine,
                )
                .ok()
                .flatten()
                .unwrap_or_else(pages::start_page_url)
            } else {
                pages::start_page_url()
            };
            let id = state.next_tab_id;
            state.next_tab_id += 1;
            state.tabs.push(Tab {
                id,
                browser: None,
                title: "Новая вкладка".to_owned(),
                url: initial_url,
                pinned: false,
                closing: false,
                is_loading: false,
                can_go_back: false,
                can_go_forward: false,
                zoom_percent: 100,
                ready: false,
            });
            state.active = Some(id);
        }
        let active = state.active.expect("initial tab");
        state.pending.push_back(active);
        let tab = state.tab(active).expect("initial tab");
        (
            tab.url.clone(),
            state.request_context.clone(),
            state.preferences.lazy_restore,
        )
    };
    platform::refresh_tabs(window);
    create_browser_with_client(window, &url, context.clone(), &mut client);
    if !lazy_restore {
        let restored = {
            let mut state = lock_controller(&controller);
            let active = state.active;
            let tabs = state
                .tabs
                .iter()
                .filter(|tab| Some(tab.id) != active)
                .map(|tab| (tab.id, tab.url.clone()))
                .collect::<Vec<_>>();
            for (id, _) in &tabs {
                state.pending.push_back(*id);
            }
            tabs
        };
        for (_, url) in restored {
            create_browser(window, controller.clone(), &url, context.clone());
        }
    }
}

fn create_browser(
    window: HWND,
    controller: SharedController,
    url: &str,
    context: Option<RequestContext>,
) {
    let mut client = KockersClient::new(controller);
    create_browser_with_client(window, url, context, &mut client);
}

fn create_browser_with_client(
    window: HWND,
    url: &str,
    mut context: Option<RequestContext>,
    client: &mut Client,
) {
    let (width, height) = unsafe { platform::client_size(window) };
    let content_top = platform::content_top(window);
    let bounds = Rect {
        x: 0,
        y: content_top,
        width,
        height: (height - content_top).max(0),
    };
    let window_info = WindowInfo {
        runtime_style: RuntimeStyle::ALLOY,
        ..Default::default()
    }
    .set_as_child(sys::HWND(window.cast()), &bounds);
    let settings = BrowserSettings {
        background_color: if storage::load_preferences().theme == storage::Theme::Dark {
            0xFF171917
        } else {
            0xFFF7F7F4
        },
        ..Default::default()
    };
    let created = browser_host_create_browser(
        Some(&window_info),
        Some(client),
        Some(&CefString::from(url)),
        Some(&settings),
        None,
        context.as_mut(),
    );
    if created == 0 {
        platform::show_fatal_error("Chromium не смог создать вкладку.");
    }
}

fn apply_visibility(controller: &SharedController, window: HWND) {
    let (active, handles) = {
        let state = lock_controller(controller);
        (state.active, state.browser_handles())
    };
    platform::apply_browser_visibility(window, &handles, active);
}

fn browser_id(browser: Option<&mut Browser>) -> Option<i32> {
    browser.map(|browser| browser.identifier())
}

fn update_download(
    controller: &SharedController,
    item: &DownloadItem,
    callback: Option<&mut DownloadItemCallback>,
) {
    let id = item.id();
    let full_path = CefString::from(&item.full_path()).to_string();
    let suggested = CefString::from(&item.suggested_file_name()).to_string();
    let path = PathBuf::from(&full_path);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(suggested.as_str())
        .to_owned();
    let status = if item.is_canceled() != 0 {
        DownloadStatus::Canceled
    } else if item.is_complete() != 0 {
        DownloadStatus::Complete
    } else if item.is_interrupted() != 0 {
        DownloadStatus::Interrupted
    } else if item.is_paused() != 0 {
        DownloadStatus::Paused
    } else {
        DownloadStatus::InProgress
    };
    let callback = callback.cloned();
    let window = {
        let mut state = lock_controller(controller);
        let snapshot = DownloadSnapshot {
            id,
            name,
            path,
            received: item.received_bytes(),
            total: item.total_bytes(),
            percent: item.percent_complete(),
            status,
        };
        if let Some(download) = state
            .downloads
            .iter_mut()
            .find(|download| download.snapshot.id == id)
        {
            download.snapshot = snapshot;
            download.callback =
                if status == DownloadStatus::InProgress || status == DownloadStatus::Paused {
                    callback
                } else {
                    None
                };
        } else {
            state.downloads.push(Download { snapshot, callback });
        }
        if state.downloads.len() > 50 {
            let index = state.downloads.iter().position(|download| {
                download.snapshot.status != DownloadStatus::InProgress
                    && download.snapshot.status != DownloadStatus::Paused
            });
            if let Some(index) = index {
                state.downloads.remove(index);
            }
        }
        state.window
    };
    platform::update_downloads(window);
}

pub fn sanitize_download_name(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "download".to_owned();
    }
    let sanitized = value
        .chars()
        .map(|character| {
            if matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) || character.is_control()
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>()
        .trim_matches([' ', '.'])
        .to_owned();
    let sanitized = if sanitized.is_empty() {
        "download"
    } else {
        &sanitized
    };
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = sanitized.split('.').next().unwrap_or(sanitized);
    let sanitized = if reserved.iter().any(|name| stem.eq_ignore_ascii_case(name)) {
        format!("_{sanitized}")
    } else {
        sanitized.to_owned()
    };
    sanitized
        .chars()
        .take(120)
        .collect::<String>()
        .trim_matches([' ', '.'])
        .to_owned()
}

wrap_client! {
    pub struct KockersClient {
        controller: SharedController,
    }

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(KockersDisplayHandler::new(self.controller.clone()))
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(KockersLifeSpanHandler::new(self.controller.clone()))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(KockersLoadHandler::new(self.controller.clone()))
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(KockersKeyboardHandler::new(self.controller.clone()))
        }

        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(KockersDownloadHandler::new(self.controller.clone()))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(KockersRequestHandler::new(self.controller.clone()))
        }

        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(KockersPermissionHandler::new(self.controller.clone()))
        }
    }
}

wrap_display_handler! {
    struct KockersDisplayHandler {
        controller: SharedController,
    }

    impl DisplayHandler {
        fn on_address_change(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            if let Some(id) = browser_id(browser) {
                let url = url.map(CefString::to_string).unwrap_or_default();
                let update = lock_controller(&self.controller).on_address_change(id, frame, &url);
                if let Some((window, active)) = update {
                    if active {
                        platform::update_address(window, &url);
                    }
                    platform::refresh_tabs(window);
                }
            }
        }

        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            if let Some(id) = browser_id(browser) {
                let title = title.map(CefString::to_string).unwrap_or_default();
                let update = lock_controller(&self.controller).on_title_change(id, &title);
                if let Some((window, active)) = update {
                    if active {
                        let base_title = if title == "KOCKERS" || title.is_empty() {
                            "KOCKERS".to_owned()
                        } else {
                            format!("{title} — KOCKERS")
                        };
                        let private = lock_controller(&self.controller).is_private();
                        let window_title = if private {
                            format!("Приватное окно — {base_title}")
                        } else {
                            base_title
                        };
                        platform::update_title(window, &window_title);
                    }
                    platform::refresh_tabs(window);
                }
            }
        }
    }
}

wrap_life_span_handler! {
    struct KockersLifeSpanHandler {
        controller: SharedController,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let handle = browser
                .as_ref()
                .and_then(|browser| browser.host())
                .map(|host| host.window_handle().0.cast::<std::ffi::c_void>());
            let window = lock_controller(&self.controller).on_after_created(browser);
            if let Some(window) = window {
                if let Some(handle) = handle.filter(|handle| !handle.is_null()) {
                    platform::apply_browser_visibility(window, &[(0, handle)], None);
                }
                platform::refresh_tabs(window);
                platform::sync_active_tab(window);
            }
        }

        fn do_close(&self, browser: Option<&mut Browser>) -> i32 {
            let (window, closing_window) = {
                let state = lock_controller(&self.controller);
                (state.window, state.closing_window)
            };
            if closing_window {
                return 0;
            }
            if let Some(host) = browser.and_then(|browser| browser.host()) {
                let handle = host.window_handle().0.cast::<std::ffi::c_void>();
                platform::post_browser_destroy(window, handle);
            }
            1
        }

        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: i32,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            user_gesture: i32,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            if user_gesture != 0 {
                let url = target_url.map(CefString::to_string).unwrap_or_default();
                if ui::internal::is_action_url(&url) {
                    let internal = lock_controller(&self.controller)
                        .active_tab()
                        .is_some_and(|tab| tab.url.starts_with("data:"));
                    if internal {
                        BrowserController::handle_internal_action(&self.controller, &url);
                    }
                } else if url.starts_with("https://") || url.starts_with("http://") {
                    BrowserController::new_tab(&self.controller, &url, false);
                }
            }
            1
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let (window, closing_window, id) = {
                let state = lock_controller(&self.controller);
                let id = browser.and_then(|browser| {
                    state.tabs.iter().find(|tab| {
                        tab.browser.as_ref().is_some_and(|current| current.identifier() == browser.identifier())
                    }).map(|tab| tab.id)
                });
                (state.window, state.closing_window, id)
            };
            if !closing_window && let Some(id) = id {
                platform::post_tab_closed(window, id);
            }
        }
    }
}

wrap_load_handler! {
    struct KockersLoadHandler {
        controller: SharedController,
    }

    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            browser: Option<&mut Browser>,
            is_loading: i32,
            can_go_back: i32,
            can_go_forward: i32,
        ) {
            if let Some(id) = browser_id(browser) {
                let loading = is_loading != 0;
                let back = can_go_back != 0;
                let forward = can_go_forward != 0;
                let update = lock_controller(&self.controller)
                    .on_loading_state_change(id, loading, back, forward);
                if let Some((window, active)) = update {
                    if active {
                        platform::update_loading_state(window, loading, back, forward);
                    }
                    platform::refresh_tabs(window);
                }
            }
        }

        fn on_load_end(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _http_status_code: i32,
        ) {
            if frame.is_none_or(|frame| frame.is_main() == 0) {
                return;
            }
            if let Some(id) = browser_id(browser) {
                let window = {
                    let mut state = lock_controller(&self.controller);
                    let window = state.window;
                    let tab = state.tabs.iter_mut().find(|tab| {
                        tab.browser
                            .as_ref()
                            .is_some_and(|browser| browser.identifier() == id)
                    });
                    tab.map(|tab| {
                        tab.ready = true;
                        window
                    })
                };
                if let Some(window) = window {
                    apply_visibility(&self.controller, window);
                }
            }
        }

        fn on_load_error(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let raw_code = sys::cef_errorcode_t::from(error_code);
            if raw_code == sys::cef_errorcode_t::ERR_ABORTED {
                return;
            }
            let (Some(browser), Some(frame)) = (browser, frame) else {
                return;
            };
            if frame.is_main() == 0 {
                return;
            }
            let known = lock_controller(&self.controller)
                .tabs
                .iter()
                .any(|tab| tab.browser.as_ref().is_some_and(|current| current.identifier() == browser.identifier()));
            if !known {
                return;
            }
            let failed_url = failed_url.map(CefString::to_string).unwrap_or_default();
            let error_text = error_text
                .map(CefString::to_string)
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| "Chromium не смог загрузить этот адрес.".to_owned());
            let page = pages::error_page_url(&failed_url, &error_text, Some(raw_code as i32));
            frame.load_url(Some(&CefString::from(page.as_str())));
        }
    }
}

wrap_download_handler! {
    struct KockersDownloadHandler {
        controller: SharedController,
    }

    impl DownloadHandler {
        fn can_download(
            &self,
            _browser: Option<&mut Browser>,
            _url: Option<&CefString>,
            _request_method: Option<&CefString>,
        ) -> i32 {
            1
        }

        fn on_before_download(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> i32 {
            let (Some(item), Some(callback)) = (download_item, callback) else { return 0 };
            let suggested = suggested_name.map(CefString::to_string).unwrap_or_default();
            let name = sanitize_download_name(&suggested);
            let (window, ask, directory) = {
                let state = lock_controller(&self.controller);
                (
                    state.window,
                    state.preferences.ask_download_path,
                    state.preferences.download_dir.clone(),
                )
            };
            if let Some(path) = platform::choose_download_path(window, &name, ask, &directory) {
                let path = CefString::from(path.to_string_lossy().as_ref());
                callback.cont(Some(&path), 0);
                update_download(&self.controller, item, None);
            }
            0
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            callback: Option<&mut DownloadItemCallback>,
        ) {
            if let Some(item) = download_item {
                update_download(&self.controller, item, callback);
            }
        }
    }
}

wrap_request_handler! {
    struct KockersRequestHandler {
        controller: SharedController,
    }

    impl RequestHandler {
        fn on_before_browse(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _user_gesture: i32,
            _is_redirect: i32,
        ) -> i32 {
            if frame.is_none_or(|frame| frame.is_main() == 0) {
                return 0;
            }
            let url = request.map(|request| CefString::from(&request.url()).to_string()).unwrap_or_default();
            if ui::internal::is_action_url(&url) {
                let internal = lock_controller(&self.controller)
                    .active_tab()
                    .is_some_and(|tab| tab.url.starts_with("data:"));
                if internal {
                    BrowserController::handle_internal_action(&self.controller, &url);
                }
                return 1;
            }
            let allowed = ["http://", "https://", "data:", "about:", "blob:"]
                .iter()
                .any(|scheme| url.starts_with(scheme));
            i32::from(!allowed)
        }

        fn on_open_urlfrom_tab(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            target_url: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            user_gesture: i32,
        ) -> i32 {
            let url = target_url.map(CefString::to_string).unwrap_or_default();
            if ui::internal::is_action_url(&url) {
                let internal = lock_controller(&self.controller)
                    .active_tab()
                    .is_some_and(|tab| tab.url.starts_with("data:"));
                if internal {
                    BrowserController::handle_internal_action(&self.controller, &url);
                }
                return 1;
            }
            if matches!(target_disposition, WindowOpenDisposition::NEW_FOREGROUND_TAB | WindowOpenDisposition::NEW_BACKGROUND_TAB | WindowOpenDisposition::NEW_POPUP | WindowOpenDisposition::NEW_WINDOW) {
                if user_gesture != 0 {
                    let url = target_url.map(CefString::to_string).unwrap_or_default();
                    if url.starts_with("https://") || url.starts_with("http://") {
                        BrowserController::new_tab(&self.controller, &url, target_disposition == WindowOpenDisposition::NEW_BACKGROUND_TAB);
                    }
                }
                return 1;
            }
            0
        }
    }
}

wrap_permission_handler! {
    struct KockersPermissionHandler {
        controller: SharedController,
    }

    impl PermissionHandler {
        fn on_request_media_access_permission(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut MediaAccessCallback>,
        ) -> i32 {
            let Some(callback) = callback else { return 0 };
            let origin = requesting_origin.map(CefString::to_string).unwrap_or_default();
            let window = lock_controller(&self.controller).window();
            let allowed = platform::confirm_permission(window, &origin, "камере и микрофону");
            callback.cont(if allowed { requested_permissions } else { 0 });
            1
        }

        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            _prompt_id: u64,
            requesting_origin: Option<&CefString>,
            _requested_permissions: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> i32 {
            let Some(callback) = callback else { return 0 };
            let origin = requesting_origin.map(CefString::to_string).unwrap_or_default();
            let window = lock_controller(&self.controller).window();
            let allowed = platform::confirm_permission(window, &origin, "запрошенным данным и устройствам");
            callback.cont(if allowed { PermissionRequestResult::ACCEPT } else { PermissionRequestResult::DENY });
            1
        }
    }
}

wrap_keyboard_handler! {
    struct KockersKeyboardHandler {
        controller: SharedController,
    }

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            _browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut sys::MSG>,
            _is_keyboard_shortcut: Option<&mut i32>,
        ) -> i32 {
            let Some(event) = event else { return 0 };
            if event.type_ != KeyEventType::RAWKEYDOWN && event.type_ != KeyEventType::KEYDOWN {
                return 0;
            }
            let control = event.modifiers & sys::cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0 as u32 != 0;
            let shift = event.modifiers & sys::cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0 as u32 != 0;
            let alt = event.modifiers & sys::cef_event_flags_t::EVENTFLAG_ALT_DOWN.0 as u32 != 0;
            let window = lock_controller(&self.controller).window();
            match event.windows_key_code {
                key if key == 'L' as i32 && control => unsafe { platform::focus_address(window) },
                key if key == 'R' as i32 && control => BrowserController::reload(&self.controller),
                key if key == 'D' as i32 && control => BrowserController::toggle_bookmark(&self.controller),
                key if key == 'H' as i32 && control => BrowserController::open_internal_page(&self.controller, ui::internal::InternalPage::History),
                key if key == 'B' as i32 && control && shift => BrowserController::toggle_bookmarks_bar(&self.controller),
                key if key == 'T' as i32 && control && shift => BrowserController::restore_closed_tab(&self.controller),
                key if key == 'T' as i32 && control => BrowserController::new_tab(&self.controller, &pages::start_page_url(), false),
                key if key == 'W' as i32 && control => BrowserController::close_active_tab(&self.controller),
                key if key == 'N' as i32 && control && shift => BrowserController::open_private_window(),
                key if key == VK_TAB as i32 && control => BrowserController::next_tab(&self.controller, shift),
                key if key == VK_F5 as i32 => BrowserController::reload(&self.controller),
                key if key == VK_LEFT as i32 && alt => BrowserController::go_back(&self.controller),
                key if key == VK_RIGHT as i32 && alt => BrowserController::go_forward(&self.controller),
                key if key == VK_ESCAPE as i32 => BrowserController::stop_loading(&self.controller),
                key if key == VK_F11 as i32 => unsafe { platform::toggle_fullscreen(window) },
                key if key == VK_F12 as i32 => BrowserController::show_dev_tools(&self.controller),
                key if control && ('1' as i32..='9' as i32).contains(&key) => {
                    let index = (key - '1' as i32) as usize;
                    let id = {
                        let state = lock_controller(&self.controller);
                        if key == '9' as i32 { state.tabs.last().map(|tab| tab.id) } else { state.tabs.get(index).map(|tab| tab.id) }
                    };
                    if let Some(id) = id { BrowserController::activate_tab(&self.controller, id); }
                }
                _ => return 0,
            }
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lazy_background_tabs_are_created_only_when_activated() {
        assert!(!should_create_browser(true, true));
        assert!(should_create_browser(true, false));
        assert!(should_create_browser(false, true));
    }

    #[test]
    fn download_names_cannot_escape_the_target_directory() {
        assert_eq!(sanitize_download_name("../../report.exe"), "_.._report.exe");
        assert_eq!(sanitize_download_name("CON"), "_CON");
        assert_eq!(sanitize_download_name("con.txt"), "_con.txt");
        assert_eq!(sanitize_download_name(" \t"), "download");
    }
}
