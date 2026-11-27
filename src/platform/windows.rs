use crate::browser::{
    BrowserController, DownloadSnapshot, DownloadStatus, SharedController, TabSnapshot,
    lock_controller,
};
use crate::{
    extensions::ExtensionInfo,
    storage,
    ui::{self, pages},
    update,
};
use std::{
    collections::HashSet,
    ffi::c_void,
    iter, mem,
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        COLORREF, ERROR_CLASS_ALREADY_EXISTS, FreeLibrary, GetLastError, HINSTANCE, HWND, LPARAM,
        LRESULT, POINT, RECT, WPARAM,
    },
    Graphics::{
        Dwm::{DwmFlush, DwmSetWindowAttribute},
        Gdi::*,
    },
    System::{
        Com::CoTaskMemFree,
        LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW},
        Threading::GetCurrentThreadId,
    },
    UI::{
        Controls::{
            Dialogs::{
                GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
                OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
            },
            EM_SETSEL, WM_MOUSELEAVE,
        },
        Input::KeyboardAndMouse::*,
        Shell::*,
        WindowsAndMessaging::*,
    },
};

pub const TAB_HEIGHT: i32 = 54;
pub const TOOLBAR_HEIGHT: i32 = 114;
const TAB_TOP: i32 = 10;
const TAB_BOTTOM: i32 = 50;
const TAB_MAX_WIDTH: f32 = 160.0;
const EDGE_MARGIN: i32 = 16;
const BOOKMARKS_HEIGHT: i32 = 36;
const TOOLBAR_GAP: i32 = 8;
const INITIAL_WIDTH: i32 = 1200;
const INITIAL_HEIGHT: i32 = 800;
const MIN_WIDTH: i32 = 680;
const MIN_HEIGHT: i32 = 480;
const BACKGROUND: COLORREF = rgb(0xF1, 0xF2, 0xEC);
const SURFACE: COLORREF = rgb(0xFC, 0xFC, 0xFA);
const FOREGROUND: COLORREF = rgb(0x20, 0x22, 0x1D);
const SECONDARY: COLORREF = rgb(0x7C, 0x80, 0x73);
const DIVIDER: COLORREF = rgb(0xDE, 0xE0, 0xD7);
const HOVER: COLORREF = rgb(0xE7, 0xE9, 0xE0);
const ACCENT: COLORREF = rgb(0x72, 0x7C, 0x45);
const WARNING: COLORREF = rgb(0xB2, 0x6A, 0x28);
const CLOSE_HOVER: COLORREF = rgb(0xC5, 0x42, 0x3B);
const ID_ADDRESS: usize = 103;
const ICON_ID: usize = 1;
const TAB_ICON_ID: usize = 2;
const TAB_LIGHT_ICON_ID: usize = 3;
const UPDATE_TIMER: usize = 2;
const REVEAL_TIMER: usize = 3;
const FAVICON_TIMER: usize = 4;
const WM_CLOSE_TAB: u32 = WM_APP + 1;
const WM_FINALIZE_TAB: u32 = WM_APP + 2;
const WM_DESTROY_TAB: u32 = WM_APP + 3;
const WM_CLOSE_PANELS: u32 = WM_APP + 4;
const WM_COMMIT_CLOSE_TAB: u32 = WM_APP + 5;
const WM_ANIMATION_FRAME: u32 = WM_APP + 6;
const WM_CLOSE_ORPHANS: u32 = WM_APP + 7;
const WM_EXTENSION_INSTALLED: u32 = WM_APP + 8;
const DWMWA_TRANSITIONS_FORCEDISABLED: u32 = 3;
const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
const DWMWA_BORDER_COLOR: u32 = 34;
const DWMWCP_DONOTROUND: u32 = 1;
const DWMWCP_ROUND: u32 = 2;
const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFF;
const MENU_PRIVATE: usize = 2001;
const MENU_HISTORY: usize = 2002;
const MENU_DOWNLOAD_PROMPT: usize = 2003;
const MENU_RESTORE: usize = 2004;
const MENU_CLEAR_HOUR: usize = 2005;
const MENU_CLEAR_DAY: usize = 2006;
const MENU_CLEAR_ALL: usize = 2007;
const MENU_CLEAR_SITE: usize = 2008;
const MENU_BOOKMARKS: usize = 2009;
const MENU_UPDATE: usize = 2010;
const MENU_PROFILE: usize = 2011;
const MENU_SETTINGS: usize = 2012;
const MENU_HISTORY_PAGE: usize = 2013;
const MENU_ZOOM: usize = 2014;
const MENU_ZOOM_MINUS: usize = 2015;
const MENU_ZOOM_RESET: usize = 2016;
const MENU_ZOOM_PLUS: usize = 2017;
const MENU_FULLSCREEN: usize = 2018;
const MENU_PIN: usize = 2101;
const MENU_CLOSE_TAB: usize = 2102;
const MENU_REMOVE_BOOKMARK: usize = 2201;
static OPEN_WINDOWS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    None,
    Back,
    Forward,
    Reload,
    Home,
    NewTab,
    Downloads,
    Extensions,
    Extension(usize),
    Menu,
    Minimize,
    Maximize,
    CloseWindow,
    BookmarkToggle,
    Bookmark(usize),
    Tab(u64),
    TabClose(u64),
}

const ANIMATION_DURATION: Duration = Duration::from_millis(120);

#[derive(Clone, Copy)]
struct Transition {
    value: f32,
    from: f32,
    target: f32,
    started: Instant,
}

impl Transition {
    fn new(value: f32) -> Self {
        Self {
            value,
            from: value,
            target: value,
            started: Instant::now(),
        }
    }

    fn set_target(&mut self, target: f32) {
        if self.target == target {
            return;
        }
        self.from = self.value;
        self.target = target;
        self.started = Instant::now();
    }

    fn snap(&mut self, value: f32) {
        self.value = value;
        self.from = value;
        self.target = value;
    }

    fn is_active(&self) -> bool {
        self.value != self.target
    }

    fn step(&mut self, now: Instant) -> bool {
        if !self.is_active() {
            return false;
        }
        let progress = (now.duration_since(self.started).as_secs_f32()
            / ANIMATION_DURATION.as_secs_f32())
        .clamp(0.0, 1.0);
        self.value = self.from + (self.target - self.from) * progress;
        if progress >= 1.0 {
            self.snap(self.target);
        }
        self.is_active()
    }
}

struct TabVisual {
    tab: TabSnapshot,
    x: Transition,
    width: Transition,
    closing: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PanelKind {
    Downloads,
    Extensions,
    Menu,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExtensionHit {
    None,
    Action(usize),
    Pin(usize),
    Manage,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DownloadHit {
    None,
    Close,
    Action(usize),
    Cancel(usize),
}

#[derive(Clone, Copy)]
struct Palette {
    background: COLORREF,
    surface: COLORREF,
    foreground: COLORREF,
    secondary: COLORREF,
    divider: COLORREF,
    hover: COLORREF,
    accent: COLORREF,
}

impl Palette {
    fn from_preferences(preferences: &storage::Preferences) -> Self {
        let dark = preferences.theme == storage::Theme::Dark;
        Self {
            background: if dark {
                rgb(0x17, 0x19, 0x17)
            } else {
                BACKGROUND
            },
            surface: if dark { rgb(0x22, 0x25, 0x21) } else { SURFACE },
            foreground: if dark {
                rgb(0xEC, 0xEE, 0xE8)
            } else {
                FOREGROUND
            },
            secondary: if dark {
                rgb(0xA2, 0xA8, 0x9C)
            } else {
                SECONDARY
            },
            divider: if dark { rgb(0x3C, 0x42, 0x39) } else { DIVIDER },
            hover: if dark { rgb(0x32, 0x37, 0x2F) } else { HOVER },
            accent: match preferences.accent {
                storage::Accent::Olive => ACCENT,
                storage::Accent::Blue => rgb(0x52, 0x7A, 0x9E),
                storage::Accent::Graphite => rgb(0x66, 0x6B, 0x70),
            },
        }
    }
}

struct NativeUi {
    controller: SharedController,
    palette: Palette,
    reduce_animations: bool,
    window: HWND,
    address: HWND,
    overlay_window: HWND,
    mouse_hook: HHOOK,
    background_brush: HBRUSH,
    surface_brush: HBRUSH,
    font: HFONT,
    small_font: HFONT,
    title_font: HFONT,
    logo_icon: HICON,
    dark_theme: bool,
    english: bool,
    current_url: String,
    address_editing: bool,
    is_loading: bool,
    can_go_back: bool,
    can_go_forward: bool,
    tabs: Vec<TabVisual>,
    downloads: Vec<DownloadSnapshot>,
    seen_downloads: HashSet<u32>,
    download_hover: DownloadHit,
    previous_download_hover: DownloadHit,
    download_hover_transition: Transition,
    bookmarks: Vec<storage::Bookmark>,
    bookmarked: bool,
    show_bookmarks: bool,
    extensions: Vec<ExtensionInfo>,
    extension_hover: ExtensionHit,
    panel: Option<PanelKind>,
    panel_open: bool,
    panel_visible: bool,
    panel_alpha: Transition,
    menu_hover: usize,
    update_status: update::Status,
    update_timer_running: bool,
    hover: Hit,
    pressed: Hit,
    drag_tab: Option<u64>,
    drag_origin: POINT,
    dragging: bool,
    animation_running: bool,
    frame_clock: Arc<FrameClock>,
    rounded_by_dwm: bool,
    fullscreen: bool,
    restore_maximized: bool,
    restore_rect: RECT,
    counted: bool,
}

impl NativeUi {
    fn new(controller: SharedController) -> Self {
        let (bookmarks, bookmarked, preferences, extensions) = {
            let state = lock_controller(&controller);
            (
                state.bookmarks_snapshot(),
                state.active_is_bookmarked(),
                state.preferences(),
                state.extensions_snapshot(),
            )
        };
        Self {
            controller,
            palette: Palette::from_preferences(&preferences),
            reduce_animations: preferences.reduce_animations,
            show_bookmarks: preferences.show_bookmarks,
            window: ptr::null_mut(),
            address: ptr::null_mut(),
            overlay_window: ptr::null_mut(),
            mouse_hook: ptr::null_mut(),
            background_brush: ptr::null_mut(),
            surface_brush: ptr::null_mut(),
            font: ptr::null_mut(),
            small_font: ptr::null_mut(),
            title_font: ptr::null_mut(),
            logo_icon: ptr::null_mut(),
            dark_theme: preferences.theme == storage::Theme::Dark,
            english: preferences.language == storage::Language::English,
            current_url: String::new(),
            address_editing: false,
            is_loading: false,
            can_go_back: false,
            can_go_forward: false,
            tabs: Vec::new(),
            downloads: Vec::new(),
            seen_downloads: HashSet::new(),
            download_hover: DownloadHit::None,
            previous_download_hover: DownloadHit::None,
            download_hover_transition: Transition::new(0.0),
            bookmarks,
            bookmarked,
            extensions,
            extension_hover: ExtensionHit::None,
            panel: None,
            panel_open: false,
            panel_visible: false,
            panel_alpha: Transition::new(0.0),
            menu_hover: 0,
            update_status: update::snapshot(),
            update_timer_running: false,
            hover: Hit::None,
            pressed: Hit::None,
            drag_tab: None,
            drag_origin: POINT::default(),
            dragging: false,
            animation_running: false,
            frame_clock: Arc::default(),
            rounded_by_dwm: true,
            fullscreen: false,
            restore_maximized: false,
            restore_rect: RECT::default(),
            counted: false,
        }
    }

    unsafe fn create_controls(&mut self, instance: HINSTANCE) -> Result<(), String> {
        unsafe {
            self.background_brush = CreateSolidBrush(self.palette.background);
            self.surface_brush = CreateSolidBrush(self.palette.surface);
            self.font = create_font(15, "Segoe UI");
            self.small_font = create_font(12, "Segoe UI");
            self.title_font = create_font(13, "Segoe UI");
            if self.background_brush.is_null()
                || self.surface_brush.is_null()
                || self.font.is_null()
                || self.small_font.is_null()
                || self.title_font.is_null()
            {
                return Err("Не удалось создать графические ресурсы панели.".to_owned());
            }
            self.address = create_child(
                "EDIT",
                "",
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                instance,
                self.window,
                ID_ADDRESS,
            )?;
            self.overlay_window = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                wide("KockersBrowserOverlay").as_ptr(),
                ptr::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                self.window,
                ptr::null_mut(),
                instance,
                ptr::null(),
            );
            if self.overlay_window.is_null() {
                return Err(format_windows_error("Не удалось создать панель загрузок"));
            }
            SendMessageW(self.address, WM_SETFONT, self.font as usize, 1);
            subclass_control(self.address);
            if !self.reload_logo_icon() {
                return Err("Не удалось загрузить встроенный логотип K.".to_owned());
            }
            self.refresh_tabs();
            self.layout();
        }
        Ok(())
    }

    fn reload_logo_icon(&mut self) -> bool {
        unsafe {
            let instance = GetModuleHandleW(wide("kockers_browser.dll").as_ptr());
            let icon = LoadImageW(
                instance,
                if self.dark_theme {
                    TAB_LIGHT_ICON_ID
                } else {
                    TAB_ICON_ID
                } as *const u16,
                IMAGE_ICON,
                28,
                28,
                LR_DEFAULTCOLOR,
            );
            if icon.is_null() {
                return false;
            }
            if !self.logo_icon.is_null() {
                DestroyIcon(self.logo_icon);
            }
            self.logo_icon = icon;
            true
        }
    }

    fn refresh_tabs(&mut self) -> bool {
        let snapshots = lock_controller(&self.controller).tabs_snapshot();
        if snapshots
            .iter()
            .eq(self.tabs.iter().map(|visual| &visual.tab))
        {
            return false;
        }
        let mut visuals = Vec::with_capacity(snapshots.len());
        for tab in snapshots {
            let previous = self.tabs.iter().find(|visual| visual.tab.id == tab.id);
            visuals.push(TabVisual {
                tab,
                x: previous
                    .map(|visual| visual.x)
                    .unwrap_or_else(|| Transition::new(0.0)),
                width: previous
                    .map(|visual| visual.width)
                    .unwrap_or_else(|| Transition::new(0.0)),
                closing: previous.is_some_and(|visual| visual.closing),
            });
        }
        self.tabs = visuals;
        self.layout_tabs();
        self.update_animation_timer();
        true
    }

    fn layout_tabs(&mut self) {
        let (width, _) = unsafe { client_size(self.window) };
        let origin = 64.0;
        let available = (width as f32 - 268.0).max(80.0);
        let pinned = self.tabs.iter().filter(|visual| visual.tab.pinned).count();
        let normal = self.tabs.len().saturating_sub(pinned);
        let gap = if self.tabs.is_empty() { 0.0 } else { 5.0 };
        let pinned_width = if pinned == 0 { 0.0 } else { 46.0 };
        let normal_width = if normal == 0 {
            0.0
        } else {
            ((available - pinned as f32 * (pinned_width + gap) - normal as f32 * gap)
                / normal as f32)
                .clamp(1.0, TAB_MAX_WIDTH)
        };
        let mut target = origin;
        for visual in &mut self.tabs {
            let width = if visual.closing {
                0.0
            } else if visual.tab.pinned {
                pinned_width
            } else {
                normal_width
            };
            visual.width.set_target(width);
            if visual.x.value == 0.0 && visual.x.target == 0.0 {
                visual.x.snap(target);
            }
            visual.x.set_target(target);
            if self.reduce_animations {
                visual.x.snap(target);
                visual.width.snap(width);
            }
            target += visual.width.target + if visual.closing { 0.0 } else { gap };
        }
    }

    fn layout_address(&self) {
        if self.address.is_null() {
            return;
        }
        let (width, _) = unsafe { client_size(self.window) };
        let address = address_rect(width, self.visible_pinned_extensions());
        let (left_pad, right_pad) = if self.current_url.starts_with("http://") {
            (32, 76)
        } else {
            (16, 60)
        };
        unsafe {
            move_window_if_needed(
                self.address,
                address.left + left_pad,
                address.top + 10,
                (address.right - address.left - right_pad).max(60),
                24,
                1,
            );
        }
    }

    fn layout(&mut self) {
        if self.address.is_null() {
            return;
        }
        self.layout_tabs();
        self.layout_address();
        self.layout_panel();
        BrowserController::resize_content(&self.controller);
        self.update_window_rounding();
    }

    fn update_dwm_transitions(&self) {
        let disabled: u32 = u32::from(self.reduce_animations);
        unsafe {
            DwmSetWindowAttribute(
                self.window,
                DWMWA_TRANSITIONS_FORCEDISABLED,
                &disabled as *const u32 as *const c_void,
                mem::size_of::<u32>() as u32,
            );
        }
    }

    fn update_window_rounding(&mut self) {
        unsafe {
            let (width, height) = client_size(self.window);
            let zoomed = IsZoomed(self.window) != 0;
            let preference = if self.fullscreen || zoomed {
                DWMWCP_DONOTROUND
            } else {
                DWMWCP_ROUND
            };
            if self.rounded_by_dwm {
                DwmSetWindowAttribute(
                    self.window,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &preference as *const u32 as *const c_void,
                    mem::size_of::<u32>() as u32,
                );
            } else if self.fullscreen || zoomed {
                SetWindowRgn(self.window, ptr::null_mut(), 1);
            } else {
                let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, 16, 16);
                if !region.is_null() {
                    SetWindowRgn(self.window, region, 1);
                }
            }
        }
    }

    fn invalidate_toolbar(&self) {
        self.invalidate_area(0, self.content_top());
    }

    fn invalidate_tab_strip(&self) {
        self.invalidate_area(0, TAB_HEIGHT);
    }

    fn invalidate_navigation(&self) {
        self.invalidate_area(TAB_HEIGHT, TOOLBAR_HEIGHT);
    }

    fn invalidate_area(&self, top: i32, bottom: i32) {
        let (width, _) = unsafe { client_size(self.window) };
        let rect = RECT {
            left: 0,
            top,
            right: width,
            bottom,
        };
        if rect.bottom > rect.top {
            unsafe { InvalidateRect(self.window, &rect, 0) };
        }
    }

    fn content_top(&self) -> i32 {
        if self.fullscreen {
            0
        } else {
            TOOLBAR_HEIGHT
                + if self.show_bookmarks {
                    BOOKMARKS_HEIGHT
                } else {
                    0
                }
        }
    }

    fn visible_pinned_extensions(&self) -> usize {
        let (width, _) = unsafe { client_size(self.window) };
        let pinned = self
            .extensions
            .iter()
            .filter(|extension| extension.pinned)
            .count();
        let fitting = ((width - 172 - 296) / 44).max(0) as usize;
        pinned.min(fitting)
    }

    fn pinned_extension_rects(&self) -> Vec<(usize, RECT)> {
        let (width, _) = unsafe { client_size(self.window) };
        let visible = self.visible_pinned_extensions();
        self.extensions
            .iter()
            .enumerate()
            .filter(|(_, extension)| extension.pinned)
            .take(visible)
            .enumerate()
            .map(|(position, (index, _))| {
                (
                    index,
                    toolbar_rect(width - 160 - (visible - position) as i32 * 44),
                )
            })
            .collect()
    }

    fn bookmark_rects(&self) -> Vec<RECT> {
        let (width, _) = unsafe { client_size(self.window) };
        let item_width = 150.min((width - EDGE_MARGIN * 2).max(80));
        let mut rects = Vec::new();
        for index in 0..self.bookmarks.len() {
            let left = EDGE_MARGIN + index as i32 * (item_width + 6);
            if left + item_width > width - EDGE_MARGIN {
                break;
            }
            rects.push(RECT {
                left,
                top: TOOLBAR_HEIGHT + 3,
                right: left + item_width,
                bottom: TOOLBAR_HEIGHT + BOOKMARKS_HEIGHT - 3,
            });
        }
        rects
    }

    fn panel_rect(&self) -> RECT {
        let (width, height) = unsafe { client_size(self.window) };
        let content_top = self.content_top();
        let (panel_width, desired_height) = match self.panel.unwrap_or(PanelKind::Downloads) {
            PanelKind::Downloads => (
                400.min(width - 24),
                if self.downloads.is_empty() {
                    102
                } else {
                    54 + self.downloads.len() as i32 * 74
                },
            ),
            PanelKind::Extensions => (
                380.min(width - 24),
                54 + self.extensions.len().max(1) as i32 * 48 + 44,
            ),
            PanelKind::Menu => (360.min(width - 24), main_menu_height()),
        };
        let panel_height = desired_height.min((height - content_top - 12).max(90));
        let left = width - panel_width - 12;
        RECT {
            left,
            top: content_top,
            right: left + panel_width,
            bottom: content_top + panel_height,
        }
    }

    fn layout_panel(&mut self) {
        if self.overlay_window.is_null() {
            return;
        }
        let rect = self.panel_rect();
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let size_changed = unsafe { client_size(self.overlay_window) } != (width, height);
        unsafe {
            let mut origin = POINT {
                x: rect.left,
                y: rect.top,
            };
            ClientToScreen(self.window, &mut origin);
            move_window_if_needed(self.overlay_window, origin.x, origin.y, width, height, 0);
            if size_changed {
                let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, 32, 32);
                if !region.is_null() {
                    SetWindowRgn(self.overlay_window, region, 1);
                }
                InvalidateRect(self.overlay_window, ptr::null(), 0);
            }
        }
    }

