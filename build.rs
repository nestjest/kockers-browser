#[cfg(target_os = "windows")]
fn main() {
    if let Err(error) = winres::WindowsResource::new()
        .set_icon_with_id("assets/app.ico", "1")
        .set_icon_with_id("assets/tab.ico", "2")
        .set_icon_with_id("assets/tab-light.ico", "3")
        .set("FileDescription", "Kockers Browser")
        .set("FileVersion", "0.2.5.0")
        .set("ProductName", "KOCKERS SOCIETY")
        .set("ProductVersion", "0.2.5.0")
        .compile()
    {
        panic!("failed to embed Windows resources: {error}");
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {}
