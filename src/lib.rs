#[cfg(target_os = "windows")]
mod app;
#[cfg(target_os = "windows")]
mod browser;
#[cfg(target_os = "windows")]
mod navigation;
#[cfg(target_os = "windows")]
pub mod platform;
#[cfg(target_os = "windows")]
mod storage;
#[cfg(target_os = "windows")]
mod ui;

/// # Safety
/// Вызывается только загрузчиком CEF с действительными WinMain- и sandbox-аргументами.
#[cfg(target_os = "windows")]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RunWinMain(
    instance: cef::sys::HINSTANCE,
    _command_line: *const u8,
    _command_show: i32,
    sandbox_info: *mut u8,
) -> i32 {
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    let main_args = cef::MainArgs { instance };
    let args = cef::args::Args::from(main_args);
    let Some(command_line) = args.as_cmd_line() else {
        platform::show_fatal_error("Не удалось разобрать аргументы командной строки CEF.");
        return 1;
    };

    app::run(args.as_main_args(), &command_line, sandbox_info)
}