    fn update_address(&mut self, url: &str) {
        let was_http = self.current_url.starts_with("http://");
        if pages::is_internal_page_url(url) {
            self.current_url.clear();
            if !self.address_editing {
                set_window_text(self.address, "");
            }
        } else if !url.starts_with("data:") {
            self.current_url = url.to_owned();
            if !self.address_editing {
                set_window_text(self.address, url);
            }
            self.address_editing = false;
        }
        if was_http != self.current_url.starts_with("http://") {
            self.layout_address();
        }
        self.bookmarked = lock_controller(&self.controller).active_is_bookmarked();
        self.close_panels();
        unsafe { InvalidateRect(self.address, ptr::null(), 1) };
        self.invalidate_navigation();
    }

    fn refresh_bookmarks(&mut self) {
        let (bookmarks, bookmarked, preferences) = {
            let state = lock_controller(&self.controller);
            (
                state.bookmarks_snapshot(),
                state.active_is_bookmarked(),
                state.preferences(),
            )
        };
        self.bookmarks = bookmarks;
        let has_pending_favicons = {
            let state = lock_controller(&self.controller);
            for bookmark in &self.bookmarks {
                state.request_favicon(&bookmark.url, &[]);
            }
            state.has_pending_favicons()
        };
        if has_pending_favicons {
            unsafe { SetTimer(self.window, FAVICON_TIMER, 100, None) };
        }
        self.bookmarked = bookmarked;
        self.show_bookmarks = preferences.show_bookmarks;
        self.english = preferences.language == storage::Language::English;
        self.palette = Palette::from_preferences(&preferences);
        let dark_theme = preferences.theme == storage::Theme::Dark;
        if self.dark_theme != dark_theme {
            self.dark_theme = dark_theme;
            self.reload_logo_icon();
        }
        self.reduce_animations = preferences.reduce_animations;
        self.update_dwm_transitions();
        unsafe {
            let background = CreateSolidBrush(self.palette.background);
            let surface = CreateSolidBrush(self.palette.surface);
            if !background.is_null() && !surface.is_null() {
                DeleteObject(self.background_brush as HGDIOBJ);
                DeleteObject(self.surface_brush as HGDIOBJ);
                self.background_brush = background;
                self.surface_brush = surface;
            } else {
                DeleteObject(background as HGDIOBJ);
                DeleteObject(surface as HGDIOBJ);
            }
        }
        if self.reduce_animations {
            for visual in &mut self.tabs {
                visual.x.snap(visual.x.target);
                visual.width.snap(visual.width.target);
            }
            if self.panel_alpha.is_active() {
                self.panel_alpha.snap(self.panel_alpha.target);
                if self.panel_open {
                    unsafe {
                        SetLayeredWindowAttributes(self.overlay_window, 0, 255, LWA_ALPHA);
                    }
                } else {
                    self.finish_close_panel();
                }
            }
        }
        self.hover = Hit::None;
        self.layout();
        self.update_animation_timer();
        unsafe {
            InvalidateRect(self.window, ptr::null(), 0);
            InvalidateRect(self.overlay_window, ptr::null(), 0);
        }
    }

    fn sync_active(&mut self) {
        let (url, loading, back, forward) = lock_controller(&self.controller).active_state();
        self.can_go_back = back;
        self.can_go_forward = forward;
        self.is_loading = loading;
        self.address_editing = false;
        self.update_address(&url);
    }

    fn update_loading_state(&mut self, loading: bool, back: bool, forward: bool) {
        self.is_loading = loading;
        self.can_go_back = back;
        self.can_go_forward = forward;
        self.invalidate_tab_strip();
        self.invalidate_navigation();
    }

    fn update_downloads(&mut self) {
        let snapshots = lock_controller(&self.controller).downloads_snapshot();
        let is_new = snapshots
            .iter()
            .any(|download| !self.seen_downloads.contains(&download.id));
        self.seen_downloads
            .extend(snapshots.iter().map(|download| download.id));
        if is_new && !(self.panel_open && self.panel == Some(PanelKind::Downloads)) {
            self.downloads = snapshots;
            self.toggle_panel(PanelKind::Downloads);
        } else if self.panel_open && self.panel == Some(PanelKind::Downloads) {
            self.downloads = snapshots;
            self.layout_panel();
            unsafe { InvalidateRect(self.overlay_window, ptr::null(), 1) };
        }
    }

    fn update_download_hover(&mut self, hit: DownloadHit) {
        if hit == self.download_hover {
            return;
        }
        self.previous_download_hover = self.download_hover;
        self.download_hover = hit;
        if hit == DownloadHit::None {
            self.download_hover_transition.set_target(0.0);
        } else {
            self.download_hover_transition.snap(0.0);
            self.download_hover_transition.set_target(1.0);
        }
        if self.reduce_animations {
            self.download_hover_transition
                .snap(self.download_hover_transition.target);
        }
        self.update_animation_timer();
        unsafe { InvalidateRect(self.overlay_window, ptr::null(), 0) };
    }

    fn refresh_extensions(&mut self) {
        self.extensions = lock_controller(&self.controller).extensions_snapshot();
        self.layout();
        self.invalidate_toolbar();
        if self.panel == Some(PanelKind::Extensions) {
            self.layout_panel();
            unsafe { InvalidateRect(self.overlay_window, ptr::null(), 0) };
        }
    }

    fn toggle_downloads(&mut self) {
        self.toggle_panel(PanelKind::Downloads);
    }

    fn toggle_extensions(&mut self) {
        self.extensions = lock_controller(&self.controller).extensions_snapshot();
        self.toggle_panel(PanelKind::Extensions);
    }

    fn toggle_menu(&mut self) {
        self.toggle_panel(PanelKind::Menu);
    }

    fn toggle_panel(&mut self, panel: PanelKind) {
        if self.panel_open && self.panel == Some(panel) {
            self.close_panels();
            return;
        }
        match panel {
            PanelKind::Downloads => {
                self.downloads = lock_controller(&self.controller).downloads_snapshot();
            }
            PanelKind::Extensions => {
                self.extension_hover = ExtensionHit::None;
                self.downloads = Vec::new();
            }
            PanelKind::Menu => {
                self.menu_hover = 0;
                self.downloads = Vec::new();
            }
        }
        self.panel = Some(panel);
        self.panel_open = true;
        self.panel_visible = true;
        self.refresh_update();
        self.layout_panel();
        unsafe {
            SetWindowPos(
                self.overlay_window,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            if self.reduce_animations {
                self.panel_alpha.snap(1.0);
                SetLayeredWindowAttributes(self.overlay_window, 0, 255, LWA_ALPHA);
                ShowWindow(self.overlay_window, SW_SHOWNOACTIVATE);
                InvalidateRect(self.overlay_window, ptr::null(), 0);
            } else {
                if IsWindowVisible(self.overlay_window) == 0 {
                    self.panel_alpha.snap(0.0);
                    SetLayeredWindowAttributes(self.overlay_window, 0, 0, LWA_ALPHA);
                    ShowWindow(self.overlay_window, SW_SHOWNOACTIVATE);
                }
                self.panel_alpha.set_target(1.0);
                InvalidateRect(self.overlay_window, ptr::null(), 0);
                self.update_animation_timer();
            }
        }
    }

    fn finish_close_panel(&mut self) {
        unsafe {
            ShowWindow(self.overlay_window, SW_HIDE);
        }
        self.panel_visible = false;
        if self.panel == Some(PanelKind::Downloads) {
            self.downloads = Vec::new();
            self.download_hover = DownloadHit::None;
            self.previous_download_hover = DownloadHit::None;
            self.download_hover_transition.snap(0.0);
        } else if self.panel == Some(PanelKind::Extensions) {
            self.extension_hover = ExtensionHit::None;
        } else if self.panel == Some(PanelKind::Menu) {
            self.menu_hover = 0;
        }
        self.panel = None;
    }

    fn close_panels(&mut self) {
        if self.panel_visible && self.panel_open {
            self.panel_open = false;
            if self.reduce_animations {
                self.panel_alpha.snap(0.0);
                self.finish_close_panel();
            } else {
                self.panel_alpha.set_target(0.0);
            }
            self.update_animation_timer();
        }
        self.update_update_timer();
    }

    fn close_panels_immediately(&mut self) {
        if self.panel_visible || self.panel_open || self.panel_alpha.is_active() {
            self.panel_open = false;
            self.panel_alpha.snap(0.0);
            self.finish_close_panel();
            self.update_animation_timer();
        }
        self.update_update_timer();
    }

    fn refresh_update(&mut self) {
        let status = update::snapshot();
        if status != self.update_status {
            self.update_status = status;
            if self.panel == Some(PanelKind::Menu) {
                unsafe { InvalidateRect(self.overlay_window, ptr::null(), 0) };
            }
        }
        self.update_update_timer();
    }

    fn update_update_timer(&mut self) {
        let needed = self.panel_open
            && self.panel == Some(PanelKind::Menu)
            && self.update_status.is_active();
        unsafe {
            if needed && !self.update_timer_running {
                SetTimer(self.window, UPDATE_TIMER, 200, None);
                self.update_timer_running = true;
            } else if !needed && self.update_timer_running {
                KillTimer(self.window, UPDATE_TIMER);
                self.update_timer_running = false;
            }
        }
    }

    fn update_animation_timer(&mut self) {
        let needed = self
            .tabs
            .iter()
            .any(|visual| visual.x.is_active() || visual.width.is_active())
            || self.download_hover_transition.is_active()
            || self.panel_alpha.is_active();
        if needed && !self.animation_running {
            self.frame_clock
                .start(self.window, animation_interval(self.window));
            self.animation_running = true;
        } else if !needed && self.animation_running {
            self.frame_clock.stop();
            self.animation_running = false;
        }
    }

    fn animate(&mut self) {
        let now = Instant::now();
        let mut repaint_tabs = false;
        for visual in &mut self.tabs {
            repaint_tabs |= visual.x.step(now) | visual.width.step(now);
        }
        if repaint_tabs {
            let (width, _) = unsafe { client_size(self.window) };
            let rect = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: TAB_HEIGHT,
            };
            unsafe { InvalidateRect(self.window, &rect, 0) };
        }
        let repaint_downloads = self.download_hover_transition.is_active();
        self.download_hover_transition.step(now);
        if repaint_downloads {
            unsafe { InvalidateRect(self.overlay_window, ptr::null(), 0) };
        }
        let panel_animating = self.panel_alpha.is_active();
        self.panel_alpha.step(now);
        if panel_animating {
            let alpha = (self.panel_alpha.value * 255.0).round().clamp(0.0, 255.0) as u8;
            unsafe { SetLayeredWindowAttributes(self.overlay_window, 0, alpha, LWA_ALPHA) };
            if self.panel_alpha.value == 0.0 && !self.panel_open {
                self.finish_close_panel();
            }
        }
        self.update_animation_timer();
        let close = self
            .tabs
            .iter()
            .find(|visual| visual.closing && !visual.width.is_active())
            .map(|visual| visual.tab.id);
        if let Some(id) = close {
            unsafe { PostMessageW(self.window, WM_COMMIT_CLOSE_TAB, id as WPARAM, 0) };
        }
    }

