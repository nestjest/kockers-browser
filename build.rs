#[cfg(target_os = "windows")]
fn main() {
    if let Err(error) = winres::WindowsResource::new()
        .set_icon_with_id("assets/app.ico", "1")
        .set_icon_with_id("assets/logo-mark.ico", "2")
        .set("FileDescription", "Kockers Browser")
        .set("ProductName", "KOCKERS SOCIETY")
        .compile()
    {
        panic!("failed to embed Windows resources: {error}");
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {}
