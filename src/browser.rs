use crate::{navigation, platform, ui::pages};
use cef::*;
use std::{
    ptr,
    sync::{Arc, Mutex, MutexGuard},
};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_F5, VK_F12, VK_LEFT, VK_RIGHT},
};

pub type SharedController = Arc<Mutex<BrowserController>>;

pub struct BrowserController {
    browser: Option<Browser>,
    window: HWND,
    is_loading: bool,
    can_go_back: bool,
    can_go_forward: bool,
}

impl Default for BrowserController {
    fn default() -> Self {
        Self {
            browser: None,
            window: ptr::null_mut(),
            is_loading: false,
            can_go_back: false,
            can_go_forward: false,
        }
    }
}

#[allow(clippy::arc_with_non_send_sync)]
impl BrowserController {
    pub fn set_window(&mut self, window: HWND) {
        self.window = window;
    }

    pub fn window(&self) -> HWND {
        self.window
    }

    pub fn resize_content(&self) {
        let browser = self
            .browser
            .as_ref()
            .and_then(Browser::host)
            .map(|host| host.window_handle().0.cast::<std::ffi::c_void>())
            .filter(|handle| !handle.is_null());
        unsafe { platform::resize_browser(self.window, browser) };
    }

    pub fn is_empty(&self) -> bool {
        self.browser.is_none()
    }

    pub fn navigate_input(controller: &SharedController, input: &str) {
        let target = match navigation::normalize_input(input) {
            Ok(Some(url)) => url,
            Ok(None) => pages::start_page_url().to_owned(),
            Err(error) => pages::error_page_url(input, &error.to_string(), None),
        };
        Self::load_url(controller, &target);
    }

    pub fn go_back(controller: &SharedController) {
        let browser = {
            let state = lock_controller(controller);
            state.can_go_back.then(|| state.browser.clone()).flatten()
        };
        if let Some(browser) = browser {
            browser.go_back();
        }
    }

    pub fn go_forward(controller: &SharedController) {
        let browser = {
            let state = lock_controller(controller);
            state
                .can_go_forward
                .then(|| state.browser.clone())
                .flatten()
        };
        if let Some(browser) = browser {
            browser.go_forward();
        }
    }