    fn close_tab(&mut self, id: u64) {
        let last = self
            .tabs
            .iter()
            .all(|visual| visual.closing || visual.tab.id == id);
        if last || self.reduce_animations {
            unsafe { PostMessageW(self.window, WM_COMMIT_CLOSE_TAB, id as WPARAM, 0) };
        }
        if self.reduce_animations {
            return;
        }
        let Some(visual) = self.tabs.iter_mut().find(|visual| visual.tab.id == id) else {
            return;
        };
        if visual.closing {
            return;
        }
        visual.closing = true;
        self.layout_tabs();
        self.update_animation_timer();
        let (width, _) = unsafe { client_size(self.window) };
        let rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: TAB_HEIGHT,
        };
        unsafe { InvalidateRect(self.window, &rect, 0) };
    }

    fn submit_address(&mut self) {
        let input = window_text(self.address);
        self.address_editing = false;
        self.close_panels();
        BrowserController::navigate_input(&self.controller, &input);
    }

    fn cancel_address_editing(&mut self) {
        let current_url = self.current_url.clone();
        self.address_editing = false;
        set_window_text(self.address, &current_url);
        BrowserController::stop_loading(&self.controller);
    }

    fn update_hover(&mut self, x: i32, y: i32) {
        let hit = self.hit_test(x, y);
        if hit != self.hover {
            self.hover = hit;
            self.invalidate_toolbar();
        }
        unsafe { track_mouse_leave(self.window) };
    }

    fn hit_test(&self, x: i32, y: i32) -> Hit {
        if self.fullscreen {
            return Hit::None;
        }
        let (width, _) = unsafe { client_size(self.window) };
        if y < TAB_HEIGHT {
            let zoomed = unsafe { IsZoomed(self.window) != 0 };
            let close_rect = caption_button_rect(width, 0);
            let maximize_rect = caption_button_rect(width, 1);
            let minimize_rect = caption_button_rect(width, 2);
            if zoomed && x >= close_rect.left && y < TAB_BOTTOM {
                return Hit::CloseWindow;
            }
            if point_in_rect(x, y, &close_rect) {
                return Hit::CloseWindow;
            }
            if point_in_rect(x, y, &maximize_rect) {
                return Hit::Maximize;
            }
            if point_in_rect(x, y, &minimize_rect) {
                return Hit::Minimize;
            }
            if point_in_rect(x, y, &new_tab_rect(self)) {
                return Hit::NewTab;
            }
            for visual in &self.tabs {
                let left = visual.x.value.round() as i32;
                let tab_width = visual.width.value.round() as i32;
                let right = left + tab_width;
                if x >= left && x < right && (TAB_TOP..TAB_BOTTOM).contains(&y) {
                    let hovered = self.hover == Hit::Tab(visual.tab.id)
                        || self.hover == Hit::TabClose(visual.tab.id);
                    let show_close =
                        !visual.tab.pinned && (tab_width >= 72 || visual.tab.active || hovered);
                    if show_close && x >= right - 32 {
                        return Hit::TabClose(visual.tab.id);
                    }
                    return Hit::Tab(visual.tab.id);
                }
            }
        } else if (TAB_HEIGHT..TOOLBAR_HEIGHT).contains(&y) {
            if point_in_rect(
                x,
                y,
                &bookmark_toggle_rect(width, self.visible_pinned_extensions()),
            ) {
                return Hit::BookmarkToggle;
            }
            for (index, rect) in self.pinned_extension_rects() {
                if point_in_rect(x, y, &rect) {
                    return Hit::Extension(index);
                }
            }
            for (hit, rect) in [
                (Hit::Back, toolbar_rect(16)),
                (Hit::Forward, toolbar_rect(60)),
                (Hit::Reload, toolbar_rect(104)),
                (Hit::Home, toolbar_rect(148)),
                (Hit::Extensions, toolbar_rect(width - 160)),
                (Hit::Downloads, toolbar_rect(width - 108)),
                (Hit::Menu, toolbar_rect(width - 56)),
            ] {
                if point_in_rect(x, y, &rect) {
                    return hit;
                }
            }
        } else if self.show_bookmarks
            && (TOOLBAR_HEIGHT..TOOLBAR_HEIGHT + BOOKMARKS_HEIGHT).contains(&y)
        {
            for (index, rect) in self.bookmark_rects().into_iter().enumerate() {
                if point_in_rect(x, y, &rect) {
                    return Hit::Bookmark(index);
                }
            }
        }
        Hit::None
    }

    fn activate_hit(&mut self, hit: Hit) {
        match hit {
            Hit::Back => BrowserController::go_back(&self.controller),
            Hit::Forward => BrowserController::go_forward(&self.controller),
            Hit::Reload => BrowserController::reload_or_stop(&self.controller),
            Hit::Home => BrowserController::go_home(&self.controller),
            Hit::NewTab => {
                let url = lock_controller(&self.controller).start_page_url();
                BrowserController::new_tab(&self.controller, &url, false);
            }
            Hit::Downloads => self.toggle_downloads(),
            Hit::Extensions => self.toggle_extensions(),
            Hit::Extension(index) => {
                if let Some(id) = self
                    .extensions
                    .get(index)
                    .map(|extension| extension.id.clone())
                {
                    BrowserController::open_extension(&self.controller, &id);
                }
            }
            Hit::Menu => self.toggle_menu(),
            Hit::Minimize => unsafe {
                PostMessageW(self.window, WM_SYSCOMMAND, SC_MINIMIZE as WPARAM, 0);
            },
            Hit::Maximize => unsafe {
                PostMessageW(
                    self.window,
                    WM_SYSCOMMAND,
                    if IsZoomed(self.window) != 0 {
                        SC_RESTORE
                    } else {
                        SC_MAXIMIZE
                    } as WPARAM,
                    0,
                );
            },
            Hit::CloseWindow => unsafe {
                PostMessageW(self.window, WM_CLOSE, 0, 0);
            },
            Hit::BookmarkToggle => BrowserController::toggle_bookmark(&self.controller),
            Hit::Bookmark(index) => BrowserController::open_bookmark(&self.controller, index),
            Hit::Tab(id) => BrowserController::activate_tab(&self.controller, id),
            Hit::TabClose(id) => unsafe {
                PostMessageW(self.window, WM_CLOSE_TAB, id as WPARAM, 0);
            },
            Hit::None => {}
        }
    }

    unsafe fn destroy_resources(&mut self) {
        unsafe {
            self.frame_clock.stop();
            if !self.mouse_hook.is_null() {
                UnhookWindowsHookEx(self.mouse_hook);
                self.mouse_hook = ptr::null_mut();
            }
            if !self.logo_icon.is_null() {
                DestroyIcon(self.logo_icon);
                self.logo_icon = ptr::null_mut();
            }
            for object in [
                self.background_brush as HGDIOBJ,
                self.surface_brush as HGDIOBJ,
                self.font as HGDIOBJ,
                self.small_font as HGDIOBJ,
                self.title_font as HGDIOBJ,
            ] {
                if !object.is_null() {
                    DeleteObject(object);
                }
            }
        }
    }
}

pub fn create_main_window(controller: SharedController) -> Result<HWND, String> {
    unsafe {
        let instance = GetModuleHandleW(wide("kockers_browser.dll").as_ptr());
        if instance.is_null() {
            return Err("Windows не вернула HMODULE sandbox DLL приложения.".to_owned());
        }
        register_classes(instance)?;
        let icon = LoadImageW(
            instance,
            ICON_ID as *const u16,
            IMAGE_ICON,
            32,
            32,
            LR_DEFAULTCOLOR | LR_SHARED,
        );
        let small_icon = LoadImageW(
            instance,
            ICON_ID as *const u16,
            IMAGE_ICON,
            16,
            16,
            LR_DEFAULTCOLOR | LR_SHARED,
        );
        let private = lock_controller(&controller).is_private();
        let title = if private {
            "Приватное окно — KOCKERS"
        } else {
            "KOCKERS"
        };
        let ui = Box::new(NativeUi::new(controller));
        let ui_ptr = Box::into_raw(ui);
        let window = CreateWindowExW(
            WS_EX_APPWINDOW,
            wide("KockersBrowserWindow").as_ptr(),
            wide(title).as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            INITIAL_WIDTH,
            INITIAL_HEIGHT,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ui_ptr.cast(),
        );
        if window.is_null() {
            drop(Box::from_raw(ui_ptr));
            return Err(format_windows_error("Не удалось создать главное окно"));
        }
        let ui = &mut *ui_ptr;
        ui.window = window;
        lock_controller(&ui.controller).set_window(window);
        if let Err(error) = ui.create_controls(instance) {
            DestroyWindow(window);
            return Err(error);
        }
        ui.mouse_hook =
            SetWindowsHookExW(WH_MOUSE, Some(mouse_proc), instance, GetCurrentThreadId());
        let preference = DWMWCP_ROUND;
        ui.rounded_by_dwm = DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &preference as *const u32 as *const c_void,
            mem::size_of::<u32>() as u32,
        ) >= 0;
        DwmSetWindowAttribute(
            window,
            DWMWA_BORDER_COLOR,
            &DWMWA_COLOR_NONE as *const u32 as *const c_void,
            mem::size_of::<u32>() as u32,
        );
        ui.update_dwm_transitions();
        if !small_icon.is_null() {
            SendMessageW(window, WM_SETICON, ICON_SMALL as usize, small_icon as isize);
        }
        if !icon.is_null() {
            SendMessageW(window, WM_SETICON, ICON_BIG as usize, icon as isize);
        }
        OPEN_WINDOWS.fetch_add(1, Ordering::SeqCst);
        ui.counted = true;
        SetWindowPos(
            window,
            ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
        );
        ShowWindow(window, SW_SHOW);
        UpdateWindow(window);
        Ok(window)
    }
}

unsafe fn register_classes(instance: HINSTANCE) -> Result<(), String> {
    unsafe {
        let class_name = wide("KockersBrowserWindow");
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: LoadImageW(
                instance,
                ICON_ID as *const u16,
                IMAGE_ICON,
                32,
                32,
                LR_DEFAULTCOLOR | LR_SHARED,
            ),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(format_windows_error(
                "Не удалось зарегистрировать класс окна",
            ));
        }
        let overlay_name = wide("KockersBrowserOverlay");
        let overlay = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(overlay_proc),
            hInstance: instance,
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            lpszClassName: overlay_name.as_ptr(),
            ..Default::default()
        };
        if RegisterClassW(&overlay) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(format_windows_error("Не удалось зарегистрировать панель"));
        }
        Ok(())
    }
}

