#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

#[cfg(target_os = "windows")]
fn main() {
    kockers_browser::platform::show_direct_launch_error();
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("Kockers Browser MVP currently supports Windows only.");
}
