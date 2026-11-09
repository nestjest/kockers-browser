#[cfg(target_os = "windows")]
fn main() {
    let mut release = false;
    let mut output = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--release" => release = true,
            "-o" | "--output" => {
                let Some(path) = arguments.next() else {
                    eprintln!("Missing path after {argument}.");
                    std::process::exit(2);
                };
                output = Some(path);
            }
            "-h" | "--help" => {
                println!("Usage: kockers-bundle [--release] [-o OUTPUT]");
                return;
            }
            _ => {
                eprintln!("Unknown argument: {argument}");
                std::process::exit(2);
            }
        }
    }

    let output = output.unwrap_or_else(|| "target/bundle".to_owned());
    match cef::build_util::win::build_bundle(
        std::path::Path::new(&output),
        "kockers_browser",
        release,
    ) {
        Ok(path) => println!("Run the app from {}", path.display()),
        Err(error) => {
            eprintln!("Failed to bundle Kockers Browser: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("The kockers-bundle MVP utility currently supports Windows only.");
}