/// # Safety
/// `window` должен быть действительным HWND текущего UI-потока.
pub unsafe fn client_size(window: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    unsafe {
        if GetClientRect(window, &mut rect) == 0 {
            return (INITIAL_WIDTH, INITIAL_HEIGHT);
        }
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

pub fn content_top(window: HWND) -> i32 {
    ui_mut(window).map_or(TOOLBAR_HEIGHT, |ui| ui.content_top())
}

/// # Safety
/// Все HWND должны быть действительными дочерними окнами текущего UI-потока.
pub unsafe fn resize_browsers(window: HWND, browsers: &[(u64, HWND)], active: Option<u64>) {
    let (width, height) = unsafe { client_size(window) };
    let top = content_top(window);
    for (id, browser) in browsers {
        if !browser.is_null() {
            unsafe {
                move_window_if_needed(
                    *browser,
                    0,
                    top,
                    width,
                    (height - top).max(0),
                    i32::from(active == Some(*id)),
                );
            }
        }
    }
}

pub(crate) fn apply_browser_visibility(
    window: HWND,
    browsers: &[(u64, HWND, bool)],
    active: Option<u64>,
) {
    let (width, height) = unsafe { client_size(window) };
    let top = content_top(window);
    let content_height = (height - top).max(0);
    if let Some((_, active_handle, ready)) = browsers
        .iter()
        .find(|(id, handle, _)| active == Some(*id) && !handle.is_null())
    {
        unsafe {
            if *ready {
                SetWindowPos(
                    *active_handle,
                    HWND_TOP,
                    0,
                    top,
                    width,
                    content_height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
                ShowWindow(*active_handle, SW_SHOW);
            } else {
                SetWindowPos(
                    *active_handle,
                    HWND_BOTTOM,
                    0,
                    top,
                    width,
                    content_height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
    }
    for (id, handle, _) in browsers {
        if handle.is_null() || active == Some(*id) {
            continue;
        }
        unsafe {
            ShowWindow(*handle, SW_HIDE);
        }
    }
}

pub(crate) const REVEAL_FALLBACK_MS: u32 = 300;

pub(crate) fn schedule_reveal(window: HWND, delay: u32) {
    unsafe { SetTimer(window, REVEAL_TIMER, delay, None) };
}

pub(crate) fn cancel_reveal(window: HWND) {
    if !window.is_null() {
        unsafe { KillTimer(window, REVEAL_TIMER) };
    }
}

pub(crate) fn schedule_favicon_refresh(window: HWND) {
    unsafe { SetTimer(window, FAVICON_TIMER, 100, None) };
}

pub(crate) fn stage_browser(window: HWND, browser: HWND) {
    let (width, height) = unsafe { client_size(window) };
    let top = content_top(window);
    unsafe {
        SetWindowPos(
            browser,
            HWND_BOTTOM,
            0,
            top,
            width,
            (height - top).max(0),
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

pub(crate) fn refresh_tabs(window: HWND) {
    if let Some(ui) = ui_mut(window)
        && ui.refresh_tabs()
    {
        ui.invalidate_tab_strip();
    }
}

pub fn close_tab(window: HWND, id: u64) {
    if let Some(ui) = ui_mut(window) {
        ui.close_tab(id);
    }
}

pub fn sync_active_tab(window: HWND) {
    if let Some(ui) = ui_mut(window) {
        ui.sync_active();
    }
}

pub fn update_address(window: HWND, url: &str) {
    if let Some(ui) = ui_mut(window) {
        ui.update_address(url);
    }
}

pub fn update_title(window: HWND, title: &str) {
    set_window_text(window, title);
}

/// # Safety
/// `window` должен быть HWND, полученным от текущего процесса.
pub unsafe fn window_is_valid(window: HWND) -> bool {
    !window.is_null() && unsafe { IsWindow(window) != 0 }
}

pub fn update_loading_state(window: HWND, loading: bool, back: bool, forward: bool) {
    if let Some(ui) = ui_mut(window) {
        ui.update_loading_state(loading, back, forward);
    }
}

pub fn update_downloads(window: HWND) {
    if let Some(ui) = ui_mut(window) {
        ui.update_downloads();
    }
}

pub(crate) fn confirm_extension_install(window: HWND, id: &str) -> bool {
    let english = ui_mut(window).is_some_and(|ui| ui.english);
    let message = if english {
        format!("Extension {id} will be installed after restarting Kockers. Restart now?")
    } else {
        format!(
            "Расширение {id} будет установлено после перезапуска Kockers. Перезапустить сейчас?"
        )
    };
    let caption = if english {
        "KOCKERS — Install extension"
    } else {
        "KOCKERS — установка расширения"
    };
    unsafe {
        MessageBoxW(
            window,
            wide(&message).as_ptr(),
            wide(caption).as_ptr(),
            MB_YESNO | MB_ICONQUESTION,
        ) == IDYES
    }
}

pub(crate) fn close_for_restart(window: HWND) {
    if !window.is_null() {
        unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
    }
}

pub fn refresh_bookmarks(window: HWND) {
    if let Some(ui) = ui_mut(window) {
        ui.refresh_bookmarks();
    }
}

pub fn refresh_extensions(window: HWND) {
    if let Some(ui) = ui_mut(window) {
        ui.refresh_extensions();
    }
}

/// # Safety
/// `window` должен быть HWND главного окна текущего UI-потока.
pub unsafe fn focus_address(window: HWND) {
    if let Some(ui) = ui_mut(window) {
        ui.address_editing = false;
        unsafe {
            SetForegroundWindow(window);
            SetFocus(ui.address);
            SendMessageW(ui.address, EM_SETSEL, 0, -1);
        }
    }
}

pub fn finish_address_editing(window: HWND) {
    if let Some(ui) = ui_mut(window) {
        ui.address_editing = false;
    }
}

pub(crate) fn finish_window_close(window: HWND) {
    if !window.is_null() {
        unsafe { DestroyWindow(window) };
    }
}

pub(crate) fn post_tab_closed(window: HWND, id: u64) {
    if !window.is_null() {
        unsafe { PostMessageW(window, WM_FINALIZE_TAB, id as WPARAM, 0) };
    }
}

pub(crate) fn post_browser_destroy(window: HWND, browser: HWND) {
    if !window.is_null() && !browser.is_null() {
        unsafe { PostMessageW(window, WM_DESTROY_TAB, browser as WPARAM, 0) };
    }
}

pub(crate) fn post_close_orphans(window: HWND) {
    if !window.is_null() {
        unsafe { PostMessageW(window, WM_CLOSE_ORPHANS, 0, 0) };
    }
}

pub(crate) fn post_extension_installed(window: HWND, result: Result<String, String>) {
    if window.is_null() {
        return;
    }
    let boxed = Box::into_raw(Box::new(result));
    if unsafe { PostMessageW(window, WM_EXTENSION_INSTALLED, boxed as WPARAM, 0) } == 0 {
        unsafe { drop(Box::from_raw(boxed)) };
    }
}

pub fn show_direct_launch_error() {
    show_fatal_error(
        "Для sandbox-сборки запускайте kockers_browser.exe из каталога target/bundle. Его создаёт команда kockers-bundle.",
    );
}

pub fn show_fatal_error(message: &str) {
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            wide(message).as_ptr(),
            wide("KOCKERS — ошибка").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

pub fn choose_download_path(
    window: HWND,
    name: &str,
    ask: bool,
    directory: &str,
) -> Option<PathBuf> {
    if !ask {
        let directory = if directory.is_empty() {
            storage::downloads_dir()?
        } else {
            PathBuf::from(directory)
        };
        std::fs::create_dir_all(&directory).ok()?;
        return Some(directory.join(name));
    }
    unsafe {
        let mut file = vec![0u16; 32_768];
        let initial = wide(name);
        let copy_len = initial.len().min(file.len() - 1);
        file[..copy_len].copy_from_slice(&initial[..copy_len]);
        let mut filter = "Все файлы".encode_utf16().collect::<Vec<_>>();
        filter.push(0);
        filter.extend("*.*".encode_utf16());
        filter.extend([0, 0]);
        let mut dialog = OPENFILENAMEW {
            lStructSize: mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: window,
            lpstrFilter: filter.as_ptr(),
            lpstrFile: file.as_mut_ptr(),
            nMaxFile: file.len() as u32,
            Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
            ..Default::default()
        };
        if GetSaveFileNameW(&mut dialog) == 0 {
            return None;
        }
        let length = file
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(file.len());
        Some(PathBuf::from(String::from_utf16_lossy(&file[..length])))
    }
}

pub fn choose_image(window: HWND) -> Option<PathBuf> {
    choose_file(
        window,
        "Изображения\0*.png;*.jpg;*.jpeg;*.webp;*.gif\0Все файлы\0*.*\0",
        "",
        false,
    )
}

pub fn import_config(window: HWND) -> Option<String> {
    let path = choose_file(window, "JSON config\0*.json\0Все файлы\0*.*\0", "", false)?;
    let metadata = std::fs::metadata(&path).ok()?;
    if metadata.len() > 1024 * 1024 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

pub fn export_config(window: HWND, contents: &str) -> Result<(), String> {
    let Some(path) = choose_file(
        window,
        "JSON config\0*.json\0Все файлы\0*.*\0",
        "KockersBrowser-config.json",
        true,
    ) else {
        return Ok(());
    };
    std::fs::write(path, contents)
        .map_err(|error| format!("Не удалось сохранить конфигурацию: {error}"))
}

fn choose_file(window: HWND, filter: &str, initial_name: &str, save: bool) -> Option<PathBuf> {
    unsafe {
        let mut file = vec![0u16; 32_768];
        let initial = wide(initial_name);
        let length = initial.len().min(file.len() - 1);
        file[..length].copy_from_slice(&initial[..length]);
        let filter = filter
            .encode_utf16()
            .chain(iter::once(0))
            .collect::<Vec<_>>();
        let mut dialog = OPENFILENAMEW {
            lStructSize: mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: window,
            lpstrFilter: filter.as_ptr(),
            lpstrFile: file.as_mut_ptr(),
            nMaxFile: file.len() as u32,
            Flags: OFN_NOCHANGEDIR
                | OFN_PATHMUSTEXIST
                | if save {
                    OFN_OVERWRITEPROMPT
                } else {
                    OFN_FILEMUSTEXIST
                },
            ..Default::default()
        };
        let accepted = if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        };
        if accepted == 0 {
            return None;
        }
        let length = file
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(file.len());
        Some(PathBuf::from(String::from_utf16_lossy(&file[..length])))
    }
}

pub fn choose_download_folder(window: HWND) -> Option<PathBuf> {
    unsafe {
        let title = wide("Выберите папку для скачанных файлов");
        let info = BROWSEINFOW {
            hwndOwner: window,
            lpszTitle: title.as_ptr(),
            ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
            ..Default::default()
        };
        let item = SHBrowseForFolderW(&info);
        if item.is_null() {
            return None;
        }
        let mut path = vec![0u16; 32_768];
        let copied = SHGetPathFromIDListW(item, path.as_mut_ptr());
        CoTaskMemFree(item.cast());
        if copied == 0 {
            return None;
        }
        let length = path
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(path.len());
        Some(PathBuf::from(String::from_utf16_lossy(&path[..length])))
    }
}

pub fn open_proxy_settings() {
    shell_open("control.exe", "inetcpl.cpl,,4");
}

pub fn open_default_browser_settings() {
    shell_open("ms-settings:defaultapps", "");
}

fn shell_open(target: &str, parameters: &str) {
    unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            wide("open").as_ptr(),
            wide(target).as_ptr(),
            if parameters.is_empty() {
                ptr::null()
            } else {
                wide(parameters).as_ptr()
            },
            ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

pub fn preferences_changed(window: HWND) {
    refresh_bookmarks(window);
}

/// # Safety
/// `window` должен быть действительным HWND главного окна текущего UI-потока.
pub unsafe fn toggle_fullscreen(window: HWND) {
    let Some(ui) = ui_mut(window) else {
        return;
    };
    ui.close_panels();
    unsafe {
        if ui.fullscreen {
            ui.fullscreen = false;
            if ui.restore_maximized {
                ShowWindow(window, SW_MAXIMIZE);
            } else {
                let rect = ui.restore_rect;
                SetWindowPos(
                    window,
                    ptr::null_mut(),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            ui.update_window_rounding();
            ShowWindow(ui.address, SW_SHOW);
        } else {
            ui.restore_maximized = IsZoomed(window) != 0;
            if ui.restore_maximized {
                ShowWindow(window, SW_RESTORE);
            }
            GetWindowRect(window, &mut ui.restore_rect);
            ui.fullscreen = true;
            ui.panel_open = false;
            ui.panel_alpha.snap(0.0);
            ui.finish_close_panel();
            ui.hover = Hit::None;
            ui.pressed = Hit::None;
            ShowWindow(ui.address, SW_HIDE);
            ui.update_window_rounding();
            let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info) != 0 {
                SetWindowPos(
                    window,
                    HWND_TOP,
                    info.rcMonitor.left,
                    info.rcMonitor.top,
                    info.rcMonitor.right - info.rcMonitor.left,
                    info.rcMonitor.bottom - info.rcMonitor.top,
                    SWP_NOACTIVATE,
                );
            }
        }
        ui.update_animation_timer();
        ui.layout();
        ui.invalidate_toolbar();
    }
}

pub(crate) fn open_downloaded_file(path: &Path) {
    let dangerous = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            [
                "exe", "msi", "bat", "cmd", "com", "scr", "ps1", "js", "vbs", "jar", "lnk",
            ]
            .iter()
            .any(|value| extension.eq_ignore_ascii_case(value))
        });
    if dangerous {
        let accepted = unsafe {
            MessageBoxW(
                ptr::null_mut(),
                wide("Этот файл может запустить программу или изменить систему. Открыть его только если вы доверяете источнику.").as_ptr(),
                wide("KOCKERS — предупреждение").as_ptr(),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            ) == IDYES
        };
        if !accepted {
            return;
        }
    }
    unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            wide("open").as_ptr(),
            wide(&path.to_string_lossy()).as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

pub(crate) fn confirm_permission(window: HWND, origin: &str, subject: &str) -> bool {
    unsafe {
        MessageBoxW(
            window,
            wide(&format!("Сайт {origin} запрашивает доступ к {subject}. Разрешить только при доверии к сайту.")).as_ptr(),
            wide("KOCKERS — разрешение сайта").as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        ) == IDYES
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_NCCREATE => {
                let create = &*(lparam as *const CREATESTRUCTW);
                SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
                return 1;
            }
            WM_NCCALCSIZE => {
                if wparam != 0 {
                    if IsZoomed(window) != 0 && !ui_mut(window).is_some_and(|ui| ui.fullscreen) {
                        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
                        let mut monitor_info = MONITORINFO {
                            cbSize: mem::size_of::<MONITORINFO>() as u32,
                            ..Default::default()
                        };
                        if GetMonitorInfoW(monitor, &mut monitor_info) != 0 {
                            let mut work = monitor_info.rcWork;
                            adjust_for_autohide_taskbar(&mut work, &monitor_info.rcMonitor);
                            let params = &mut *(lparam as *mut NCCALCSIZE_PARAMS);
                            params.rgrc[0] = work;
                        }
                    }
                    return 0;
                }
            }
            WM_NCHITTEST => return non_client_hit_test(window, lparam),
            WM_NCPAINT => return 0,
            WM_NCACTIVATE => {
                if IsIconic(window) != 0 {
                    return DefWindowProcW(window, message, wparam, lparam);
                }
                return 1;
            }
            WM_ERASEBKGND => return 1,
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(window, &mut paint);
                if let Some(ui) = ui_mut(window) {
                    let tabs_only = paint.rcPaint.bottom <= TAB_HEIGHT;
                    paint_toolbar(ui, dc, tabs_only);
                    if !tabs_only && paint.rcPaint.bottom > ui.content_top() {
                        let (width, height) = client_size(window);
                        let content_rect = RECT {
                            left: 0,
                            top: ui.content_top(),
                            right: width,
                            bottom: height,
                        };
                        let color = if ui.dark_theme {
                            rgb(0x17, 0x19, 0x17)
                        } else {
                            rgb(0xF7, 0xF7, 0xF4)
                        };
                        let brush = CreateSolidBrush(color);
                        if !brush.is_null() {
                            FillRect(dc, &content_rect, brush);
                            DeleteObject(brush as HGDIOBJ);
                        }
                    }
                }
                EndPaint(window, &paint);
                return 0;
            }
            WM_PRINTCLIENT => {
                if let Some(ui) = ui_mut(window) {
                    paint_toolbar(ui, wparam as HDC, false);
                }
                return 0;
            }
            WM_MOVE => {
                if let Some(ui) = ui_mut(window) {
                    ui.layout_panel();
                }
                return 0;
            }
            WM_SIZE => {
                if let Some(ui) = ui_mut(window) {
                    if wparam == SIZE_MINIMIZED as usize {
                        ui.close_panels_immediately();
                        return 0;
                    }
                    ui.layout();
                    ui.invalidate_tab_strip();
                }
                return 0;
            }
            WM_DPICHANGED => {
                if let Some(ui) = ui_mut(window) {
                    ui.layout();
                }
                let suggested = &*(lparam as *const RECT);
                SetWindowPos(
                    window,
                    ptr::null_mut(),
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                return 0;
            }
            WM_GETMINMAXINFO => {
                let info = &mut *(lparam as *mut MINMAXINFO);
                info.ptMinTrackSize.x = MIN_WIDTH;
                info.ptMinTrackSize.y = MIN_HEIGHT;
                let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
                let mut monitor_info = MONITORINFO {
                    cbSize: mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if GetMonitorInfoW(monitor, &mut monitor_info) != 0 {
                    let monitor_rect = monitor_info.rcMonitor;
                    if ui_mut(window).is_some_and(|ui| ui.fullscreen) {
                        info.ptMaxPosition.x = 0;
                        info.ptMaxPosition.y = 0;
                        info.ptMaxSize.x = monitor_rect.right - monitor_rect.left;
                        info.ptMaxSize.y = monitor_rect.bottom - monitor_rect.top;
                    } else {
                        let work = monitor_info.rcWork;
                        info.ptMaxPosition.x = work.left - monitor_rect.left;
                        info.ptMaxPosition.y = work.top - monitor_rect.top;
                        info.ptMaxSize.x = work.right - work.left;
                        info.ptMaxSize.y = work.bottom - work.top;
                    }
                    info.ptMaxTrackSize = info.ptMaxSize;
                }
                return 0;
            }
            WM_ANIMATION_FRAME => {
                if let Some(ui) = ui_mut(window) {
                    ui.frame_clock.frame_handled();
                    ui.animate();
                }
                UpdateWindow(window);
                return 0;
            }
            WM_TIMER if wparam == UPDATE_TIMER => {
                if let Some(ui) = ui_mut(window) {
                    ui.refresh_update();
                }
                return 0;
            }
            WM_TIMER if wparam == FAVICON_TIMER => {
                if let Some(ui) = ui_mut(window) {
                    if ui.refresh_tabs() {
                        ui.invalidate_tab_strip();
                    }
                    ui.invalidate_area(TOOLBAR_HEIGHT, ui.content_top());
                    if !lock_controller(&ui.controller).has_pending_favicons() {
                        KillTimer(window, FAVICON_TIMER);
                    }
                }
                return 0;
            }
            WM_TIMER if wparam == REVEAL_TIMER => {
                KillTimer(window, REVEAL_TIMER);
                let controller = ui_mut(window).map(|ui| ui.controller.clone());
                if let Some(controller) = controller {
                    BrowserController::reveal_active_tab(&controller);
                }
                return 0;
            }
            WM_MOUSEMOVE => {
                let x = low_word(lparam) as i32;
                let y = high_word(lparam) as i32;
                if let Some(ui) = ui_mut(window) {
                    if let Some(id) = ui.drag_tab {
                        if !ui.dragging
                            && ((x - ui.drag_origin.x).abs() > 5
                                || (y - ui.drag_origin.y).abs() > 5)
                        {
                            ui.dragging = true;
                        }
                        if ui.dragging {
                            let target = ui
                                .tabs
                                .iter()
                                .position(|visual| {
                                    let center = visual.x.value + visual.width.value / 2.0;
                                    (x as f32) < center
                                })
                                .unwrap_or(ui.tabs.len().saturating_sub(1));
                            BrowserController::reorder_tab(&ui.controller, id, target);
                        }
                    }
                    ui.update_hover(x, y);
                }
                return 0;
            }
            WM_SETCURSOR => {
                if let Some(ui) = ui_mut(window) {
                    let mut point = POINT::default();
                    GetCursorPos(&mut point);
                    ScreenToClient(window, &mut point);
                    ui.update_hover(point.x, point.y);
                }
            }
            WM_MOUSELEAVE => {
                if let Some(ui) = ui_mut(window) {
                    ui.hover = Hit::None;
                    ui.invalidate_toolbar();
                }
                return 0;
            }
            WM_LBUTTONDOWN => {
                let x = low_word(lparam) as i32;
                let y = high_word(lparam) as i32;
                if let Some(ui) = ui_mut(window) {
                    let hit = ui.hit_test(x, y);
                    let toggle_hit = match ui.panel {
                        Some(PanelKind::Downloads) => Hit::Downloads,
                        Some(PanelKind::Extensions) => Hit::Extensions,
                        Some(PanelKind::Menu) => Hit::Menu,
                        None => Hit::None,
                    };
                    if hit != toggle_hit {
                        ui.close_panels();
                    }
                    ui.pressed = hit;
                    if let Hit::Tab(id) = hit {
                        ui.drag_tab = Some(id);
                        ui.drag_origin = POINT { x, y };
                        ui.dragging = false;
                    }
                    SetCapture(window);
                    ui.invalidate_toolbar();
                }
                return 0;
            }
            WM_LBUTTONUP => {
                let x = low_word(lparam) as i32;
                let y = high_word(lparam) as i32;
                if let Some(ui) = ui_mut(window) {
                    let hit = ui.hit_test(x, y);
                    let pressed = ui.pressed;
                    let dragging = ui.dragging;
                    ui.pressed = Hit::None;
                    ui.drag_tab = None;
                    ui.dragging = false;
                    ReleaseCapture();
                    if !dragging && pressed == hit {
                        ui.activate_hit(hit);
                    }
                    ui.invalidate_toolbar();
                }
                return 0;
            }
            WM_MBUTTONUP => {
                let x = low_word(lparam) as i32;
                let y = high_word(lparam) as i32;
                if let Some(ui) = ui_mut(window)
                    && let Hit::Tab(id) | Hit::TabClose(id) = ui.hit_test(x, y)
                {
                    PostMessageW(window, WM_CLOSE_TAB, id as WPARAM, 0);
                }
                return 0;
            }
            WM_CLOSE_TAB => {
                if let Some(ui) = ui_mut(window) {
                    ui.close_tab(wparam as u64);
                }
                return 0;
            }
            WM_DESTROY_TAB => {
                let browser = wparam as HWND;
                if !browser.is_null() {
                    DestroyWindow(browser);
                }
                return 0;
            }
            WM_COMMIT_CLOSE_TAB => {
                let controller = ui_mut(window).map(|ui| ui.controller.clone());
                if let Some(controller) = controller {
                    BrowserController::close_tab(&controller, wparam as u64);
                }
                return 0;
            }
            WM_FINALIZE_TAB => {
                if let Some(ui) = ui_mut(window) {
                    let controller = ui.controller.clone();
                    BrowserController::finish_tab_close(&controller, wparam as u64);
                }
                return 0;
            }
            WM_CLOSE_ORPHANS => {
                if let Some(controller) = ui_mut(window).map(|ui| ui.controller.clone()) {
                    BrowserController::close_orphan_browsers(&controller);
                }
                return 0;
            }
            WM_EXTENSION_INSTALLED => {
                let raw = wparam as *mut Result<String, String>;
                if !raw.is_null() {
                    let result = *Box::from_raw(raw);
                    if let Some(controller) = ui_mut(window).map(|ui| ui.controller.clone()) {
                        BrowserController::handle_extension_installed(&controller, window, result);
                    }
                }
                return 0;
            }
            WM_RBUTTONUP => {
                let x = low_word(lparam) as i32;
                let y = high_word(lparam) as i32;
                if let Some(ui) = ui_mut(window) {
                    match ui.hit_test(x, y) {
                        Hit::Tab(id) => show_tab_menu(window, id),
                        Hit::Bookmark(index) => show_bookmark_menu(window, index),
                        _ => {}
                    }
                }
                return 0;
            }
            WM_COMMAND => {
                let id = wparam & 0xFFFF;
                let notification = (wparam >> 16) & 0xFFFF;
                if notification == 0 && id == MENU_PRIVATE {
                    BrowserController::open_private_window();
                    return 0;
                }
                if id == ID_ADDRESS {
                    if notification == EN_CHANGE as usize {
                        if let Some(ui) = ui_mut(window) {
                            ui.address_editing = true;
                            InvalidateRect(ui.address, ptr::null(), 0);
                        }
                    } else if notification == EN_KILLFOCUS as usize {
                        finish_address_editing(window);
                    } else if notification == EN_SETFOCUS as usize
                        && let Some(ui) = ui_mut(window)
                    {
                        ui.close_panels();
                    }
                    return 0;
                }
            }
            WM_CLOSE_PANELS => {
                if let Some(ui) = ui_mut(window) {
                    ui.close_panels();
                }
                return 0;
            }
            WM_ACTIVATE => {
                if wparam == WA_INACTIVE as usize
                    && let Some(ui) = ui_mut(window)
                {
                    ui.hover = Hit::None;
                    ui.close_panels();
                    InvalidateRect(window, ptr::null(), 0);
                }
            }
            WM_CTLCOLOREDIT => {
                if let Some(ui) = ui_mut(window) {
                    SetBkColor(wparam as HDC, ui.palette.surface);
                    SetTextColor(wparam as HDC, ui.palette.foreground);
                    return ui.surface_brush as LRESULT;
                }
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if handle_native_shortcut(window, wparam as i32) {
                    return 0;
                }
            }
            WM_CLOSE => {
                let should_close = ui_mut(window)
                    .map(|ui| ui.controller.clone())
                    .is_none_or(|controller| BrowserController::close_requested(&controller));
                if should_close {
                    DestroyWindow(window);
                }
                return 0;
            }
            WM_NCDESTROY => {
                let ui_pointer = ui_ptr(window);
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                if !ui_pointer.is_null() {
                    let mut ui = Box::from_raw(ui_pointer);
                    ui.destroy_resources();
                    let last_window =
                        ui.counted && OPEN_WINDOWS.fetch_sub(1, Ordering::SeqCst) == 1;
                    if last_window {
                        super::icons::clear_image_cache();
                    }
                    let keep_background = {
                        let state = lock_controller(&ui.controller);
                        !state.is_private() && state.preferences().continue_background
                    };
                    if last_window && (!keep_background || crate::extensions::is_restart_requested())
                    {
                        cef::quit_message_loop();
                    }
                }
                return DefWindowProcW(window, message, wparam, lparam);
            }
            _ => {}
        }
        DefWindowProcW(window, message, wparam, lparam)
    }
}

unsafe extern "system" fn mouse_proc(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32
            && matches!(
                message as u32,
                WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN
            )
        {
            let mouse = &*(data as *const MOUSEHOOKSTRUCT);
            let window = GetAncestor(mouse.hwnd, GA_ROOT);
            if let Some(ui) = ui_mut(window)
                && ui.panel_open
                && mouse.hwnd != ui.overlay_window
            {
                let mut point = mouse.pt;
                ScreenToClient(window, &mut point);
                let hit = ui.hit_test(point.x, point.y);
                let toggle_hit = match ui.panel {
                    Some(PanelKind::Downloads) => Hit::Downloads,
                    Some(PanelKind::Extensions) => Hit::Extensions,
                    Some(PanelKind::Menu) => Hit::Menu,
                    None => Hit::None,
                };
                if hit != toggle_hit {
                    PostMessageW(window, WM_CLOSE_PANELS, 0, 0);
                }
            }
        }
        CallNextHookEx(ptr::null_mut(), code, message, data)
    }
}

unsafe extern "system" fn overlay_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let parent = GetWindow(window, GW_OWNER);
        match message {
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            WM_ERASEBKGND => 1,
            WM_PRINTCLIENT => {
                if let Some(ui) = ui_mut(parent) {
                    match ui.panel {
                        Some(PanelKind::Downloads) => draw_download_panel(ui, wparam as HDC),
                        Some(PanelKind::Extensions) => draw_extensions_panel(ui, wparam as HDC),
                        Some(PanelKind::Menu) => draw_main_menu(ui, wparam as HDC),
                        None => {}
                    }
                }
                0
            }
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(window, &mut paint);
                let mut client = RECT::default();
                GetClientRect(window, &mut client);
                let width = client.right - client.left;
                let height = client.bottom - client.top;
                let memory = CreateCompatibleDC(dc);
                let bitmap = CreateCompatibleBitmap(dc, width, height);
                if !memory.is_null() && !bitmap.is_null() {
                    let old_bitmap = SelectObject(memory, bitmap as HGDIOBJ);
                    if let Some(ui) = ui_mut(parent) {
                        match ui.panel {
                            Some(PanelKind::Downloads) => draw_download_panel(ui, memory),
                            Some(PanelKind::Extensions) => draw_extensions_panel(ui, memory),
                            Some(PanelKind::Menu) => draw_main_menu(ui, memory),
                            None => {}
                        }
                    }
                    BitBlt(dc, 0, 0, width, height, memory, 0, 0, SRCCOPY);
                    SelectObject(memory, old_bitmap);
                } else if let Some(ui) = ui_mut(parent) {
                    match ui.panel {
                        Some(PanelKind::Downloads) => draw_download_panel(ui, dc),
                        Some(PanelKind::Extensions) => draw_extensions_panel(ui, dc),
                        Some(PanelKind::Menu) => draw_main_menu(ui, dc),
                        None => {}
                    }
                }
                if !bitmap.is_null() {
                    DeleteObject(bitmap as HGDIOBJ);
                }
                if !memory.is_null() {
                    DeleteDC(memory);
                }
                EndPaint(window, &paint);
                0
            }
            WM_MOUSEMOVE => {
                let x = low_word(lparam) as i32;
                let y = high_word(lparam) as i32;
                if let Some(ui) = ui_mut(parent) {
                    if !ui.panel_open {
                        return 0;
                    }
                    match ui.panel {
                        Some(PanelKind::Menu) => {
                            let hover = main_menu_command_at(y).unwrap_or(0);
                            if ui.menu_hover != hover {
                                ui.menu_hover = hover;
                                InvalidateRect(window, ptr::null(), 0);
                            }
                        }
                        Some(PanelKind::Downloads) => {
                            ui.update_download_hover(download_hit_at(ui, x, y));
                        }
                        Some(PanelKind::Extensions) => {
                            let hover = extension_hit_at(ui, x, y);
                            if ui.extension_hover != hover {
                                ui.extension_hover = hover;
                                InvalidateRect(window, ptr::null(), 0);
                            }
                        }
                        None => {}
                    }
                    track_mouse_leave(window);
                }
                0
            }
            WM_MOUSELEAVE => {
                if let Some(ui) = ui_mut(parent) {
                    match ui.panel {
                        Some(PanelKind::Menu) if ui.menu_hover != 0 => {
                            ui.menu_hover = 0;
                            InvalidateRect(window, ptr::null(), 0);
                        }
                        Some(PanelKind::Downloads) => {
                            ui.update_download_hover(DownloadHit::None);
                        }
                        Some(PanelKind::Extensions) => {
                            ui.extension_hover = ExtensionHit::None;
                            InvalidateRect(window, ptr::null(), 0);
                        }
                        _ => {}
                    }
                }
                0
            }
            WM_LBUTTONDOWN => {
                SetCapture(window);
                0
            }
            WM_LBUTTONUP => {
                ReleaseCapture();
                let x = low_word(lparam) as i32;
                let y = high_word(lparam) as i32;
                if let Some(ui) = ui_mut(parent) {
                    if !ui.panel_open {
                        return 0;
                    }
                    match ui.panel {
                        Some(PanelKind::Menu) => {
                            if let Some(mut command) = main_menu_command_at(y) {
                                if command == MENU_ZOOM {
                                    let (width, _) = client_size(window);
                                    command = if x >= width - 46 {
                                        MENU_FULLSCREEN
                                    } else if x >= width - 90 {
                                        MENU_ZOOM_PLUS
                                    } else if x >= width - 140 {
                                        MENU_ZOOM_RESET
                                    } else if x >= width - 174 {
                                        MENU_ZOOM_MINUS
                                    } else {
                                        0
                                    };
                                }
                                if command == 0 {
                                    return 0;
                                }
                                let keep_open = matches!(
                                    command,
                                    MENU_UPDATE
                                        | MENU_ZOOM_MINUS
                                        | MENU_ZOOM_RESET
                                        | MENU_ZOOM_PLUS
                                );
                                if !keep_open {
                                    ui.close_panels();
                                }
                                execute_main_menu_command(parent, command);
                                if command == MENU_UPDATE {
                                    ui.refresh_update();
                                }
                                if matches!(
                                    command,
                                    MENU_ZOOM_MINUS | MENU_ZOOM_RESET | MENU_ZOOM_PLUS
                                ) {
                                    InvalidateRect(window, ptr::null(), 0);
                                }
                            }
                        }
                        Some(PanelKind::Extensions) => match extension_hit_at(ui, x, y) {
                            ExtensionHit::Action(index) => {
                                if let Some(id) = ui
                                    .extensions
                                    .get(index)
                                    .map(|extension| extension.id.clone())
                                {
                                    ui.close_panels();
                                    BrowserController::open_extension(&ui.controller, &id);
                                }
                            }
                            ExtensionHit::Pin(index) => {
                                if let Some(id) = ui
                                    .extensions
                                    .get(index)
                                    .map(|extension| extension.id.clone())
                                {
                                    BrowserController::toggle_extension_pin(&ui.controller, &id);
                                }
                            }
                            ExtensionHit::Manage => {
                                ui.close_panels();
                                BrowserController::open_extensions(&ui.controller);
                            }
                            ExtensionHit::None => {}
                        },
                        Some(PanelKind::Downloads) => match download_hit_at(ui, x, y) {
                            DownloadHit::Close => ui.close_panels(),
                            DownloadHit::Cancel(index) => {
                                if let Some(download) = ui.downloads.get(index).cloned() {
                                    BrowserController::cancel_download(
                                        &ui.controller,
                                        download.id,
                                    );
                                }
                            }
                            DownloadHit::Action(index) => {
                                if let Some(download) = ui.downloads.get(index).cloned() {
                                    if download.status == DownloadStatus::Complete {
                                        BrowserController::open_download(
                                            &ui.controller,
                                            download.id,
                                        );
                                    } else {
                                        BrowserController::toggle_download_pause(
                                            &ui.controller,
                                            download.id,
                                        );
                                    }
                                }
                            }
                            DownloadHit::None => {}
                        },
                        None => {}
                    }
                }
                0
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

unsafe extern "system" fn control_proc(
    control: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let window = GetParent(control);
        if message == WM_LBUTTONDOWN
            && let Some(ui) = ui_mut(window)
            && control == ui.address
        {
            ui.close_panels();
        }
        if message == WM_KEYDOWN || message == WM_SYSKEYDOWN {
            if handle_native_shortcut(window, wparam as i32) {
                return 0;
            }
            if let Some(ui) = ui_mut(window)
                && control == ui.address
                && wparam as i32 == VK_ESCAPE as i32
            {
                ui.cancel_address_editing();
                return 0;
            }
        }
        if message == WM_KEYDOWN
            && wparam as i32 == 0x0D
            && let Some(ui) = ui_mut(window)
            && control == ui.address
        {
            ui.submit_address();
            return 0;
        }
        if matches!(message, WM_SETFOCUS | WM_KILLFOCUS | WM_MOUSEMOVE) {
            InvalidateRect(control, ptr::null(), 0);
        }
        let previous = GetWindowLongPtrW(control, GWLP_USERDATA);
        if previous == 0 {
            return DefWindowProcW(control, message, wparam, lparam);
        }
        let previous_proc: WNDPROC = mem::transmute::<isize, WNDPROC>(previous);
        let result = match previous_proc {
            Some(previous_proc) => {
                CallWindowProcW(Some(previous_proc), control, message, wparam, lparam)
            }
            None => DefWindowProcW(control, message, wparam, lparam),
        };
        if message == WM_PAINT {
            draw_address_overlay(control);
        }
        if message == WM_NCDESTROY {
            SetWindowLongPtrW(control, GWLP_WNDPROC, previous);
            SetWindowLongPtrW(control, GWLP_USERDATA, 0);
        }
        result
    }
}

unsafe fn draw_address_overlay(address: HWND) {
    unsafe {
        let parent = GetParent(address);
        let Some(ui) = ui_mut(parent) else { return };
        let text = window_text(address);
        if !text.is_empty() && !ui.current_url.starts_with("http://") {
            return;
        }
        let dc = GetDC(address);
        if dc.is_null() {
            return;
        }
        let mut rect = RECT::default();
        GetClientRect(address, &mut rect);
        SetBkMode(dc, TRANSPARENT as i32);
        SelectObject(dc, ui.font as HGDIOBJ);
        if text.is_empty() {
            SetTextColor(dc, ui.palette.secondary);
            let mut placeholder = wide(if ui.english {
                "Address or search"
            } else {
                "Адрес или поиск"
            });
            rect.left += 6;
            rect.right -= 8;
            DrawTextW(
                dc,
                placeholder.as_mut_ptr(),
                placeholder.len() as i32 - 1,
                &mut rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
        }
        ReleaseDC(address, dc);
    }
}

fn handle_native_shortcut(window: HWND, key: i32) -> bool {
    let control = key_down(VK_CONTROL as i32);
    let shift = key_down(VK_SHIFT as i32);
    let alt = key_down(VK_MENU as i32);
    let Some(ui) = ui_mut(window) else {
        return false;
    };
    let controller = ui.controller.clone();
    match key {
        0x4C if control => unsafe { focus_address(window) },
        0x52 if control => BrowserController::reload(&controller),
        0x44 if control => BrowserController::toggle_bookmark(&controller),
        0x48 if control => {
            BrowserController::open_internal_page(&controller, ui::internal::InternalPage::History)
        }
        0x42 if control && shift => BrowserController::toggle_bookmarks_bar(&controller),
        0x54 if control && shift => BrowserController::restore_closed_tab(&controller),
        0x54 if control => {
            let url = lock_controller(&controller).start_page_url();
            BrowserController::new_tab(&controller, &url, false);
        }
        0x57 if control => BrowserController::close_active_tab(&controller),
        0x4E if control && shift => BrowserController::open_private_window(),
        value if value == VK_TAB as i32 && control => {
            BrowserController::next_tab(&controller, shift)
        }
        value if value == VK_F5 as i32 => BrowserController::reload(&controller),
        value if value == VK_LEFT as i32 && alt => BrowserController::go_back(&controller),
        value if value == VK_RIGHT as i32 && alt => BrowserController::go_forward(&controller),
        value if value == VK_F11 as i32 => unsafe { toggle_fullscreen(window) },
        value if value == VK_F12 as i32 => BrowserController::show_dev_tools(&controller),
        value if value == VK_ESCAPE as i32 => {
            if ui.fullscreen {
                unsafe { toggle_fullscreen(window) };
            } else {
                BrowserController::stop_loading(&controller);
            }
        }
        value if control && ('1' as i32..='9' as i32).contains(&value) => {
            let index = (value - '1' as i32) as usize;
            let id = {
                let state = lock_controller(&controller);
                if value == '9' as i32 {
                    state.last_tab_id()
                } else {
                    state.tab_id_at(index)
                }
            };
            if let Some(id) = id {
                BrowserController::activate_tab(&controller, id);
            }
        }
        _ => return false,
    }
    true
}

unsafe fn subclass_control(control: HWND) {
    unsafe {
        let previous = SetWindowLongPtrW(control, GWLP_WNDPROC, control_proc as *const () as isize);
        SetWindowLongPtrW(control, GWLP_USERDATA, previous);
    }
}

unsafe fn non_client_hit_test(window: HWND, lparam: LPARAM) -> LRESULT {
    unsafe {
        if ui_mut(window).is_some_and(|ui| ui.fullscreen) {
            return HTCLIENT as LRESULT;
        }
        let mut point = POINT {
            x: low_word(lparam) as i32,
            y: high_word(lparam) as i32,
        };
        ScreenToClient(window, &mut point);
        let (width, height) = client_size(window);
        let zoomed = IsZoomed(window) != 0;
        let border = 6;
        if !zoomed {
            let left = point.x < border;
            let right = point.x >= width - border;
            let top = point.y < border;
            let bottom = point.y >= height - border;
            match (left, right, top, bottom) {
                (true, _, true, _) => return HTTOPLEFT as LRESULT,
                (_, true, true, _) => return HTTOPRIGHT as LRESULT,
                (true, _, _, true) => return HTBOTTOMLEFT as LRESULT,
                (_, true, _, true) => return HTBOTTOMRIGHT as LRESULT,
                (true, ..) => return HTLEFT as LRESULT,
                (_, true, ..) => return HTRIGHT as LRESULT,
                (_, _, true, _) => return HTTOP as LRESULT,
                (_, _, _, true) => return HTBOTTOM as LRESULT,
                _ => {}
            }
        }
        if point.y < TAB_HEIGHT
            && let Some(ui) = ui_mut(window)
            && ui.hit_test(point.x, point.y) == Hit::None
        {
            return HTCAPTION as LRESULT;
        }
        HTCLIENT as LRESULT
    }
}

unsafe fn move_window_if_needed(
    window: HWND,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    repaint: i32,
) {
    unsafe {
        let parent = GetParent(window);
        let mut bounds = RECT::default();
        if GetWindowRect(window, &mut bounds) != 0 {
            let mut origin = POINT {
                x: bounds.left,
                y: bounds.top,
            };
            if !parent.is_null() {
                ScreenToClient(parent, &mut origin);
            }
            if origin.x == x
                && origin.y == y
                && bounds.right - bounds.left == width
                && bounds.bottom - bounds.top == height
            {
                return;
            }
        }
        SetWindowPos(
            window,
            ptr::null_mut(),
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE | if repaint == 0 { SWP_NOREDRAW } else { 0 },
        );
    }
}

fn point_in_rect(x: i32, y: i32, rect: &RECT) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

fn toolbar_rect(left: i32) -> RECT {
    RECT {
        left,
        top: TAB_HEIGHT + TOOLBAR_GAP,
        right: left + 40,
        bottom: TOOLBAR_HEIGHT - TOOLBAR_GAP,
    }
}

fn new_tab_rect(ui: &NativeUi) -> RECT {
    let (width, _) = unsafe { client_size(ui.window) };
    let after_tabs = ui
        .tabs
        .last()
        .map(|visual| visual.x.value + visual.width.value + 6.0)
        .unwrap_or(64.0);
    let left = (after_tabs.round() as i32).min((width - 198).max(64));
    RECT {
        left,
        top: TAB_TOP,
        right: left + 36,
        bottom: TAB_BOTTOM,
    }
}

fn caption_button_rect(width: i32, slot_from_right: i32) -> RECT {
    let right = width - EDGE_MARGIN - slot_from_right * 46;
    RECT {
        left: right - 46,
        top: TAB_TOP,
        right,
        bottom: TAB_BOTTOM,
    }
}

fn address_rect(width: i32, pinned_extensions: usize) -> RECT {
    RECT {
        left: 196,
        top: TAB_HEIGHT + TOOLBAR_GAP,
        right: (width - 172 - pinned_extensions as i32 * 44).max(296),
        bottom: TOOLBAR_HEIGHT - TOOLBAR_GAP,
    }
}

fn bookmark_toggle_rect(width: i32, pinned_extensions: usize) -> RECT {
    let address = address_rect(width, pinned_extensions);
    RECT {
        left: address.right - 40,
        top: address.top + 2,
        right: address.right - 4,
        bottom: address.bottom - 2,
    }
}

unsafe fn paint_toolbar(ui: &mut NativeUi, dc: HDC, tabs_only: bool) {
    unsafe {
        if ui.fullscreen {
            return;
        }
        let (width, _) = client_size(ui.window);
        let toolbar_height = if tabs_only {
            TAB_HEIGHT
        } else {
            ui.content_top()
        };
        let memory = CreateCompatibleDC(dc);
        let bitmap = CreateCompatibleBitmap(dc, width, toolbar_height);
        if memory.is_null() || bitmap.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap as HGDIOBJ);
            }
            if !memory.is_null() {
                DeleteDC(memory);
            }
            return;
        }
        let old_bitmap = SelectObject(memory, bitmap as HGDIOBJ);
        let rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: toolbar_height,
        };
        FillRect(memory, &rect, ui.background_brush);
        SetBkMode(memory, TRANSPARENT as i32);
        DrawIconEx(
            memory,
            16,
            (TAB_TOP + TAB_BOTTOM - 28) / 2,
            ui.logo_icon,
            28,
            28,
            0,
            ptr::null_mut(),
            DI_NORMAL,
        );
        if !tabs_only {
            let pinned_extensions = ui.visible_pinned_extensions();
            let address = address_rect(width, pinned_extensions);
            fill_round_rect(memory, &address, ui.palette.surface, 22, None);
            if ui.current_url.starts_with("http://") {
                let warning = RECT {
                    left: address.left + 8,
                    top: address.top,
                    right: address.left + 32,
                    bottom: address.bottom,
                };
                draw_text(memory, "!", &warning, ui.font, WARNING, DT_CENTER);
            }
            draw_button(
                memory,
                ui,
                Hit::BookmarkToggle,
                bookmark_toggle_rect(width, pinned_extensions),
                if ui.bookmarked {
                    Icon::BookmarkFilled
                } else {
                    Icon::Bookmark
                },
                ui.current_url.starts_with("https://") || ui.current_url.starts_with("http://"),
            );
        }
        for visual in &ui.tabs {
            draw_tab(ui, memory, visual);
        }
        draw_button(
            memory,
            ui,
            Hit::NewTab,
            new_tab_rect(ui),
            Icon::NewTab,
            true,
        );
        if !tabs_only {
            draw_button(
                memory,
                ui,
                Hit::Back,
                toolbar_rect(16),
                Icon::Back,
                ui.can_go_back,
            );
            draw_button(
                memory,
                ui,
                Hit::Forward,
                toolbar_rect(60),
                Icon::Forward,
                ui.can_go_forward,
            );
            draw_button(
                memory,
                ui,
                Hit::Reload,
                toolbar_rect(104),
                if ui.is_loading {
                    Icon::Close
                } else {
                    Icon::Reload
                },
                true,
            );
            draw_button(memory, ui, Hit::Home, toolbar_rect(148), Icon::Home, true);
            for (index, rect) in ui.pinned_extension_rects() {
                let hit = Hit::Extension(index);
                if ui.hover == hit || ui.pressed == hit {
                    fill_round_rect(memory, &rect, ui.palette.hover, 12, None);
                }
                let icon_drawn = ui.extensions[index]
                    .icon_path
                    .as_ref()
                    .is_some_and(|path| draw_image(memory, &rect, path, 16));
                if !icon_drawn {
                    draw_icon(memory, &rect, Icon::Extensions, ui.palette.secondary);
                }
            }
            draw_button(
                memory,
                ui,
                Hit::Extensions,
                toolbar_rect(width - 160),
                Icon::Extensions,
                true,
            );
            draw_button(
                memory,
                ui,
                Hit::Downloads,
                toolbar_rect(width - 108),
                Icon::Download,
                true,
            );
            draw_button(
                memory,
                ui,
                Hit::Menu,
                toolbar_rect(width - 56),
                Icon::Menu,
                true,
            );
        }
        draw_button(
            memory,
            ui,
            Hit::Minimize,
            caption_button_rect(width, 2),
            Icon::Minimize,
            true,
        );
        draw_button(
            memory,
            ui,
            Hit::Maximize,
            caption_button_rect(width, 1),
            if IsZoomed(ui.window) != 0 {
                Icon::Restore
            } else {
                Icon::Maximize
            },
            true,
        );
        draw_caption_close(ui, memory, width);
        if !tabs_only {
            draw_bookmarks(ui, memory);
            let divider = RECT {
                left: 0,
                top: toolbar_height - 1,
                right: width,
                bottom: toolbar_height,
            };
            let brush = CreateSolidBrush(ui.palette.divider);
            FillRect(memory, &divider, brush);
            DeleteObject(brush as HGDIOBJ);
        }
        BitBlt(dc, 0, 0, width, toolbar_height, memory, 0, 0, SRCCOPY);
        SelectObject(memory, old_bitmap);
        DeleteObject(bitmap as HGDIOBJ);
        DeleteDC(memory);
    }
}

unsafe fn draw_bookmarks(ui: &NativeUi, dc: HDC) {
    if !ui.show_bookmarks {
        return;
    }
    unsafe {
        if ui.bookmarks.is_empty() {
            let empty = RECT {
                left: EDGE_MARGIN,
                top: TOOLBAR_HEIGHT,
                right: client_size(ui.window).0 - EDGE_MARGIN,
                bottom: TOOLBAR_HEIGHT + BOOKMARKS_HEIGHT,
            };
            draw_text(
                dc,
                if ui.english {
                    "Ctrl+D — bookmark this page"
                } else {
                    "Ctrl+D — добавить текущую страницу в закладки"
                },
                &empty,
                ui.small_font,
                ui.palette.secondary,
                DT_LEFT | DT_END_ELLIPSIS,
            );
            return;
        }
        let favicons = {
            let state = lock_controller(&ui.controller);
            ui.bookmarks
                .iter()
                .map(|bookmark| state.favicon(&bookmark.url))
                .collect::<Vec<_>>()
        };
        for (index, rect) in ui.bookmark_rects().into_iter().enumerate() {
            if ui.hover == Hit::Bookmark(index) {
                fill_round_rect(dc, &rect, ui.palette.hover, 9, None);
            }
            let icon = RECT {
                left: rect.left + 6,
                top: rect.top,
                right: rect.left + 28,
                bottom: rect.bottom,
            };
            if !favicons[index]
                .as_ref()
                .is_some_and(|path| draw_image(dc, &icon, path, 16))
            {
                draw_icon(dc, &icon, Icon::Bookmark, ui.palette.secondary);
            }
            let title = RECT {
                left: rect.left + 29,
                top: rect.top,
                right: rect.right - 8,
                bottom: rect.bottom,
            };
            let bookmark = &ui.bookmarks[index];
            let title_text = if bookmark.title.is_empty() {
                &bookmark.url
            } else {
                &bookmark.title
            };
            draw_text(
                dc,
                title_text,
                &title,
                ui.title_font,
                ui.palette.foreground,
                DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
        }
    }
}

unsafe fn draw_tab(ui: &NativeUi, dc: HDC, visual: &TabVisual) {
    unsafe {
        if visual.width.value < 1.0 {
            return;
        }
        let x = visual.x.value.round() as i32;
        let rect = RECT {
            left: x,
            top: TAB_TOP,
            right: x + visual.width.value.round() as i32,
            bottom: TAB_BOTTOM,
        };
        let active = visual.tab.active;
        let hovered =
            ui.hover == Hit::Tab(visual.tab.id) || ui.hover == Hit::TabClose(visual.tab.id);
        if active || hovered {
            fill_round_rect(
                dc,
                &rect,
                if active {
                    ui.palette.surface
                } else {
                    ui.palette.hover
                },
                12,
                None,
            );
        }
        SetBkMode(dc, TRANSPARENT as i32);
        let saved = SaveDC(dc);
        if saved == 0 {
            return;
        }
        IntersectClipRect(dc, rect.left, rect.top, rect.right, rect.bottom);
        let tab_width = rect.right - rect.left;
        let indicator = if visual.tab.pinned {
            let left = rect.left + (tab_width - 18) / 2;
            RECT {
                left,
                top: rect.top + 11,
                right: left + 18,
                bottom: rect.top + 29,
            }
        } else {
            RECT {
                left: rect.left + 8,
                top: rect.top + 11,
                right: rect.left + 26,
                bottom: rect.top + 29,
            }
        };
        let mut icon_drawn = visual
            .tab
            .favicon
            .as_ref()
            .is_some_and(|path| draw_image(dc, &indicator, path, 18));
        if !icon_drawn && visual.tab.internal {
            draw_icon(dc, &indicator, Icon::Kockers, ui.palette.secondary);
            icon_drawn = true;
        }
        if visual.tab.pinned {
            if !icon_drawn {
                let initial = visual.tab.title.chars().next().unwrap_or('•').to_string();
                draw_text(
                    dc,
                    &initial,
                    &rect,
                    ui.title_font,
                    if active {
                        ui.palette.accent
                    } else {
                        ui.palette.secondary
                    },
                    DT_CENTER,
                );
            }
            RestoreDC(dc, saved);
            return;
        }
        let show_close = tab_width >= 72 || active || hovered;
        let has_leading_indicator = icon_drawn || visual.tab.loading;
        let mut title_rect = rect;
        title_rect.left += if has_leading_indicator { 30 } else { 12 };
        title_rect.right -= if show_close { 34 } else { 8 };
        draw_text(
            dc,
            &visual.tab.title,
            &title_rect,
            ui.title_font,
            if active {
                ui.palette.foreground
            } else {
                ui.palette.secondary
            },
            DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        if visual.tab.loading && !icon_drawn {
            draw_text(
                dc,
                "•",
                &indicator,
                ui.small_font,
                ui.palette.accent,
                DT_CENTER,
            );
        }
        if show_close {
            let close = RECT {
                left: rect.right - 32,
                top: TAB_TOP + 7,
                right: rect.right - 6,
                bottom: TAB_BOTTOM - 7,
            };
            if ui.hover == Hit::TabClose(visual.tab.id) {
                fill_round_rect(dc, &close, ui.palette.hover, 9, None);
            }
            draw_icon(dc, &close, Icon::Close, ui.palette.secondary);
        }
        RestoreDC(dc, saved);
    }
}

use super::icons::{Icon, draw_icon, draw_image};

unsafe fn draw_button(dc: HDC, ui: &NativeUi, hit: Hit, rect: RECT, icon: Icon, enabled: bool) {
    unsafe {
        if enabled && (ui.hover == hit || ui.pressed == hit) {
            if matches!(hit, Hit::Minimize | Hit::Maximize) {
                let highlight = RECT {
                    left: rect.left + 3,
                    top: rect.top + 2,
                    right: rect.right - 3,
                    bottom: rect.bottom - 2,
                };
                fill_round_rect(dc, &highlight, ui.palette.hover, 10, None);
            } else {
                fill_round_rect(dc, &rect, ui.palette.hover, 12, None);
            }
        }
        draw_icon(
            dc,
            &rect,
            icon,
            if enabled {
                ui.palette.secondary
            } else {
                mix_color(ui.palette.background, ui.palette.secondary, 0.48)
            },
        );
    }
}

unsafe fn draw_caption_close(ui: &NativeUi, dc: HDC, width: i32) {
    unsafe {
        let rect = caption_button_rect(width, 0);
        let hovered = ui.hover == Hit::CloseWindow || ui.pressed == Hit::CloseWindow;
        if hovered {
            let highlight = RECT {
                left: rect.left + 3,
                top: rect.top + 2,
                right: rect.right - 3,
                bottom: rect.bottom - 2,
            };
            fill_round_rect(dc, &highlight, CLOSE_HOVER, 10, None);
        }
        draw_icon(
            dc,
            &rect,
            Icon::Close,
            if hovered {
                rgb(0xFF, 0xFF, 0xFF)
            } else {
                ui.palette.foreground
            },
        );
    }
}

const EXTENSION_LIST_TOP: i32 = 54;
const EXTENSION_ROW_HEIGHT: i32 = 48;
const EXTENSION_MANAGE_HEIGHT: i32 = 44;

fn extension_hit_at(ui: &NativeUi, x: i32, y: i32) -> ExtensionHit {
    let (width, height) = unsafe { client_size(ui.overlay_window) };
    if y >= height - EXTENSION_MANAGE_HEIGHT {
        return ExtensionHit::Manage;
    }
    if y < EXTENSION_LIST_TOP || y >= height - EXTENSION_MANAGE_HEIGHT {
        return ExtensionHit::None;
    }
    let index = ((y - EXTENSION_LIST_TOP) / EXTENSION_ROW_HEIGHT) as usize;
    if index >= ui.extensions.len() {
        return ExtensionHit::None;
    }
    if x >= width - 116 {
        ExtensionHit::Pin(index)
    } else {
        ExtensionHit::Action(index)
    }
}

unsafe fn draw_extensions_panel(ui: &NativeUi, dc: HDC) {
    unsafe {
        let mut client = RECT::default();
        GetClientRect(ui.overlay_window, &mut client);
        FillRect(dc, &client, ui.background_brush);
        fill_round_rect(
            dc,
            &client,
            ui.palette.surface,
            16,
            Some(ui.palette.divider),
        );
        SetBkMode(dc, TRANSPARENT as i32);
        let english = ui.english;
        let text = |russian, english_text| if english { english_text } else { russian };
        let title = RECT {
            left: 18,
            top: 8,
            right: client.right - 18,
            bottom: 44,
        };
        draw_text(
            dc,
            text("Расширения", "Extensions"),
            &title,
            ui.font,
            ui.palette.foreground,
            DT_LEFT | DT_END_ELLIPSIS,
        );
        if ui.extensions.is_empty() {
            let empty = RECT {
                left: 18,
                top: EXTENSION_LIST_TOP,
                right: client.right - 18,
                bottom: EXTENSION_LIST_TOP + EXTENSION_ROW_HEIGHT,
            };
            draw_text(
                dc,
                text("Нет установленных расширений", "No installed extensions"),
                &empty,
                ui.small_font,
                ui.palette.secondary,
                DT_LEFT | DT_END_ELLIPSIS,
            );
        }
        let list_bottom = client.bottom - EXTENSION_MANAGE_HEIGHT;
        for (index, extension) in ui.extensions.iter().enumerate() {
            let top = EXTENSION_LIST_TOP + index as i32 * EXTENSION_ROW_HEIGHT;
            if top >= list_bottom {
                break;
            }
            let row = RECT {
                left: 6,
                top,
                right: client.right - 6,
                bottom: (top + EXTENSION_ROW_HEIGHT).min(list_bottom),
            };
            let action_hover = ui.extension_hover == ExtensionHit::Action(index);
            let pin_hover = ui.extension_hover == ExtensionHit::Pin(index);
            if action_hover {
                fill_round_rect(dc, &row, ui.palette.hover, 10, None);
            }
            let icon = RECT {
                left: 12,
                top: row.top,
                right: 44,
                bottom: row.bottom,
            };
            let icon_drawn = extension
                .icon_path
                .as_ref()
                .is_some_and(|path| draw_image(dc, &icon, path, 20));
            if !icon_drawn {
                draw_icon(dc, &icon, Icon::Extensions, ui.palette.secondary);
            }
            let name = RECT {
                left: 48,
                top: row.top,
                right: client.right - 122,
                bottom: row.bottom,
            };
            draw_text(
                dc,
                &extension.name,
                &name,
                ui.title_font,
                ui.palette.foreground,
                DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            let pin_pill = RECT {
                left: client.right - 114,
                top: row.top + 8,
                right: client.right - 10,
                bottom: row.bottom - 8,
            };
            if pin_hover {
                fill_round_rect(dc, &pin_pill, ui.palette.hover, 10, None);
            }
            let pin = RECT {
                left: pin_pill.left + 8,
                top: pin_pill.top,
                right: pin_pill.right - 8,
                bottom: pin_pill.bottom,
            };
            draw_text(
                dc,
                if extension.pinned {
                    text("Открепить", "Unpin")
                } else {
                    text("Закрепить", "Pin")
                },
                &pin,
                ui.small_font,
                if extension.pinned {
                    ui.palette.accent
                } else {
                    ui.palette.secondary
                },
                DT_CENTER | DT_END_ELLIPSIS,
            );
        }
        let divider = RECT {
            left: 14,
            top: client.bottom - EXTENSION_MANAGE_HEIGHT - 1,
            right: client.right - 14,
            bottom: client.bottom - EXTENSION_MANAGE_HEIGHT,
        };
        let divider_brush = CreateSolidBrush(ui.palette.divider);
        if !divider_brush.is_null() {
            FillRect(dc, &divider, divider_brush);
            DeleteObject(divider_brush as HGDIOBJ);
        }
        let manage = RECT {
            left: 6,
            top: client.bottom - EXTENSION_MANAGE_HEIGHT + 2,
            right: client.right - 6,
            bottom: client.bottom - 4,
        };
        FillRect(dc, &manage, ui.surface_brush);
        if ui.extension_hover == ExtensionHit::Manage {
            fill_round_rect(dc, &manage, ui.palette.hover, 10, None);
        }
        let manage_label = RECT {
            left: 18,
            top: manage.top,
            right: manage.right - 12,
            bottom: manage.bottom,
        };
        draw_text(
            dc,
            text("Управление расширениями", "Manage extensions"),
            &manage_label,
            ui.title_font,
            ui.palette.foreground,
            DT_LEFT | DT_END_ELLIPSIS,
        );
    }
}

fn download_action_rect(width: i32, index: usize) -> RECT {
    let top = 54 + index as i32 * 74;
    RECT {
        left: width - 126,
        top: top + 18,
        right: width - 62,
        bottom: top + 52,
    }
}

fn download_cancel_rect(width: i32, index: usize) -> RECT {
    let top = 54 + index as i32 * 74;
    RECT {
        left: width - 56,
        top: top + 18,
        right: width - 14,
        bottom: top + 52,
    }
}

fn download_hit_at(ui: &NativeUi, x: i32, y: i32) -> DownloadHit {
    let (width, _) = unsafe { client_size(ui.overlay_window) };
    let close = RECT {
        left: width - 46,
        top: 8,
        right: width - 10,
        bottom: 44,
    };
    if point_in_rect(x, y, &close) {
        return DownloadHit::Close;
    }
    for (index, download) in ui.downloads.iter().enumerate() {
        let action = download_action_rect(width, index);
        let cancel = download_cancel_rect(width, index);
        match download.status {
            DownloadStatus::Complete if point_in_rect(x, y, &action) => {
                return DownloadHit::Action(index);
            }
            DownloadStatus::InProgress | DownloadStatus::Paused => {
                if point_in_rect(x, y, &cancel) {
                    return DownloadHit::Cancel(index);
                }
                if point_in_rect(x, y, &action) {
                    return DownloadHit::Action(index);
                }
            }
            _ => {}
        }
    }
    DownloadHit::None
}

fn download_hover_amount(ui: &NativeUi, hit: DownloadHit) -> f32 {
    if ui.download_hover == hit
        || (ui.download_hover == DownloadHit::None && ui.previous_download_hover == hit)
    {
        ui.download_hover_transition.value
    } else {
        0.0
    }
}

unsafe fn draw_download_panel(ui: &NativeUi, dc: HDC) {
    unsafe {
        let mut client = RECT::default();
        GetClientRect(ui.overlay_window, &mut client);
        FillRect(dc, &client, ui.background_brush);
        fill_round_rect(
            dc,
            &client,
            ui.palette.surface,
            16,
            Some(ui.palette.divider),
        );
        SetBkMode(dc, TRANSPARENT as i32);
        let english = ui.english;
        let text = |russian, english_text| if english { english_text } else { russian };
        let title = RECT {
            left: 20,
            top: 8,
            right: client.right - 52,
            bottom: 45,
        };
        draw_text(
            dc,
            text("Загрузки", "Downloads"),
            &title,
            ui.font,
            ui.palette.foreground,
            DT_LEFT | DT_VCENTER,
        );
        let close = RECT {
            left: client.right - 46,
            top: 8,
            right: client.right - 10,
            bottom: 44,
        };
        let close_hover = download_hover_amount(ui, DownloadHit::Close);
        if close_hover > 0.0 {
            fill_round_rect(
                dc,
                &close,
                mix_color(ui.palette.surface, ui.palette.hover, close_hover),
                12,
                None,
            );
        }
        draw_icon(dc, &close, Icon::Close, ui.palette.secondary);
        if ui.downloads.is_empty() {
            let empty = RECT {
                left: 20,
                top: 55,
                right: client.right - 20,
                bottom: 94,
            };
            draw_text(
                dc,
                text("Загрузок пока нет", "No downloads yet"),
                &empty,
                ui.title_font,
                ui.palette.secondary,
                DT_LEFT | DT_VCENTER,
            );
            return;
        }
        for (index, download) in ui.downloads.iter().enumerate() {
            let top = 54 + index as i32 * 74;
            let name = RECT {
                left: 18,
                top,
                right: client.right - 132,
                bottom: top + 30,
            };
            draw_text(
                dc,
                &download.name,
                &name,
                ui.title_font,
                ui.palette.foreground,
                DT_LEFT | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            let is_crx = download
                .path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("crx"));
            let status = match download.status {
                DownloadStatus::InProgress => format!("{}%", download.percent.clamp(0, 100)),
                DownloadStatus::Paused => text("Пауза", "Paused").to_owned(),
                DownloadStatus::Complete if is_crx => {
                    text("Готово к установке", "Ready to install").to_owned()
                }
                DownloadStatus::Complete => {
                    format_bytes_localized(download.received.max(download.total), english)
                }
                DownloadStatus::Canceled => text("Отменено", "Canceled").to_owned(),
                DownloadStatus::Interrupted => {
                    text("Загрузка прервана", "Interrupted").to_owned()
                }
            };
            let info = RECT {
                left: 18,
                top: top + 30,
                right: client.right - 132,
                bottom: top + 57,
            };
            draw_text(
                dc,
                &status,
                &info,
                ui.small_font,
                ui.palette.secondary,
                DT_LEFT | DT_VCENTER,
            );
            let action = download_action_rect(client.right, index);
            let action_hover = download_hover_amount(ui, DownloadHit::Action(index));
            match download.status {
                DownloadStatus::Complete => {
                    fill_round_rect(
                        dc,
                        &action,
                        mix_color(
                            ui.palette.accent,
                            ui.palette.foreground,
                            action_hover * 0.18,
                        ),
                        17,
                        None,
                    );
                    draw_text(
                        dc,
                        text("Открыть", "Open"),
                        &action,
                        ui.small_font,
                        rgb(0xFF, 0xFF, 0xFF),
                        DT_CENTER,
                    );
                }
                DownloadStatus::InProgress | DownloadStatus::Paused => {
                    if action_hover > 0.0 {
                        fill_round_rect(
                            dc,
                            &action,
                            mix_color(ui.palette.surface, ui.palette.hover, action_hover),
                            12,
                            None,
                        );
                    }
                    draw_icon(
                        dc,
                        &action,
                        if download.status == DownloadStatus::Paused {
                            Icon::Play
                        } else {
                            Icon::Pause
                        },
                        ui.palette.foreground,
                    );
                }
                _ => {}
            }
            if matches!(
                download.status,
                DownloadStatus::InProgress | DownloadStatus::Paused
            ) {
                let cancel = download_cancel_rect(client.right, index);
                let cancel_hover = download_hover_amount(ui, DownloadHit::Cancel(index));
                if cancel_hover > 0.0 {
                    fill_round_rect(
                        dc,
                        &cancel,
                        mix_color(ui.palette.surface, ui.palette.hover, cancel_hover),
                        12,
                        None,
                    );
                }
                draw_icon(dc, &cancel, Icon::Close, ui.palette.secondary);
                let progress = RECT {
                    left: 18,
                    top: top + 62,
                    right: client.right - 18,
                    bottom: top + 65,
                };
                fill_round_rect(dc, &progress, ui.palette.divider, 2, None);
                let mut filled = progress;
                filled.right = filled.left
                    + (progress.right - progress.left) * download.percent.clamp(0, 100) / 100;
                if filled.right > filled.left {
                    fill_round_rect(dc, &filled, ui.palette.accent, 2, None);
                }
            }
        }
    }
}

unsafe fn fill_round_rect(
    dc: HDC,
    rect: &RECT,
    color: COLORREF,
    radius: i32,
    border: Option<COLORREF>,
) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let pen = CreatePen(PS_SOLID, 1, border.unwrap_or(color));
        let old_brush = SelectObject(dc, brush as HGDIOBJ);
        let old_pen = SelectObject(dc, pen as HGDIOBJ);
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius * 2,
            radius * 2,
        );
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(brush as HGDIOBJ);
        DeleteObject(pen as HGDIOBJ);
    }
}

unsafe fn draw_text(dc: HDC, text: &str, rect: &RECT, font: HFONT, color: COLORREF, flags: u32) {
    unsafe {
        SelectObject(dc, font as HGDIOBJ);
        SetTextColor(dc, color);
        SetBkMode(dc, TRANSPARENT as i32);
        let mut text = wide(text);
        let mut rect = *rect;
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            text.len() as i32 - 1,
            &mut rect,
            flags | DT_SINGLELINE | DT_VCENTER,
        );
    }
}

fn format_bytes_localized(bytes: i64, english: bool) -> String {
    if bytes >= 1_048_576 {
        format!(
            "{:.1} {}",
            bytes as f64 / 1_048_576.0,
            if english { "MB" } else { "МБ" }
        )
    } else if bytes >= 1024 {
        format!(
            "{:.0} {}",
            bytes as f64 / 1024.0,
            if english { "KB" } else { "КБ" }
        )
    } else {
        format!("{bytes} {}", if english { "B" } else { "Б" })
    }
}

const MAIN_MENU_LAYOUT: &[(usize, bool)] = &[
    (MENU_PRIVATE, false),
    (MENU_RESTORE, false),
    (MENU_PROFILE, false),
    (MENU_HISTORY_PAGE, false),
    (MENU_SETTINGS, true),
    (MENU_BOOKMARKS, false),
    (MENU_ZOOM, true),
    (MENU_HISTORY, false),
    (MENU_DOWNLOAD_PROMPT, false),
    (MENU_CLEAR_HOUR, true),
    (MENU_CLEAR_DAY, false),
    (MENU_CLEAR_ALL, false),
    (MENU_CLEAR_SITE, false),
    (MENU_UPDATE, true),
];
const MAIN_MENU_ROW_HEIGHT: i32 = 34;
const UPDATE_MENU_ROW_HEIGHT: i32 = 66;
const MAIN_MENU_SEPARATOR_HEIGHT: i32 = 9;

struct MainMenuItem {
    command: usize,
    label: String,
    shortcut: &'static str,
    checked: bool,
}

fn main_menu_row_height(command: usize) -> i32 {
    if command == MENU_UPDATE {
        UPDATE_MENU_ROW_HEIGHT
    } else {
        MAIN_MENU_ROW_HEIGHT
    }
}

fn main_menu_height() -> i32 {
    16 + MAIN_MENU_LAYOUT
        .iter()
        .map(|(command, separator)| {
            main_menu_row_height(*command)
                + if *separator {
                    MAIN_MENU_SEPARATOR_HEIGHT
                } else {
                    0
                }
        })
        .sum::<i32>()
}

fn main_menu_items(ui: &NativeUi) -> Vec<MainMenuItem> {
    let preferences = lock_controller(&ui.controller).preferences();
    let use_english = preferences.language == storage::Language::English;
    let text = |russian, english| if use_english { english } else { russian };
    MAIN_MENU_LAYOUT
        .iter()
        .map(|(command, _)| {
            let (label, shortcut, checked) = match *command {
                MENU_PRIVATE => (
                    text("Новое приватное окно", "New private window").to_owned(),
                    "Ctrl+Shift+N",
                    false,
                ),
                MENU_RESTORE => (
                    text("Восстановить закрытую вкладку", "Reopen closed tab").to_owned(),
                    "Ctrl+Shift+T",
                    false,
                ),
                MENU_PROFILE => (text("Профиль", "Profile").to_owned(), "", false),
                MENU_HISTORY_PAGE => (text("История", "History").to_owned(), "Ctrl+H", false),
                MENU_SETTINGS => (text("Настройки", "Settings").to_owned(), "", false),
                MENU_ZOOM => (text("Масштаб", "Zoom").to_owned(), "", false),
                MENU_BOOKMARKS => (
                    text("Панель закладок", "Bookmarks bar").to_owned(),
                    "Ctrl+Shift+B",
                    preferences.show_bookmarks,
                ),
                MENU_HISTORY => (
                    text("Сохранять историю", "Save history").to_owned(),
                    "",
                    preferences.save_history,
                ),
                MENU_DOWNLOAD_PROMPT => (
                    text("Спрашивать путь загрузки", "Ask download location").to_owned(),
                    "",
                    preferences.ask_download_path,
                ),
                MENU_CLEAR_HOUR => (
                    text("Очистить историю за час", "Clear last hour").to_owned(),
                    "",
                    false,
                ),
                MENU_CLEAR_DAY => (
                    text("Очистить историю за сутки", "Clear last 24 hours").to_owned(),
                    "",
                    false,
                ),
                MENU_CLEAR_ALL => (
                    text("Очистить всю историю", "Clear all history").to_owned(),
                    "",
                    false,
                ),
                MENU_CLEAR_SITE => (
                    text(
                        "Очистить cookies, кэш и авторизацию",
                        "Clear cookies, cache and sign-ins",
                    )
                    .to_owned(),
                    "",
                    false,
                ),
                MENU_UPDATE => (
                    update_menu_label_localized(&ui.update_status, use_english),
                    "",
                    false,
                ),
                _ => unreachable!(),
            };
            MainMenuItem {
                command: *command,
                label,
                shortcut,
                checked,
            }
        })
        .collect()
}

fn update_menu_label_localized(status: &update::Status, english: bool) -> String {
    match status {
        update::Status::Idle | update::Status::UpToDate => if english {
            "Check for updates"
        } else {
            "Проверить обновления"
        }
        .to_owned(),
        update::Status::Error(_) => if english {
            "Retry update check"
        } else {
            "Повторить проверку обновлений"
        }
        .to_owned(),
        update::Status::Checking => if english {
            "Checking for updates…"
        } else {
            "Проверка обновлений…"
        }
        .to_owned(),
        update::Status::Downloading { version, .. } => {
            if english {
                format!("Downloading {version}")
            } else {
                format!("Скачивание {version}")
            }
        }
        update::Status::Verifying { version } => {
            if english {
                format!("Verifying update {version}")
            } else {
                format!("Проверка обновления {version}")
            }
        }
        update::Status::Installing { version } => {
            if english {
                format!("Installing {version}")
            } else {
                format!("Установка {version}")
            }
        }
    }
}

#[cfg(test)]
fn update_menu_detail(status: &update::Status) -> String {
    update_menu_detail_localized(status, false)
}

fn update_menu_detail_localized(status: &update::Status, english: bool) -> String {
    match status {
        update::Status::Idle => {
            if english {
                format!("Current version {}", crate::APP_VERSION)
            } else {
                format!("Текущая версия {}", crate::APP_VERSION)
            }
        }
        update::Status::Checking => if english {
            "Checking GitHub for a new release"
        } else {
            "Ищем новую версию на GitHub"
        }
        .to_owned(),
        update::Status::Downloading {
            received,
            total: Some(total),
            ..
        } if *total > 0 => format!(
            "{}% · {} {} {}",
            received.saturating_mul(100) / total,
            format_bytes_localized((*received).min(i64::MAX as u64) as i64, english),
            if english { "of" } else { "из" },
            format_bytes_localized((*total).min(i64::MAX as u64) as i64, english)
        ),
        update::Status::Downloading { received, .. } => format!(
            "{} {}",
            if english { "Received" } else { "Получено" },
            format_bytes_localized((*received).min(i64::MAX as u64) as i64, english)
        ),
        update::Status::Verifying { .. } => if english {
            "Verifying installer SHA-256"
        } else {
            "Проверяем SHA-256 установщика"
        }
        .to_owned(),
        update::Status::Installing { .. } => if english {
            "Installer started, browser will restart"
        } else {
            "Установщик запущен, браузер перезапустится"
        }
        .to_owned(),
        update::Status::UpToDate => if english {
            "Up to date"
        } else {
            "Установлена последняя версия"
        }
        .to_owned(),
        update::Status::Error(error) => {
            if english {
                format!("Error: {error}")
            } else {
                format!("Ошибка: {error}")
            }
        }
    }
}

fn update_menu_progress(status: &update::Status) -> Option<u64> {
    match status {
        update::Status::Downloading {
            received,
            total: Some(total),
            ..
        } if *total > 0 => Some((received.saturating_mul(100) / total).min(100)),
        update::Status::Verifying { .. } | update::Status::Installing { .. } => Some(100),
        _ => None,
    }
}

fn main_menu_command_at(y: i32) -> Option<usize> {
    let mut top = 8;
    for (command, separator) in MAIN_MENU_LAYOUT {
        if *separator {
            top += MAIN_MENU_SEPARATOR_HEIGHT;
        }
        let height = main_menu_row_height(*command);
        if (top..top + height).contains(&y) {
            return Some(*command);
        }
        top += height;
    }
    None
}

unsafe fn draw_main_menu(ui: &NativeUi, dc: HDC) {
    unsafe {
        let mut client = RECT::default();
        GetClientRect(ui.overlay_window, &mut client);
        FillRect(dc, &client, ui.background_brush);
        fill_round_rect(
            dc,
            &client,
            ui.palette.surface,
            16,
            Some(ui.palette.divider),
        );
        SetBkMode(dc, TRANSPARENT as i32);
        let items = main_menu_items(ui);
        let mut top = 8;
        for ((_, separator), item) in MAIN_MENU_LAYOUT.iter().zip(items) {
            if *separator {
                let line = RECT {
                    left: 14,
                    top: top + MAIN_MENU_SEPARATOR_HEIGHT / 2,
                    right: client.right - 14,
                    bottom: top + MAIN_MENU_SEPARATOR_HEIGHT / 2 + 1,
                };
                let brush = CreateSolidBrush(ui.palette.divider);
                FillRect(dc, &line, brush);
                DeleteObject(brush as HGDIOBJ);
                top += MAIN_MENU_SEPARATOR_HEIGHT;
            }
            let row = RECT {
                left: 6,
                top,
                right: client.right - 6,
                bottom: top + main_menu_row_height(item.command),
            };
            if ui.menu_hover == item.command {
                fill_round_rect(dc, &row, ui.palette.hover, 9, None);
            }
            if item.checked {
                let check = RECT {
                    left: 12,
                    top: row.top,
                    right: 36,
                    bottom: row.bottom,
                };
                draw_icon(dc, &check, Icon::Check, ui.palette.accent);
            }
            let label_bottom = if item.command == MENU_UPDATE {
                row.top + 32
            } else {
                row.bottom
            };
            let label = RECT {
                left: 38,
                top: row.top,
                right: client.right - if item.shortcut.is_empty() { 14 } else { 132 },
                bottom: label_bottom,
            };
            draw_text(
                dc,
                &item.label,
                &label,
                ui.title_font,
                ui.palette.foreground,
                DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            if !item.shortcut.is_empty() {
                let shortcut = RECT {
                    left: client.right - 130,
                    top: row.top,
                    right: client.right - 14,
                    bottom: row.bottom,
                };
                draw_text(
                    dc,
                    item.shortcut,
                    &shortcut,
                    ui.small_font,
                    ui.palette.secondary,
                    DT_RIGHT | DT_END_ELLIPSIS,
                );
            }
            if item.command == MENU_ZOOM {
                let search = RECT {
                    left: 10,
                    top: row.top,
                    right: 36,
                    bottom: row.bottom,
                };
                draw_icon(dc, &search, Icon::Search, ui.palette.secondary);
                let zoom = lock_controller(&ui.controller).active_zoom_percent();
                let controls = [
                    (client.right - 174, client.right - 142, Icon::Minus),
                    (client.right - 88, client.right - 52, Icon::NewTab),
                    (client.right - 46, client.right - 10, Icon::Fullscreen),
                ];
                for (left, right, icon) in controls {
                    let control = RECT {
                        left,
                        top: row.top + 4,
                        right,
                        bottom: row.bottom - 4,
                    };
                    draw_icon(dc, &control, icon, ui.palette.secondary);
                }
                let percent = RECT {
                    left: client.right - 140,
                    top: row.top,
                    right: client.right - 90,
                    bottom: row.bottom,
                };
                draw_text(
                    dc,
                    &format!("{zoom} %"),
                    &percent,
                    ui.small_font,
                    ui.palette.foreground,
                    DT_CENTER,
                );
            }
            if item.command == MENU_UPDATE {
                let detail = RECT {
                    left: 38,
                    top: row.top + 29,
                    right: client.right - 14,
                    bottom: row.top + 51,
                };
                draw_text(
                    dc,
                    &update_menu_detail_localized(&ui.update_status, ui.english),
                    &detail,
                    ui.small_font,
                    if matches!(&ui.update_status, update::Status::Error(_)) {
                        WARNING
                    } else {
                        ui.palette.secondary
                    },
                    DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX,
                );
                if let Some(percent) = update_menu_progress(&ui.update_status) {
                    let progress = RECT {
                        left: 38,
                        top: row.top + 55,
                        right: client.right - 14,
                        bottom: row.top + 58,
                    };
                    fill_round_rect(dc, &progress, ui.palette.divider, 2, None);
                    let mut filled = progress;
                    filled.right = filled.left
                        + ((progress.right - progress.left) as u64 * percent / 100) as i32;
                    if filled.right > filled.left {
                        fill_round_rect(dc, &filled, ui.palette.accent, 2, None);
                    }
                }
            }
            top += main_menu_row_height(item.command);
        }
    }
}

unsafe fn execute_main_menu_command(window: HWND, command: usize) {
    unsafe {
        let Some(ui) = ui_mut(window) else { return };
        let controller = ui.controller.clone();
        match command {
            MENU_PRIVATE => BrowserController::open_private_window(),
            MENU_RESTORE => BrowserController::restore_closed_tab(&controller),
            MENU_PROFILE => BrowserController::open_internal_page(
                &controller,
                ui::internal::InternalPage::Profile,
            ),
            MENU_HISTORY_PAGE => BrowserController::open_internal_page(
                &controller,
                ui::internal::InternalPage::History,
            ),
            MENU_SETTINGS => BrowserController::open_internal_page(
                &controller,
                ui::internal::InternalPage::Settings(ui::internal::Section::Sync),
            ),
            MENU_HISTORY => BrowserController::toggle_history(&controller),
            MENU_BOOKMARKS => BrowserController::toggle_bookmarks_bar(&controller),
            MENU_DOWNLOAD_PROMPT => BrowserController::toggle_download_prompt(&controller),
            MENU_ZOOM_MINUS => BrowserController::change_zoom(&controller, -1),
            MENU_ZOOM_RESET => BrowserController::change_zoom(&controller, 0),
            MENU_ZOOM_PLUS => BrowserController::change_zoom(&controller, 1),
            MENU_FULLSCREEN => toggle_fullscreen(window),
            MENU_UPDATE => update::check_now(),
            MENU_CLEAR_HOUR => BrowserController::clear_history(Some(3600)),
            MENU_CLEAR_DAY => BrowserController::clear_history(Some(86_400)),
            MENU_CLEAR_ALL => BrowserController::clear_history(None),
            MENU_CLEAR_SITE
                if MessageBoxW(
                    window,
                    wide("Удалить cookies, кэш и сохранённые авторизации для всех сайтов?")
                        .as_ptr(),
                    wide("KOCKERS — очистка данных").as_ptr(),
                    MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
                ) == IDYES =>
            {
                BrowserController::clear_site_data();
            }
            _ => {}
        }
    }
}

unsafe fn show_tab_menu(window: HWND, id: u64) {
    unsafe {
        let Some(ui) = ui_mut(window) else { return };
        let controller = ui.controller.clone();
        let english = ui.english;
        apply_menu_theme(ui.dark_theme);
        let menu = CreatePopupMenu();
        AppendMenuW(
            menu,
            MF_STRING,
            MENU_PIN,
            wide(if english {
                "Pin or unpin tab"
            } else {
                "Закрепить или открепить"
            })
            .as_ptr(),
        );
        AppendMenuW(
            menu,
            MF_STRING,
            MENU_CLOSE_TAB,
            wide(if english {
                "Close tab"
            } else {
                "Закрыть вкладку"
            })
            .as_ptr(),
        );
        let point = cursor_position();
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            point.x,
            point.y,
            0,
            window,
            ptr::null(),
        );
        DestroyMenu(menu);
        if command == MENU_PIN as i32 {
            BrowserController::toggle_pin(&controller, id);
        } else if command == MENU_CLOSE_TAB as i32 {
            close_tab(window, id);
        }
    }
}

unsafe fn show_bookmark_menu(window: HWND, index: usize) {
    unsafe {
        let Some(ui) = ui_mut(window) else { return };
        let controller = ui.controller.clone();
        let english = ui.english;
        apply_menu_theme(ui.dark_theme);
        let menu = CreatePopupMenu();
        AppendMenuW(
            menu,
            MF_STRING,
            MENU_REMOVE_BOOKMARK,
            wide(if english {
                "Remove bookmark"
            } else {
                "Удалить закладку"
            })
            .as_ptr(),
        );
        let point = cursor_position();
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            point.x,
            point.y,
            0,
            window,
            ptr::null(),
        );
        DestroyMenu(menu);
        if command == MENU_REMOVE_BOOKMARK as i32 {
            BrowserController::remove_bookmark(&controller, index);
        }
    }
}

fn cursor_position() -> POINT {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) };
    point
}

unsafe fn track_mouse_leave(window: HWND) {
    unsafe {
        let mut track = TRACKMOUSEEVENT {
            cbSize: mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: window,
            dwHoverTime: 0,
        };
        TrackMouseEvent(&mut track);
    }
}

unsafe fn create_child(
    class: &str,
    text: &str,
    style: WINDOW_STYLE,
    instance: HINSTANCE,
    parent: HWND,
    id: usize,
) -> Result<HWND, String> {
    unsafe {
        let window = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            style,
            0,
            0,
            0,
            0,
            parent,
            id as HMENU,
            instance,
            ptr::null(),
        );
        if window.is_null() {
            Err(format_windows_error(&format!(
                "Не удалось создать элемент управления {class}"
            )))
        } else {
            Ok(window)
        }
    }
}

unsafe fn create_font(height: i32, family: &str) -> HFONT {
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.into(),
            OUT_DEFAULT_PRECIS.into(),
            CLIP_DEFAULT_PRECIS.into(),
            CLEARTYPE_QUALITY.into(),
            (DEFAULT_PITCH | FF_DONTCARE).into(),
            wide(family).as_ptr(),
        )
    }
}