    pub fn reload_or_stop(controller: &SharedController) {
        let (browser, is_loading) = {
            let state = lock_controller(controller);
            (state.browser.clone(), state.is_loading)
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
        let browser = lock_controller(controller).browser.clone();
        if let Some(browser) = browser {
            browser.reload();
        }
    }

    pub fn stop_loading(controller: &SharedController) {
        let browser = lock_controller(controller).browser.clone();
        if let Some(browser) = browser {
            browser.stop_load();
        }
    }

    pub fn go_home(controller: &SharedController) {
        Self::load_url(controller, pages::start_page_url());
    }

    pub fn show_dev_tools(controller: &SharedController) {
        let host = lock_controller(controller)
            .browser
            .as_ref()
            .and_then(Browser::host);
        let Some(host) = host else { return };
        let window_info =
            WindowInfo::default().set_as_popup(sys::HWND(ptr::null_mut()), "KOCKERS DevTools");
        let settings = BrowserSettings::default();
        let mut client = KockersClient::new(Arc::new(Mutex::new(BrowserController::default())));
        host.show_dev_tools(Some(&window_info), Some(&mut client), Some(&settings), None);
    }

    pub fn close_requested(controller: &SharedController) -> bool {
        let host = lock_controller(controller)
            .browser
            .as_ref()
            .and_then(Browser::host);
        host.is_none_or(|host| host.try_close_browser() == 1)
    }

    fn load_url(controller: &SharedController, url: &str) {
        let frame = lock_controller(controller)
            .browser
            .as_ref()
            .and_then(Browser::main_frame);
        if let Some(frame) = frame {
            frame.load_url(Some(&CefString::from(url)));
        }
    }

    fn on_after_created(&mut self, browser: Option<&mut Browser>) {
        let Some(browser) = browser else { return };
        if browser.is_popup() != 0 {
            return;
        }
        self.browser = Some(browser.clone());
        unsafe {
            platform::resize_browser(
                self.window,
                browser.host().and_then(|host| {
                    let handle = host.window_handle().0.cast::<std::ffi::c_void>();
                    (!handle.is_null()).then_some(handle)
                }),
            )
        };
    }

    fn on_before_close(&mut self, browser: Option<&mut Browser>) {
        let Some(browser) = browser else { return };
        if !self.is_main_browser(browser) {
            return;
        }
        self.browser = None;
        quit_message_loop();
    }

    fn on_address_change(&mut self, frame: Option<&mut Frame>, url: Option<&CefString>) {
        if frame.is_none_or(|frame| frame.is_main() == 0) {
            return;
        }
        let url = url.map(CefString::to_string).unwrap_or_default();
        platform::update_address(self.window, &url);
    }

    fn on_title_change(&mut self, title: Option<&CefString>) {
        let title = title.map(CefString::to_string).unwrap_or_default();
        let title = if title.is_empty() || title == "KOCKERS SOCIETY" {
            "KOCKERS SOCIETY".to_owned()
        } else {
            format!("{title} — KOCKERS")
        };
        platform::update_title(self.window, &title);
    }

    fn on_loading_state_change(
        &mut self,
        is_loading: bool,
        can_go_back: bool,
        can_go_forward: bool,
    ) {
        self.is_loading = is_loading;
        self.can_go_back = can_go_back;
        self.can_go_forward = can_go_forward;
        platform::update_loading_state(self.window, is_loading, can_go_back, can_go_forward);
    }

    fn is_main_browser(&self, browser: &mut Browser) -> bool {
        self.browser
            .as_ref()
            .is_some_and(|current| current.clone().is_same(Some(browser)) != 0)
    }
}

pub fn lock_controller(controller: &SharedController) -> MutexGuard<'_, BrowserController> {
    match controller.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub fn create_main_browser(window: HWND, controller: SharedController, mut client: Client) {
    lock_controller(&controller).set_window(window);
    let (width, height) = unsafe { platform::client_size(window) };
    let bounds = Rect {
        x: 0,
        y: platform::TOOLBAR_HEIGHT,
        width,
        height: (height - platform::TOOLBAR_HEIGHT).max(0),
    };
    let window_info = WindowInfo {
        runtime_style: RuntimeStyle::ALLOY,
        ..Default::default()
    }
    .set_as_child(sys::HWND(window.cast()), &bounds);
    let settings = BrowserSettings {
        background_color: 0xFFF7F6F3,
        ..Default::default()
    };
    let created = browser_host_create_browser(
        Some(&window_info),
        Some(&mut client),
        Some(&CefString::from(pages::start_page_url())),
        Some(&settings),
        None,
        None,
    );
    if created == 0 {
        platform::show_fatal_error("Chromium не смог создать браузерное окно.");
        quit_message_loop();
    }
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
            let Some(browser) = browser else { return };
            let mut controller = lock_controller(&self.controller);
            if controller.is_main_browser(browser) {
                controller.on_address_change(frame, url);
            }
        }

        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            let Some(browser) = browser else { return };
            let mut controller = lock_controller(&self.controller);
            if controller.is_main_browser(browser) {
                controller.on_title_change(title);
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
            lock_controller(&self.controller).on_after_created(browser);
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            lock_controller(&self.controller).on_before_close(browser);
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
            let Some(browser) = browser else { return };
            let mut controller = lock_controller(&self.controller);
            if controller.is_main_browser(browser) {
                controller.on_loading_state_change(
                    is_loading != 0,
                    can_go_back != 0,
                    can_go_forward != 0,
                );
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
            let controller = lock_controller(&self.controller);
            if !controller.is_main_browser(browser) {
                return;
            }
            drop(controller);

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

            let control = event.modifiers
                & sys::cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0 as u32
                != 0;
            let alt = event.modifiers & sys::cef_event_flags_t::EVENTFLAG_ALT_DOWN.0 as u32 != 0;
            let window = lock_controller(&self.controller).window();
            match event.windows_key_code {
                key if key == 'L' as i32 && control => unsafe { platform::focus_address(window) },
                key if key == 'R' as i32 && control => BrowserController::reload(&self.controller),
                key if key == VK_F5 as i32 => BrowserController::reload(&self.controller),
                key if key == VK_LEFT as i32 && alt => BrowserController::go_back(&self.controller),
                key if key == VK_RIGHT as i32 && alt => {
                    BrowserController::go_forward(&self.controller)
                }
                key if key == VK_ESCAPE as i32 => BrowserController::stop_loading(&self.controller),
                key if key == VK_F12 as i32 => BrowserController::show_dev_tools(&self.controller),
                _ => return 0,
            }
            1
        }
    }
}
