use crate::browser::{BrowserController, SharedController, lock_controller};
use crate::ui::pages;
use std::{iter, mem, ptr};
use windows_sys::Win32::{
    Foundation::{COLORREF, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::{
        LibraryLoader::GetModuleHandleW,
        SystemServices::{SS_CENTER, SS_CENTERIMAGE, SS_ICON},
    },
    UI::{
        Controls::{
            DRAWITEMSTRUCT, EM_SETCUEBANNER, EM_SETSEL, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED,
            ODT_BUTTON,
        },
        Input::KeyboardAndMouse::{
            EnableWindow, GetKeyState, SetFocus, VK_CONTROL, VK_ESCAPE, VK_F5, VK_F12, VK_LEFT,
            VK_MENU, VK_RIGHT,
        },
        WindowsAndMessaging::*,
    },
};

pub const TOOLBAR_HEIGHT: i32 = 60;
const INITIAL_WIDTH: i32 = 1200;
const INITIAL_HEIGHT: i32 = 800;
const MIN_WIDTH: i32 = 800;
const MIN_HEIGHT: i32 = 600;
const BACKGROUND: COLORREF = rgb(0xF7, 0xF6, 0xF3);
const FOREGROUND: COLORREF = rgb(0x02, 0x02, 0x02);
const SECONDARY: COLORREF = rgb(0x8D, 0x8D, 0x89);
const DIVIDER: COLORREF = rgb(0xE3, 0xE1, 0xDC);
const ID_BACK: usize = 100;
const ID_FORWARD: usize = 101;
const ID_RELOAD: usize = 102;
const ID_ADDRESS: usize = 103;
const ID_HOME: usize = 104;
const ID_LOADING: usize = 105;
const ICON_ID: usize = 1;

struct NativeUi {
    controller: SharedController,
    window: HWND,
    address: HWND,
    back: HWND,
    forward: HWND,
    reload: HWND,
    home: HWND,
    loading: HWND,
    logo: HWND,
    background_brush: HBRUSH,
    divider_brush: HBRUSH,
    active_brush: HBRUSH,
    font: HFONT,
    icon_font: HFONT,
    logo_icon: HICON,
    current_url: String,
    address_editing: bool,
    is_loading: bool,
}

impl NativeUi {
    fn new(controller: SharedController) -> Self {
        Self {
            controller,
            window: ptr::null_mut(),
            address: ptr::null_mut(),
            back: ptr::null_mut(),
            forward: ptr::null_mut(),
            reload: ptr::null_mut(),
            home: ptr::null_mut(),
            loading: ptr::null_mut(),
            logo: ptr::null_mut(),
            background_brush: ptr::null_mut(),
            divider_brush: ptr::null_mut(),
            active_brush: ptr::null_mut(),
            font: ptr::null_mut(),
            icon_font: ptr::null_mut(),
            logo_icon: ptr::null_mut(),
            current_url: String::new(),
            address_editing: false,
            is_loading: false,
        }
    }

    unsafe fn create_controls(&mut self, instance: HINSTANCE) -> Result<(), String> {
        unsafe {
            self.background_brush = CreateSolidBrush(BACKGROUND);
            self.divider_brush = CreateSolidBrush(DIVIDER);
            self.active_brush = CreateSolidBrush(FOREGROUND);
            self.font = create_font(16, "Segoe UI");
            self.icon_font = create_font(18, "Segoe UI Symbol");
            if self.background_brush.is_null()
                || self.divider_brush.is_null()
                || self.active_brush.is_null()
                || self.font.is_null()
                || self.icon_font.is_null()
            {
                return Err("Не удалось создать графические ресурсы панели.".to_owned());
            }

            self.logo = create_child(
                "STATIC",
                "",
                WS_CHILD | WS_VISIBLE | SS_ICON | SS_CENTERIMAGE,
                instance,
                self.window,
                0,
            )?;
            self.back = self.create_button("←", ID_BACK, instance)?;
            self.forward = self.create_button("→", ID_FORWARD, instance)?;
            self.reload = self.create_button("↻", ID_RELOAD, instance)?;
            self.address = create_child(
                "EDIT",
                "",
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
                instance,
                self.window,
                ID_ADDRESS,
            )?;
            self.loading = create_child(
                "STATIC",
                "",
                WS_CHILD | WS_VISIBLE | SS_CENTER,
                instance,
                self.window,
                ID_LOADING,
            )?;
            self.home = self.create_button("＋", ID_HOME, instance)?;

            for control in [self.back, self.forward, self.reload, self.home] {
                SendMessageW(control, WM_SETFONT, self.icon_font as usize, 1);
                subclass_control(control);
            }
            SendMessageW(self.address, WM_SETFONT, self.font as usize, 1);
            SendMessageW(self.loading, WM_SETFONT, self.font as usize, 1);
            SendMessageW(
                self.address,
                EM_SETCUEBANNER,
                0,
                wide("URL или поиск").as_ptr() as isize,
            );
            subclass_control(self.address);

            self.logo_icon = LoadImageW(
                instance,
                ICON_ID as *const u16,
                IMAGE_ICON,
                28,
                32,
                LR_DEFAULTCOLOR,
            );
            if self.logo_icon.is_null() {
                return Err("Не удалось загрузить встроенный логотип K.".to_owned());
            }
            SendMessageW(self.logo, STM_SETICON, self.logo_icon as usize, 0);
            EnableWindow(self.back, 0);
            EnableWindow(self.forward, 0);
            self.layout();
        }
        Ok(())
    }

    unsafe fn create_button(
        &self,
        text: &str,
        id: usize,
        instance: HINSTANCE,
    ) -> Result<HWND, String> {
        unsafe {
            create_child(
                "BUTTON",
                text,
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
                instance,
                self.window,
                id,
            )
        }
    }

    fn layout(&self) {
        if self.address.is_null() {
            return;
        }
        let (width, _) = unsafe { client_size(self.window) };
        let button_y = 12;
        let button_size = 36;
        unsafe {
            MoveWindow(self.logo, 16, 14, 28, 32, 1);
            MoveWindow(self.back, 58, button_y, button_size, button_size, 1);
            MoveWindow(self.forward, 98, button_y, button_size, button_size, 1);
            MoveWindow(self.reload, 138, button_y, button_size, button_size, 1);
            let home_x = (width - 50).max(184);
            let loading_x = (home_x - 32).max(152);
            let address_right = (loading_x - 10).max(184);
            MoveWindow(
                self.address,
                184,
                button_y,
                (address_right - 184).max(0),
                button_size,
                1,
            );
            MoveWindow(self.loading, loading_x, button_y, 24, button_size, 1);
            MoveWindow(self.home, home_x, button_y, button_size, button_size, 1);
            lock_controller(&self.controller).resize_content();
        }
    }

    fn update_address(&mut self, url: &str) {
        if url == pages::start_page_url() {
            self.current_url.clear();
            self.address_editing = false;
            set_window_text(self.address, "");
        } else if !url.starts_with("data:") {
            self.current_url = url.to_owned();
            set_window_text(self.address, url);
            self.address_editing = false;
        }
    }

    fn update_loading_state(&mut self, loading: bool, back: bool, forward: bool) {
        self.is_loading = loading;
        unsafe {
            EnableWindow(self.back, i32::from(back));
            EnableWindow(self.forward, i32::from(forward));
            set_window_text(self.reload, if loading { "×" } else { "↻" });
            set_window_text(self.loading, if loading { "●" } else { "" });
            InvalidateRect(self.reload, ptr::null(), 1);
            InvalidateRect(self.loading, ptr::null(), 1);
        }
    }

    fn submit_address(&mut self) {
        let input = window_text(self.address);
        self.address_editing = false;
        BrowserController::navigate_input(&self.controller, &input);
    }

    fn cancel_address_editing(&mut self) {
        let current_url = self.current_url.clone();
        self.address_editing = false;
        set_window_text(self.address, &current_url);
        BrowserController::stop_loading(&self.controller);
    }

    unsafe fn destroy_resources(&mut self) {
        unsafe {
            if !self.logo_icon.is_null() {
                DestroyIcon(self.logo_icon);
                self.logo_icon = ptr::null_mut();
            }
            for object in [
                self.background_brush as HGDIOBJ,
                self.divider_brush as HGDIOBJ,
                self.active_brush as HGDIOBJ,
                self.font as HGDIOBJ,
                self.icon_font as HGDIOBJ,
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
        let icon = LoadImageW(
            instance,
            ICON_ID as *const u16,
            IMAGE_ICON,
            32,
            32,
            LR_DEFAULTCOLOR,
        );
        let small_icon = LoadImageW(
            instance,
            ICON_ID as *const u16,
            IMAGE_ICON,
            16,
            16,
            LR_DEFAULTCOLOR,
        );
        let class_name = wide("KockersBrowserWindow");
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: icon,
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        if RegisterClassW(&class) == 0 {
            return Err(format_windows_error(
                "Не удалось зарегистрировать класс окна",
            ));
        }

        let ui = Box::new(NativeUi::new(controller));
        let ui_ptr = Box::into_raw(ui);
        let window = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("KOCKERS SOCIETY").as_ptr(),
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
        if let Err(error) = ui.create_controls(instance) {
            DestroyWindow(window);
            return Err(error);
        }
        if !small_icon.is_null() {
            SendMessageW(window, WM_SETICON, ICON_SMALL as usize, small_icon as isize);
        }
        ShowWindow(window, SW_SHOW);
        UpdateWindow(window);
        Ok(window)
    }
}

/// # Safety
/// `window` должен быть действительным HWND, принадлежащим текущему UI-потоку.
pub unsafe fn client_size(window: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    unsafe {
        if GetClientRect(window, &mut rect) == 0 {
            return (INITIAL_WIDTH, INITIAL_HEIGHT - TOOLBAR_HEIGHT);
        }
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

/// # Safety
/// Оба HWND должны быть действительными и принадлежать текущему UI-потоку.
pub unsafe fn resize_browser(_window: HWND, browser: Option<HWND>) {
    if let Some(browser) = browser {
        let (width, height) = unsafe { client_size(_window) };
        resize_browser_handle(browser, width, height);
    }
}

fn resize_browser_handle(browser: HWND, width: i32, height: i32) {
    unsafe {
        MoveWindow(
            browser,
            0,
            TOOLBAR_HEIGHT,
            width,
            (height - TOOLBAR_HEIGHT).max(0),
            1,
        );
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

pub fn update_loading_state(window: HWND, loading: bool, back: bool, forward: bool) {
    if let Some(ui) = ui_mut(window) {
        ui.update_loading_state(loading, back, forward);
    }
}

/// # Safety
/// `window` должен быть HWND главного окна, принадлежащим текущему UI-потоку.
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
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(window, &mut paint);
                let mut client = RECT::default();
                GetClientRect(window, &mut client);
                if let Some(ui) = ui_mut(window) {
                    FillRect(dc, &client, ui.background_brush);
                    let divider = RECT {
                        left: 0,
                        top: TOOLBAR_HEIGHT - 1,
                        right: client.right,
                        bottom: TOOLBAR_HEIGHT,
                    };
                    FillRect(dc, &divider, ui.divider_brush);
                }
                EndPaint(window, &paint);
                return 0;
            }
            WM_SIZE => {
                if let Some(ui) = ui_mut(window) {
                    ui.layout();
                }
                return 0;
            }
            WM_GETMINMAXINFO => {
                let info = &mut *(lparam as *mut MINMAXINFO);
                info.ptMinTrackSize.x = MIN_WIDTH;
                info.ptMinTrackSize.y = MIN_HEIGHT;
                return 0;
            }
            WM_COMMAND => {
                let id = wparam & 0xFFFF;
                let notification = (wparam >> 16) & 0xFFFF;
                if id == ID_ADDRESS {
                    if notification == EN_CHANGE as usize {
                        if let Some(ui) = ui_mut(window) {
                            ui.address_editing = true;
                        }
                    } else if notification == EN_KILLFOCUS as usize {
                        finish_address_editing(window);
                    }
                    return 0;
                }
                if notification == BN_CLICKED as usize {
                    let Some(ui) = ui_mut(window) else { return 0 };
                    let controller = ui.controller.clone();
                    match id {
                        ID_BACK => BrowserController::go_back(&controller),
                        ID_FORWARD => BrowserController::go_forward(&controller),
                        ID_RELOAD => BrowserController::reload_or_stop(&controller),
                        ID_HOME => BrowserController::go_home(&controller),
                        _ => return DefWindowProcW(window, message, wparam, lparam),
                    }
                    return 0;
                }
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                if let Some(ui) = ui_mut(window) {
                    SetBkColor(wparam as HDC, BACKGROUND);
                    SetTextColor(wparam as HDC, FOREGROUND);
                    return ui.background_brush as LRESULT;
                }
            }
            WM_CTLCOLOREDIT => {
                if let Some(ui) = ui_mut(window) {
                    SetBkColor(wparam as HDC, BACKGROUND);
                    SetTextColor(wparam as HDC, FOREGROUND);
                    return ui.background_brush as LRESULT;
                }
            }
            WM_DRAWITEM => {
                let draw = &*(lparam as *const DRAWITEMSTRUCT);
                if draw.CtlType == ODT_BUTTON {
                    draw_button(draw);
                    return 1;
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
            WM_DESTROY => {
                if ui_mut(window).is_some_and(|ui| lock_controller(&ui.controller).is_empty()) {
                    cef::quit_message_loop();
                }
                return 0;
            }
            WM_NCDESTROY => {
                let ui_pointer = ui_ptr(window);
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                if !ui_pointer.is_null() {
                    let mut ui = Box::from_raw(ui_pointer);
                    ui.destroy_resources();
                }
                return DefWindowProcW(window, message, wparam, lparam);
            }
            _ => {}
        }
        DefWindowProcW(window, message, wparam, lparam)
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
        if message == WM_NCDESTROY {
            let previous = GetWindowLongPtrW(control, GWLP_USERDATA);
            SetWindowLongPtrW(control, GWLP_WNDPROC, previous);
        }
        let previous = GetWindowLongPtrW(control, GWLP_USERDATA);
        if previous == 0 {
            return DefWindowProcW(control, message, wparam, lparam);
        }
        let previous_proc: WNDPROC = mem::transmute::<isize, WNDPROC>(previous);
        match previous_proc {
            Some(previous_proc) => {
                CallWindowProcW(Some(previous_proc), control, message, wparam, lparam)
            }
            None => DefWindowProcW(control, message, wparam, lparam),
        }
    }
}

fn handle_native_shortcut(window: HWND, key: i32) -> bool {
    let control = key_down(VK_CONTROL as i32);
    let alt = key_down(VK_MENU as i32);
    let Some(ui) = ui_mut(window) else {
        return false;
    };
    let controller = ui.controller.clone();
    match key {
        0x4C if control => unsafe { focus_address(window) },
        0x52 if control => BrowserController::reload(&controller),
        value if value == VK_F5 as i32 => BrowserController::reload(&controller),
        value if value == VK_LEFT as i32 && alt => BrowserController::go_back(&controller),
        value if value == VK_RIGHT as i32 && alt => BrowserController::go_forward(&controller),
        value if value == VK_F12 as i32 => BrowserController::show_dev_tools(&controller),
        value if value == VK_ESCAPE as i32 => BrowserController::stop_loading(&controller),
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

unsafe fn draw_button(draw: &DRAWITEMSTRUCT) {
    unsafe {
        let selected = draw.itemState & ODS_SELECTED != 0;
        let enabled = draw.itemState & ODS_DISABLED == 0;
        let background = if selected { FOREGROUND } else { BACKGROUND };
        let foreground = if !enabled {
            SECONDARY
        } else if selected {
            BACKGROUND
        } else {
            FOREGROUND
        };
        let brush = CreateSolidBrush(background);
        FillRect(draw.hDC, &draw.rcItem, brush);
        DeleteObject(brush as HGDIOBJ);
        SetBkColor(draw.hDC, background);
        SetTextColor(draw.hDC, foreground);
        let text = window_text(draw.hwndItem);
        let mut text = wide(&text);
        DrawTextW(
            draw.hDC,
            text.as_mut_ptr(),
            text.len() as i32 - 1,
            &mut draw.rcItem.clone(),
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        if draw.itemState & ODS_FOCUS != 0 && !selected {
            let mut focus_rect = draw.rcItem;
            focus_rect.left += 4;
            focus_rect.top += 4;
            focus_rect.right -= 4;
            focus_rect.bottom -= 4;
            DrawFocusRect(draw.hDC, &focus_rect);
        }
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

fn format_windows_error(prefix: &str) -> String {
    let code = unsafe { GetLastError() };
    format!("{prefix} (код Windows {code}).")
}

fn set_window_text(window: HWND, text: &str) {
    unsafe {
        SetWindowTextW(window, wide(text).as_ptr());
    }
}

fn window_text(window: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(window);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32);
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

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as COLORREF | ((green as COLORREF) << 8) | ((blue as COLORREF) << 16)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(iter::once(0)).collect()
}