#[derive(Default)]
struct FrameClock {
    generation: AtomicU64,
    pending: AtomicBool,
}

impl FrameClock {
    fn start(self: &Arc<Self>, window: HWND, fallback_interval: u32) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending.store(false, Ordering::SeqCst);
        let clock = Arc::clone(self);
        let window = window as usize;
        let fallback = Duration::from_millis(fallback_interval.into());
        thread::spawn(move || {
            while clock.generation.load(Ordering::SeqCst) == generation {
                if unsafe { DwmFlush() } < 0 {
                    thread::sleep(fallback);
                }
                if clock.generation.load(Ordering::SeqCst) != generation {
                    break;
                }
                if !clock.pending.swap(true, Ordering::SeqCst) {
                    unsafe { PostMessageW(window as HWND, WM_ANIMATION_FRAME, 0, 0) };
                }
            }
        });
    }

    fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    fn frame_handled(&self) {
        self.pending.store(false, Ordering::SeqCst);
    }
}

fn animation_interval(window: HWND) -> u32 {
    unsafe {
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;
        let mut mode = DEVMODEW {
            dmSize: mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        if !monitor.is_null()
            && GetMonitorInfoW(monitor, &mut info.monitorInfo) != 0
            && EnumDisplaySettingsW(info.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) != 0
            && mode.dmDisplayFrequency > 1
        {
            return (1000.0 / mode.dmDisplayFrequency as f32)
                .round()
                .clamp(1.0, 16.0) as u32;
        }
    }
    16
}

fn mix_color(from: COLORREF, to: COLORREF, amount: f32) -> COLORREF {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |shift| {
        let start = ((from >> shift) & 0xFFu32) as f32;
        let end = ((to >> shift) & 0xFFu32) as f32;
        (start + (end - start) * amount).round() as COLORREF
    };
    channel(0) | (channel(8) << 8) | (channel(16) << 16)
}

unsafe fn adjust_for_autohide_taskbar(work: &mut RECT, monitor: &RECT) {
    if work.left != monitor.left
        || work.top != monitor.top
        || work.right != monitor.right
        || work.bottom != monitor.bottom
    {
        return;
    }
    unsafe {
        let mut state_data = APPBARDATA {
            cbSize: mem::size_of::<APPBARDATA>() as u32,
            ..Default::default()
        };
        let state = SHAppBarMessage(ABM_GETSTATE, &mut state_data);
        if (state as u32 & ABS_AUTOHIDE) == 0 {
            return;
        }
        let mut pos_data = APPBARDATA {
            cbSize: mem::size_of::<APPBARDATA>() as u32,
            ..Default::default()
        };
        if SHAppBarMessage(ABM_GETTASKBARPOS, &mut pos_data) != 0 {
            match pos_data.uEdge {
                ABE_TOP => work.top += 2,
                ABE_BOTTOM => work.bottom -= 2,
                ABE_LEFT => work.left += 2,
                ABE_RIGHT => work.right -= 2,
                _ => work.bottom -= 2,
            }
        } else {
            work.bottom -= 2;
        }
    }
}

unsafe fn apply_menu_theme(dark: bool) {
    type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
    type FlushMenuThemes = unsafe extern "system" fn();
    unsafe {
        let library = LoadLibraryW(wide("uxtheme.dll").as_ptr());
        if library.is_null() {
            return;
        }
        if let Some(function) = GetProcAddress(library, 135usize as *const u8) {
            let set_preferred: SetPreferredAppMode = mem::transmute(function);
            set_preferred(if dark { 2 } else { 3 });
        }
        if let Some(function) = GetProcAddress(library, 136usize as *const u8) {
            let flush: FlushMenuThemes = mem::transmute(function);
            flush();
        }
        FreeLibrary(library);
    }
}

fn format_windows_error(prefix: &str) -> String {
    let code = unsafe { GetLastError() };
    format!("{prefix} (код Windows {code}).")
}

fn set_window_text(window: HWND, text: &str) {
    unsafe { SetWindowTextW(window, wide(text).as_ptr()) };
}

fn window_text(window: HWND) -> String {
    unsafe {
        let length = SendMessageW(window, WM_GETTEXTLENGTH, 0, 0);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = SendMessageW(
            window,
            WM_GETTEXT,
            buffer.len(),
            buffer.as_mut_ptr() as isize,
        );
        if copied <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buffer[..copied as usize])
    }
}

fn ui_ptr(window: HWND) -> *mut NativeUi {
    unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *mut NativeUi }
}

fn ui_mut(window: HWND) -> Option<&'static mut NativeUi> {
    let pointer = ui_ptr(window);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { &mut *pointer })
    }
}

fn key_down(key: i32) -> bool {
    unsafe { (GetKeyState(key) as u16 & 0x8000) != 0 }
}

const fn low_word(value: LPARAM) -> i16 {
    (value as usize & 0xFFFF) as u16 as i16
}

const fn high_word(value: LPARAM) -> i16 {
    ((value as usize >> 16) & 0xFFFF) as u16 as i16
}

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as COLORREF | ((green as COLORREF) << 8) | ((blue as COLORREF) << 16)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_are_linear_bounded_and_finish() {
        let now = Instant::now();
        let mut transition = Transition::new(30.0);
        transition.set_target(0.0);
        transition.started = now;
        assert!(transition.step(now + ANIMATION_DURATION / 2));
        assert!((transition.value - 15.0).abs() < 0.001);
        assert!(!transition.step(now + ANIMATION_DURATION));
        assert_eq!(transition.value, 0.0);
        assert!(!transition.step(now + ANIMATION_DURATION * 2));
    }

    #[test]
    fn panel_transitions_do_not_overshoot() {
        let now = Instant::now();
        let mut panel = Transition::new(12.0);
        panel.set_target(0.0);
        panel.started = now;
        for millisecond in 0..=ANIMATION_DURATION.as_millis() as u64 {
            panel.step(now + Duration::from_millis(millisecond));
            assert!((0.0..=12.0).contains(&panel.value));
        }
        assert_eq!(panel.value, 0.0);
        assert!(!panel.step(now + ANIMATION_DURATION));

        panel.set_target(12.0);
        panel.started = now + ANIMATION_DURATION;
        assert!(!panel.step(now + ANIMATION_DURATION * 2));
        assert_eq!(panel.value, 12.0);
    }

    #[test]
    fn menu_hit_testing_skips_separators() {
        assert_eq!(main_menu_command_at(8), Some(MENU_PRIVATE));
        assert_eq!(main_menu_command_at(145), None);
        assert_eq!(main_menu_command_at(190), Some(MENU_BOOKMARKS));
        assert_eq!(
            main_menu_command_at(main_menu_height() - 9),
            Some(MENU_UPDATE)
        );
        assert_eq!(main_menu_command_at(main_menu_height()), None);
    }

    #[test]
    fn update_progress_uses_the_actual_download_size() {
        let downloading = update::Status::Downloading {
            version: "0.2.3.1".to_owned(),
            received: 25,
            total: Some(200),
        };
        assert_eq!(update_menu_progress(&downloading), Some(12));
        assert!(update_menu_detail(&downloading).starts_with("12%"));
        assert_eq!(
            update_menu_progress(&update::Status::Installing {
                version: "0.2.3.1".to_owned()
            }),
            Some(100)
        );
        assert_eq!(
            update_menu_label_localized(&update::Status::Idle, true),
            "Check for updates"
        );
        assert!(update_menu_detail_localized(&downloading, true).contains("of"));
        assert!(format_bytes_localized(2048, true).ends_with("KB"));
    }
}
